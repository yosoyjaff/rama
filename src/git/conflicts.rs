//! A merge, rebase, cherry-pick or revert that stopped on conflicts: what it
//! is, who is on each side, which files are left, and how to finish it,
//! skip a commit, or abort.
//!
//! git notes what it is in the middle of with files in the git directory
//! (`MERGE_HEAD`, `rebase-merge/`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`), so
//! reading those is enough to know it, whichever tool started it.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::actions::{conflicted_files, git, git_stdout};

/// What git is in the middle of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OperationState {
    #[default]
    None,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

impl OperationState {
    /// "Merging", "Rebasing", …, for "Merging topic into main".
    pub fn verb(self) -> &'static str {
        match self {
            Self::None => "In progress",
            Self::Merging => "Merging",
            Self::Rebasing => "Rebasing",
            Self::CherryPicking => "Cherry-picking",
            Self::Reverting => "Reverting",
        }
    }

    /// "Merge", "Rebase", …, for "Abort Rebase".
    pub fn noun(self) -> &'static str {
        match self {
            Self::None => "Operation",
            Self::Merging => "Merge",
            Self::Rebasing => "Rebase",
            Self::CherryPicking => "Cherry-pick",
            Self::Reverting => "Revert",
        }
    }

    /// "into", or "onto" for a rebase.
    pub fn conjunction(self) -> &'static str {
        match self {
            Self::Rebasing => "onto",
            _ => "into",
        }
    }
}

/// The operation under way, and the people and files in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperationStatus {
    pub state: OperationState,
    pub head_sha: Option<String>,
    /// Files still in conflict.
    pub conflicted_files: Vec<String>,
    /// Files the operation conflicted, settled since or not, which is what
    /// the panel lists as resolved once they are staged.
    pub originally_conflicted: Vec<String>,
    /// The branch it happens on.
    pub current_branch: Option<String>,
    /// What comes in: the branch merged, or what a rebase replays onto.
    pub incoming: Option<String>,
    pub incoming_sha: Option<String>,
    /// Changes were set aside for it and come back when it ends.
    pub autostash_pending: bool,
}

/// What happened to the changes set aside for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoStash {
    /// None were.
    #[default]
    None,
    /// They are still aside, the operation not being over.
    Stashed,
    /// They are back.
    Restored,
    /// They did not apply cleanly and are still in the stash.
    Kept,
}

/// How finishing, skipping or aborting went.
#[derive(Debug, Clone, Default)]
pub struct ConflictOutcome {
    pub success: bool,
    /// Files still in conflict, when it could not go on.
    pub conflicts: Vec<String>,
    pub auto_stash: AutoStash,
}

impl ConflictOutcome {
    fn blocked(conflicts: Vec<String>) -> Self {
        Self {
            success: false,
            conflicts,
            auto_stash: AutoStash::Stashed,
        }
    }

    fn done(auto_stash: AutoStash) -> Self {
        Self {
            success: true,
            conflicts: Vec::new(),
            auto_stash,
        }
    }
}

fn git_dir(root: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(git(
        root,
        &["rev-parse", "--absolute-git-dir"],
    )?))
}

/// Where the sha of the changes set aside for an operation is noted.
pub(crate) fn marker_path(root: &Path) -> Result<PathBuf> {
    Ok(git_dir(root)?.join("rama-autostash"))
}

/// A state file's first line, if it is there and not empty.
fn first_line(dir: &Path, file: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(file)).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// A name for a commit people would recognise: a branch or tag at it, or
/// its short sha.
fn name_for(root: &Path, sha: &str) -> Option<String> {
    let named = git(
        root,
        &[
            "name-rev",
            "--name-only",
            "--refs=refs/heads/*",
            "--refs=refs/remotes/*",
            "--refs=refs/tags/*",
            sha,
        ],
    )
    .ok()
    .filter(|name| !name.is_empty() && name != "undefined");
    named.or_else(|| {
        git(root, &["rev-parse", "--short", sha])
            .ok()
            .filter(|s| !s.is_empty())
    })
}

/// The branch merged in, from the "Merge branch 'x'" line git wrote, for
/// when its commit no longer has a name.
fn merged_branch_from_message(dir: &Path) -> Option<String> {
    let line = first_line(dir, "MERGE_MSG")?;
    let (_, rest) = line.split_once('\'')?;
    let (name, _) = rest.split_once('\'')?;
    Some(name.to_string())
}

/// Read what git is in the middle of, if anything.
pub fn read_status(root: &Path) -> Result<OperationStatus> {
    let dir = git_dir(root)?;
    let rebase = ["rebase-merge", "rebase-apply"]
        .into_iter()
        .find(|d| dir.join(d).exists());
    let state = match () {
        _ if dir.join("MERGE_HEAD").exists() => OperationState::Merging,
        _ if rebase.is_some() => OperationState::Rebasing,
        _ if dir.join("CHERRY_PICK_HEAD").exists() => OperationState::CherryPicking,
        _ if dir.join("REVERT_HEAD").exists() => OperationState::Reverting,
        _ => return Ok(OperationStatus::default()),
    };
    let checked_out = || {
        git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .ok()
            .filter(|b| !b.is_empty() && b != "HEAD")
    };
    let rebase_dir = rebase.unwrap_or("rebase-merge");
    // The other side's commit, as each operation records it, and the one
    // to re-run the merge with.
    let (incoming_sha, their_tip, current_branch) = match state {
        OperationState::Merging => (
            first_line(&dir, "MERGE_HEAD"),
            first_line(&dir, "MERGE_HEAD"),
            checked_out(),
        ),
        OperationState::CherryPicking => (
            first_line(&dir, "CHERRY_PICK_HEAD"),
            first_line(&dir, "CHERRY_PICK_HEAD"),
            checked_out(),
        ),
        OperationState::Reverting => (
            first_line(&dir, "REVERT_HEAD"),
            first_line(&dir, "REVERT_HEAD"),
            checked_out(),
        ),
        OperationState::Rebasing => {
            let replayed = first_line(&dir, &format!("{rebase_dir}/head-name"))
                .map(|name| {
                    name.strip_prefix("refs/heads/")
                        .map(String::from)
                        .unwrap_or(name)
                })
                .or_else(checked_out);
            (
                first_line(&dir, &format!("{rebase_dir}/onto")),
                first_line(&dir, "REBASE_HEAD"),
                replayed,
            )
        }
        OperationState::None => (None, None, None),
    };
    let incoming = incoming_sha
        .as_deref()
        .and_then(|sha| name_for(root, sha))
        .or_else(|| match state {
            OperationState::Merging => merged_branch_from_message(&dir),
            _ => None,
        });
    let conflicted = conflicted_files(root)?;
    Ok(OperationStatus {
        state,
        head_sha: git(root, &["rev-parse", "HEAD"])
            .ok()
            .filter(|s| !s.is_empty()),
        originally_conflicted: ever_conflicted(root, &conflicted, their_tip.as_deref()),
        conflicted_files: conflicted,
        current_branch,
        incoming,
        incoming_sha,
        autostash_pending: first_line(&dir, "rama-autostash").is_some(),
    })
}

/// The files the operation conflicted on: those still in conflict, and
/// those a merge of HEAD with the other side would conflict on, worked out
/// in memory without touching anything (`git merge-tree`). That second part
/// is what still knows about files settled since, after a restart.
fn ever_conflicted(root: &Path, still: &[String], their_tip: Option<&str>) -> Vec<String> {
    let mut files: Vec<String> = still.to_vec();
    if let Some(tip) = their_tip {
        // The tree's id comes first, then the conflicting paths until a blank
        // line. A conflict makes it exit 1, which is the answer, not a failure.
        let answer = git_stdout(
            root,
            &["merge-tree", "--write-tree", "--name-only", "HEAD", tip],
        );
        for path in answer.lines().skip(1).take_while(|line| !line.is_empty()) {
            if !files.iter().any(|f| f == path) {
                files.push(path.to_string());
            }
        }
    }
    files.sort();
    files
}

/// Settle files as they are: staging an unmerged file is how git is told it
/// is resolved.
pub fn mark_resolved(root: &Path, paths: &[String]) -> Result<()> {
    super::status::stage(root, paths)
}

/// Put files back in conflict, rewriting them from the index with their
/// conflict markers.
pub fn mark_conflicted(root: &Path, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut argv = vec!["checkout", "--merge", "--"];
    argv.extend(paths.iter().map(String::as_str));
    git(root, &argv).map(drop)
}

/// Which sides of a conflicted file the index has: 2 is ours, 3 theirs. A
/// side missing deleted the file.
fn sides_present(root: &Path, path: &str) -> Result<[bool; 2]> {
    let mut present = [false; 2];
    // `<mode> <sha> <stage>\t<path>` per stage.
    for line in git(root, &["ls-files", "--unmerged", "--", path])?.lines() {
        let stage = line
            .split('\t')
            .next()
            .and_then(|meta| meta.split_whitespace().nth(2));
        match stage {
            Some("2") => present[0] = true,
            Some("3") => present[1] = true,
            _ => {}
        }
    }
    Ok(present)
}

/// Settle files by taking one side whole. "Theirs" is in git's sense, so
/// during a rebase it is the commit being replayed. If that side deleted the
/// file, taking it deletes it here too.
pub fn take_side(root: &Path, paths: &[String], theirs: bool) -> Result<()> {
    for path in paths {
        let present = sides_present(root, path)?;
        if present == [false, false] {
            continue;
        }
        if present[usize::from(theirs)] {
            git(
                root,
                &[
                    "checkout",
                    if theirs { "--theirs" } else { "--ours" },
                    "--",
                    path,
                ],
            )?;
            git(root, &["add", "--", path])?;
        } else {
            git(root, &["rm", "--force", "--quiet", "--", path])?;
        }
    }
    Ok(())
}

/// Run `argv`; if it stops on conflicts, that is not an error but the list
/// of what is left.
fn go_on(root: &Path, argv: &[&str]) -> Result<Option<Vec<String>>> {
    let ran = git(root, argv);
    let left = conflicted_files(root).unwrap_or_default();
    match (ran, left.is_empty()) {
        (_, false) => Ok(Some(left)),
        (Ok(_), true) => Ok(None),
        (Err(error), true) => Err(error),
    }
}

/// Finish the operation: commit the merge, cherry-pick or revert git has
/// staged, or let a rebase go on to its next commit (which may stop in its
/// turn). Nothing happens while a file is still in conflict.
pub fn continue_operation(root: &Path) -> Result<ConflictOutcome> {
    let status = read_status(root)?;
    let argv: &[&str] = match status.state {
        OperationState::None => bail!("nothing is in progress"),
        _ if !status.conflicted_files.is_empty() => {
            return Ok(ConflictOutcome::blocked(status.conflicted_files));
        }
        OperationState::Rebasing => &["-c", "core.editor=true", "rebase", "--continue"],
        _ => &["-c", "core.editor=true", "commit", "--no-edit"],
    };
    if let Some(left) = go_on(root, argv)? {
        return Ok(ConflictOutcome::blocked(left));
    }
    // The tree is settled for the first time since it began: bring back
    // what was set aside.
    Ok(ConflictOutcome::done(autostash_restore(root)?))
}

/// Leave out the commit a rebase stopped on, and go on with the rest.
pub fn rebase_skip(root: &Path) -> Result<ConflictOutcome> {
    if let Some(left) = go_on(root, &["-c", "core.editor=true", "rebase", "--skip"])? {
        return Ok(ConflictOutcome::blocked(left));
    }
    Ok(ConflictOutcome::done(autostash_restore(root)?))
}

/// Undo the operation, whichever it is, and bring back what was set aside.
pub fn abort_operation(root: &Path) -> Result<ConflictOutcome> {
    let command = match read_status(root)?.state {
        OperationState::Merging => "merge",
        OperationState::Rebasing => "rebase",
        OperationState::CherryPicking => "cherry-pick",
        OperationState::Reverting => "revert",
        OperationState::None => bail!("nothing is in progress"),
    };
    git(root, &[command, "--abort"])?;
    Ok(ConflictOutcome::done(autostash_restore(root)?))
}

/// The message git prepared for the operation, without the comment lines it
/// adds for an editor. Merges, cherry-picks and reverts write one.
pub fn pending_message(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(git_dir(root).ok()?.join("MERGE_MSG")).ok()?;
    let kept: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
    let message = kept.join("\n").trim().to_string();
    (!message.is_empty()).then_some(message)
}

/// Bring back the changes set aside for an operation, found by the sha
/// noted when they were, wherever they are in the stash by now. If they do
/// not apply cleanly they are left in the stash rather than lost.
pub(crate) fn autostash_restore(root: &Path) -> Result<AutoStash> {
    let Some(sha) = first_line(&git_dir(root)?, "rama-autostash") else {
        return Ok(AutoStash::None);
    };
    std::fs::remove_file(marker_path(root)?).ok();
    let listed = git(root, &["stash", "list", "--format=%gd%x1f%H"])?;
    let Some(selector) = listed.lines().find_map(|line| {
        let (selector, entry) = line.split_once('\u{1f}')?;
        (entry.trim() == sha).then(|| selector.trim().to_string())
    }) else {
        return Ok(AutoStash::None);
    };
    // With what was staged put back as staged if possible; plainly if not.
    let popped = git(root, &["stash", "pop", "--index", &selector]).is_ok()
        || git(root, &["stash", "pop", &selector]).is_ok();
    Ok(if popped {
        AutoStash::Restored
    } else {
        AutoStash::Kept
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_repo::TestRepo;

    fn commit(repo: &TestRepo, file: &str, text: &str, message: &str) {
        repo.write(file, text);
        repo.git(&["add", "."]);
        repo.git(&["commit", "-q", "-m", message]);
    }

    /// `main` and `topic` both change `both.txt`, and main has merged topic,
    /// stopping on the conflict.
    fn stopped_merge() -> TestRepo {
        let repo = TestRepo::new();
        commit(&repo, "both.txt", "base\n", "base");
        repo.git(&["checkout", "-q", "-b", "topic"]);
        commit(&repo, "both.txt", "theirs\n", "theirs");
        repo.git(&["checkout", "-q", "main"]);
        commit(&repo, "both.txt", "ours\n", "ours");
        assert!(!repo.git_allow_failure(&["merge", "--no-commit", "topic"]));
        repo
    }

    fn read(repo: &TestRepo, file: &str) -> String {
        std::fs::read_to_string(repo.path().join(file)).unwrap()
    }

    #[test]
    fn nothing_under_way_reads_as_none() {
        let repo = TestRepo::new();
        repo.commit("first");
        assert_eq!(
            read_status(repo.path()).unwrap(),
            OperationStatus::default()
        );
        assert!(abort_operation(repo.path()).is_err());
    }

    #[test]
    fn a_stopped_merge_names_its_sides_and_files() {
        let repo = stopped_merge();
        let status = read_status(repo.path()).unwrap();
        assert_eq!(status.state, OperationState::Merging);
        assert_eq!(status.current_branch.as_deref(), Some("main"));
        assert_eq!(status.incoming.as_deref(), Some("topic"));
        assert_eq!(status.conflicted_files, ["both.txt"]);
        assert_eq!(status.originally_conflicted, ["both.txt"]);
        assert_eq!(status.head_sha, Some(repo.git(&["rev-parse", "HEAD"])));
        assert!(!status.autostash_pending);
        assert!(
            pending_message(repo.path())
                .unwrap()
                .starts_with("Merge branch 'topic'")
        );
        assert_eq!(
            (status.state.verb(), status.state.conjunction()),
            ("Merging", "into")
        );
    }

    #[test]
    fn taking_a_side_settles_the_file_and_it_is_still_known_as_conflicted() {
        let repo = stopped_merge();
        take_side(repo.path(), &["both.txt".into()], false).unwrap();
        assert_eq!(read(&repo, "both.txt"), "ours\n");
        let status = read_status(repo.path()).unwrap();
        assert!(status.conflicted_files.is_empty());
        assert_eq!(status.originally_conflicted, ["both.txt"]);

        mark_conflicted(repo.path(), &["both.txt".into()]).unwrap();
        assert!(read(&repo, "both.txt").contains("<<<<<<<"));
        assert_eq!(
            read_status(repo.path()).unwrap().conflicted_files,
            ["both.txt"]
        );
    }

    #[test]
    fn finishing_waits_for_the_conflicts_then_commits() {
        let repo = stopped_merge();
        let blocked = continue_operation(repo.path()).unwrap();
        assert!(!blocked.success);
        assert_eq!(blocked.conflicts, ["both.txt"]);

        take_side(repo.path(), &["both.txt".into()], true).unwrap();
        let done = continue_operation(repo.path()).unwrap();
        assert!(done.success && done.conflicts.is_empty());
        assert_eq!(read(&repo, "both.txt"), "theirs\n");
        assert_eq!(
            read_status(repo.path()).unwrap().state,
            OperationState::None
        );
        assert!(repo.git(&["log", "-1", "--format=%s"]).starts_with("Merge"));
    }

    #[test]
    fn aborting_puts_main_back() {
        let repo = stopped_merge();
        let before = repo.git(&["rev-parse", "HEAD"]);
        abort_operation(repo.path()).unwrap();
        assert_eq!(
            read_status(repo.path()).unwrap().state,
            OperationState::None
        );
        assert_eq!(repo.git(&["rev-parse", "HEAD"]), before);
        assert_eq!(read(&repo, "both.txt"), "ours\n");
    }

    #[test]
    fn changes_set_aside_come_back_when_the_merge_is_finished() {
        let repo = TestRepo::new();
        commit(&repo, "both.txt", "base\n", "base");
        commit(&repo, "notes.txt", "committed\n", "notes");
        repo.git(&["checkout", "-q", "-b", "topic"]);
        commit(&repo, "both.txt", "theirs\n", "theirs");
        repo.git(&["checkout", "-q", "main"]);
        commit(&repo, "both.txt", "ours\n", "ours");
        repo.write("notes.txt", "uncommitted\n");

        assert!(
            super::super::actions::autostash_push(repo.path(), "aside")
                .unwrap()
                .is_some()
        );
        assert!(!repo.git_allow_failure(&["merge", "--no-commit", "topic"]));
        assert!(read_status(repo.path()).unwrap().autostash_pending);

        take_side(repo.path(), &["both.txt".into()], true).unwrap();
        let done = continue_operation(repo.path()).unwrap();
        assert_eq!(done.auto_stash, AutoStash::Restored);
        assert_eq!(read(&repo, "notes.txt"), "uncommitted\n");
        assert!(!read_status(repo.path()).unwrap().autostash_pending);
    }
}
