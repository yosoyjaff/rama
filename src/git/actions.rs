//! Every change the graph can make to a repository: checking out, merging,
//! rebasing, tagging, pushing, stashing, undoing…
//!
//! Each is one [`GitAction`], run by [`run`] as one or more `git` commands.
//! git never gets a terminal: no editor opens (`GIT_EDITOR=true`, and
//! `--no-edit` where git takes it), no password prompt waits, and its
//! messages are kept in English so the few this module reads mean the same
//! everywhere.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};

use super::conflicts::{self, AutoStash, ConflictOutcome};

/// Something to do to the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitAction {
    /// Switch to a local branch.
    Checkout {
        name: String,
    },
    /// Switch to the local branch `local` for the remote branch `reference`,
    /// making it, tracking `reference`, if it does not exist.
    CheckoutTracking {
        local: String,
        reference: String,
    },
    /// Check out a commit with HEAD detached.
    CheckoutDetached {
        sha: String,
    },
    /// Make a branch at `start` and switch to it.
    CheckoutNewBranch {
        name: String,
        start: String,
    },
    /// Make a branch at `sha` and stay where HEAD is.
    CreateBranch {
        name: String,
        sha: String,
    },
    RenameBranch {
        old: String,
        new: String,
    },
    /// Without `force`, git only deletes a branch that is merged.
    DeleteBranch {
        name: String,
        force: bool,
    },
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    /// Push a branch, setting its upstream if it has none.
    PushBranch {
        branch: String,
        remote: String,
    },
    /// Push over the remote branch, unless it moved since the last fetch
    /// (`--force-with-lease`): someone else's push is refused, not erased.
    PushForce {
        branch: String,
        remote: String,
    },
    /// Make `branch` track the branch of the same name on `remote`.
    SetUpstream {
        branch: String,
        remote: String,
    },
    PushTag {
        name: String,
        remote: String,
    },
    DeleteRemoteTag {
        name: String,
        remote: String,
    },
    /// Replace the tag `name` with an annotated one at `sha`.
    AnnotateTag {
        name: String,
        sha: String,
        message: String,
    },
    /// Merge into the checked-out branch, or only fast-forward to it.
    Merge {
        reference: String,
        ff_only: bool,
    },
    /// Finish the merge, rebase, cherry-pick or revert under way.
    ConflictContinue,
    /// Leave out the commit a rebase stopped on, and go on.
    ConflictSkip,
    /// Undo the operation under way.
    ConflictAbort,
    /// Replay the checked-out branch on `onto`.
    Rebase {
        onto: String,
    },
    CherryPick {
        sha: String,
    },
    Revert {
        sha: String,
    },
    Reset {
        sha: String,
        mode: ResetMode,
    },
    /// A plain tag, or an annotated one when there is a message.
    CreateTag {
        name: String,
        sha: String,
        message: Option<String>,
    },
    DeleteTag {
        name: String,
    },
    /// A second working directory at `reference`: on a new branch, on one
    /// tracking a remote branch, or detached.
    AddWorktree {
        path: String,
        reference: String,
        new_branch: Option<String>,
        track: bool,
        detach: bool,
    },
    /// Write the patch of one commit into `dir`.
    FormatPatch {
        sha: String,
        dir: String,
    },
    Fetch,
    Pull {
        mode: PullMode,
    },
    /// Push the checked-out branch, setting its upstream if it has none.
    Push,
    StashPush {
        message: Option<String>,
        include_untracked: bool,
    },
    StashPop {
        index: u32,
    },
    /// Commit what is staged, or amend HEAD with it and a new message.
    Commit {
        message: String,
        amend: bool,
    },
    /// Replay the run of commits above `base` following `steps` (an empty
    /// `base` means from the root): squash, drop or reorder them.
    RebaseInteractive {
        base: String,
        steps: Vec<RebaseStep>,
    },
    /// Take back the last move of HEAD the reflog shows.
    Undo,
    /// Do again what the last undo took back.
    Redo,
}

/// One line of an interactive rebase's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseStep {
    pub op: RebaseOp,
    pub sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseOp {
    Pick,
    /// Fold into the commit before, keeping that one's message.
    Fixup,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    /// Fast-forward, or merge when it cannot.
    FfPossible,
    FfOnly,
    Rebase,
}

/// What an action has to say once it worked.
#[derive(Debug, Clone, Default)]
pub struct GitActionOutcome {
    /// The branch checked out after it, when it moved HEAD.
    pub branch: Option<String>,
    /// A line for the user: where a patch was written, what a push updated,
    /// a stash that could not come back.
    pub detail: Option<String>,
}

fn said(text: impl Into<String>) -> Result<GitActionOutcome> {
    Ok(GitActionOutcome {
        branch: None,
        detail: Some(text.into()),
    })
}

/// Where HEAD is now: re-read, since git may have landed somewhere else
/// than asked. With `note`, that is said instead of the branch.
fn landed(root: &Path, note: Option<String>) -> Result<GitActionOutcome> {
    let branch = current_branch(root);
    let detail = note.or_else(|| branch.as_ref().map(|b| format!("on {b}")));
    Ok(GitActionOutcome { branch, detail })
}

/// The first seven characters of a sha.
fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

// ── Running git ─────────────────────────────────────────────────────────────

/// A `git` command set up to never wait on a person.
fn command(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    let mut git = Command::new("git");
    git.arg("--no-pager")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null());
    git.env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    for (key, value) in env {
        git.env(key, value);
    }
    super::command::no_console_window(&mut git);
    git
}

fn finish(output: std::process::Output, args: &[&str]) -> Result<String> {
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();
    if output.status.success() {
        return Ok(text(&output.stdout));
    }
    // A merge or rebase that stops says why on stdout; stderr then only has
    // the summary, or nothing.
    let error = text(&output.stderr);
    let message = match (error.is_empty(), text(&output.stdout)) {
        (false, _) => error,
        (true, out) if !out.is_empty() => out,
        _ => format!(
            "git {} failed ({})",
            args.first().unwrap_or(&"?"),
            output.status
        ),
    };
    bail!(message)
}

fn git_env(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let output = command(root, args, env)
        .output()
        .context("could not start git")?;
    finish(output, args)
}

/// Run git in `root`; its trimmed output, or what it said went wrong.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String> {
    git_env(root, args, &[])
}

/// git's output whether it succeeded or not, for the commands that answer
/// with a failing exit code (`merge-tree` reporting conflicts).
pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> String {
    command(root, args, &[])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default()
}

fn current_branch(root: &Path) -> Option<String> {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty())
}

/// Refuse a branch name git would refuse, before using it.
fn check_branch_name(root: &Path, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("name is empty");
    }
    git(root, &["check-ref-format", "--branch", name])
        .map(drop)
        .map_err(|_| anyhow!("`{name}` is not a valid branch name"))
}

/// The paths still in conflict.
pub(crate) fn conflicted_files(root: &Path) -> Result<Vec<String>> {
    let listed = git(root, &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(listed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// The upstream of `branch` (or of HEAD), if it has one.
fn upstream_of(root: &Path, branch: Option<&str>) -> Option<String> {
    let spec = format!("{}@{{u}}", branch.unwrap_or(""));
    git(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", &spec],
    )
    .ok()
    .filter(|u| !u.is_empty())
}

// ── Setting work aside ──────────────────────────────────────────────────────
//
// A checkout or a merge that would touch uncommitted changes would be
// refused by git. Instead, the changes are stashed first and brought back
// after. The stash's sha is written to a file in the git directory, so that
// if a merge stops on conflicts, finishing or aborting it later finds the
// right entry again, whatever else was stashed meanwhile.

/// Stash the uncommitted changes under `message` and clean the tree. `None`
/// when there was nothing to stash.
pub(crate) fn autostash_push(root: &Path, message: &str) -> Result<Option<String>> {
    // `stash create` makes the entry without touching anything, and prints
    // nothing for a clean tree.
    let sha = git(root, &["stash", "create", message])?;
    if sha.is_empty() {
        return Ok(None);
    }
    // Stored before the tree is cleaned: a reset with nothing to come back
    // from is how work gets lost.
    git(root, &["stash", "store", "-m", message, &sha])?;
    std::fs::write(conflicts::marker_path(root)?, &sha).context("could not note the stash")?;
    git(root, &["reset", "--hard", "--quiet"])?;
    Ok(Some(sha))
}

/// Switch with `argv`, with the uncommitted changes set aside and brought
/// back. If they do not apply cleanly on the new branch, they stay stashed,
/// and that is said; the switch still counts as done.
fn switch_carrying_changes(root: &Path, target: &str, argv: &[&str]) -> Result<Option<String>> {
    let stashed = autostash_push(
        root,
        &format!("Auto stash before checking out \"{target}\""),
    )?;
    if let Err(refused) = git(root, argv) {
        if stashed.is_some() {
            conflicts::autostash_restore(root).ok();
        }
        return Err(refused);
    }
    let back = match stashed {
        Some(_) => conflicts::autostash_restore(root)?,
        None => AutoStash::None,
    };
    Ok((back == AutoStash::Kept).then(|| {
        format!("checked out {target}, but your changes did not apply cleanly here: they are kept in the stash")
    }))
}

/// Merge with the uncommitted changes set aside. If the merge stops on
/// conflicts they stay aside until it is finished or aborted, rather than
/// being mixed into a tree full of conflict markers.
fn merge(root: &Path, reference: &str, ff_only: bool) -> Result<GitActionOutcome> {
    let ours = current_branch(root).unwrap_or_else(|| "HEAD".into());
    let stashed = autostash_push(
        root,
        &format!("Auto stash before merge of \"{ours}\" and \"{reference}\""),
    )?;
    let mut argv = vec!["merge", "--no-edit"];
    if ff_only {
        argv.push("--ff-only");
    }
    argv.push(reference);
    match git(root, &argv) {
        Ok(_) => {
            let mut outcome = landed(root, None)?;
            if conflicts::autostash_restore(root)? == AutoStash::Kept {
                outcome.detail = Some(
                    "merged, but your changes did not apply cleanly: they are kept in the stash"
                        .into(),
                );
            }
            Ok(outcome)
        }
        Err(refused) => {
            let clashes = conflicted_files(root).unwrap_or_default();
            if clashes.is_empty() {
                // Refused outright: nothing is under way, so the changes
                // come straight back.
                if stashed.is_some() {
                    conflicts::autostash_restore(root).ok();
                }
                return Err(refused);
            }
            let files: Vec<String> = clashes.iter().map(|f| format!("  {f}")).collect();
            let aside = match stashed {
                Some(_) => {
                    "\n\nYour uncommitted changes are set aside and come back when the merge is finished or aborted."
                }
                None => "",
            };
            bail!(
                "The merge stopped on conflicts in:\n{}{aside}\n\nResolve them and commit, or abort the merge from the working-tree panel.",
                files.join("\n")
            )
        }
    }
}

/// What finishing, skipping or aborting an operation comes to.
fn settled(outcome: ConflictOutcome, done: &str) -> Result<GitActionOutcome> {
    if !outcome.success {
        let files: Vec<String> = outcome.conflicts.iter().map(|f| format!("  {f}")).collect();
        bail!(
            "still unresolved:\n{}\n\nSettle every file first, or abort.",
            files.join("\n")
        );
    }
    let tail = match outcome.auto_stash {
        AutoStash::Restored => ", and your set-aside changes are back",
        AutoStash::Kept => {
            ", but your set-aside changes did not apply cleanly: they are kept in the stash"
        }
        AutoStash::None | AutoStash::Stashed => "",
    };
    said(format!("{done}{tail}"))
}

// ── Undo and redo ───────────────────────────────────────────────────────────

/// The newest `count` moves of HEAD: where it went, and git's line for why
/// (`commit: …`, `checkout: moving from a to b`, `reset: moving to …`).
fn reflog(root: &Path, count: usize) -> Result<Vec<(String, String)>> {
    let listed = git(
        root,
        &["reflog", "--format=%H%x00%gs", "-n", &count.to_string()],
    )?;
    Ok(listed
        .lines()
        .filter_map(|line| line.split_once('\0'))
        .map(|(sha, why)| (sha.to_string(), why.to_string()))
        .collect())
}

/// The `a` of a `checkout: moving from a to b` line.
fn checked_out_from(why: &str) -> Option<&str> {
    let rest = why.strip_prefix("checkout: moving from ")?;
    let from = rest.split(" to ").next()?.trim();
    (!from.is_empty()).then_some(from)
}

/// Take back the last move of HEAD: a checkout goes back where it was; a
/// commit is undone with its changes left staged, so nothing is lost; a
/// merge, pull, reset or rebase is reset away, tree and all.
fn undo(root: &Path) -> Result<GitActionOutcome> {
    let moves = reflog(root, 2)?;
    let [(_, why), (before, _)] = moves.as_slice() else {
        bail!("nothing to undo");
    };
    if why.starts_with("clone:") {
        bail!("nothing to undo");
    }
    if let Some(from) = checked_out_from(why) {
        git(root, &["checkout", from])?;
        return landed(root, Some(format!("checked out {from} back")));
    }
    if why.starts_with("commit") {
        git(root, &["reset", "--soft", before])?;
        return landed(root, Some("commit undone, changes staged".into()));
    }
    git(root, &["reset", "--hard", before])?;
    landed(root, Some("HEAD moved back".into()))
}

/// Do again what an undo took back: a checkout is made again, and a reset
/// is itself reset away.
fn redo(root: &Path) -> Result<GitActionOutcome> {
    let moves = reflog(root, 2)?;
    let Some((_, why)) = moves.first() else {
        bail!("nothing to redo");
    };
    if let Some(from) = checked_out_from(why) {
        git(root, &["checkout", from])?;
        return landed(root, Some(format!("checked out {from} again")));
    }
    match (why.starts_with("reset: moving to "), moves.get(1)) {
        (true, Some((before, _))) => {
            git(root, &["reset", "--hard", before])?;
            landed(root, Some("redone".into()))
        }
        _ => bail!("nothing to redo"),
    }
}

/// Where [`rebase_steps`] tells its "editor" to copy the steps from.
const STEPS_PATH: &str = "RAMA_REBASE_STEPS";
/// The "editor" itself; git appends the file to overwrite.
const COPY_STEPS: &str = "cp \"$RAMA_REBASE_STEPS\"";

/// Run an interactive rebase without an editor: git asks
/// `GIT_SEQUENCE_EDITOR` for the list of steps, and that "editor" copies
/// ours over the one git prepared.
fn rebase_steps(root: &Path, base: &str, steps: &[RebaseStep]) -> Result<GitActionOutcome> {
    if steps.is_empty() {
        bail!("an interactive rebase needs at least one step");
    }
    let todo: String = steps
        .iter()
        .map(|step| {
            let word = match step.op {
                RebaseOp::Pick => "pick",
                RebaseOp::Fixup => "fixup",
                RebaseOp::Drop => "drop",
            };
            format!("{word} {} {}\n", step.sha, short(&step.sha))
        })
        .collect();
    let file = tempfile::NamedTempFile::new().context("could not write the rebase steps")?;
    std::fs::write(file.path(), todo).context("could not write the rebase steps")?;
    // Git runs the editor through `sh -c`, so the path travels in the
    // environment, quoted where it is read: a temp dir with quotes, `$` or
    // backslashes in it is never shell syntax.
    let path = file.path().to_string_lossy();
    let from = if base.is_empty() { "--root" } else { base };
    git_env(
        root,
        &["-c", "rebase.autoStash=true", "rebase", "-i", from],
        &[("GIT_SEQUENCE_EDITOR", COPY_STEPS), (STEPS_PATH, &path)],
    )?;
    said(format!("rebased onto {}", short(base)))
}

/// Run `action` in the repository at `root`.
pub fn run(root: &Path, action: GitAction) -> Result<GitActionOutcome> {
    use GitAction::*;
    match action {
        // `--` so a branch with the name of a file is not read as "restore
        // this file", which would throw away its changes.
        Checkout { name } => {
            let note = switch_carrying_changes(root, &name, &["checkout", &name, "--"])?;
            let mut outcome = landed(root, None)?;
            outcome.branch = outcome.branch.or(Some(name));
            outcome.detail = note.or(outcome.detail);
            Ok(outcome)
        }
        CheckoutTracking { local, reference } => {
            let exists = git(
                root,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{local}"),
                ],
            )
            .is_ok();
            let argv: Vec<&str> = match exists {
                true => vec!["checkout", &local, "--"],
                false => vec!["checkout", "-b", &local, "--track", &reference],
            };
            let note = switch_carrying_changes(root, &local, &argv)?;
            let mut outcome = landed(root, None)?;
            outcome.detail = note.or(outcome.detail);
            Ok(outcome)
        }
        CheckoutDetached { sha } => {
            git(root, &["checkout", "--detach", &sha])?;
            said(format!("HEAD detached at {}", short(&sha)))
        }
        CheckoutNewBranch { name, start } => {
            check_branch_name(root, &name)?;
            git(root, &["checkout", "-b", &name, &start])?;
            landed(root, None)
        }
        CreateBranch { name, sha } => {
            check_branch_name(root, &name)?;
            git(root, &["branch", "--", &name, &sha])?;
            said(format!("{name} created at {}", short(&sha)))
        }
        RenameBranch { old, new } => {
            check_branch_name(root, &new)?;
            git(root, &["branch", "--move", "--", &old, &new])?;
            said(format!("{old} renamed to {new}"))
        }
        DeleteBranch { name, force } => {
            git(
                root,
                &["branch", if force { "-D" } else { "-d" }, "--", &name],
            )?;
            said(format!("{name} deleted"))
        }
        DeleteRemoteBranch { remote, branch } => {
            git(root, &["push", &remote, "--delete", "--", &branch])?;
            said(format!("{branch} deleted on {remote}"))
        }
        PushBranch { branch, remote } => match upstream_of(root, Some(&branch)) {
            Some(upstream) => {
                git(root, &["push", &remote, &branch])?;
                said(format!("{branch} pushed to {upstream}"))
            }
            None => {
                git(root, &["push", "--set-upstream", &remote, &branch])?;
                said(format!("{branch} pushed to {remote}, now tracking it"))
            }
        },
        PushForce { branch, remote } => {
            git(root, &["push", "--force-with-lease", &remote, &branch])?;
            said(format!("{branch} force pushed to {remote}"))
        }
        SetUpstream { branch, remote } => {
            git(
                root,
                &[
                    "branch",
                    &format!("--set-upstream-to={remote}/{branch}"),
                    &branch,
                ],
            )?;
            said(format!("{branch} tracks {remote}/{branch}"))
        }
        PushTag { name, remote } => {
            git(root, &["push", &remote, &format!("refs/tags/{name}")])?;
            said(format!("{name} pushed to {remote}"))
        }
        DeleteRemoteTag { name, remote } => {
            git(
                root,
                &["push", &remote, "--delete", &format!("refs/tags/{name}")],
            )?;
            said(format!("{name} deleted on {remote}"))
        }
        AnnotateTag { name, sha, message } => {
            git(
                root,
                &[
                    "tag",
                    "--force",
                    "--annotate",
                    "--message",
                    &message,
                    "--",
                    &name,
                    &sha,
                ],
            )?;
            said(format!("{name} annotated at {}", short(&sha)))
        }
        Merge { reference, ff_only } => merge(root, &reference, ff_only),
        ConflictContinue => settled(conflicts::continue_operation(root)?, "finished"),
        ConflictSkip => settled(
            conflicts::rebase_skip(root)?,
            "commit skipped, rebase going on",
        ),
        ConflictAbort => settled(conflicts::abort_operation(root)?, "aborted"),
        Rebase { onto } => {
            git(root, &["rebase", &onto])?;
            landed(root, None)
        }
        CherryPick { sha } => {
            git(root, &["cherry-pick", &sha])?;
            said(format!("{} cherry picked", short(&sha)))
        }
        Revert { sha } => {
            git(root, &["revert", "--no-edit", &sha])?;
            said(format!("{} reverted", short(&sha)))
        }
        Reset { sha, mode } => {
            let flag = match mode {
                ResetMode::Soft => "--soft",
                ResetMode::Mixed => "--mixed",
                ResetMode::Hard => "--hard",
            };
            git(root, &["reset", flag, &sha])?;
            landed(root, None)
        }
        CreateTag { name, sha, message } => {
            let message = message.as_deref().map(str::trim).filter(|m| !m.is_empty());
            let mut argv = vec!["tag"];
            if let Some(message) = message {
                argv.extend(["--annotate", "--message", message]);
            }
            argv.extend(["--", &name, &sha]);
            git(root, &argv)?;
            said(format!("{name} created at {}", short(&sha)))
        }
        DeleteTag { name } => {
            git(root, &["tag", "--delete", &name])?;
            said(format!("{name} deleted"))
        }
        AddWorktree {
            path,
            reference,
            new_branch,
            track,
            detach,
        } => {
            let mut argv = vec!["worktree", "add"];
            if detach {
                argv.push("--detach");
            }
            let branch = new_branch
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty());
            if let Some(branch) = branch {
                check_branch_name(root, branch)?;
                if track {
                    argv.push("--track");
                }
                argv.extend(["-b", branch]);
            }
            // `--`: a path the user typed is never read as an option.
            argv.extend(["--", path.as_str(), reference.as_str()]);
            git(root, &argv)?;
            said(format!("worktree created at {path}"))
        }
        FormatPatch { sha, dir } => {
            let written = git(root, &["format-patch", "-1", &sha, "-o", &dir])?;
            match written
                .lines()
                .last()
                .map(str::trim)
                .filter(|f| !f.is_empty())
            {
                Some(file) => said(format!("patch written to {file}")),
                None => said(format!("patch written to {dir}")),
            }
        }
        Fetch => {
            git(root, &["fetch", "--all", "--prune"])?;
            said("remotes fetched")
        }
        Pull { mode } => {
            let how = match mode {
                // --no-rebase: ff when possible, merge commit otherwise.
                // Spelled out because git refuses to guess on diverged
                // branches when pull.rebase is not configured.
                PullMode::FfPossible => "--no-rebase",
                PullMode::FfOnly => "--ff-only",
                PullMode::Rebase => "--rebase",
            };
            // --autostash: uncommitted work is stashed before the merge or
            // rebase and reapplied after, so a dirty tree never blocks a
            // pull (what GitKraken does around its own merge).
            git(root, &["pull", how, "--no-edit", "--autostash"])?;
            landed(root, None)
        }
        Push => match upstream_of(root, None) {
            Some(upstream) => {
                git(root, &["push"])?;
                said(format!("pushed to {upstream}"))
            }
            None => {
                let branch = current_branch(root)
                    .ok_or_else(|| anyhow!("HEAD is detached: there is no branch to push"))?;
                git(root, &["push", "--set-upstream", "origin", &branch])?;
                said(format!("pushed {branch} to origin"))
            }
        },
        StashPush {
            message,
            include_untracked,
        } => {
            let mut argv = vec!["stash", "push"];
            let message = message.as_deref().map(str::trim).filter(|m| !m.is_empty());
            if let Some(message) = message {
                argv.extend(["-m", message]);
            }
            if include_untracked {
                argv.push("--include-untracked");
            }
            // A clean tree "succeeds" with this line; say so instead.
            if git(root, &argv)?
                .to_lowercase()
                .contains("no local changes to save")
            {
                bail!("no local changes to stash");
            }
            said("changes stashed")
        }
        StashPop { index } => {
            git(root, &["stash", "pop", &format!("stash@{{{index}}}")])?;
            said("stash restored")
        }
        Commit { message, amend } => {
            let sha = super::status::commit(root, &message, amend)?;
            said(format!("committed {}", short(&sha)))
        }
        RebaseInteractive { base, steps } => rebase_steps(root, &base, &steps),
        Undo => undo(root),
        Redo => redo(root),
    }
}

/// A stash entry, for the side panel's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// Its `n` in `stash@{n}`.
    pub index: u32,
    pub selector: String,
    pub message: String,
}

/// The stash, newest first. No stash is an empty list.
pub fn list_stashes(root: &Path) -> Result<Vec<StashEntry>> {
    let listed = git(root, &["stash", "list", "--format=%gd%x09%gs"])?;
    Ok(listed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .zip(0..)
        .map(|(line, index)| {
            let (selector, message) = line.split_once('\t').unwrap_or((line, ""));
            StashEntry {
                index,
                selector: selector.trim().to_string(),
                message: message.trim().to_string(),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn the_steps_path_is_never_shell_syntax() {
        use super::{COPY_STEPS, STEPS_PATH};
        let dir = tempfile::tempdir().unwrap();
        let hostile = dir.path().join("it's $(touch pwned) & `x` ; a b");
        std::fs::create_dir_all(&hostile).unwrap();
        let steps = hostile.join("steps");
        std::fs::write(&steps, "pick abc\n").unwrap();
        let target = dir.path().join("git-rebase-todo");
        // How git runs a configured editor: `sh -c '<editor> "$@"' <editor> <file>`.
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{COPY_STEPS} \"$@\""))
            .arg(COPY_STEPS)
            .arg(&target)
            .env(STEPS_PATH, &steps)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "pick abc\n");
        assert!(
            !dir.path().join("pwned").exists(),
            "the path ran as a command"
        );
    }

    use super::*;
    use crate::git::test_repo::TestRepo;

    fn tracked(repo: &TestRepo, file: &str, text: &str, message: &str) -> String {
        repo.write(file, text);
        repo.git(&["add", file]);
        repo.git(&["commit", "-q", "-m", message]);
        repo.git(&["rev-parse", "HEAD"])
    }

    fn head(repo: &TestRepo) -> String {
        repo.git(&["rev-parse", "HEAD"])
    }

    #[test]
    fn a_new_branch_is_made_and_checked_out_and_bad_names_are_refused() {
        let repo = TestRepo::new();
        let base = repo.commit("first");
        let done = run(
            repo.path(),
            GitAction::CheckoutNewBranch {
                name: "fix/x".into(),
                start: base.clone(),
            },
        )
        .unwrap();
        assert_eq!(done.branch.as_deref(), Some("fix/x"));
        let refused = run(
            repo.path(),
            GitAction::CreateBranch {
                name: "bad name..".into(),
                sha: base,
            },
        )
        .unwrap_err();
        assert!(refused.to_string().contains("not a valid branch name"));
    }

    #[test]
    fn changes_travel_with_a_checkout() {
        let repo = TestRepo::new();
        tracked(&repo, "a.txt", "one\n", "a");
        repo.git(&["branch", "other"]);
        tracked(&repo, "b.txt", "b\n", "b");
        repo.write("a.txt", "edited\n");
        let done = run(
            repo.path(),
            GitAction::Checkout {
                name: "other".into(),
            },
        )
        .unwrap();
        assert_eq!(done.branch.as_deref(), Some("other"));
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).unwrap(),
            "edited\n"
        );
        assert!(
            repo.git(&["stash", "list"]).is_empty(),
            "nothing left in the stash"
        );
    }

    #[test]
    fn changes_that_clash_with_the_branch_stay_stashed() {
        let repo = TestRepo::new();
        tracked(&repo, "a.txt", "base\n", "a");
        repo.git(&["checkout", "-q", "-b", "other"]);
        tracked(&repo, "a.txt", "other side\n", "other");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("a.txt", "local edit\n");
        let done = run(
            repo.path(),
            GitAction::Checkout {
                name: "other".into(),
            },
        )
        .unwrap();
        assert!(done.detail.unwrap().contains("kept in the stash"));
        assert_eq!(repo.git(&["stash", "list"]).lines().count(), 1);
    }

    #[test]
    fn revert_and_stash_need_no_editor() {
        let repo = TestRepo::new();
        tracked(&repo, "a.txt", "1\n", "first");
        let second = tracked(&repo, "a.txt", "2\n", "second");
        run(repo.path(), GitAction::Revert { sha: second }).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).unwrap(),
            "1\n"
        );

        let nothing = run(
            repo.path(),
            GitAction::StashPush {
                message: None,
                include_untracked: false,
            },
        )
        .unwrap_err();
        assert!(nothing.to_string().contains("no local changes"));

        repo.write("a.txt", "dirty\n");
        run(
            repo.path(),
            GitAction::StashPush {
                message: Some("kept".into()),
                include_untracked: false,
            },
        )
        .unwrap();
        let stashes = list_stashes(repo.path()).unwrap();
        assert_eq!((stashes.len(), stashes[0].index), (1, 0));
        assert!(stashes[0].message.contains("kept"));
        run(repo.path(), GitAction::StashPop { index: 0 }).unwrap();
        assert!(list_stashes(repo.path()).unwrap().is_empty());
    }

    #[test]
    fn an_interactive_rebase_squashes_and_drops_without_an_editor() {
        let repo = TestRepo::new();
        let base = tracked(&repo, "a.txt", "a\n", "base");
        let one = tracked(&repo, "b.txt", "b\n", "one");
        let two = tracked(&repo, "c.txt", "c\n", "two");
        run(
            repo.path(),
            GitAction::RebaseInteractive {
                base: base.clone(),
                steps: vec![
                    RebaseStep {
                        op: RebaseOp::Pick,
                        sha: one,
                    },
                    RebaseStep {
                        op: RebaseOp::Fixup,
                        sha: two,
                    },
                ],
            },
        )
        .unwrap();
        assert_eq!(
            repo.git(&["rev-list", "--count", &format!("{base}..HEAD")]),
            "1"
        );
        assert_eq!(repo.git(&["log", "-1", "--format=%s"]), "one");
        assert!(
            repo.path().join("c.txt").exists(),
            "the fixup's change stayed"
        );
    }

    #[test]
    fn a_tag_is_annotated_in_place_and_an_upstream_is_set() {
        let repo = TestRepo::new();
        let sha = repo.commit("first");
        repo.git(&["tag", "v1"]);
        run(
            repo.path(),
            GitAction::AnnotateTag {
                name: "v1".into(),
                sha,
                message: "notes".into(),
            },
        )
        .unwrap();
        assert_eq!(repo.git(&["cat-file", "-t", "v1"]), "tag");

        let remote = tempfile::tempdir().unwrap();
        repo.git(&["init", "-q", "--bare", remote.path().to_str().unwrap()]);
        repo.git(&["remote", "add", "origin", remote.path().to_str().unwrap()]);
        repo.git(&["push", "-q", "origin", "main"]);
        run(
            repo.path(),
            GitAction::SetUpstream {
                branch: "main".into(),
                remote: "origin".into(),
            },
        )
        .unwrap();
        assert_eq!(
            repo.git(&["rev-parse", "--abbrev-ref", "main@{u}"]),
            "origin/main"
        );
    }

    #[test]
    fn undo_and_redo_a_commit() {
        let repo = TestRepo::new();
        let first = tracked(&repo, "a.txt", "1\n", "first");
        let second = tracked(&repo, "a.txt", "2\n", "second");
        run(repo.path(), GitAction::Undo).unwrap();
        assert_eq!(head(&repo), first);
        assert!(
            repo.git(&["diff", "--cached", "--name-only"])
                .contains("a.txt"),
            "the change is staged"
        );
        run(repo.path(), GitAction::Redo).unwrap();
        assert_eq!(head(&repo), second);
    }

    #[test]
    fn undo_and_redo_a_checkout() {
        let repo = TestRepo::new();
        repo.commit("first");
        repo.git(&["branch", "other"]);
        repo.git(&["checkout", "-q", "other"]);
        run(repo.path(), GitAction::Undo).unwrap();
        assert_eq!(repo.git(&["branch", "--show-current"]), "main");
        run(repo.path(), GitAction::Redo).unwrap();
        assert_eq!(repo.git(&["branch", "--show-current"]), "other");
    }

    #[test]
    fn undo_and_redo_a_merge() {
        let repo = TestRepo::new();
        tracked(&repo, "a.txt", "a\n", "base");
        repo.git(&["checkout", "-q", "-b", "topic"]);
        tracked(&repo, "b.txt", "b\n", "topic");
        repo.git(&["checkout", "-q", "main"]);
        let before = tracked(&repo, "c.txt", "c\n", "main");
        run(
            repo.path(),
            GitAction::Merge {
                reference: "topic".into(),
                ff_only: false,
            },
        )
        .unwrap();
        let merged = head(&repo);
        run(repo.path(), GitAction::Undo).unwrap();
        assert_eq!(head(&repo), before);
        run(repo.path(), GitAction::Redo).unwrap();
        assert_eq!(head(&repo), merged);
    }

    #[test]
    fn pull_with_a_dirty_tree_autostashes_and_reapplies() {
        // A bare repo stands in for the remote; a clone of it stands in for
        // a commit pushed elsewhere, so the pull has to merge.
        let remote_dir = tempfile::tempdir().unwrap();
        let remote = remote_dir.path().to_str().unwrap();
        let repo = TestRepo::new();
        repo.git(&["init", "--bare", "--initial-branch=main", remote]);
        tracked(&repo, "a.txt", "a\n", "base");
        repo.git(&["remote", "add", "origin", remote]);
        repo.git(&["push", "-q", "-u", "origin", "main"]);

        let elsewhere_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", "-q", remote])
            .arg(elsewhere_dir.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .expect("git runs");
        let elsewhere = TestRepo::from(elsewhere_dir);
        tracked(&elsewhere, "b.txt", "b\n", "theirs");
        elsewhere.git(&["push", "-q"]);

        // Diverge locally and dirty the tree: without --autostash the
        // merge would refuse to touch the working tree.
        tracked(&repo, "c.txt", "c\n", "mine");
        repo.write("wip.txt", "not committed\n");
        run(
            repo.path(),
            GitAction::Pull {
                mode: PullMode::FfPossible,
            },
        )
        .unwrap();

        assert!(
            repo.git_allow_failure(&["merge-base", "--is-ancestor", "origin/main", "HEAD"]),
            "their commit was merged"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("wip.txt")).unwrap(),
            "not committed\n",
            "the uncommitted change survived the pull"
        );
        assert!(
            repo.git(&["stash", "list"]).is_empty(),
            "the autostash entry was reapplied, not left behind"
        );
    }

    #[test]
    fn a_checkout_line_names_where_it_came_from() {
        assert_eq!(
            checked_out_from("checkout: moving from main to topic"),
            Some("main")
        );
        assert_eq!(checked_out_from("commit: x"), None);
        assert_eq!(short("0123456789"), "0123456");
        assert_eq!(short("abc"), "abc");
    }
}
