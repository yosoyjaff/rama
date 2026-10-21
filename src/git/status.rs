//! The working tree: what `git status` says, and staging, unstaging,
//! discarding and committing on top of it.
//!
//! This reads `git status` and not a diff, because only the status tells
//! staged from unstaged changes and names the untracked files.

use std::path::Path;

use anyhow::{Result, bail};

use super::command::{git, git_bytes};
use super::types::{GitStatus, GitStatusEntry, conflict_kind};

/// The status of the checkout, with the lines each path adds and removes.
pub fn status(root: &Path) -> Result<GitStatus> {
    let records = git_bytes(
        root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ],
    )?;
    let mut status = parse_status(&records);
    // The status has no line counts, so they come from a second read. Before
    // the first commit there is no HEAD to diff with, and the counts stay 0.
    if let Ok(counts) = git_bytes(root, &["diff", "HEAD", "--numstat", "-z"]) {
        apply_numstat(&mut status, &counts);
    }
    Ok(status)
}

/// Run `git <command> <paths…>`, unless there are no paths.
fn on_paths(root: &Path, command: &[&str], paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let args: Vec<&str> = command
        .iter()
        .copied()
        .chain(paths.iter().map(String::as_str))
        .collect();
    git(root, &args).map(drop)
}

/// Stage these paths, deletions and new files included.
pub fn stage(root: &Path, paths: &[String]) -> Result<()> {
    on_paths(root, &["add", "--all", "--"], paths)
}

/// Take these paths out of the index and leave the files alone. Before the
/// first commit there is no HEAD to restore from, so `rm --cached` does it.
pub fn unstage(root: &Path, paths: &[String]) -> Result<()> {
    on_paths(root, &["restore", "--staged", "--"], paths)
        .or_else(|_| on_paths(root, &["rm", "--cached", "-r", "--quiet", "--"], paths))
}

/// Throw away the changes to exactly these paths, and nothing else: tracked
/// ones go back to HEAD in the index and on disk, untracked ones are deleted.
/// It cannot be undone, which is why it only ever acts on what it is given.
pub fn discard(root: &Path, tracked: &[String], untracked: &[String]) -> Result<()> {
    on_paths(
        root,
        &["restore", "--staged", "--worktree", "--source=HEAD", "--"],
        tracked,
    )?;
    on_paths(root, &["clean", "-fd", "--"], untracked)
}

/// Commit the index, or amend HEAD with it, and return the new commit.
pub fn commit(root: &Path, message: &str, amend: bool) -> Result<String> {
    if !amend && message.trim().is_empty() {
        bail!("commit message is empty");
    }
    let mut args = vec!["commit", "--message", message];
    if amend {
        args.push("--amend");
    }
    git(root, &args)?;
    git(root, &["rev-parse", "HEAD"])
}

fn records(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .map(|record| String::from_utf8_lossy(record).into_owned())
        .collect()
}

/// The path at the end of a status record, after `skip` fields.
fn tail_field(record: &str, skip: usize) -> Option<&str> {
    record
        .splitn(skip + 1, ' ')
        .nth(skip)
        .filter(|path| !path.is_empty())
}

fn entry(path: &str, xy: &str, old_path: Option<String>) -> GitStatusEntry {
    let mut letters = xy.chars().map(String::from);
    GitStatusEntry {
        path: path.to_string(),
        old_path,
        index: letters.next().unwrap_or_else(|| ".".into()),
        worktree: letters.next().unwrap_or_else(|| ".".into()),
        ..GitStatusEntry::default()
    }
}

fn read_branch_header(status: &mut GitStatus, header: &str) {
    let Some((key, value)) = header.split_once(' ') else {
        return;
    };
    match key {
        // "(detached)" is a state, not a branch name.
        "branch.head" if value != "(detached)" => status.branch = Some(value.to_string()),
        "branch.upstream" => status.upstream = Some(value.to_string()),
        "branch.ab" => {
            for count in value.split_whitespace() {
                let number = count.get(1..).and_then(|n| n.parse().ok());
                match (count.chars().next(), number) {
                    (Some('+'), Some(n)) => status.ahead = n,
                    (Some('-'), Some(n)) => status.behind = n,
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Read `git status --porcelain=v2 --branch -z`. Every record ends in a NUL,
/// and a rename takes two: its own and then the path it was renamed from.
pub fn parse_status(bytes: &[u8]) -> GitStatus {
    let mut status = GitStatus::default();
    let mut records = records(bytes).into_iter();
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("# ") {
            read_branch_header(&mut status, header);
            continue;
        }
        let xy = record.get(2..4).unwrap_or("..");
        // The fields of each kind of record, after the kind and the letters:
        //   1 XY sub mH mI mW hH hI path
        //   2 XY sub mH mI mW hH hI score path, then the old path
        //   u XY sub m1 m2 m3 mW h1 h2 h3 path
        let found = match record.split(' ').next() {
            Some("1") => tail_field(&record, 8).map(|path| entry(path, xy, None)),
            Some("2") => {
                let old = records.next().filter(|old| !old.is_empty());
                tail_field(&record, 9).map(|path| entry(path, xy, old))
            }
            Some("u") => tail_field(&record, 10).map(|path| GitStatusEntry {
                conflicted: true,
                conflict_kind: conflict_kind(xy),
                ..entry(path, xy, None)
            }),
            Some("?") => tail_field(&record, 1).map(|path| GitStatusEntry {
                untracked: true,
                ..entry(path, "??", None)
            }),
            // `!` (ignored) is only listed on request, and the pane has no
            // use for it.
            _ => None,
        };
        status.entries.extend(found);
    }
    status.entries.sort_by(|a, b| a.path.cmp(&b.path));
    status
}

/// Add the counts from `git diff HEAD --numstat -z` to the entries. A rename
/// has no path of its own in its record: the two paths follow it, old first.
/// A binary file counts `-`, which reads as 0.
pub fn apply_numstat(status: &mut GitStatus, bytes: &[u8]) {
    let mut records = records(bytes).into_iter();
    while let Some(record) = records.next() {
        let mut fields = record.splitn(3, '\t');
        let (Some(added), Some(removed)) = (fields.next(), fields.next()) else {
            continue;
        };
        let path = match fields.next() {
            Some(path) if !path.is_empty() => path.to_string(),
            _ => {
                records.next();
                records.next().unwrap_or_default()
            }
        };
        if let Some(entry) = status.entries.iter_mut().find(|e| e.path == path) {
            entry.additions = added.parse().unwrap_or(0);
            entry.deletions = removed.parse().unwrap_or(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    fn nul_separated(records: &[&str]) -> Vec<u8> {
        records.iter().flat_map(|r| r.bytes().chain([0])).collect()
    }

    fn strings(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn the_branch_header_gives_upstream_and_counts() {
        let status = parse_status(&nul_separated(&[
            "# branch.oid 0f0f0f",
            "# branch.head topic/x",
            "# branch.upstream origin/topic/x",
            "# branch.ab +4 -1",
        ]));
        assert_eq!(status.branch.as_deref(), Some("topic/x"));
        assert_eq!(status.upstream.as_deref(), Some("origin/topic/x"));
        assert_eq!((status.ahead, status.behind), (4, 1));
        let detached = parse_status(&nul_separated(&["# branch.head (detached)"]));
        assert_eq!(detached.branch, None);
    }

    #[test]
    fn every_kind_of_record_is_read_and_a_rename_keeps_its_old_path() {
        let status = parse_status(&nul_separated(&[
            "2 R. N... 100644 100644 100644 aaa bbb R100 lib/new name.rs",
            "lib/old.rs",
            "1 .M N... 100644 100644 100644 ccc ddd a b.txt",
            "u UU N... 100644 100644 100644 100644 e1 e2 e3 clash.rs",
            "? fresh.txt",
            "! ignored.log",
        ]));
        let paths: Vec<&str> = status.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            ["a b.txt", "clash.rs", "fresh.txt", "lib/new name.rs"]
        );
        let [edited, clash, fresh, renamed] = &status.entries[..] else {
            unreachable!()
        };
        assert!(edited.is_unstaged() && !edited.is_staged());
        assert!(clash.conflicted);
        assert_eq!(
            clash.conflict_kind,
            Some(crate::git::types::ConflictKind::BothModified)
        );
        assert!(fresh.untracked && !fresh.is_staged() && !fresh.is_unstaged());
        assert_eq!(renamed.old_path.as_deref(), Some("lib/old.rs"));
        assert!(renamed.is_staged());
    }

    #[test]
    fn line_counts_land_on_their_paths_renames_included() {
        let mut status = parse_status(&nul_separated(&[
            "1 .M N... 100644 100644 100644 a b x.rs",
            "2 R. N... 100644 100644 100644 a b R90 y.rs",
            "x_old.rs",
            "1 .M N... 100644 100644 100644 a b logo.png",
        ]));
        apply_numstat(
            &mut status,
            &nul_separated(&["3\t1\tx.rs", "2\t2\t", "x_old.rs", "y.rs", "-\t-\tlogo.png"]),
        );
        let counts: Vec<(u32, u32)> = status
            .entries
            .iter()
            .map(|e| (e.additions, e.deletions))
            .collect();
        assert_eq!(counts, [(0, 0), (3, 1), (2, 2)]);
    }

    #[test]
    fn a_real_checkout_separates_staged_unstaged_and_untracked() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.write("staged.txt", "s\n");
        repo.git(&["add", "staged.txt"]);
        repo.write("log.txt", "edited\nmore\n");
        repo.write("new.txt", "n\n");

        let status = status(repo.path()).unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        let find = |path: &str| status.entries.iter().find(|e| e.path == path).unwrap();
        assert!(find("staged.txt").is_staged());
        assert!(find("log.txt").is_unstaged());
        assert_eq!(find("log.txt").additions, 2);
        assert!(find("new.txt").untracked);
    }

    #[test]
    fn stage_unstage_and_commit() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.write("a.txt", "a\n");
        repo.write("b.txt", "b\n");
        stage(repo.path(), &strings(&["a.txt", "b.txt"])).unwrap();
        unstage(repo.path(), &strings(&["b.txt"])).unwrap();
        let entries = status(repo.path()).unwrap().entries;
        assert!(entries.iter().any(|e| e.path == "a.txt" && e.is_staged()));
        assert!(entries.iter().any(|e| e.path == "b.txt" && e.untracked));

        let sha = commit(repo.path(), "add a", false).unwrap();
        assert_eq!(sha, repo.git(&["rev-parse", "HEAD"]));
        assert_eq!(repo.git(&["log", "-1", "--format=%s"]), "add a");
        assert!(commit(repo.path(), "  ", false).is_err());
        assert!(stage(repo.path(), &[]).is_ok(), "nothing to do is fine");
    }

    #[test]
    fn unstaging_works_before_the_first_commit() {
        let repo = TestRepo::new();
        repo.write("a.txt", "a\n");
        stage(repo.path(), &strings(&["a.txt"])).unwrap();
        unstage(repo.path(), &strings(&["a.txt"])).unwrap();
        assert!(status(repo.path()).unwrap().entries[0].untracked);
    }

    #[test]
    fn discard_touches_only_what_it_is_given() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.write("log.txt", "edit\n");
        repo.write("keep.txt", "k\n");
        repo.write("drop.txt", "d\n");
        discard(repo.path(), &strings(&["log.txt"]), &strings(&["drop.txt"])).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("log.txt")).unwrap(),
            "first\n"
        );
        assert!(repo.path().join("keep.txt").exists());
        assert!(!repo.path().join("drop.txt").exists());
    }
}
