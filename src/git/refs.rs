//! What the side panel lists: branches, remotes, tags and stashes.
//!
//! One read per kind, all of them `for-each-ref` or its equivalent, so the
//! panel never has to walk the history to know what exists.

use std::path::Path;

use anyhow::Result;

use super::actions::{StashEntry, list_stashes};
use super::command::git;

/// A branch, local or remote-tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    /// `main`, or `origin/main` for a remote one.
    pub name: String,
    /// True for the branch HEAD points at.
    pub current: bool,
    /// The upstream it tracks, when it has one.
    pub upstream: Option<String>,
    /// Commits ahead of / behind that upstream.
    pub ahead: u32,
    pub behind: u32,
}

/// Everything the side panel shows for one checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoRefs {
    pub local: Vec<BranchRef>,
    pub remote: Vec<BranchRef>,
    pub tags: Vec<String>,
    pub stashes: Vec<StashEntry>,
}

/// Read every ref the panel lists. Each kind is best effort: a repo with no
/// remote, no tags or no stash is a normal repo, not a failure.
pub fn repo_refs(root: &Path) -> Result<RepoRefs> {
    Ok(RepoRefs {
        local: local_branches(root)?,
        remote: remote_branches(root).unwrap_or_default(),
        tags: tags(root).unwrap_or_default(),
        stashes: list_stashes(root).unwrap_or_default(),
    })
}

/// Local branches, with the checked-out one marked and its divergence from
/// the upstream it tracks.
///
/// `%(HEAD)` is git's own answer to "is this the current branch", which is
/// what makes a detached HEAD report no current branch instead of guessing.
pub fn local_branches(root: &Path) -> Result<Vec<BranchRef>> {
    let out = git(
        root,
        &[
            "for-each-ref",
            "--format=%(HEAD)%00%(refname:short)%00%(upstream:short)%00%(upstream:track)",
            "--sort=refname",
            "refs/heads",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            let head = fields.next()?;
            let name = fields.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let upstream = fields.next().unwrap_or_default().trim();
            let (ahead, behind) = parse_track(fields.next().unwrap_or_default());
            Some(BranchRef {
                name: name.to_string(),
                current: head.trim() == "*",
                upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                ahead,
                behind,
            })
        })
        .collect())
}

/// Remote-tracking branches, minus the `origin/HEAD` pointer — it is a
/// symbolic ref to another row in the same list, not a branch of its own.
pub fn remote_branches(root: &Path) -> Result<Vec<BranchRef>> {
    let out = git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)%00%(symref)",
            "--sort=refname",
            "refs/remotes",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\0');
            let name = fields.next()?.trim();
            let symref = fields.next().unwrap_or_default().trim();
            if name.is_empty() || !symref.is_empty() {
                return None;
            }
            Some(BranchRef {
                name: name.to_string(),
                current: false,
                upstream: None,
                ahead: 0,
                behind: 0,
            })
        })
        .collect())
}

pub fn tags(root: &Path) -> Result<Vec<String>> {
    let out = git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            // Newest first: a tag list is read from the last release backwards.
            "--sort=-creatordate",
            "refs/tags",
        ],
    )?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect())
}

/// `%(upstream:track)` reads `[ahead 2, behind 1]`, `[gone]`, or nothing.
fn parse_track(track: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(count) = part.strip_prefix("ahead ") {
            ahead = count.trim().parse().unwrap_or(0);
        } else if let Some(count) = part.strip_prefix("behind ") {
            behind = count.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    #[test]
    fn the_track_field_reads_both_counts() {
        assert_eq!(parse_track("[ahead 2, behind 1]"), (2, 1));
        assert_eq!(parse_track("[ahead 3]"), (3, 0));
        assert_eq!(parse_track("[behind 4]"), (0, 4));
        assert_eq!(parse_track("[gone]"), (0, 0));
        assert_eq!(parse_track(""), (0, 0));
    }

    #[test]
    fn local_branches_mark_the_checked_out_one() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.git(&["branch", "feature"]);
        repo.git(&["branch", "another"]);

        let branches = local_branches(repo.path()).unwrap();
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["another", "feature", "main"]);
        let current: Vec<&str> = branches
            .iter()
            .filter(|b| b.current)
            .map(|b| b.name.as_str())
            .collect();
        assert_eq!(current, ["main"]);
    }

    #[test]
    fn a_detached_head_has_no_current_branch() {
        let repo = TestRepo::new();
        let sha = repo.commit("first");
        repo.commit("second");
        repo.git(&["checkout", "--detach", &sha]);

        assert!(
            local_branches(repo.path())
                .unwrap()
                .iter()
                .all(|b| !b.current)
        );
    }

    #[test]
    fn tags_and_stashes_come_back_with_the_refs() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.git(&["tag", "v1"]);
        // A second apart, explicitly: `creatordate` has one-second resolution,
        // and two commits in the same second sort by name instead.
        repo.write("log.txt", "first\nsecond\n");
        repo.git(&["add", "log.txt"]);
        repo.git_env(
            &["commit", "-m", "second"],
            &[
                ("GIT_AUTHOR_DATE", "2030-01-01T00:00:00+00:00"),
                ("GIT_COMMITTER_DATE", "2030-01-01T00:00:00+00:00"),
            ],
        );
        repo.git(&["tag", "v2"]);
        repo.write("log.txt", "work in progress\n");
        repo.git(&["stash", "push", "-m", "wip"]);

        let refs = repo_refs(repo.path()).unwrap();
        // Newest first.
        assert_eq!(refs.tags, vec!["v2".to_string(), "v1".to_string()]);
        assert_eq!(refs.stashes.len(), 1);
        assert!(refs.remote.is_empty(), "{:?}", refs.remote);
        assert_eq!(refs.local.len(), 1);
    }
}
