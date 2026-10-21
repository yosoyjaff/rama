//! What the history knows about its checkout, and how it stays current.
//!
//! Pages of commits, the working tree's status and fetches from the remotes
//! are all read on the background executor; this side stores what comes back
//! and asks for a repaint. Anything that comes back for a checkout the pane
//! has since left is dropped: every reply carries the path it was read from,
//! and is compared with [`Checkout::shown`].

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use gpui_kit::{ClipboardItem, Context, Entity, ListAlignment, ListState, SharedString, Task, px};

use super::actions::LinearRun;
use super::lanes::{Graph, branch_ref_key, collapse_branch_runs, lay_out, merge_stash_rows};
use super::photos::Photos;
use super::rail::Sizing;
use super::row::ROW_HEIGHT;
use super::search::Search;
use super::{HistoryEvent, HistoryView, Popups, Scope, WORKTREE_ROW, branch_column};
use crate::git::conflicts::OperationState;
use crate::git::{Commit, HistoryPage, RefBadge};
use crate::settings;
use crate::ui::text_input::TextInput;

const PAGE_LEN: usize = 100;
/// The first quiet fetch, soon after a checkout opens; then one a minute,
/// so remote branches stay current without anyone pressing Fetch.
const FIRST_AUTO_FETCH: Duration = Duration::from_secs(2);
const AUTO_FETCH_EVERY: Duration = Duration::from_secs(60);
/// How long a copied sha says so.
const COPIED_FOR: Duration = Duration::from_millis(1_200);

/// Which checkout is read.
#[derive(Default)]
pub(super) struct Checkout {
    /// The one asked for.
    pub wanted: Option<String>,
    /// The one whose data is held. It catches up with `wanted` on the next
    /// render, which drops everything read before.
    pub shown: Option<String>,
}

/// Reading the history a page at a time.
#[derive(Default)]
pub(super) struct Paging {
    /// Where the next page starts, while there is one.
    pub next: Option<usize>,
    pub reading: bool,
    pub failure: Option<SharedString>,
    pub task: Option<Task<()>>,
}

/// Folded branches, by [`branch_ref_key`], and how many commits each hides.
#[derive(Default)]
pub(super) struct Folds {
    pub closed: HashSet<String>,
    pub hidden: HashMap<String, usize>,
}

/// Fetching from the remotes, by hand or on the timer.
#[derive(Default)]
pub(super) struct Fetching {
    pub running: bool,
    /// Only shown for a fetch someone asked for; the timer's stay quiet, so
    /// being offline does not raise an error every minute.
    pub error: Option<SharedString>,
    pub task: Option<Task<()>>,
    pub timer: Option<Task<()>>,
}

impl HistoryView {
    /// A pane with no checkout.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::build(None, cx)
    }

    /// A pane reading the checkout at `path`.
    pub fn with_repo(path: String, cx: &mut Context<Self>) -> Self {
        Self::build(Some(path), cx)
    }

    /// The summary box of the working tree's row, which the pane owns.
    pub fn set_wip_input(&mut self, input: Entity<TextInput>) {
        self.wip_input = Some(input);
    }

    /// Read another checkout, or none.
    pub fn open_repo(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        if self.checkout.wanted != path {
            self.checkout.wanted = path;
            self.sync_checkout(cx);
            cx.notify();
        }
    }

    fn build(path: Option<String>, cx: &mut Context<Self>) -> Self {
        Self {
            checkout: Checkout {
                wanted: path,
                shown: None,
            },
            paging: Paging::default(),
            repo: None,
            commits: Vec::new(),
            shown_rows: Vec::new(),
            tips: Vec::new(),
            stashes: Vec::new(),
            head_sha: None,
            tree_status: None,
            scope: Scope::default(),
            scope_epoch: 0,
            scope_change: None,
            graph: Graph::default(),
            sizing: Sizing::new(),
            search: Search::default(),
            folds: Folds::default(),
            fetch: Fetching::default(),
            photos: Photos::default(),
            popups: Popups::default(),
            list: ListState::new(0, ListAlignment::Top, px(ROW_HEIGHT * 5.0)),
            gestures: Default::default(),
            just_copied: None,
            git_running: false,
            lit_button: None,
            branch_column_open: settings::read(cx, |s| s.history.badges),
            selected: None,
            badge_hover: branch_column::BadgeHover::default(),
            wip_input: None,
            tree_task: None,
            scope_task: None,
            copy_reset: None,
            git_task: None,
        }
    }

    /// The path of the checkout being read.
    pub(super) fn root(&self) -> Option<String> {
        self.checkout.wanted.clone()
    }

    /// Bring what is held in line with the checkout asked for: drop it all
    /// when that changed, and read a first page when nothing is held.
    pub fn sync_checkout(&mut self, cx: &mut Context<Self>) {
        if self.checkout.shown == self.checkout.wanted {
            let idle = !self.paging.reading && self.paging.failure.is_none();
            if self.checkout.shown.is_some() && idle && self.commits.is_empty() {
                self.read_page(None, cx);
            }
            return;
        }
        self.drop_checkout(cx);
        self.checkout.shown = self.checkout.wanted.clone();
        if self.checkout.shown.is_some() {
            self.read_page(None, cx);
            self.schedule_fetches(cx);
        }
    }

    /// Forget everything read from the checkout held. Tasks in flight go
    /// with it, which cancels them.
    fn drop_checkout(&mut self, cx: &mut Context<Self>) {
        self.paging = Paging::default();
        self.fetch = Fetching::default();
        self.folds = Folds::default();
        self.search = Search::default();
        self.photos.clear();
        self.repo = None;
        self.commits.clear();
        self.shown_rows.clear();
        self.tips.clear();
        self.head_sha = None;
        self.graph = Graph::default();
        self.sizing.lanes = 0;
        self.list.reset(0);
        self.clear_selection(cx);
    }

    /// Read the first page again, whatever is being read now.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.paging.task = None;
        self.paging.reading = false;
        self.read_page(None, cx);
    }

    /// Read the page after the last one.
    pub(super) fn read_older(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.paging.next {
            self.read_page(Some(next), cx);
        }
    }

    /// Read a page: the first (`None`), which replaces what is held and
    /// brings the stash and the checkout's details, or the one starting
    /// `from` commits in, which is added below.
    fn read_page(&mut self, from: Option<usize>, cx: &mut Context<Self>) {
        let Some(root) = self.checkout.shown.clone() else {
            return;
        };
        if self.paging.reading {
            return;
        }
        self.paging.reading = true;
        self.paging.failure = None;
        cx.notify();
        let path = root.clone();
        let job = cx.background_executor().spawn(async move {
            let path = Path::new(&path);
            let page = crate::git::history(path, from.unwrap_or(0), PAGE_LEN)?;
            let stash = from
                .is_none()
                .then(|| crate::git::history::stash_commits(path));
            anyhow::Ok((page, stash))
        });
        self.paging.task = Some(cx.spawn(async move |this, cx| {
            let read = job.await;
            this.update(cx, |history, cx| {
                if history.checkout.shown.as_deref() != Some(root.as_str()) {
                    return;
                }
                history.paging.reading = false;
                match read {
                    Ok((page, stash)) => history.take_page(page, stash, from.is_none(), cx),
                    Err(error) => history.paging.failure = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn take_page(
        &mut self,
        page: HistoryPage,
        stash: Option<Vec<Commit>>,
        first: bool,
        cx: &mut Context<Self>,
    ) {
        let shown_before = self.shown_rows.len();
        let list_before = self.list_len();
        if let Some(stash) = stash {
            self.stashes = stash;
        }
        if first {
            self.commits = page.commits;
            self.tips = page.tips;
        } else {
            let held: HashSet<String> = self.commits.iter().map(|c| c.sha.clone()).collect();
            let fresh = page.commits.into_iter().filter(|c| !held.contains(&c.sha));
            self.commits.extend(fresh);
        }
        self.head_sha = page.head_sha;
        self.paging.next = page.next;
        // Only the first page describes the checkout.
        if let Some(repo) = page.state {
            self.repo = Some(repo);
            self.want_remote_photos(cx);
        }
        // The working tree may have changed while nothing was read.
        self.reread_status(cx);

        // Older commits can go straight under the rows already listed, as
        // long as nothing can land among them: a fold, a search, a stash.
        let plain = self.scope == Scope::Everything
            && self.folds.closed.is_empty()
            && self.stashes.is_empty()
            && !self.search.active();
        if first || !plain {
            self.reset_list(cx);
        } else {
            self.show_derived_rows();
            self.list
                .splice(shown_before..list_before, self.list_len() - shown_before);
        }
        self.want_author_photos(cx);
    }

    /// Fetch every remote, as the toolbar's Fetch does.
    pub fn fetch_remotes(&mut self, cx: &mut Context<Self>) {
        self.fetch_now(false, cx);
    }

    /// Fetch, then read the first page again. A `quiet` fetch keeps a
    /// failure to itself.
    pub(super) fn fetch_now(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let Some(root) = self.checkout.shown.clone() else {
            return;
        };
        if self.fetch.running {
            return;
        }
        self.fetch.running = true;
        if !quiet {
            self.fetch.error = None;
        }
        cx.notify();
        let path = root.clone();
        let job = cx.background_executor().spawn(async move {
            crate::git::actions::run(Path::new(&path), crate::git::GitAction::Fetch)
        });
        self.fetch.task = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |history, cx| {
                if history.checkout.shown.as_deref() != Some(root.as_str()) {
                    return;
                }
                history.fetch.running = false;
                match result {
                    Ok(_) => {
                        history.fetch.error = None;
                        // A page read before the fetch would miss what it
                        // brought in.
                        history.refresh(cx);
                    }
                    Err(error) if !quiet => history.fetch.error = Some(error.to_string().into()),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The quiet fetch timer for the checkout held. Leaving the checkout
    /// drops it, which stops it.
    fn schedule_fetches(&mut self, cx: &mut Context<Self>) {
        self.fetch.timer = Some(cx.spawn(async move |this, cx| {
            let mut wait = FIRST_AUTO_FETCH;
            loop {
                cx.background_executor().timer(wait).await;
                wait = AUTO_FETCH_EVERY;
                let open = this.update(cx, |history, cx| {
                    // Busy: the next round tries again.
                    if !history.fetch.running && !history.git_running {
                        history.fetch_now(true, cx);
                    }
                });
                if open.is_err() {
                    break;
                }
            }
        }));
    }

    /// The checked-out branch, or `HEAD` when detached.
    pub fn branch(&self) -> &str {
        let branch = self.repo.as_ref().and_then(|repo| repo.branch.as_deref());
        branch.unwrap_or("HEAD")
    }

    pub fn remote_url(&self) -> Option<&str> {
        Some(self.repo.as_ref()?.tracked.as_ref()?.url.as_str())
    }

    /// Whether the list ends in a row that reads older commits.
    pub(super) fn has_more_row(&self) -> bool {
        self.paging.next.is_some() && self.scope == Scope::Everything
    }

    pub(super) fn list_len(&self) -> usize {
        self.shown_rows.len() + usize::from(self.has_more_row())
    }

    /// The rows the view mode calls for, and how many commits each folded
    /// branch hides.
    ///
    /// Every commit: folded branches folded, stashes beside the commits they
    /// were taken from, and the working tree's row on top. Branch tips: one
    /// row per branch, without lines, since a tip's parent is rarely
    /// another tip; the badges say which is which.
    pub(super) fn derive_rows(&self) -> (Vec<Commit>, HashMap<String, usize>) {
        match self.scope {
            Scope::Tips => {
                let tips = self.tips.iter().map(|tip| Commit {
                    parents: Vec::new(),
                    ..tip.clone()
                });
                (tips.collect(), HashMap::new())
            }
            Scope::Everything => {
                let head = self.head_sha.as_deref();
                let (kept, hidden) = collapse_branch_runs(&self.commits, &self.folds.closed, head);
                let mut rows = merge_stash_rows(kept, &self.stashes);
                if let Some(worktree) = self.worktree_row() {
                    rows.insert(0, worktree);
                }
                (rows, hidden)
            }
        }
    }

    /// Put the derived rows on screen without animating, and lay out
    /// everything that depends on them.
    pub(super) fn show_derived_rows(&mut self) {
        let (rows, hidden) = self.derive_rows();
        self.shown_rows = rows;
        self.folds.hidden = hidden;
        self.relayout_graph();
        self.search.rematch(&self.shown_rows);
    }

    /// Derive the rows and give the list a fresh start on them.
    pub(super) fn reset_list(&mut self, cx: &mut Context<Self>) {
        self.scope_change = None;
        self.scope_task = None;
        self.show_derived_rows();
        self.list
            .reset_with_uniform_height(self.list_len(), px(ROW_HEIGHT));
        cx.notify();
    }

    /// Lay the graph out again. Its column stays as wide as the most lanes
    /// this checkout has needed, folds undone, so folding does not shift
    /// every subject sideways.
    pub(super) fn relayout_graph(&mut self) {
        let head = self.head_sha.as_deref();
        self.graph = lay_out(&self.shown_rows, head);
        let unfolded = lay_out(&self.commits, head).lanes;
        self.sizing.lanes = self.sizing.lanes.max(unfolded).max(self.graph.lanes);
    }

    /// The row for uncommitted changes, when there are any: above HEAD, and
    /// mid-merge also above what is coming in. During a conflict, its subject
    /// says what stopped and on how many files.
    pub(super) fn worktree_row(&self) -> Option<Commit> {
        let status = self
            .tree_status
            .as_ref()
            .filter(|s| !s.entries.is_empty())?;
        let head = match &self.head_sha {
            Some(head) => head.clone(),
            None => self.commits.first()?.sha.clone(),
        };
        let operation = self
            .repo
            .as_ref()
            .map(|repo| repo.operation.clone())
            .unwrap_or_default();
        let conflicted = status.entries.iter().filter(|e| e.conflicted).count();
        let subject = if conflicted == 0 {
            "Work in progress".to_owned()
        } else {
            let incoming = operation.incoming.as_deref().unwrap_or("another branch");
            let ours = operation
                .current_branch
                .as_deref()
                .unwrap_or_else(|| self.branch());
            let files = if conflicted == 1 { "file" } else { "files" };
            format!(
                "{conflicted} {files} in conflict · {} {incoming} {} {ours}",
                operation.state.verb(),
                operation.state.conjunction()
            )
        };
        let incoming = match operation.state {
            OperationState::None => None,
            _ => operation.incoming_sha.filter(|sha| *sha != head),
        };
        Some(Commit {
            sha: WORKTREE_ROW.into(),
            parents: std::iter::once(head).chain(incoming).collect(),
            subject,
            author: crate::git::Signature {
                when: chrono::Utc::now().to_rfc3339(),
                ..Default::default()
            },
            ..Commit::default()
        })
    }

    /// The line a reshape of `target` replays: its parent, itself, and on
    /// up to HEAD, with what the replay starts from. `None` when a merge is
    /// on the way, or the line runs past what is loaded.
    pub(super) fn linear_run_to(&self, target: &str) -> Option<LinearRun> {
        let parents = |sha: &str| {
            self.commits
                .iter()
                .find(|c| c.sha == sha)
                .map(|c| c.parents.as_slice())
        };
        let only_parent = |sha: &str| match parents(sha)? {
            [one] => Some(one.clone()),
            _ => None,
        };
        let mut line = vec![self.head_sha.clone()?];
        while line.last()? != target {
            let next = only_parent(line.last()?)?;
            line.push(next);
        }
        let parent = only_parent(target)?;
        let base = match parents(&parent)? {
            [] => None,
            [grandparent] => Some(grandparent.clone()),
            _ => return None,
        };
        line.push(parent);
        line.reverse();
        Some(LinearRun {
            base,
            commits: line,
        })
    }

    /// Read the working tree's status again; it decides the working tree's
    /// row.
    pub(super) fn reread_status(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.checkout.shown.clone() else {
            return;
        };
        let path = root.clone();
        let job = cx
            .background_executor()
            .spawn(async move { crate::git::status::status(Path::new(&path)) });
        self.tree_task = Some(cx.spawn(async move |this, cx| {
            let status = job.await.ok();
            this.update(cx, |history, cx| {
                if history.checkout.shown.as_deref() != Some(root.as_str()) {
                    return;
                }
                history.tree_status = status;
                history.show_derived_rows();
                cx.notify();
            })
            .ok();
        }));
    }

    /// Every commit, or only the branch tips. Back to every commit also
    /// opens every fold.
    pub(super) fn show_mode(&mut self, mode: Scope, cx: &mut Context<Self>) {
        let unfold = mode == Scope::Everything && !self.folds.closed.is_empty();
        if unfold {
            self.folds.closed.clear();
        }
        if mode != self.scope || unfold {
            self.scope = mode;
            self.scope_epoch = self.scope_epoch.wrapping_add(1);
            self.change_view(false, cx);
        }
    }

    /// Fold the branch `reference`, or open it again.
    pub(super) fn toggle_fold(&mut self, reference: RefBadge, cx: &mut Context<Self>) {
        let Some(key) = branch_ref_key(&reference) else {
            return;
        };
        if !self.folds.closed.remove(&key) {
            self.folds.closed.insert(key);
        }
        self.scope = Scope::Everything;
        self.scope_epoch = self.scope_epoch.wrapping_add(1);
        self.change_view(true, cx);
    }

    /// Copy a sha, and show it as copied for a moment.
    pub(super) fn copy_sha(&mut self, sha: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()));
        self.just_copied = Some(sha);
        self.copy_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            this.update(cx, |history, cx| {
                history.just_copied = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Select a row, and tell the side panel.
    pub(super) fn select_commit(&mut self, commit: Commit, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(commit.sha.as_str()) {
            return;
        }
        self.selected = Some(commit.sha.clone());
        cx.emit(HistoryEvent::SelectCommit(Box::new(commit)));
        cx.notify();
    }

    /// Select nothing. Runs every frame nothing is selected, so it only
    /// repaints when something was.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected.take().is_some() {
            cx.notify();
        }
    }

    /// Where the lanes begin: after the badges, when they are shown.
    pub(super) fn lane_origin_x(&self) -> f32 {
        if self.branch_column_open {
            branch_column::REFS_COLUMN_WIDTH
        } else {
            0.0
        }
    }
}
