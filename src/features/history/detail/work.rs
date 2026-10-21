//! What the panel reads from git and what it changes, all off the UI
//! thread.
//!
//! Every change to the working tree reads `git status` again in the same
//! background job, so the panel shows the tree as it ended up, not as the
//! change meant to leave it.

use std::collections::HashSet;
use std::path::Path;

use gpui_kit::{Context, Pixels, Point, SharedString, Task, Window};

use super::listing::Staging;
use super::{DetailPanel, PanelEvent, Shown};
use crate::git::actions::GitAction;
use crate::git::conflicts::{self, OperationState, OperationStatus};
use crate::git::{DiffFileSummary, GitStatus};
use crate::ui::popover::Layer;

/// Summaries up to this long show whole in most tools; the counter warns
/// past it but still lets the commit through.
pub(super) const SUMMARY_GUIDE: usize = 72;

/// A read that may still be running.
#[derive(Default)]
pub(super) enum Fetched<T> {
    #[default]
    Waiting,
    Ready(T),
    Failed(SharedString),
}

/// The files of the commit shown.
#[derive(Default)]
pub(super) struct CommitFiles {
    /// The checkout and commit they are read for.
    pub source: Option<(String, String)>,
    pub files: Fetched<Vec<DiffFileSummary>>,
    task: Option<Task<()>>,
}

impl CommitFiles {
    pub fn listed(&self) -> &[DiffFileSummary] {
        match &self.files {
            Fetched::Ready(files) => files,
            _ => &[],
        }
    }
}

/// The working tree's lists that fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Section {
    Unstaged,
    Staged,
    Conflicted,
    Resolved,
}

/// What the working tree is busy with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Job {
    #[default]
    Idle,
    Changing,
    Committing,
}

/// The menu over a conflicted file.
pub(super) struct ConflictMenu {
    pub at: Point<Pixels>,
    pub path: String,
}

/// The working tree as the panel knows it, and the commit being written.
#[derive(Default)]
pub(super) struct Worktree {
    /// The checkout it was read for.
    pub source: Option<String>,
    pub status: Option<GitStatus>,
    pub operation: OperationStatus,
    pub folded: HashSet<Section>,
    pub amend: bool,
    pub job: Job,
    pub asking_to_discard: bool,
    /// The last thing that went wrong, or that git had to say.
    pub problem: Option<SharedString>,
    /// The message git prepared for the operation, as (summary, body), for
    /// the commit box once a window is at hand.
    pub prepared: Option<(String, String)>,
    pub menu: Layer<ConflictMenu>,
    task: Option<Task<()>>,
}

impl Worktree {
    pub fn busy(&self) -> bool {
        self.job != Job::Idle
    }

    pub fn staging(&self) -> Staging {
        self.status
            .as_ref()
            .map(|status| Staging::of(&status.entries))
            .unwrap_or_default()
    }

    pub fn conflicted(&self) -> Vec<String> {
        let entries = self.status.as_ref().map_or(&[][..], |s| &s.entries);
        entries
            .iter()
            .filter(|entry| entry.conflicted)
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Staged files that the operation had in conflict: the settled ones.
    pub fn resolved(&self) -> Vec<DiffFileSummary> {
        let was = &self.operation.originally_conflicted;
        self.staging()
            .staged
            .into_iter()
            .filter(|file| was.contains(&file.path))
            .collect()
    }

    pub fn in_operation(&self) -> bool {
        self.operation.state != OperationState::None
    }

    fn take(&mut self, (status, operation): Reading) {
        self.operation = operation;
        match status {
            Ok(status) => self.status = Some(status),
            Err(error) => self.problem = Some(error.to_string().into()),
        }
    }
}

/// A change to some files of the working tree.
pub(super) enum Edit {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    /// Back to the last commit; untracked files are deleted.
    Discard {
        tracked: Vec<String>,
        untracked: Vec<String>,
    },
    /// A settled file back in conflict, markers and all.
    Reconflict(String),
    /// A conflict settled with one side whole.
    Take {
        path: String,
        theirs: bool,
    },
}

impl Edit {
    fn apply(self, root: &Path) -> anyhow::Result<()> {
        use crate::git::status;
        match self {
            Self::Stage(paths) => status::stage(root, &paths),
            Self::Unstage(paths) => status::unstage(root, &paths),
            Self::Discard { tracked, untracked } => status::discard(root, &tracked, &untracked),
            Self::Reconflict(path) => conflicts::mark_conflicted(root, &[path]),
            Self::Take { path, theirs } => conflicts::take_side(root, &[path], theirs),
        }
    }
}

/// How a stopped operation ends from the commit box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WayOut {
    pub label: String,
    pub action: GitAction,
    pub ready: bool,
}

/// While files are in conflict a rebase can only drop the commit it stopped
/// on, and the others cannot finish; once settled, each goes on.
pub(super) fn way_out(state: OperationState, unresolved: bool) -> WayOut {
    let noun = state.noun();
    let (label, action, ready) = match (state, unresolved) {
        (OperationState::Rebasing, true) => ("Skip Commit".into(), GitAction::ConflictSkip, true),
        (OperationState::Rebasing, false) => {
            ("Continue Rebase".into(), GitAction::ConflictContinue, true)
        }
        (_, true) => (
            format!("No Changes to {noun}"),
            GitAction::ConflictContinue,
            false,
        ),
        (OperationState::Merging, false) => {
            ("Commit and Merge".into(), GitAction::ConflictContinue, true)
        }
        (_, false) => (format!("Commit {noun}"), GitAction::ConflictContinue, true),
    };
    WayOut {
        label,
        action,
        ready,
    }
}

/// A message git prepared, split at its first line.
fn split_message(message: &str) -> (String, String) {
    let (summary, body) = message.split_once('\n').unwrap_or((message, ""));
    (summary.to_string(), body.trim().to_string())
}

/// Summary and body as one message: a blank line between, when there is a
/// body.
pub(super) fn join_message(summary: &str, body: &str) -> String {
    match (summary.trim(), body.trim()) {
        (summary, "") => summary.to_string(),
        (summary, body) => format!("{summary}\n\n{body}"),
    }
}

type Reading = (anyhow::Result<GitStatus>, OperationStatus);

fn read_worktree(root: &Path) -> Reading {
    (
        crate::git::status::status(root),
        conflicts::read_status(root).unwrap_or_default(),
    )
}

impl DetailPanel {
    /// Read what is shown, unless it was already read for this checkout.
    pub(super) fn read_shown(&mut self, cx: &mut Context<Self>) {
        match &self.shown {
            Shown::Nothing => {}
            Shown::Commit(commit) => {
                let sha = commit.sha.clone();
                self.read_commit_files(sha, cx);
            }
            Shown::WorkingTree => self.read_worktree(cx),
        }
    }

    fn read_commit_files(&mut self, sha: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let source = Some((repo.clone(), sha.clone()));
        if self.commit.source == source {
            return;
        }
        self.commit.source = source.clone();
        self.commit.files = Fetched::Waiting;
        let read = cx.background_executor().spawn(async move {
            crate::git::capture_commit_diff(Path::new(&repo), &sha, false, 3)
        });
        self.commit.task = Some(cx.spawn(async move |this, cx| {
            let diff = read.await;
            this.update(cx, |panel, cx| {
                // Another commit was picked meanwhile.
                if panel.commit.source != source {
                    return;
                }
                panel.commit.files = match diff {
                    Ok(diff) => Fetched::Ready(diff.files),
                    Err(error) => Fetched::Failed(error.to_string().into()),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    /// Read the working tree. During an operation, also the message git
    /// prepared for it.
    fn read_worktree(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.worktree.source.as_ref() == Some(&repo) {
            return;
        }
        self.worktree.source = Some(repo.clone());
        let root = repo.clone();
        let read = cx.background_executor().spawn(async move {
            let root = Path::new(&root);
            let reading = read_worktree(root);
            let prepared = match reading.1.state {
                OperationState::None => None,
                _ => conflicts::pending_message(root),
            };
            (reading, prepared)
        });
        self.worktree.task = Some(cx.spawn(async move |this, cx| {
            let (reading, prepared) = read.await;
            this.update(cx, |panel, cx| {
                if panel.worktree.source.as_ref() != Some(&repo) {
                    return;
                }
                panel.worktree.prepared = prepared.as_deref().map(split_message);
                panel.worktree.problem = None;
                panel.worktree.take(reading);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Change some files of the working tree.
    pub(super) fn edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        self.run(cx, move |root| edit.apply(root).map(|()| None));
    }

    /// Finish, skip or abort the operation under way.
    pub(super) fn settle(&mut self, action: GitAction, cx: &mut Context<Self>) {
        self.run(cx, move |root| {
            crate::git::actions::run(root, action).map(|outcome| outcome.detail)
        });
    }

    /// Run `work` on the checkout, then read the tree again. What it says
    /// back (a file still unresolved, a stash that could not come back) or
    /// its failure is put under the commit box.
    fn run(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Path) -> anyhow::Result<Option<String>> + Send + 'static,
    ) {
        if self.worktree.busy() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.worktree.job = Job::Changing;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let root = Path::new(&repo);
            let said = work(root);
            (said, read_worktree(root))
        });
        cx.spawn(async move |this, cx| {
            let (said, reading) = job.await;
            this.update(cx, |panel, cx| {
                let tree = &mut panel.worktree;
                tree.job = Job::Idle;
                tree.problem = None;
                tree.take(reading);
                match said {
                    Ok(Some(note)) => tree.problem = Some(note.into()),
                    Ok(None) => {}
                    Err(error) => tree.problem = Some(error.to_string().into()),
                }
                cx.emit(PanelEvent::TreeChanged);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What the commit button says, and whether it can be pressed. With
    /// nothing staged it stages everything first.
    pub(super) fn commit_button(&self, cx: &gpui_kit::App) -> (String, bool) {
        if self.worktree.job == Job::Committing {
            return ("Committing…".into(), false);
        }
        let typed = !self.summary.read(cx).text().trim().is_empty();
        if self.worktree.amend {
            return ("Amend Previous Commit".into(), typed);
        }
        let staging = self.worktree.staging();
        match (staging.staged.len(), staging.unstaged.len()) {
            (0, 0) => ("Nothing to Commit".into(), false),
            (0, _) => ("Stage Changes to Commit".into(), typed),
            (1, _) => ("Commit 1 File".into(), typed),
            (n, _) => (format!("Commit {n} Files"), typed),
        }
    }

    /// Commit (or amend) with what is typed. With nothing staged, everything
    /// is staged first in the same job, so the commit never runs ahead of it.
    pub(super) fn commit(&mut self, cx: &mut Context<Self>) {
        let (_, ready) = self.commit_button(cx);
        if !ready || self.worktree.busy() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let message = join_message(
            self.summary.read(cx).text(),
            self.description.read(cx).text(),
        );
        let staging = self.worktree.staging();
        let stage_first: Vec<String> = match staging.staged.is_empty() {
            true => staging.unstaged.into_iter().map(|file| file.path).collect(),
            false => Vec::new(),
        };
        let amend = self.worktree.amend;
        self.worktree.job = Job::Committing;
        self.worktree.problem = None;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let root = Path::new(&repo);
            crate::git::status::stage(root, &stage_first)?;
            crate::git::status::commit(root, &message, amend)
        });
        cx.spawn(async move |this, cx| {
            let done = job.await;
            this.update(cx, |panel, cx| {
                panel.worktree.job = Job::Idle;
                match done {
                    Ok(_) => {
                        panel.worktree.amend = false;
                        panel.clear_message = true;
                        panel.worktree.source = None;
                        panel.read_shown(cx);
                        cx.emit(PanelEvent::TreeChanged);
                    }
                    Err(error) => panel.worktree.problem = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Put into the commit box what is waiting for a window: emptiness after
    /// a commit, or the message git prepared, if nothing has been typed.
    pub(super) fn fill_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fill = match (
            std::mem::take(&mut self.clear_message),
            self.worktree.prepared.take(),
        ) {
            (true, _) => Some((String::new(), String::new())),
            (false, Some(prepared)) if self.summary.read(cx).text().trim().is_empty() => {
                Some(prepared)
            }
            _ => None,
        };
        if let Some((summary, body)) = fill {
            self.summary
                .update(cx, |input, cx| input.set_text(summary, window, cx));
            self.description
                .update(cx, |input, cx| input.set_text(body, window, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_operation_offers_its_way_out() {
        let says = |state, unresolved| {
            let out = way_out(state, unresolved);
            (out.label, out.ready)
        };
        assert_eq!(
            says(OperationState::Rebasing, true),
            ("Skip Commit".into(), true)
        );
        assert_eq!(
            way_out(OperationState::Rebasing, false).action,
            GitAction::ConflictContinue
        );
        assert_eq!(
            says(OperationState::Merging, true),
            ("No Changes to Merge".into(), false)
        );
        assert_eq!(
            says(OperationState::Merging, false),
            ("Commit and Merge".into(), true)
        );
        assert_eq!(
            says(OperationState::CherryPicking, false),
            ("Commit Cherry-pick".into(), true)
        );
        assert_eq!(
            says(OperationState::Reverting, true),
            ("No Changes to Revert".into(), false)
        );
    }

    #[test]
    fn messages_split_and_join_at_the_first_line() {
        assert_eq!(
            split_message("Merge branch 'x'\n\n# Conflicts:\n#\tsrc/a.rs\n"),
            (
                "Merge branch 'x'".into(),
                "# Conflicts:\n#\tsrc/a.rs".into()
            )
        );
        assert_eq!(
            split_message("one line"),
            ("one line".into(), String::new())
        );
        assert_eq!(join_message(" fix ", "  "), "fix");
        assert_eq!(join_message("fix", "why\n"), "fix\n\nwhy");
    }
}
