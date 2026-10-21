//! The history: a page at a time of commits, each beside its place in the
//! branch graph, with a toolbar above and the context menu over the rows.
//!
//! [`HistoryView`] is the view. Where it is mounted and with what is up to
//! [`pane::HistoryPane`] and the shell. Its parts:
//!
//! * [`state`]: reading pages and the working tree, the rows shown, search.
//! * [`lanes`] and [`rail`]: where each commit sits in the graph, and how
//!   its lines are drawn.
//! * [`row`], [`node`], [`cells`]: one row, its node, and its cells.
//! * [`header`] and [`columns`]: the optional columns and their header.
//! * [`search`], [`bar`], [`menu_view`], [`actions`]: the search field, the
//!   toolbar, and the right-click menu.

use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, ListSizingBehavior, ListState, Pixels, Point,
    Render, SharedString, Task, Window, container_query, div, list, prelude::*, px,
};

use crate::git::Commit;
use crate::ui::motion::{self, AnimationExt as _};
use crate::ui::popover::{self, Layer};
use crate::ui::text_input::TextInput;
use crate::ui::theme::Theme;

pub mod actions;
pub mod bar;
pub mod branch_column;
pub mod branch_picker;
mod cells;
mod columns;
pub mod detail;
mod header;
mod lanes;
mod menu_view;
pub mod nav;
mod node;
pub mod pane;
mod photos;
mod rail;
mod row;
mod search;
mod state;
mod transition;

pub use cells::history_author_name;
use lanes::Graph;
use rail::{Morph, Sizing};
pub use search::{ScopeToggle, SearchField};
use transition::ScopeChange;

/// The id of the working tree's row. It is not a commit, so no sha can
/// collide with it.
pub(crate) const WORKTREE_ROW: &str = "(working tree)";

/// Every commit, or only the tip of each branch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Everything,
    Tips,
}

/// The history view.
pub struct HistoryView {
    checkout: state::Checkout,
    paging: state::Paging,
    /// Every commit loaded, in order.
    commits: Vec<Commit>,
    /// The rows shown: folded, with stashes and the working tree.
    shown_rows: Vec<Commit>,
    tips: Vec<Commit>,
    scope: Scope,
    /// Bumped on each change of view, to name its animation.
    scope_epoch: usize,
    scope_change: Option<ScopeChange>,
    scope_task: Option<Task<()>>,
    head_sha: Option<String>,
    /// The working tree, which decides the work-in-progress row.
    tree_status: Option<crate::git::GitStatus>,
    tree_task: Option<Task<()>>,
    /// The stash, as rows.
    stashes: Vec<Commit>,
    graph: Graph,
    sizing: Sizing,
    search: search::Search,
    folds: state::Folds,
    fetch: state::Fetching,
    photos: photos::Photos,
    popups: Popups,
    list: ListState,
    gestures: header::HeaderGestures,
    just_copied: Option<String>,
    copy_reset: Option<Task<()>>,
    /// The checkout's branch, remotes, stash and any operation under way.
    repo: Option<crate::git::RepoState>,
    git_running: bool,
    git_task: Option<Task<()>>,
    /// The toolbar command running, which lights its button.
    lit_button: Option<bar::Tool>,
    branch_column_open: bool,
    /// The row the side panel shows.
    selected: Option<String>,
    /// The row whose badges the pointer is over.
    badge_hover: branch_column::BadgeHover,
    /// The summary field on the work-in-progress row, owned by the pane.
    wip_input: Option<Entity<TextInput>>,
}

/// Everything that floats over the rows. At most one menu is up at a time;
/// the dialog of a pick can outlive its menu.
#[derive(Default)]
struct Popups {
    columns: Layer<Point<Pixels>>,
    author: Layer<Point<Pixels>>,
    pull: Layer<Point<Pixels>>,
    context: Layer<menu_view::ContextMenu>,
    pending: Option<menu_view::PendingPick>,
}

pub enum HistoryEvent {
    /// Something moved a ref, HEAD or the working tree: other views of the
    /// checkout are out of date.
    RepositoryChanged,
    /// A row was selected.
    SelectCommit(Box<Commit>),
    /// The working tree's row was selected.
    SelectWorkingTree,
}

impl EventEmitter<HistoryEvent> for HistoryView {}

/// A centred message where the rows would be.
fn notice(lines: impl IntoIterator<Item = AnyElement>) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .px(px(20.0))
        .children(lines)
        .into_any_element()
}

fn notice_line(text: impl Into<SharedString>, size: f32, color: gpui_kit::Hsla) -> AnyElement {
    div()
        .text_size(px(size))
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

/// A strip under the toolbar saying something failed.
fn failure_strip(text: SharedString, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .min_h(px(28.0))
        .flex()
        .items_center()
        .px(px(8.0))
        .py(px(6.0))
        .border_b_1()
        .border_color(theme.danger.opacity(0.16))
        .bg(theme.danger.opacity(0.05))
        .truncate()
        .text_size(px(11.0))
        .text_color(theme.danger_muted)
        .child(text)
        .into_any_element()
}

impl HistoryView {
    /// What fills the space under the header: a message, or the rows with
    /// the graph's lines over them.
    fn render_body(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.checkout.shown.is_none() {
            return notice([
                notice_line("No repository open", 12.0, theme.text_muted),
                notice_line(
                    "Open a folder that is a git repository and its history appears here.",
                    11.5,
                    theme.text_faint,
                ),
            ]);
        }
        if self.paging.reading && self.commits.is_empty() {
            return notice([
                crate::ui::loaders::glyph_spinner("history-loading", 3.0, theme.indicator)
                    .into_any_element(),
                notice_line("Loading history…", 12.0, theme.text_faint),
            ]);
        }
        if self.shown_rows.is_empty() {
            let (text, color) = match (&self.paging.failure, self.scope) {
                (Some(error), _) => (error.clone(), theme.warning),
                (None, Scope::Tips) => ("No branch tips found".into(), theme.text_faint),
                (None, Scope::Everything) => ("No commits found".into(), theme.text_faint),
            };
            return notice([notice_line(text, 12.0, color)]);
        }
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(self.graph_paths(theme, cx))
            .child(
                list(self.list.clone(), cx.processor(Self::render_row))
                    .size_full()
                    .with_sizing_behavior(ListSizingBehavior::Auto),
            )
            .into_any_element()
    }

    /// Each popup that is up, as a floating layer.
    fn render_popups(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut layers = Vec::new();
        let at = |at: Option<Point<Pixels>>, closing, id: &'static str, menu: AnyElement| {
            at.map(|position| popover::menu_at(id, position, menu, closing))
        };
        if self.popups.columns.shown().is_some() {
            let menu = self.render_column_menu(theme, cx);
            layers.extend(at(
                self.popups.columns.shown().copied(),
                self.popups.columns.leaving_since(),
                "history-columns-menu",
                menu,
            ));
        }
        if self.popups.author.shown().is_some() {
            let menu = self.render_author_menu(theme, cx);
            layers.extend(at(
                self.popups.author.shown().copied(),
                self.popups.author.leaving_since(),
                "history-author-menu",
                menu,
            ));
        }
        if self.popups.pull.shown().is_some() {
            let menu = self.render_pull_options(theme, cx);
            layers.extend(at(
                self.popups.pull.shown().copied(),
                self.popups.pull.leaving_since(),
                "history-pull-menu",
                menu,
            ));
        }
        if let Some(at) = self.popups.context.shown().map(|menu| menu.at) {
            let closing = self.popups.context.leaving_since();
            let menu = self.render_context_menu(theme, cx);
            layers.push(popover::menu_at("history-context-menu", at, menu, closing));
        }
        layers.extend(self.render_pending(window.viewport_size(), theme, window, cx));
        layers
    }
}

impl Render for HistoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_checkout(cx);
        if !cx.has_active_drag() {
            self.gestures.reorder = None;
        }
        let theme = Theme::of(cx).clone();
        let taken = self.width_beside_graph(cx);

        let toolbar = self
            .checkout
            .shown
            .is_some()
            .then(|| self.render_toolbar(&theme, cx));
        let mut failures = Vec::new();
        if let Some(error) = &self.fetch.error {
            failures.push(failure_strip(
                format!("Fetch failed: {error}").into(),
                &theme,
            ));
        }
        // With no rows, the body says it in their place.
        if let Some(error) = self
            .paging
            .failure
            .clone()
            .filter(|_| !self.shown_rows.is_empty())
        {
            failures.push(failure_strip(error, &theme));
        }
        let header = (!self.shown_rows.is_empty()).then(|| self.header_parts(&theme, cx));
        let body = self.render_body(&theme, cx);
        let popups = self.render_popups(&theme, window, cx);

        // The graph's width follows the pane's, so the header and the rows
        // sit in a container query that settles it before either is laid out.
        let sizing = self.sizing.clone();
        let sized = container_query(move |size, window, cx| {
            let morph = sizing.settle(
                f32::from(size.width),
                taken,
                window.scale_factor(),
                !cx.reduce_motion(),
            );
            let header = header.map(|parts| parts.assemble(graph_slot(&sizing, morph)));
            div()
                .size_full()
                .flex()
                .flex_col()
                .children(header)
                .child(body)
        })
        .w_full()
        .flex_1()
        .min_h_0();

        div()
            .size_full()
            .flex()
            .flex_col()
            .on_drag_move(cx.listener(Self::on_column_resize))
            .children(toolbar)
            .children(failures)
            .child(sized)
            .children(popups)
    }
}

/// The header's stand-in for the graph column, as wide as the graph is
/// drawn, easing along with it during a morph.
fn graph_slot(sizing: &Sizing, morph: Option<Morph>) -> AnyElement {
    let Some(morph) = morph else {
        return div()
            .flex_none()
            .w(px(sizing.shown.get().width()))
            .into_any_element();
    };
    let shown = sizing.shown.clone();
    div()
        .id("history-graph-slot")
        .flex_none()
        .w(px(morph.from.width()))
        .with_animation(
            SharedString::from(format!("history-graph-slot-{}", morph.epoch)),
            motion::COLLAPSE.animation(),
            move |slot, t| {
                let step = morph.from.toward(morph.to, t);
                shown.set(step);
                slot.w(px(step.width()))
            },
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, subject: &str) -> Commit {
        Commit {
            sha: sha.into(),
            subject: subject.into(),
            ..Commit::default()
        }
    }

    #[gpui_kit::test]
    fn matches_are_marked_in_place_and_the_arrows_go_round(cx: &mut gpui_kit::TestAppContext) {
        let history = cx.new(HistoryView::new);
        history.update(cx, |history, cx| {
            history.commits = vec![
                commit("c3", "Speed up the parser"),
                commit("c2", "Docs"),
                commit("c1", "Parser tests"),
            ];
            history.show_derived_rows();
            history.search_for("PARSER".into(), cx);
            assert_eq!(history.shown_rows.len(), 3, "nothing is filtered out");
            assert_eq!(
                (history.search.hits.clone(), history.search.at),
                (vec![0, 2], Some(0))
            );
            assert_eq!(history.selected, None, "typing does not select");

            history.next_hit(true, cx);
            assert_eq!(
                (history.search.at, history.selected.as_deref()),
                (Some(1), Some("c1"))
            );
            history.next_hit(true, cx);
            assert_eq!(history.search.at, Some(0));
            history.next_hit(false, cx);
            assert_eq!(history.search.at, Some(1));

            history.search_for("   ".into(), cx);
            assert!(history.search.hits.is_empty());
        });
    }

    #[gpui_kit::test]
    fn the_tips_view_has_no_lines_between_rows(cx: &mut gpui_kit::TestAppContext) {
        let history = cx.new(HistoryView::new);
        history.update(cx, |history, cx| {
            let mut tip = commit("t1", "tip");
            tip.parents = vec!["base".into()];
            history.tips = vec![tip];
            history.show_mode(Scope::Tips, cx);
            assert_eq!(history.shown_rows.len(), 1);
            assert!(history.shown_rows[0].parents.is_empty());
        });
    }
}
