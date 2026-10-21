//! Who wrote which line of a file, and the commits that touched it.
//!
//! Both answers come from git in one pass each: `blame --porcelain` for the
//! line map, and `log -- <path>` for the file's history. Porcelain is verbose
//! but self-describing — every header names its fields, so a git that grows
//! new ones cannot break this parser the way a column format would.

use std::path::Path;

use anyhow::{Result, bail};

use super::command::git;
use super::types::Commit;

/// One commit the blame names. The fields the gutter shows, nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameCommit {
    pub sha: String,
    pub author: String,
    pub author_email: String,
    /// Unix seconds, as `author-time` reports them.
    pub authored_at: i64,
    pub summary: String,
    /// The commit sits before the file's earliest history (`-C`/rename
    /// limits): git says so, and the gutter may show it dimmer.
    pub is_boundary: bool,
}

/// The blame of one file: the commits it names, and which of them wrote each
/// final line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameResult {
    pub commits: Vec<BlameCommit>,
    /// Index into `commits` for each line of the file, in order.
    pub lines: Vec<usize>,
}

/// `git blame --porcelain -- <path>`.
pub fn blame(root: &Path, path: &str) -> Result<BlameResult> {
    let out = git(root, &["blame", "--porcelain", "--", path])?;
    let mut commits: Vec<BlameCommit> = Vec::new();
    let mut by_sha: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut lines: Vec<usize> = Vec::new();

    let mut current_sha = String::new();
    let mut current: Option<BlameCommit> = None;
    for line in out.lines() {
        let mut fields = line.splitn(4, ' ');
        let first = fields.next().unwrap_or_default();
        // A header: `<sha> <orig-line> <final-line> [<count>]`, possibly
        // followed by repeated half headers for the same commit.
        if first.len() == 40
            && fields
                .next()
                .map(|f| f.chars().all(|c| c.is_ascii_digit()))
                .unwrap_or(false)
        {
            current_sha = first.to_string();
            if let Some(index) = by_sha.get(&current_sha) {
                lines.push(*index);
                continue;
            }
            current = Some(BlameCommit {
                sha: current_sha.clone(),
                author: String::new(),
                author_email: String::new(),
                authored_at: 0,
                summary: String::new(),
                is_boundary: false,
            });
            continue;
        }
        let Some(commit) = current.as_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("author ") {
            commit.author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-mail ") {
            commit.author_email = rest
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            commit.authored_at = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("summary ") {
            commit.summary = rest.to_string();
        } else if line == "boundary" {
            commit.is_boundary = true;
        } else if line.starts_with('\t') {
            // The line's content — the end of this entry. Commit it.
            let index = match by_sha.get(&current_sha) {
                Some(index) => *index,
                None => {
                    let index = commits.len();
                    by_sha.insert(current_sha.clone(), index);
                    let finished = current.take().unwrap_or_else(|| BlameCommit {
                        sha: current_sha.clone(),
                        author: String::new(),
                        author_email: String::new(),
                        authored_at: 0,
                        summary: String::new(),
                        is_boundary: false,
                    });
                    commits.push(finished);
                    index
                }
            };
            lines.push(index);
        }
    }
    if lines.is_empty() {
        bail!("blame returned nothing for {path}");
    }
    Ok(BlameResult { commits, lines })
}

/// The commits that touched one path, newest first — the file's history.
pub fn file_history(root: &Path, path: &str, limit: usize) -> Result<Vec<Commit>> {
    let format = "%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%ce%x1f%cI%x1f%s%x1f%b%x1e";
    let out = git(
        root,
        &[
            "log",
            &format!("--max-count={limit}"),
            &format!("--format={format}"),
            "--",
            path,
        ],
    )?;
    let mut commits = Vec::new();
    for record in out.split('\u{1e}') {
        let record = record.trim_start_matches('\n');
        if record.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.split('\u{1f}').collect();
        if fields.len() < 9 {
            bail!("unexpected log record for {path}");
        }
        commits.push(Commit {
            sha: fields[0].trim().to_string(),
            parents: fields[1].split_whitespace().map(str::to_string).collect(),
            author: crate::git::Signature {
                name: fields[2].to_string(),
                email: fields[3].to_string(),
                when: fields[4].to_string(),
            },
            subject: fields[8].to_string(),
            body: fields[9].to_string(),
            refs: Vec::new(),
            is_stash: false,
        });
    }
    Ok(commits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    fn commit(repo: &TestRepo, path: &str, content: &str, message: &str) {
        repo.write(path, content);
        repo.git(&["add", "."]);
        repo.git_env(
            &["commit", "-q", "-m", message],
            &[("GIT_AUTHOR_NAME", "Ada"), ("GIT_COMMITTER_NAME", "Ada")],
        );
    }

    /// Two commits, three lines: the first two carry the first commit, the
    /// added line carries the second.
    #[test]
    fn the_blame_maps_lines_to_their_commits() {
        let repo = TestRepo::new();
        commit(&repo, "notes.txt", "one\ntwo\n", "first");
        let first = repo.git(&["rev-parse", "HEAD"]);
        commit(&repo, "notes.txt", "one\ntwo\nthree\n", "second");
        let second = repo.git(&["rev-parse", "HEAD"]);

        let result = blame(repo.path(), "notes.txt").expect("blame");
        assert_eq!(result.lines.len(), 3);
        assert_eq!(result.commits.len(), 2);
        let by_sha = |sha: &str| {
            result
                .commits
                .iter()
                .position(|commit| commit.sha == sha)
                .expect("commit present")
        };
        assert_eq!(result.lines[0], by_sha(&first));
        assert_eq!(result.lines[1], by_sha(&first));
        assert_eq!(result.lines[2], by_sha(&second));
        assert_eq!(result.commits[0].author, "Ada");
        assert_eq!(result.commits[0].summary, "first");
        // The root commit is where blame stops, so git marks it boundary.
        assert!(result.commits[0].is_boundary);
    }

    /// The file's history is its commits alone — the branch's other work
    /// stays out.
    #[test]
    fn the_file_history_lists_only_what_touched_the_path() {
        let repo = TestRepo::new();
        commit(&repo, "a.txt", "a\n", "a first");
        commit(&repo, "b.txt", "b\n", "b only");
        commit(&repo, "a.txt", "a\nmore\n", "a second");

        let history = file_history(repo.path(), "a.txt", 10).expect("history");
        let subjects: Vec<&str> = history.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, vec!["a second", "a first"]);
        assert_eq!(history[0].sha, repo.git(&["rev-parse", "HEAD"]));
        // "a first" is the repository's root commit: no parents.
        assert!(history[1].parents.is_empty());
    }
}
