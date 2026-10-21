//! One repository's history screen: what it contains, its graph, and the
//! selected commit.
//!
//! Three regions. [`super::nav::RepoNav`] lists what the repository contains.
//! The middle is the graph, with its own strip of controls above it.
//! [`super::detail::DetailPanel`] keeps the selected commit's files on the
//! right, which is what a diff is opened from.
//!
//! Picking a file opens its patch **in the graph's own place**, with an ✕
//! that puts the graph back. One patch at a time: the next one replaces it,
//! so reading a commit file by file never accumulates anything to clean up,
//! and the window's tab strip stays a strip of projects. While a patch (or
//! the merge editor) is open the navigation panel is dropped: the
//! patch and the panel it was opened from share the width.
//!
//! The commit panel exists only while something is selected. Its ✕ drops the
//! selection, and the graph takes the width back.
//!
//! Selecting a row does not open anything: it costs one scoped read and moves
//! the panel. A patch is opened deliberately, because opening one per click
//! would make browsing a history unreadable.

use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use super::branch_picker::{BranchPicker, BranchPickerEvent};
use super::detail::{DetailPanel, PanelEvent};
use super::nav::{RepoNav, RepoNavEvent};
use super::{HistoryEvent, HistoryView, ScopeToggle, SearchField};
use crate::features::diff::DiffView;
use crate::features::diff::view::{DiffSource, DiffViewEvent};
use crate::settings::SavePolicy;
use crate::ui::chrome::{CONTROL_RADIUS, CONTROL_SIZE, EDGE_INSET};
use crate::ui::splitter::{SplitterDrag, ratio_from_pointer, splitter};
use crate::ui::text_input::TextInput;
use crate::ui::theme::{Theme, wash};
use crate::ui::toast::{self, ToastKind};
use crate::ui::typography::ui_rems;
use crate::ui::{icons, motion};

/// The divider between the graph (or the patch) and the commit panel.
const DETAIL_SPLITTER: &str = "history-detail-splitter";

/// Height of the strip above the graph. The same 24px control on a 36px band
/// the rest of the chrome uses, so the two read as one bar.
const HEADER_HEIGHT: f32 = 36.0;

/// The patch currently standing in for the graph.
struct OpenDiff {
    /// Its full identity, so picking the same row again is not a reopen.
    key: String,
    view: Entity<DiffView>,
    _subscription: Subscription,
}

pub struct HistoryPane {
    repo: Option<String>,
    nav: Entity<RepoNav>,
    graph: Entity<HistoryView>,
    detail: Entity<DetailPanel>,
    /// The patch standing in for the graph, when one is open.
    diff: Option<OpenDiff>,
    /// The strip above the graph: the branch that is out, and the controls
    /// that change what the graph reads.
    branch_picker: Entity<BranchPicker>,
    search: Entity<SearchField>,
    view_button: Entity<ScopeToggle>,
    focus_handle: FocusHandle,
    /// The git-dir watch for the current checkout: its task reloads the pane
    /// when the repository moves under someone else's feet. Cancelled and
    /// replaced on `open_repo`.
    watch_tasks: Vec<Task<()>>,
    watch: Option<crate::git::watch::RepoWatch>,
    _subscriptions: Vec<Subscription>,
}

impl HistoryPane {
    pub fn new(repo: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let graph = cx.new(|cx| match repo.clone() {
            Some(cwd) => HistoryView::with_repo(cwd, cx),
            None => HistoryView::new(cx),
        });
        let detail = cx.new(|cx| DetailPanel::new(repo.clone(), window, cx));
        let nav = cx.new(|cx| RepoNav::new(repo.clone(), window, cx));
        let branch_picker = cx.new(|cx| BranchPicker::new(repo.clone(), window, cx));
        let search = cx.new(|cx| SearchField::new(graph.clone(), window, cx));
        let view_button = cx.new(|cx| ScopeToggle::new(graph.clone(), cx));

        let mut subscriptions = Vec::new();
        // The picker names the branch the graph read: it follows every page.
        subscriptions.push(cx.observe(&graph, {
            let branch_picker = branch_picker.clone();
            move |_, graph, cx| {
                let branch = SharedString::from(graph.read(cx).branch().to_string());
                branch_picker.update(cx, |picker, cx| picker.set_branch(branch, cx));
            }
        }));
        subscriptions.push(cx.subscribe(
            &branch_picker,
            |pane: &mut Self, _, event, cx| match event {
                BranchPickerEvent::RepositoryChanged => {
                    pane.reload(cx);
                    cx.notify();
                }
                BranchPickerEvent::ActionFailed(message) => {
                    toast::show(ToastKind::Error, message.to_string(), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(&graph, {
            let detail = detail.clone();
            move |pane: &mut Self, _, event, cx| match event {
                // A row was picked: the panel follows the selection rather
                // than opening anything, which is what makes arrowing through
                // the history readable.
                HistoryEvent::SelectCommit(commit) => {
                    let commit = (**commit).clone();
                    detail.update(cx, |detail, cx| detail.show(commit, cx));
                }
                HistoryEvent::SelectWorkingTree => {
                    detail.update(cx, |detail, cx| detail.show_working_tree(cx));
                }
                // Refs, HEAD or the tree moved: everything on screen was read
                // before that.
                HistoryEvent::RepositoryChanged => {
                    detail.update(cx, |detail, cx| detail.reread(cx));
                    pane.nav.update(cx, |nav, cx| nav.reload(cx));
                    pane.reload_diff(cx);
                    cx.notify();
                }
            }
        }));
        subscriptions.push(
            cx.subscribe(&detail, |pane: &mut Self, _, event, cx| match event {
                PanelEvent::OpenCommitFile { commit, path } => {
                    pane.open_file(commit.sha.clone(), path.clone(), cx);
                }
                PanelEvent::OpenTreeFile { path, staged } => {
                    pane.open_working_file(path.clone(), *staged, cx);
                }
                // The panel committed the working tree, so the graph has a commit
                // it has never read.
                PanelEvent::TreeChanged => {
                    pane.graph.update(cx, |graph, cx| graph.refresh(cx));
                    pane.nav.update(cx, |nav, cx| nav.reload(cx));
                    pane.reload_diff(cx);
                }
                PanelEvent::OpenConflict(path) => {
                    pane.open_conflict(path.clone(), cx);
                }
            }),
        );
        // The panel's file list is what prev/next walks in an open patch,
        // and it changes as the working tree does.
        subscriptions.push(cx.observe(&detail, |pane: &mut Self, detail, cx| {
            if let Some(open) = &pane.diff {
                let conflict = matches!(
                    open.view.read(cx).source(),
                    Some(DiffSource::Conflict { .. })
                );
                let files = nav_files(detail.read(cx), conflict);
                open.view.update(cx, |view, cx| view.set_files(files, cx));
            }
            cx.notify();
        }));
        subscriptions.push(
            cx.subscribe(&nav, |pane: &mut Self, _, event, cx| match event {
                RepoNavEvent::RepositoryChanged => {
                    pane.graph.update(cx, |graph, cx| graph.refresh(cx));
                    pane.detail.update(cx, |detail, cx| detail.reread(cx));
                    pane.reload_diff(cx);
                    cx.notify();
                }
                RepoNavEvent::ActionFailed(message) => {
                    toast::show(ToastKind::Error, message.to_string(), cx);
                }
            }),
        );

        // The WIP row carries the commit summary in place (placeholder
        // `// WIP`), the same text as the
        // commit panel's summary: typing in either shows in the other. Each
        // side copies the other on every notify — typing, clearing after a
        // commit, a merge prefill — and `set_text` ignores an equal text,
        // which is what stops the echo.
        let wip_input = cx.new(|cx| {
            TextInput::new("// WIP", window, cx)
                .with_text_size(12.0)
                .compact()
        });
        graph.update(cx, |graph, _| graph.set_wip_input(wip_input.clone()));
        let summary_input = detail.read(cx).summary_input();
        subscriptions.push(cx.observe_in(&wip_input, window, {
            let summary_input = summary_input.clone();
            move |_, wip, window, cx| {
                let text = wip.read(cx).text().clone();
                summary_input.update(cx, |input, cx| input.set_text(text, window, cx));
            }
        }));
        subscriptions.push(cx.observe_in(&summary_input, window, {
            let wip_input = wip_input.clone();
            move |_, summary, window, cx| {
                let text = summary.read(cx).text().clone();
                wip_input.update(cx, |input, cx| input.set_text(text, window, cx));
            }
        }));

        let mut watch_tasks = Vec::new();
        let mut watch = None;
        if let Some(repo) = repo.clone() {
            Self::start_watching(repo, &mut watch, &mut watch_tasks, cx);
        }

        Self {
            repo,
            nav,
            graph,
            detail,
            diff: None,
            branch_picker,
            search,
            view_button,
            focus_handle: cx.focus_handle(),
            watch_tasks,
            watch,
            _subscriptions: subscriptions,
        }
    }

    /// Watch this checkout's git dir and reload the pane when it moves.
    ///
    /// This is how a commit, merge or checkout made outside the application
    /// — the terminal, an editor, another client — shows up: git always
    /// writes the same state files, and the watcher sees them. The reload is
    /// the pane-wide one, so the graph, the working tree, the nav and any
    /// open patch all re-read together.
    fn start_watching(
        repo: String,
        watch: &mut Option<crate::git::watch::RepoWatch>,
        tasks: &mut Vec<Task<()>>,
        cx: &mut Context<Self>,
    ) {
        let watcher = crate::git::watch::RepoWatch::spawn(std::path::Path::new(&repo));
        let changes = watcher.changes();
        *watch = Some(watcher);
        tasks.push(cx.spawn(async move |this, cx| {
            loop {
                changes.next().await;
                // Let the rest of the burst land, then read once.
                cx.background_executor()
                    .timer(crate::git::watch::SETTLE)
                    .await;
                changes.clear();
                this.update(cx, |pane, cx| pane.reload(cx)).ok();
            }
        }));
    }

    /// Point the screen at another checkout.
    pub fn open_repo(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        self.repo = repo.clone();
        // The patch on screen was read out of the previous one, and the
        // watcher was pointed at it too.
        self.diff = None;
        self.watch_tasks.clear();
        self.watch = None;
        if let Some(repo) = repo.clone() {
            Self::start_watching(repo, &mut self.watch, &mut self.watch_tasks, cx);
        }
        self.graph
            .update(cx, |graph, cx| graph.open_repo(repo.clone(), cx));
        self.detail
            .update(cx, |detail, cx| detail.open_repo(repo.clone(), cx));
        self.branch_picker
            .update(cx, |picker, cx| picker.open_repo(repo.clone(), cx));
        self.nav.update(cx, |nav, cx| nav.open_repo(repo, cx));
        cx.notify();
    }

    /// Read this repository again — something moved a ref, here or elsewhere.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.graph.update(cx, |graph, cx| {
            graph.refresh(cx);
            // The tree decides whether the WIP row exists and what it counts,
            // and a change from outside never went through `run_git`,
            // which is the only other place status is refreshed.
            graph.reread_status(cx);
        });
        self.detail.update(cx, |detail, cx| detail.reread(cx));
        self.nav.update(cx, |nav, cx| nav.reload(cx));
        self.reload_diff(cx);
    }

    /// Put the merge editor where the graph is. One at a time: the next
    /// file replaces the one before it.
    fn open_conflict(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let operation = self.detail.read(cx).operation().clone();
        // The two sides are named "Commit <sha> on
        // <branch>". A rebase swaps them — your commits are the ones being
        // replayed onto the other branch.
        let rebasing = operation.state == crate::git::conflicts::OperationState::Rebasing;
        let (ours_branch, theirs_branch) = if rebasing {
            (operation.incoming.clone(), operation.current_branch.clone())
        } else {
            (operation.current_branch.clone(), operation.incoming.clone())
        };
        let ours = side_label(operation.head_sha.as_deref(), ours_branch.as_deref());
        let theirs = side_label(operation.incoming_sha.as_deref(), theirs_branch.as_deref());
        self.open_diff(DiffSource::Conflict { repo, path }, cx);
        if let Some(open) = &self.diff {
            open.view
                .update(cx, |view, cx| view.set_conflict_labels(ours, theirs, cx));
        }
    }

    /// The open patch was read before whatever just moved.
    fn reload_diff(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = &self.diff {
            open.view.update(cx, |view, cx| view.reload(cx));
        }
    }

    fn open_file(&mut self, sha: String, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        // The working-tree row is not a commit, so git has nothing to show
        // for it. One of its files opens as a patch of the working tree,
        // pinned to that path.
        let source = if sha == super::WORKTREE_ROW {
            DiffSource::WorkingTreeFile {
                repo,
                path,
                staged: false,
            }
        } else {
            DiffSource::CommitFile { repo, sha, path }
        };
        self.open_diff(source, cx);
    }

    /// One file of the working tree, on the side of the index it sits on.
    fn open_working_file(&mut self, path: String, staged: bool, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.open_diff(DiffSource::WorkingTreeFile { repo, path, staged }, cx);
    }

    /// Put this patch where the graph is. The same one again is a no-op, and a
    /// different one replaces it.
    fn open_diff(&mut self, source: DiffSource, cx: &mut Context<Self>) {
        let key = source.key();
        if self.diff.as_ref().is_some_and(|open| open.key == key) {
            return;
        }
        let conflict = matches!(source, DiffSource::Conflict { .. });
        let files = nav_files(self.detail.read(cx), conflict);
        let view = cx.new(DiffView::new);
        view.update(cx, |view, cx| {
            view.set_files(files, cx);
            view.show(source, cx);
        });
        let subscription = cx.subscribe(&view, |pane: &mut Self, _, event, cx| match event {
            DiffViewEvent::Close => pane.close_diff(cx),
            // The file moved between the index and the tree: the panel's
            // lists and the WIP row were read before that.
            DiffViewEvent::WorkingTreeChanged => {
                pane.graph.update(cx, |graph, cx| graph.reread_status(cx));
                pane.detail.update(cx, |detail, cx| detail.reread(cx));
            }
            // Saved and marked resolved: the file is settled, and the
            // panel's list is what comes next.
            DiffViewEvent::ConflictResolved => {
                pane.close_diff(cx);
                pane.reload(cx);
            }
        });
        self.diff = Some(OpenDiff {
            key,
            view,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// The ✕ of the commit panel: nothing is selected any more.
    fn close_detail(&mut self, cx: &mut Context<Self>) {
        self.detail.update(cx, |detail, cx| detail.clear(cx));
        self.graph.update(cx, |graph, cx| graph.clear_selection(cx));
        cx.notify();
    }

    /// Put the graph back.
    fn close_diff(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        cx.notify();
    }

    /// The strip above the commit panel: what the panel
    /// shows, and its buttons — copy the sha and open it on the remote for a
    /// commit, and the ✕ for both.
    fn render_detail_header(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = Theme::of(cx).clone();
        let detail = self.detail.read(cx);
        let sha = detail.selected().map(str::to_string);
        let label = match &sha {
            Some(sha) if !detail.shows_working_tree() => {
                format!("commit: {}", sha.chars().take(7).collect::<String>())
            }
            _ => "working tree".to_string(),
        };
        let remote = sha.as_ref().and_then(|sha| {
            let hosted = crate::git::forge::hosted(self.graph.read(cx).remote_url()?)?;
            Some(hosted.page(crate::git::forge::Page::Commit(sha)))
        });
        let button = |id: &'static str, glyph: &'static str, hint: &'static str| {
            div()
                .id(id)
                .size(px(CONTROL_SIZE))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(CONTROL_RADIUS))
                .cursor_pointer()
                .bg(motion::fade_color(id, wash(0.0), wash(0.10)))
                .on_hover(motion::on_hover_fade(id))
                .tooltip(crate::ui::tip::tip(hint))
                .child(
                    icons::icon(glyph)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
        };
        div()
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .child(
                div()
                    .font_family(theme.font_mono.clone())
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(label)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .when_some(sha.filter(|_| !detail.shows_working_tree()), |row, sha| {
                        row.child(
                            button(
                                "history-detail-copy-sha",
                                icons::COPY_01,
                                "Copy the full commit SHA to the clipboard",
                            )
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                    sha.clone(),
                                ));
                            }),
                        )
                    })
                    .when_some(remote, |row, url| {
                        row.child(
                            button(
                                "history-detail-open-remote",
                                icons::LINK_SQUARE_02,
                                "Open this commit on the remote host in your browser",
                            )
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                    })
                    .child(
                        button(
                            "history-detail-close",
                            icons::CANCEL_01,
                            "Close the details panel",
                        )
                        .on_click(cx.listener(|pane, _, _, cx| pane.close_detail(cx))),
                    ),
            )
    }

    /// The strip above the graph: the branch selector, and the graph's own
    /// controls.
    fn render_header(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = Theme::of(cx).clone();
        div()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(EDGE_INSET))
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(self.branch_picker.clone())
            .child(div().flex_1().min_w_0())
            .child(self.search.clone())
            .child(self.view_button.clone())
    }
}

impl HistoryPane {
    /// The graph (or the patch) and, while something is selected, the commit
    /// panel beside it, split by a divider that drags. The
    /// navigation panel stays outside the split, so the ratio is of the width the two actually share.
    fn render_split(
        &mut self,
        middle: gpui_kit::AnyElement,
        detail_header: Option<gpui_kit::Div>,
        corner: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let ratio = crate::settings::current(cx).history.panel.share;
        let has_detail = detail_header.is_some();
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_row()
            .on_drag_move(
                cx.listener(|_, event: &gpui_kit::DragMoveEvent<SplitterDrag>, _, cx| {
                    if event.drag(cx).0.as_ref() != DETAIL_SPLITTER {
                        return;
                    }
                    let ratio = 1.0 - ratio_from_pointer(event.event.position.x, event.bounds);
                    crate::settings::update(SavePolicy::Debounced, cx, |settings| {
                        settings.history.panel.share = crate::settings::Panel::bounded_share(ratio);
                    });
                    cx.notify();
                }),
            )
            .child(
                // `min_w_0` on the middle: without it the flexible side keeps
                // its content-based minimum width and the panel is pushed off
                // the window instead of the rows shrinking.
                div()
                    .when(has_detail, |el| {
                        el.flex_grow(1.0 - ratio).flex_basis(px(0.0))
                    })
                    .when(!has_detail, |el| el.flex_1())
                    .min_w_0()
                    .h_full()
                    .child(middle),
            )
            .when_some(detail_header, |row, header| {
                row.child(splitter(DETAIL_SPLITTER, theme, true, |_, cx| {
                    crate::settings::update(SavePolicy::Immediate, cx, |settings| {
                        settings.history.panel.share = crate::settings::Panel::DEFAULT_SHARE;
                    });
                    cx.refresh_windows();
                }))
                .child(
                    div()
                        .flex_grow(ratio)
                        .flex_basis(px(0.0))
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        .rounded_br(px(corner))
                        .bg(theme.surface)
                        .child(header)
                        .child(div().flex_1().min_h_0().child(self.detail.clone())),
                )
            })
    }
}

impl Focusable for HistoryPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HistoryPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The pane fills the window's bottom edge: whatever paints into its
        // bottom corners rounds them (see `ui::window_frame`).
        let corner = crate::ui::window_frame::corner_radius(window);
        // The patch stands in for the graph, and the navigation panel is
        // dropped while it is open: the patch and the panel it was
        // opened from are what is being read.
        let open_view = self.diff.as_ref().map(|open| open.view.clone());
        let reading = open_view.is_some();
        let middle = match open_view {
            Some(view) => view.into_any_element(),
            None => div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.render_header(cx))
                .child(div().flex_1().min_h_0().w_full().child(self.graph.clone()))
                .into_any_element(),
        };
        let detail_header = self
            .detail
            .read(cx)
            .has_selection()
            .then(|| self.render_detail_header(cx));
        let theme = Theme::of(cx).clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .rounded_b(px(corner))
            .bg(theme.bg)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_row()
                    .when(!reading, |row| row.child(self.nav.clone()))
                    .child(self.render_split(middle, detail_header, corner, &theme, cx)),
            )
    }
}

/// What prev/next walks in an open patch: the panel's files, or — in the
/// merge editor — the files still in conflict.
fn nav_files(
    detail: &DetailPanel,
    conflict: bool,
) -> Vec<crate::features::diff::view::DiffNavFile> {
    if !conflict {
        return detail.nav_files();
    }
    detail
        .conflicted_files()
        .into_iter()
        .map(|path| crate::features::diff::view::DiffNavFile {
            path,
            staged: false,
        })
        .collect()
}

/// "Commit abc1234 on main": how a merge side is named.
fn side_label(sha: Option<&str>, branch: Option<&str>) -> String {
    let commit = match sha {
        Some(sha) => format!("Commit {}", sha.chars().take(7).collect::<String>()),
        None => "Incoming commit".to_string(),
    };
    match branch {
        Some(branch) => format!("{commit} on {branch}"),
        None => commit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::theme::init(cx);
            crate::settings::init(cx);
        });
    }

    fn pane(cx: &mut TestAppContext) -> gpui_kit::WindowHandle<HistoryPane> {
        init(cx);
        cx.add_window(|window, cx| HistoryPane::new(Some("/repos/one".to_string()), window, cx))
    }

    #[gpui_kit::test]
    fn a_file_opens_in_the_graphs_place_and_the_close_button_puts_it_back(cx: &mut TestAppContext) {
        let window = pane(cx);
        window
            .update(cx, |pane, _, cx| {
                assert!(pane.diff.is_none(), "the graph is what opens");

                pane.open_file("abc1234".into(), "src/main.rs".into(), cx);
                let open = pane.diff.as_ref().expect("the patch is on screen");
                assert_eq!(
                    open.view.read(cx).source().map(DiffSource::path),
                    Some("src/main.rs")
                );

                pane.close_diff(cx);
                assert!(pane.diff.is_none(), "the graph is back");
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn one_patch_at_a_time_and_the_same_one_twice_is_not_a_reopen(cx: &mut TestAppContext) {
        let window = pane(cx);
        window
            .update(cx, |pane, _, cx| {
                pane.open_file("abc1234".into(), "src/main.rs".into(), cx);
                let first = pane.diff.as_ref().expect("a patch").view.clone();

                // The same row again: the view it is already showing stays,
                // rather than being read a second time.
                pane.open_file("abc1234".into(), "src/main.rs".into(), cx);
                let same = pane.diff.as_ref().expect("a patch").view.clone();
                assert_eq!(first.entity_id(), same.entity_id());

                // A different file replaces it — one patch, never a pile.
                pane.open_file("abc1234".into(), "src/shell.rs".into(), cx);
                let open = pane.diff.as_ref().expect("a patch");
                assert_eq!(
                    open.view.read(cx).source().map(DiffSource::path),
                    Some("src/shell.rs")
                );
                assert_ne!(first.entity_id(), open.view.entity_id());
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn the_working_tree_row_opens_a_working_tree_file_not_a_bad_revision(cx: &mut TestAppContext) {
        let window = pane(cx);
        window
            .update(cx, |pane, _, cx| {
                pane.open_file(
                    super::super::WORKTREE_ROW.to_string(),
                    "src/main.rs".into(),
                    cx,
                );
                let open = pane.diff.as_ref().expect("a patch");
                // Not `git show`: the working tree has no sha.
                assert!(matches!(
                    open.view.read(cx).source(),
                    Some(DiffSource::WorkingTreeFile { .. })
                ));
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn opening_another_checkout_drops_the_patch_read_from_the_last_one(cx: &mut TestAppContext) {
        let window = pane(cx);
        window
            .update(cx, |pane, _, cx| {
                pane.open_file("abc1234".into(), "src/main.rs".into(), cx);
                assert!(pane.diff.is_some());
                pane.open_repo(Some("/repos/two".into()), cx);
                assert!(pane.diff.is_none());
            })
            .expect("window update");
    }

    /// The one check that cannot be made by reading the code: both states of
    /// the middle region actually paint, with the panels either side of them.
    #[gpui_kit::test]
    fn the_graph_and_a_patch_both_paint(cx: &mut TestAppContext) {
        init(cx);
        let window = cx
            .add_window(|window, cx| HistoryPane::new(Some("/repos/one".to_string()), window, cx));
        let pane = window.root(cx).expect("the pane is the window's root");
        let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        let draw = |visual: &mut gpui_kit::VisualTestContext| {
            visual.draw(
                gpui_kit::point(px(0.0), px(0.0)),
                gpui_kit::size(px(1280.0), px(800.0)),
                |_, _| gpui_kit::Empty,
            );
        };
        draw(&mut visual);

        visual.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                pane.open_file("abc1234".into(), "src/main.rs".into(), cx)
            })
        });
        draw(&mut visual);
    }
}
