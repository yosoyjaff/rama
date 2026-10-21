//! Reading diffs: what a commit changed, what the working tree has changed,
//! and the patch of a single file.
//!
//! A commit is compared with its first parent, or with the empty tree if it
//! has none. The working tree is compared with HEAD, and its untracked files,
//! which `git diff` never shows, are added as new-file patches made here.

use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};

use super::command::{Capture, git, git_capture};
use super::types::{DiffFileSummary, DiffSnapshot};

/// The id of the empty tree in every git repository: diffing with it shows
/// everything as added.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// The most of a patch kept; past it the view says it was cut.
const PATCH_LIMIT: usize = 3 * 1024 * 1024;
const LIST_LIMIT: usize = 2 * 1024 * 1024;

/// Which file diff to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDiffSide<'a> {
    /// What this commit did to it.
    Commit(&'a str),
    /// What is staged, against HEAD.
    Staged,
    /// What is not staged yet, against the index.
    Unstaged,
}

/// One file's patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub patch: String,
    /// The whole file came as context, so the patch shows all of it.
    pub full: bool,
    /// It was cut at the size limit.
    pub truncated: bool,
    /// The file was valid UTF-8.
    pub utf8: bool,
}

/// `git diff` with the options every read shares.
struct DiffRead<'a> {
    root: &'a Path,
    ignore_whitespace: bool,
    context: String,
}

impl DiffRead<'_> {
    fn run(&self, what: &[&str], limit: usize) -> Result<Capture> {
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--find-renames",
            self.context.as_str(),
        ];
        if self.ignore_whitespace {
            args.push("-w");
        }
        args.extend_from_slice(what);
        git_capture(self.root, &args, limit)
    }

    /// The files between `range`, with their line counts, and the patch.
    fn changes(&self, range: &[&str]) -> Result<(Vec<DiffFileSummary>, Capture, bool)> {
        let with = |mode: &'static str| {
            let mut what = vec![mode, "-z"];
            what.extend_from_slice(range);
            what.push("--");
            what
        };
        let names = self.run(&with("--name-status"), LIST_LIMIT)?;
        let counts = self.run(&with("--numstat"), LIST_LIMIT)?;
        let mut patch_args = range.to_vec();
        patch_args.push("--");
        let patch = self.run(&patch_args, PATCH_LIMIT)?;
        let mut files = read_name_status(&names.stdout);
        add_line_counts(&mut files, &counts.stdout);
        Ok((files, patch, names.truncated || counts.truncated))
    }
}

/// The commit `sha` is compared with: its first parent, or the empty tree.
fn parent_of(root: &Path, sha: &str) -> String {
    git(root, &["rev-parse", "--verify", &format!("{sha}^")])
        .ok()
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| EMPTY_TREE.to_string())
}

fn branch_name(root: &Path) -> String {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "HEAD".into())
}

/// The patch text, cut at its last whole line if it hit the limit.
fn patch_text(capture: &Capture) -> String {
    let mut text = String::from_utf8_lossy(&capture.stdout).into_owned();
    if capture.truncated {
        text.truncate(text.rfind('\n').unwrap_or(0));
        text.push_str("\n# diff truncated\n");
    }
    text
}

fn snapshot(
    root: &Path,
    head_sha: Option<String>,
    patch: String,
    files: Vec<DiffFileSummary>,
    truncated: bool,
) -> DiffSnapshot {
    DiffSnapshot {
        branch: branch_name(root),
        head_sha,
        additions: files.iter().map(|f| f.additions).sum(),
        deletions: files.iter().map(|f| f.deletions).sum(),
        patch,
        files,
        truncated,
    }
}

/// What the commit `sha` changed.
pub fn capture_commit_diff(
    root: &Path,
    sha: &str,
    ignore_whitespace: bool,
    context: u32,
) -> Result<DiffSnapshot> {
    let read = DiffRead {
        root,
        ignore_whitespace,
        context: format!("--unified={context}"),
    };
    let parent = parent_of(root, sha);
    let (files, patch, lists_cut) = read.changes(&[&parent, sha])?;
    let truncated = lists_cut || patch.truncated;
    Ok(snapshot(
        root,
        Some(sha.to_string()),
        patch_text(&patch),
        files,
        truncated,
    ))
}

/// What the working tree has changed since HEAD, untracked files included.
pub fn capture_working_diff(
    root: &Path,
    ignore_whitespace: bool,
    context: u32,
) -> Result<DiffSnapshot> {
    let head = git(root, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .filter(|h| !h.is_empty());
    let read = DiffRead {
        root,
        ignore_whitespace,
        context: format!("--unified={context}"),
    };
    let (mut files, patch, lists_cut) = read.changes(&[head.as_deref().unwrap_or(EMPTY_TREE)])?;
    let mut truncated = lists_cut || patch.truncated;
    let mut text = patch_text(&patch);
    // Read-only: listing must not rewrite the index under the user.
    let status = git_capture(
        root,
        &["--no-optional-locks", "status", "--porcelain", "-z"],
        LIST_LIMIT,
    )?;
    truncated |= status.truncated;
    for path in untracked_paths(&status.stdout) {
        let Ok(found) = std::fs::metadata(root.join(&path)) else {
            continue;
        };
        let too_big = found.len() > PATCH_LIMIT as u64;
        let bytes = match too_big {
            true => None,
            false => std::fs::read(root.join(&path)).ok(),
        };
        let text_of = bytes
            .as_deref()
            .filter(|b| !b.contains(&0))
            .map(String::from_utf8_lossy);
        let mut added = DiffFileSummary {
            path: path.clone(),
            old_path: None,
            status: "added".into(),
            additions: 0,
            deletions: 0,
            binary: text_of.is_none(),
        };
        truncated |= too_big;
        if let Some(content) = text_of {
            added.additions = content.split_terminator('\n').count() as u32;
            let made = new_file_patch(&path, &content);
            if text.len() + made.len() > PATCH_LIMIT {
                truncated = true;
            } else {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&made);
            }
        }
        files.push(added);
    }
    Ok(snapshot(root, head, text, files, truncated))
}

/// One file's patch. With no `context` it comes with the whole file around
/// the changes; if that is too big, with git's usual three lines instead.
pub fn capture_file_diff(
    root: &Path,
    side: FileDiffSide<'_>,
    path: &str,
    ignore_whitespace: bool,
    context: Option<u32>,
) -> Result<FilePatch> {
    let parent = match side {
        FileDiffSide::Commit(sha) => Some(parent_of(root, sha)),
        _ => None,
    };
    let read = |context: String| {
        let diff = DiffRead {
            root,
            ignore_whitespace,
            context,
        };
        let mut what: Vec<&str> = match (side, parent.as_deref()) {
            (FileDiffSide::Commit(sha), Some(parent)) => vec![parent, sha],
            (FileDiffSide::Staged, _) => vec!["--cached"],
            _ => Vec::new(),
        };
        what.extend(["--", path]);
        diff.run(&what, PATCH_LIMIT)
    };
    let (capture, full) = match context {
        Some(lines) => (read(format!("-U{lines}"))?, false),
        None => match read("-U1000000".into())? {
            whole if !whole.truncated => (whole, true),
            _ => (read("-U3".into())?, false),
        },
    };
    let patch = patch_text(&capture);
    // An untracked file has no diff; make its new-file patch, which is the
    // whole file.
    if patch.is_empty()
        && side == FileDiffSide::Unstaged
        && git(root, &["ls-files", "--", path])?.is_empty()
    {
        let bytes = std::fs::read(root.join(inside_checkout(path)?))?;
        if !bytes.contains(&0) {
            return Ok(FilePatch {
                patch: new_file_patch(path, &String::from_utf8_lossy(&bytes)),
                full: true,
                truncated: false,
                utf8: std::str::from_utf8(&bytes).is_ok(),
            });
        }
    }
    Ok(FilePatch {
        utf8: std::str::from_utf8(&capture.stdout).is_ok(),
        truncated: capture.truncated,
        patch,
        full,
    })
}

/// The non-empty records of a `-z` output.
fn records(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .filter(|record| !record.is_empty())
        .map(|record| String::from_utf8_lossy(record).into_owned())
        .collect()
}

/// The untracked paths in `git status --porcelain -z`, sorted. A rename or
/// copy record is followed by its old path, which is skipped.
fn untracked_paths(bytes: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut all = records(bytes).into_iter();
    while let Some(record) = all.next() {
        let Some((code, path)) = record.split_at_checked(2) else {
            continue;
        };
        if code.starts_with(['R', 'C']) {
            all.next();
        }
        if code == "??" {
            paths.push(path.trim_start().to_string());
        }
    }
    paths.sort();
    paths
}

/// The files in `git diff --name-status -z`: a letter, then the path, or
/// for a rename or copy the old path and the new.
fn read_name_status(bytes: &[u8]) -> Vec<DiffFileSummary> {
    let mut files = Vec::new();
    let mut fields = records(bytes).into_iter();
    while let (Some(code), Some(first)) = (fields.next(), fields.next()) {
        let letter = code.chars().next().unwrap_or('M');
        let (path, old_path) = match letter {
            'R' | 'C' => match fields.next() {
                Some(new) => (new, Some(first)),
                None => (first, None),
            },
            _ => (first, None),
        };
        let status = match letter {
            'A' => "added",
            'D' => "deleted",
            'R' => "renamed",
            'C' => "copied",
            'U' => "unmerged",
            _ => "modified",
        };
        files.push(DiffFileSummary {
            path,
            old_path,
            status: status.into(),
            additions: 0,
            deletions: 0,
            binary: false,
        });
    }
    files
}

/// Put the counts of `git diff --numstat -z` on their files. A rename's
/// record has no path; the old and new paths follow it. A binary file counts
/// `-`.
fn add_line_counts(files: &mut [DiffFileSummary], bytes: &[u8]) {
    let mut all = bytes
        .split(|&b| b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());
    while let Some(record) = all.next() {
        let mut fields = record.splitn(3, '\t');
        let (Some(added), Some(removed)) = (fields.next(), fields.next()) else {
            continue;
        };
        let path = match fields.next() {
            Some(path) if !path.is_empty() => path.to_string(),
            _ => {
                all.next();
                all.next().unwrap_or_default()
            }
        };
        if let Some(file) = files.iter_mut().find(|f| f.path == path) {
            file.additions = added.parse().unwrap_or(0);
            file.deletions = removed.parse().unwrap_or(0);
            file.binary = added == "-" || removed == "-";
        }
    }
}

/// A path as a patch header writes it: in quotes, escaped, when it has
/// spaces, quotes or backslashes.
fn header_path(path: &str) -> String {
    if !path.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\') {
        return path.to_string();
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The patch that adds `content` as the new file `path`.
fn new_file_patch(path: &str, content: &str) -> String {
    // Split on `\n` only: a `\r` at a line's end is part of the file.
    let lines: Vec<&str> = content
        .strip_suffix('\n')
        .unwrap_or(content)
        .split('\n')
        .collect();
    let lines = if content.is_empty() {
        Vec::new()
    } else {
        lines
    };
    let new = header_path(&format!("b/{path}"));
    let mut patch = format!(
        "diff --git {} {new}\nnew file mode 100644\n--- /dev/null\n+++ {new}\n@@ -0,0 +1,{} @@\n",
        header_path(&format!("a/{path}")),
        lines.len()
    );
    for line in lines {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    patch
}

/// `path` as a path inside the checkout, or an error if it would leave it.
fn inside_checkout(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    let plain = path
        .components()
        .all(|part| matches!(part, Component::Normal(_)));
    if path.as_os_str().is_empty() || !plain {
        bail!("the path leaves the checkout");
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    fn commit_all(repo: &TestRepo, message: &str) -> String {
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", message]);
        repo.git(&["rev-parse", "HEAD"])
    }

    #[test]
    fn a_commit_lists_its_files_with_counts() {
        let repo = TestRepo::new();
        let root = repo.commit("first");
        repo.write("notes.md", "a\nb\nc\n");
        let sha = commit_all(&repo, "notes");
        let diff = capture_commit_diff(repo.path(), &sha, false, 3).unwrap();
        assert_eq!(diff.files.len(), 1);
        let file = &diff.files[0];
        assert_eq!(
            (file.path.as_str(), file.status.as_str(), file.additions),
            ("notes.md", "added", 3)
        );
        assert_eq!(diff.additions, 3);
        assert!(diff.patch.contains("+++ b/notes.md"));
        assert!(!diff.truncated);
        // The first commit is compared with nothing.
        let first = capture_commit_diff(repo.path(), &root, false, 3).unwrap();
        assert_eq!(first.files[0].status, "added");
    }

    #[test]
    fn renames_keep_both_paths_and_binaries_have_no_counts() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.git(&["mv", "log.txt", "moved.txt"]);
        std::fs::write(repo.path().join("data.bin"), [0u8, 7, 0, 9]).unwrap();
        let sha = commit_all(&repo, "move");
        let diff = capture_commit_diff(repo.path(), &sha, false, 3).unwrap();
        let moved = diff.files.iter().find(|f| f.path == "moved.txt").unwrap();
        assert_eq!(
            (moved.status.as_str(), moved.old_path.as_deref()),
            ("renamed", Some("log.txt"))
        );
        let blob = diff.files.iter().find(|f| f.path == "data.bin").unwrap();
        assert!(blob.binary && blob.additions == 0);
    }

    #[test]
    fn the_working_tree_includes_untracked_files() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.write("log.txt", "first\nmore\n");
        repo.write("my new.txt", "x\ny\n");
        let diff = capture_working_diff(repo.path(), false, 3).unwrap();
        let new = diff.files.iter().find(|f| f.path == "my new.txt").unwrap();
        assert_eq!((new.status.as_str(), new.additions), ("added", 2));
        assert!(diff.files.iter().any(|f| f.path == "log.txt"));
        assert!(
            diff.patch.contains("+++ \"b/my new.txt\""),
            "{}",
            diff.patch
        );
        assert!(diff.patch.contains("+y\n"));
    }

    #[test]
    fn a_file_comes_whole_or_with_three_lines_of_context() {
        let repo = TestRepo::new();
        let text: String = (1..=30).map(|n| format!("row {n}\n")).collect();
        repo.write("long.txt", &text);
        commit_all(&repo, "long");
        repo.write("long.txt", &text.replace("row 15\n", "row fifteen\n"));
        let sha = commit_all(&repo, "edit");

        let whole = capture_file_diff(
            repo.path(),
            FileDiffSide::Commit(&sha),
            "long.txt",
            false,
            None,
        )
        .unwrap();
        assert!(whole.full && whole.utf8 && !whole.truncated);
        assert!(whole.patch.contains("\n row 1\n") && whole.patch.contains("\n row 30\n"));

        repo.write("long.txt", &text);
        let near = capture_file_diff(
            repo.path(),
            FileDiffSide::Unstaged,
            "long.txt",
            false,
            Some(3),
        )
        .unwrap();
        assert!(!near.full);
        assert!(near.patch.contains(" row 12\n") && !near.patch.contains(" row 1\n"));
    }

    #[test]
    fn staged_and_unstaged_are_read_apart_and_untracked_files_are_made_up() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.write("log.txt", "staged\n");
        repo.git(&["add", "log.txt"]);
        repo.write("log.txt", "unstaged\n");
        repo.write("fresh.txt", "hi\n");
        let read = |side, path| {
            capture_file_diff(repo.path(), side, path, false, None)
                .unwrap()
                .patch
        };
        let staged = read(FileDiffSide::Staged, "log.txt");
        assert!(staged.contains("-first") && staged.contains("+staged"));
        let unstaged = read(FileDiffSide::Unstaged, "log.txt");
        assert!(unstaged.contains("-staged") && unstaged.contains("+unstaged"));
        let fresh = read(FileDiffSide::Unstaged, "fresh.txt");
        assert!(fresh.contains("new file mode") && fresh.contains("+hi"));
    }

    #[test]
    fn paths_outside_the_checkout_are_refused() {
        for path in ["../x", "/etc/passwd", "", "a/../../b"] {
            assert!(inside_checkout(path).is_err(), "{path:?}");
        }
        assert!(inside_checkout("src/main.rs").is_ok());
        assert_eq!(header_path("a/plain.rs"), "a/plain.rs");
        assert_eq!(header_path("a/say \"hi\".txt"), "\"a/say \\\"hi\\\".txt\"");
    }
}
