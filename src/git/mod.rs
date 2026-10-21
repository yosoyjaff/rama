//! The git data layer: everything the product knows about a repository.
//!
//! One rule holds the module together: **this is the git CLI**. Every read
//! spawns `git` and parses what it writes, which is why worktrees, submodules,
//! sparse-checkout, LFS and signatures behave correctly — they are not
//! reimplemented here, they are git's own behaviour.
//!
//! * [`command`] — running `git` and reporting what it said when it failed.
//! * [`types`] — the data a history page, a status and a diff are made of.
//! * [`history`] — pages of commits, their refs, and the checkout's identity.
//! * [`open`] — finding the repository to read.
//! * [`create`] — cloning one, or starting one.
//! * [`avatars`] — author photographs, the one network this application makes.
//! * [`refs`] — the branches, tags and stashes the side panel lists.
//! * [`status`] — the working tree, and staging, discarding and committing it.
//! * [`diff`] — the patch of a commit or of the working tree.
//! * [`actions`] — every mutation the graph offers: checkout, merge, rebase,
//!   cherry-pick, tag, stash, push, pull.
//!
//! Nothing in here draws, and nothing in here knows about a window: the calls
//! block, and the history feature runs them on the background executor.

// The data layer lands before the pane that renders it, so for now its own
// tests are the only caller. Drop the allow when the history feature mounts.
#![allow(dead_code, unused_imports)]

pub mod actions;
pub mod avatars;
pub mod blame;
pub mod command;
pub mod conflict_file;
pub mod conflicts;
pub mod create;
pub mod diff;
pub mod forge;
pub mod history;
pub mod open;
pub mod refs;
pub mod status;
#[cfg(test)]
pub(crate) mod test_repo;
pub mod types;
pub mod watch;

pub use actions::{GitAction, GitActionOutcome, PullMode, RebaseOp, RebaseStep, ResetMode};
pub use diff::{
    FileDiffSide, FilePatch, capture_commit_diff, capture_file_diff, capture_working_diff,
};
pub use history::history;
pub use refs::{BranchRef, RepoRefs, repo_refs};
pub use types::{
    Commit, DiffFileSummary, DiffSnapshot, GitRemote, GitStatus, GitStatusEntry, HistoryPage,
    RefBadge, RefKind, RepoRef, RepoState, Signature,
};
