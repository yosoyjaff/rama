//! The panel beside the graph. It shows a commit (its message, author and
//! the files it touched), the working tree (the changes on each side of the
//! index, the commit box, and the way through a stopped merge or rebase),
//! or nothing.
//!
//! Picking a row reads only that commit's list of files. A diff opens when
//! a file is clicked, so stepping through the history opens no tabs.
//!
//! * [`change`]: what happened to a file, and how many of each.
//! * [`listing`]: the order and the folder tree files are listed in.
//! * [`work`]: reading git and changing the working tree, off the UI thread.
//! * [`rows`], [`commit_pane`] and [`tree_pane`]: the drawing.

use gpui_kit::{Context, Entity, EventEmitter, Render, Window, div, prelude::*, px};

use crate::features::diff::view::DiffNavFile;
use crate::git::Commit;
use crate::git::conflicts::OperationStatus;
use crate::settings::{self, SavePolicy};
use crate::ui::text_input::TextInput;
use crate::ui::theme::Theme;

mod change;
mod commit_pane;
mod listing;
mod rows;
mod tree_pane;
mod work;

pub use change::{Change, Tally};

use listing::Listing;
use work::{CommitFiles, Worktree};

/// What the panel shows.
enum Shown {
    Nothing,
    Commit(Box<Commit>),
    WorkingTree,
}

#[derive(Clone)]
pub enum PanelEvent {
    /// The diff of one file of a commit.
    OpenCommitFile { commit: Box<Commit>, path: String },
    /// The diff of one file of the working tree, on the side of the index
    /// it is listed on.
    OpenTreeFile { path: String, staged: bool },
    /// The working tree changed from here; other views of it are stale.
    TreeChanged,
    /// The merge editor, on this file.
    OpenConflict(String),
}

/// The side panel.
pub struct DetailPanel {
    repo: Option<String>,
    shown: Shown,
    listing: Listing,
    commit: CommitFiles,
    worktree: Worktree,
    summary: Entity<TextInput>,
    description: Entity<TextInput>,
    /// Empty the commit box on the next render, where the window is.
    clear_message: bool,
}

impl EventEmitter<PanelEvent> for DetailPanel {}

impl DetailPanel {
    pub fn new(repo: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panel = &settings::current(cx).history.panel;
        let listing = Listing {
            as_tree: panel.tree,
            by_name: panel.by_name,
            ..Listing::default()
        };
        Self {
            repo,
            shown: Shown::Nothing,
            listing,
            commit: CommitFiles::default(),
            worktree: Worktree::default(),
            summary: cx.new(|cx| TextInput::new("Commit summary", window, cx)),
            description: cx.new(|cx| TextInput::multi_line("Description", 6, window, cx)),
            clear_message: false,
        }
    }

    /// The summary field, which the graph's working-tree row edits too.
    pub fn summary_input(&self) -> Entity<TextInput> {
        self.summary.clone()
    }

    pub fn has_selection(&self) -> bool {
        !matches!(self.shown, Shown::Nothing)
    }

    pub fn shows_working_tree(&self) -> bool {
        matches!(self.shown, Shown::WorkingTree)
    }

    pub fn selected(&self) -> Option<&str> {
        match &self.shown {
            Shown::Commit(commit) => Some(&commit.sha),
            _ => None,
        }
    }

    /// Show `commit` and read its files.
    pub fn show(&mut self, commit: Commit, cx: &mut Context<Self>) {
        if self.selected() == Some(commit.sha.as_str()) {
            return;
        }
        self.listing.opened.clear();
        self.shown = Shown::Commit(Box::new(commit));
        self.read_shown(cx);
        cx.notify();
    }

    /// Show the working tree, where commits are made.
    pub fn show_working_tree(&mut self, cx: &mut Context<Self>) {
        if self.shows_working_tree() {
            return;
        }
        self.listing.opened.clear();
        self.shown = Shown::WorkingTree;
        self.read_shown(cx);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.has_selection() {
            self.shown = Shown::Nothing;
            self.commit = CommitFiles::default();
            cx.notify();
        }
    }

    /// Read again: the repository changed under the panel.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        self.commit.source = None;
        self.worktree.source = None;
        self.read_shown(cx);
        cx.notify();
    }

    /// Follow another checkout.
    pub fn open_repo(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        if self.repo == repo {
            return;
        }
        self.repo = repo;
        self.commit = CommitFiles::default();
        self.worktree.source = None;
        self.worktree.status = None;
        self.read_shown(cx);
        cx.notify();
    }

    /// The files in the panel, in its order, for a diff's previous and next.
    pub fn nav_files(&self) -> Vec<DiffNavFile> {
        let side = |files: Vec<crate::git::DiffFileSummary>, staged: bool| {
            files.into_iter().map(move |file| DiffNavFile {
                path: file.path,
                staged,
            })
        };
        match &self.shown {
            Shown::Nothing => Vec::new(),
            Shown::Commit(_) => side(self.commit.listed().to_vec(), false).collect(),
            Shown::WorkingTree => {
                let staging = self.worktree.staging();
                side(staging.unstaged, false)
                    .chain(side(staging.staged, true))
                    .collect()
            }
        }
    }

    /// The paths still in conflict, which the merge editor steps through.
    pub fn conflicted_files(&self) -> Vec<String> {
        self.worktree.conflicted()
    }

    /// The merge, rebase or cherry-pick under way.
    pub fn operation(&self) -> &OperationStatus {
        &self.worktree.operation
    }

    /// List files as a tree or flat, and remember it.
    fn set_tree(&mut self, as_tree: bool, cx: &mut Context<Self>) {
        self.listing.as_tree = as_tree;
        settings::update(SavePolicy::Immediate, cx, |s| {
            s.history.panel.tree = as_tree
        });
        cx.notify();
    }

    /// Sort files by name or keep git's order, and remember it.
    fn flip_by_name(&mut self, cx: &mut Context<Self>) {
        self.listing.by_name = !self.listing.by_name;
        let by_name = self.listing.by_name;
        settings::update(SavePolicy::Immediate, cx, |s| {
            s.history.panel.by_name = by_name
        });
        cx.notify();
    }
}

impl Render for DetailPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fill_message(window, cx);
        let theme = Theme::of(cx).clone();
        match &self.shown {
            Shown::Commit(commit) => {
                let commit = commit.clone();
                self.render_commit(&commit, &theme, cx)
            }
            Shown::WorkingTree => self.render_worktree(&theme, window, cx),
            Shown::Nothing => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.0))
                .px(px(16.0))
                .text_center()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.text_muted)
                        .child("Nothing selected"),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.text_faint)
                        .child("Click a commit in the graph to list its files here."),
                )
                .into_any_element(),
        }
    }
}
