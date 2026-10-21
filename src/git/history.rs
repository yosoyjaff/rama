//! Reading the history the graph shows, a page at a time.
//!
//! Only branches, remote branches and tags (and HEAD) start the walk: git's
//! internal refs never show up, and never keep a commit alive in the graph
//! that nothing visible reaches.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::command::git;
use super::types::{Commit, GitRemote, HistoryPage, RefBadge, RefKind, RepoState, Signature};

/// The most a page holds, whatever is asked.
const MAX_PAGE_ROWS: usize = 200;
/// How much of a message body comes along with its commit, in characters.
const BODY_LIMIT: usize = 2_048;

/// The fields `git log` writes per commit, each followed by a NUL, which no
/// commit message can contain.
const LOG_FIELDS: [&str; 7] = ["%H", "%P", "%s", "%an", "%ae", "%aI", "%b"];
/// Per ref: its name, what it points at and that object's type, the commit a
/// tag peels to and its type, and the target of a symbolic ref.
const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)%00%(symref)%00";

/// The visible refs: every branch, remote branch and tag.
const SEEDS: [&str; 3] = ["--branches", "--remotes", "--tags"];

fn log_format() -> String {
    let fields: Vec<String> = LOG_FIELDS.iter().map(|f| format!("{f}%x00")).collect();
    format!("--format={}", fields.concat())
}

/// `git log` over HEAD and `seeds`, with `extra` options.
fn log(repo: &Path, head: bool, seeds: &[&str], extra: &[&str]) -> Result<String> {
    let format = log_format();
    let mut args = vec![
        "log",
        "--no-color",
        "--no-decorate",
        "--no-show-signature",
        "--no-patch",
    ];
    args.extend_from_slice(extra);
    args.push(&format);
    if head {
        args.push("HEAD");
    }
    args.extend_from_slice(seeds);
    git(repo, &args)
}

/// The page of history `limit` rows long that starts `cursor` rows in.
///
/// Commits come in date order: a commit always comes before its parents,
/// and among those ready, the newest goes first, so parallel branches
/// interleave by when the work was done. The first page also carries the
/// branch tips, the counts, the comparison with the main branch, and the
/// checkout's own details.
pub fn history(repo: &Path, cursor: usize, limit: usize) -> Result<HistoryPage> {
    let limit = limit.clamp(1, MAX_PAGE_ROWS);
    let head_sha = git(repo, &["rev-parse", "--verify", "HEAD^{commit}"])
        .ok()
        .filter(|sha| !sha.is_empty());
    let has_head = head_sha.is_some();
    let refs = read_refs(&git(
        repo,
        &[
            "for-each-ref",
            REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?);
    let first = cursor == 0;

    if !has_head && refs.is_empty() {
        return Ok(HistoryPage {
            state: Some(read_repo(repo)),
            ..HistoryPage::default()
        });
    }

    // One row more than asked for says whether another page exists.
    let window = [
        format!("--skip={cursor}"),
        format!("--max-count={}", limit + 1),
    ];
    let mut commits = read_commits(
        &log(
            repo,
            has_head,
            &SEEDS,
            &["--date-order", &window[0], &window[1]],
        )?,
        &refs,
    );
    let more = commits.len() > limit;
    commits.truncate(limit);

    // The tips list ignores paging: one row per distinct commit at the end of
    // a branch. Tags are left out, or every tag would add an overview row.
    let tips = match first {
        true => read_commits(
            &log(
                repo,
                has_head,
                &["--branches", "--remotes"],
                &["--no-walk=sorted"],
            )?,
            &refs,
        ),
        false => Vec::new(),
    };
    Ok(HistoryPage {
        next: more.then_some(cursor + commits.len()),
        commits,
        tips,
        state: first.then(|| read_repo(repo)),
        head_sha,
    })
}

/// The checkout's branch, remote and stash depth, and any operation under
/// way. Each part is optional: no remote, no commits or a detached HEAD are
/// ordinary states, not errors.
fn read_repo(repo: &Path) -> RepoState {
    let read = |args: &[&str]| git(repo, args).ok().filter(|out| !out.is_empty());
    let branch = read(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    // The remote the branch tracks, else `origin`, else the first one listed.
    let tracked_name = branch
        .as_deref()
        .and_then(|b| read(&["config", "--get", &format!("branch.{b}.remote")]))
        .or_else(|| {
            let listed = read(&["remote"])?;
            let names: Vec<&str> = listed
                .lines()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect();
            let chosen = names.iter().find(|n| **n == "origin").or(names.first())?;
            Some(chosen.to_string())
        });
    let tracked = tracked_name.and_then(|name| {
        let url = read(&["remote", "get-url", &name])?;
        Some(GitRemote { name, url })
    });
    let stash_count = read(&["stash", "list"]).map_or(0, |list| {
        list.lines().filter(|line| !line.trim().is_empty()).count()
    });
    let remotes = read(&["config", "--get-regexp", r"^remote\..*\.url$"])
        .map_or_else(Vec::new, |out| read_remote_urls(&out));
    RepoState {
        branch,
        tracked,
        stash_count,
        operation: super::conflicts::read_status(repo).unwrap_or_default(),
        remotes,
    }
}

/// Remotes from `remote.<name>.url <url>` lines. A name can contain dots, so
/// it is everything between the leading `remote.` and the trailing `.url`.
fn read_remote_urls(out: &str) -> Vec<GitRemote> {
    let mut remotes = Vec::new();
    for line in out.lines() {
        let Some((key, url)) = line.trim().split_once(' ') else {
            continue;
        };
        let name = key
            .strip_prefix("remote.")
            .and_then(|k| k.strip_suffix(".url"));
        if let Some(name) = name.filter(|n| !n.is_empty())
            && !url.trim().is_empty()
        {
            remotes.push(GitRemote {
                name: name.to_string(),
                url: url.trim().to_string(),
            });
        }
    }
    remotes
}

fn clipped(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Commits from `git log` output in [`LOG_FIELDS`] order, each with its refs.
fn read_commits(out: &str, refs: &HashMap<String, Vec<RefBadge>>) -> Vec<Commit> {
    let fields: Vec<&str> = out.split('\0').collect();
    let mut commits = Vec::with_capacity(fields.len() / LOG_FIELDS.len());
    for record in fields.as_chunks::<{ LOG_FIELDS.len() }>().0 {
        // Each record after the first begins with the newline git puts
        // between them.
        let sha = record[0].trim_start_matches(['\r', '\n']);
        if sha.is_empty() {
            continue;
        }
        commits.push(Commit {
            sha: sha.to_string(),
            parents: record[1].split_whitespace().map(String::from).collect(),
            subject: clipped(record[2], 4_096),
            author: Signature {
                name: clipped(record[3], 512),
                email: clipped(record[4], 512),
                when: clipped(record[5], 128),
            },
            // `%b` keeps the blank line that followed the subject.
            body: clipped(record[6].trim(), BODY_LIMIT),
            refs: refs.get(sha).cloned().unwrap_or_default(),
            is_stash: false,
        });
    }
    commits
}

/// The refs from `for-each-ref` in [`REF_FORMAT`], by the commit each points
/// at. A tag counts for the commit it peels to; symbolic refs such as
/// `origin/HEAD` and refs to anything but a commit are skipped. On a commit,
/// branches come first, then tags, then remote branches, each by name.
fn read_refs(out: &str) -> HashMap<String, Vec<RefBadge>> {
    let mut by_commit: HashMap<String, Vec<RefBadge>> = HashMap::new();
    let fields: Vec<&str> = out.split('\0').collect();
    for record in fields.as_chunks::<6>().0 {
        let name = record[0].trim_start_matches(['\r', '\n']);
        let (object, kind, peeled, peeled_kind, symbolic) =
            (record[1], record[2], record[3], record[4], record[5]);
        if !symbolic.is_empty() {
            continue;
        }
        let found = [
            ("refs/heads/", RefKind::Branch),
            ("refs/remotes/", RefKind::Remote),
            ("refs/tags/", RefKind::Tag),
        ]
        .into_iter()
        .find_map(|(prefix, ref_kind)| Some((ref_kind, name.strip_prefix(prefix)?)));
        let Some((ref_kind, label)) = found else {
            continue;
        };
        let commit = match (kind, peeled_kind) {
            ("commit", _) => object,
            ("tag", "commit") => peeled,
            _ => continue,
        };
        by_commit
            .entry(commit.to_string())
            .or_default()
            .push(RefBadge {
                kind: ref_kind,
                label: clipped(label, 1_024),
            });
    }
    let rank = |kind: RefKind| match kind {
        RefKind::Branch => 0,
        RefKind::Tag => 1,
        RefKind::Remote => 2,
    };
    for refs in by_commit.values_mut() {
        refs.sort_by(|a, b| {
            rank(a.kind)
                .cmp(&rank(b.kind))
                .then_with(|| a.label.cmp(&b.label))
        });
    }
    by_commit
}

/// The message of a stash without the `On main: ` (or `WIP on main: `) git
/// puts before it; the row already sits on that branch.
fn stash_message(subject: &str) -> String {
    let subject = subject.trim();
    ["On ", "WIP on "]
        .iter()
        .find_map(|prefix| Some(subject.strip_prefix(prefix)?.split_once(": ")?.1))
        .unwrap_or(subject)
        .to_string()
}

/// The stash, newest first, as rows of the graph.
///
/// A stash is a commit off the one it was taken from, with nothing on top of
/// it. Only its first parent is kept: the other two are git's records of the
/// index and of the untracked files, and drawing lines to them would show
/// history that never happened. No stash is an empty list.
pub fn stash_commits(repo: &Path) -> Vec<Commit> {
    const FIELD: char = '\x1f';
    const RECORD: char = '\x1e';
    let Ok(out) = git(
        repo,
        &[
            "stash",
            "list",
            "--format=%H%x1f%P%x1f%gs%x1f%s%x1f%an%x1f%ae%x1f%aI%x1e",
        ],
    ) else {
        return Vec::new();
    };
    out.split(RECORD)
        .filter_map(|record| {
            let fields: Vec<&str> = record.trim().split(FIELD).collect();
            let &[sha, parents, reflog, subject, name, email, date] = fields.as_slice() else {
                return None;
            };
            // `stash store -m` leaves the message bare in the reflog; a plain
            // `stash push` wraps it like any other.
            let message = if reflog.trim().is_empty() {
                subject
            } else {
                reflog
            };
            Some(Commit {
                sha: sha.to_string(),
                parents: vec![parents.split_whitespace().next()?.to_string()],
                subject: stash_message(message),
                author: Signature {
                    name: name.to_string(),
                    email: email.to_string(),
                    when: date.to_string(),
                },
                is_stash: true,
                ..Commit::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    fn record(sha: &str, subject: &str, body: &str) -> String {
        format!("{sha}\0\0{subject}\0Ana\0ana@example.org\0T\0{body}\0")
    }

    #[test]
    fn log_records_split_on_nul_and_keep_the_body() {
        let out = format!(
            "{}\n{}",
            record(&"1".repeat(40), "First", "\nwhy it changed\nmore"),
            record(&"2".repeat(40), "Second", "")
        );
        let commits = read_commits(&out, &HashMap::new());
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].body, "why it changed\nmore");
        assert_eq!(commits[1].sha, "2".repeat(40));
        assert!(commits[1].body.is_empty());
    }

    #[test]
    fn a_long_body_is_cut_by_characters_not_bytes() {
        let body = "é".repeat(BODY_LIMIT + 10);
        let commits = read_commits(&record(&"3".repeat(40), "S", &body), &HashMap::new());
        assert_eq!(commits[0].body.chars().count(), BODY_LIMIT);
    }

    #[test]
    fn refs_go_to_the_commit_and_annotated_tags_peel() {
        let row = |fields: [&str; 6]| fields.map(|f| format!("{f}\0")).concat();
        let out = [
            row(["refs/heads/main", "c1", "commit", "", "", ""]),
            row(["refs/tags/v1", "t1", "tag", "c1", "commit", ""]),
            row(["refs/remotes/origin/main", "c1", "commit", "", "", ""]),
            row([
                "refs/remotes/origin/HEAD",
                "c1",
                "commit",
                "",
                "",
                "refs/remotes/origin/main",
            ]),
            row(["refs/tags/blob", "b1", "blob", "", "", ""]),
        ]
        .join("\n");
        let refs = read_refs(&out);
        let labels: Vec<(&str, RefKind)> = refs["c1"]
            .iter()
            .map(|r| (r.label.as_str(), r.kind))
            .collect();
        assert_eq!(
            labels,
            [
                ("main", RefKind::Branch),
                ("v1", RefKind::Tag),
                ("origin/main", RefKind::Remote),
            ]
        );
        assert_eq!(refs.len(), 1, "the blob tag is not a commit");
    }

    #[test]
    fn stash_messages_lose_their_prefix() {
        assert_eq!(
            stash_message("On main: before the merge"),
            "before the merge"
        );
        assert_eq!(
            stash_message("WIP on dev: abc1234 subject"),
            "abc1234 subject"
        );
        assert_eq!(stash_message("  bare  "), "bare");
    }

    #[test]
    fn remote_names_may_hold_dots() {
        assert_eq!(
            read_remote_urls(
                "remote.origin.url git@host:a/b.git\nremote.my.fork.url https://x/y\n"
            ),
            [
                GitRemote {
                    name: "origin".into(),
                    url: "git@host:a/b.git".into()
                },
                GitRemote {
                    name: "my.fork".into(),
                    url: "https://x/y".into()
                },
            ]
        );
    }

    #[test]
    fn a_first_page_has_the_commits_refs_and_checkout() {
        let repo = TestRepo::new();
        let first = repo.commit("first");
        repo.git(&["tag", "v1"]);
        let second = repo.commit("second");
        repo.git(&["remote", "add", "origin", "https://github.com/a/b.git"]);

        let page = history(repo.path(), 0, 100).unwrap();
        let shas: Vec<&str> = page.commits.iter().map(|c| c.sha.as_str()).collect();
        assert_eq!(shas, [second.as_str(), first.as_str()]);
        assert_eq!(page.commits[0].parents, std::slice::from_ref(&first));
        assert_eq!(page.head_sha.as_deref(), Some(second.as_str()));
        assert_eq!(page.next, None);
        assert!(page.commits[0].refs.iter().any(|r| r.label == "main"));
        assert!(page.commits[1].refs.iter().any(|r| r.label == "v1"));
        assert_eq!(page.tips.len(), 1);

        let checkout = page.state.unwrap();
        assert_eq!(checkout.branch.as_deref(), Some("main"));
        let tracked = checkout.tracked.expect("a tracked remote");
        assert_eq!(
            (tracked.name.as_str(), tracked.url.as_str()),
            ("origin", "https://github.com/a/b.git")
        );
        assert_eq!(checkout.stash_count, 0);
    }

    #[test]
    fn pages_follow_on_and_only_the_first_carries_the_extras() {
        let repo = TestRepo::new();
        for n in 0..5 {
            repo.commit(&format!("c{n}"));
        }
        let pages: Vec<HistoryPage> = [0, 2, 4]
            .iter()
            .map(|&at| history(repo.path(), at, 2).unwrap())
            .collect();
        let cursors: Vec<Option<usize>> = pages.iter().map(|p| p.next).collect();
        assert_eq!(cursors, [Some(2), Some(4), None]);
        assert_eq!(pages[2].commits.len(), 1);
        assert!(pages[1].state.is_none() && pages[1].tips.is_empty());
    }

    #[test]
    fn an_empty_repository_is_an_empty_page() {
        let repo = TestRepo::new();
        let page = history(repo.path(), 0, 10).unwrap();
        assert!(page.commits.is_empty() && page.head_sha.is_none());
        assert!(page.state.is_some());
    }

    #[test]
    fn a_stash_hangs_off_the_commit_it_was_taken_from() {
        let repo = TestRepo::new();
        let base = repo.commit("first");
        assert!(stash_commits(repo.path()).is_empty());
        repo.write("log.txt", "edited\n");
        repo.git(&["stash", "push", "-m", "keep this"]);
        let stashes = stash_commits(repo.path());
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].parents, [base]);
        assert_eq!(stashes[0].subject, "keep this");
        assert!(stashes[0].is_stash);
    }
}
