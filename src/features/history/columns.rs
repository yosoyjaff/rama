//! The optional columns beside each commit's subject (author, date, sha):
//! which show, in what order, how wide.
//!
//! The settings are the one place this is kept. [`ColumnLayout`] is a copy
//! read from them for a frame, with the arithmetic the header's gestures
//! need; a change is written straight back, so the header, the rows and the
//! settings page always agree.

use gpui_kit::{
    App, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px,
};

use crate::settings::{self, Authors, ColumnId, Columns, SavePolicy};
use crate::ui::theme::Theme;

/// The narrowest the subject column gets.
pub(super) const SUBJECT_MIN: f32 = 80.0;

/// The column settings, as one frame sees them.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct ColumnLayout {
    pub columns: Columns,
    pub authors: Authors,
}

impl ColumnLayout {
    pub fn of(cx: &App) -> Self {
        settings::read(cx, |s| Self {
            columns: s.history.columns.clone(),
            authors: s.history.authors,
        })
    }

    /// Edit the layout and save it as `policy` says.
    pub fn change(policy: SavePolicy, cx: &mut App, edit: impl FnOnce(&mut ColumnLayout)) {
        let mut layout = Self::of(cx);
        edit(&mut layout);
        settings::update(policy, cx, move |s| {
            s.history.columns = layout.columns;
            s.history.authors = layout.authors;
        });
    }

    pub fn is_shown(&self, id: ColumnId) -> bool {
        self.columns.get(id).shown
    }

    pub fn toggle(&mut self, id: ColumnId) {
        let column = self.columns.get_mut(id);
        column.shown = !column.shown;
    }

    /// The columns on screen, left to right.
    pub fn visible(&self) -> Vec<ColumnId> {
        let shown = |id: &ColumnId| self.is_shown(*id);
        self.columns.order.iter().copied().filter(shown).collect()
    }

    pub fn width(&self, id: ColumnId) -> f32 {
        self.columns.get(id).width
    }

    /// What the visible columns take together.
    pub fn visible_width(&self) -> f32 {
        self.visible().into_iter().map(|id| self.width(id)).sum()
    }

    /// Anything other than the defaults, how authors show aside.
    pub fn customized(&self) -> bool {
        self.columns != Columns::default()
    }

    /// The slot among the visible columns that a header dragged to `x`
    /// lands in. `x` is measured from the columns' left edge, over a strip
    /// drawn `drawn` wide: flex can squeeze it below the widths' sum, so `x`
    /// is scaled back first. A slot is won by passing a column's middle.
    pub fn landing(&self, x: f32, drawn: f32) -> usize {
        let visible = self.visible();
        if visible.is_empty() || drawn <= 0.0 {
            return 0;
        }
        let x = x.clamp(0.0, drawn) * self.visible_width() / drawn;
        let mut edge = 0.0;
        visible
            .iter()
            .position(|&id| {
                let middle = edge + self.width(id) / 2.0;
                edge += self.width(id);
                x < middle
            })
            .unwrap_or(visible.len() - 1)
    }

    /// The order with `dragged` taking `onto`'s place: past it going right,
    /// before it going left. Hidden columns stay where they are.
    pub fn moved(&self, dragged: ColumnId, onto: ColumnId) -> Vec<ColumnId> {
        let mut order = self.columns.order.clone();
        let from = order.iter().position(|&c| c == dragged);
        let to = order.iter().position(|&c| c == onto);
        if let (Some(from), Some(to)) = (from, to) {
            let id = order.remove(from);
            order.insert(to, id);
        }
        order
    }
}

/// One side of a divider: the subject, which takes what is left over, the
/// branch column before the graph, or a column with a width of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Edge {
    Subject,
    Branch,
    Column(ColumnId),
}

impl Edge {
    pub fn width(self, columns: &Columns) -> f32 {
        match self {
            Edge::Subject => SUBJECT_MIN,
            Edge::Branch => columns.branch,
            Edge::Column(id) => columns.get(id).width,
        }
    }

    /// The narrowest and widest it may be dragged to.
    pub fn limits(self) -> (f32, f32) {
        match self {
            Edge::Subject => (SUBJECT_MIN, f32::INFINITY),
            Edge::Branch => Columns::BRANCH_LIMITS,
            Edge::Column(id) => id.limits(),
        }
    }

    fn write(self, columns: &mut Columns, width: f32) {
        match self {
            Edge::Subject => {}
            Edge::Branch => columns.branch = width,
            Edge::Column(id) => columns.get_mut(id).width = width,
        }
    }
}

/// A divider being dragged, as it was when the drag began.
#[derive(Debug, Clone, Copy)]
pub(super) struct Divider {
    pub left: Edge,
    pub right: Edge,
    /// The pointer's x at the start.
    pub grabbed_at: f32,
    pub left_was: f32,
    pub right_was: f32,
}

impl Divider {
    pub fn grab(left: Edge, right: Edge, x: f32, columns: &Columns) -> Self {
        Self {
            left,
            right,
            grabbed_at: x,
            left_was: left.width(columns),
            right_was: right.width(columns),
        }
    }

    /// `columns` with the divider dragged to `x`.
    ///
    /// Beside the subject, only the other column changes: the subject takes
    /// up the slack. Between two columns, width passes from one to the other
    /// with their sum fixed, and stops where either would leave its limits.
    pub fn dragged_to(&self, x: f32, columns: &mut Columns) {
        let shift = x - self.grabbed_at;
        let (right_min, right_max) = self.right.limits();
        let (left_min, left_max) = self.left.limits();
        if self.left == Edge::Subject {
            let right = (self.right_was - shift).clamp(right_min, right_max);
            self.right.write(columns, right);
            return;
        }
        if self.right == Edge::Subject {
            let left = (self.left_was + shift).clamp(left_min, left_max);
            self.left.write(columns, left);
            return;
        }
        let sum = self.left_was + self.right_was;
        let low = left_min.max(sum - right_max);
        let high = left_max.min(sum - right_min);
        let left = (self.left_was + shift).clamp(low, high);
        self.left.write(columns, left);
        self.right.write(columns, sum - left);
    }
}

/// A header being dragged to a new place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reorder {
    /// Its slot among the visible columns when picked up.
    pub from: usize,
    /// The slot it would land in now.
    pub over: usize,
}

/// What a header drag carries.
#[derive(Clone)]
pub(super) struct HeaderDrag {
    pub column: ColumnId,
    pub label: SharedString,
}

/// What a divider drag carries: nothing, the divider itself is the feedback.
pub(super) struct DividerDrag;

/// Drawn under the pointer while a divider moves: nothing.
pub(super) struct Invisible;

impl Render for Invisible {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

/// Drawn under the pointer while a header moves: a chip with its name.
pub(super) struct HeaderChip(pub SharedString);

impl Render for HeaderChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .h(px(24.0))
            .min_w(px(64.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.surface_raised)
            .shadow_md()
            .opacity(0.9)
            .text_size(px(10.5))
            .text_color(theme.text_muted)
            .child(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ColumnId::{Author, Date, Sha};

    fn layout(order: Vec<ColumnId>) -> ColumnLayout {
        let mut columns = Columns {
            order,
            ..Columns::default()
        };
        columns.author.width = 120.0;
        columns.date.width = 90.0;
        columns.sha.width = 60.0;
        ColumnLayout {
            columns,
            ..ColumnLayout::default()
        }
    }

    #[test]
    fn a_hidden_column_is_skipped_but_holds_its_place() {
        let mut columns = layout(vec![Date, Sha, Author]);
        columns.toggle(Sha);
        assert_eq!(columns.visible(), [Date, Author]);
        assert_eq!(columns.moved(Author, Date), [Author, Date, Sha]);
        assert_eq!(columns.visible_width(), 210.0);
    }

    #[test]
    fn a_moved_header_passes_its_target() {
        let columns = layout(vec![Author, Date, Sha]);
        assert_eq!(columns.moved(Date, Sha), [Author, Sha, Date]);
        assert_eq!(columns.moved(Sha, Author), [Sha, Author, Date]);
        assert_eq!(columns.moved(Author, Author), [Author, Date, Sha]);
    }

    #[test]
    fn landing_follows_column_middles_at_any_drawn_width() {
        // Author 0–120 (middle 60), Date 120–210 (165), Sha 210–270 (240).
        let columns = layout(vec![Author, Date, Sha]);
        assert_eq!(columns.landing(59.0, 270.0), 0);
        assert_eq!(columns.landing(61.0, 270.0), 1);
        assert_eq!(columns.landing(239.0, 270.0), 2);
        assert_eq!(columns.landing(500.0, 270.0), 2);
        // Squeezed to a third, the same split points scale with it.
        assert_eq!(columns.landing(21.0, 90.0), 1);
        assert_eq!(columns.landing(5.0, 0.0), 0);
    }

    #[test]
    fn beside_the_subject_only_the_right_column_resizes() {
        let layout = layout(vec![Author, Date, Sha]);
        let divider = Divider::grab(Edge::Subject, Edge::Column(Date), 400.0, &layout.columns);
        let mut columns = layout.columns.clone();
        divider.dragged_to(420.0, &mut columns);
        assert_eq!((columns.date.width, columns.author.width), (70.0, 120.0));
        divider.dragged_to(-1000.0, &mut columns);
        assert_eq!(columns.date.width, Date.limits().1);
    }

    #[test]
    fn the_branch_column_resizes_alone_within_its_limits() {
        let layout = layout(vec![Author, Date, Sha]);
        let divider = Divider::grab(Edge::Branch, Edge::Subject, 160.0, &layout.columns);
        let mut columns = layout.columns.clone();
        divider.dragged_to(200.0, &mut columns);
        assert_eq!(columns.branch, Columns::DEFAULT_BRANCH + 40.0);
        assert_eq!(columns.author.width, 120.0);
        divider.dragged_to(-1000.0, &mut columns);
        assert_eq!(columns.branch, Columns::BRANCH_LIMITS.0);
    }

    #[test]
    fn between_columns_the_sum_holds_within_both_limits() {
        let layout = layout(vec![Author, Date, Sha]);
        let divider = Divider::grab(
            Edge::Column(Author),
            Edge::Column(Date),
            0.0,
            &layout.columns,
        );
        let mut columns = layout.columns.clone();
        divider.dragged_to(-15.0, &mut columns);
        assert_eq!((columns.author.width, columns.date.width), (105.0, 105.0));
        divider.dragged_to(999.0, &mut columns);
        assert_eq!(columns.date.width, Date.limits().0);
        assert_eq!(columns.author.width + columns.date.width, 210.0);
    }

    #[test]
    fn customized_ignores_how_authors_are_shown() {
        let mut columns = ColumnLayout {
            authors: Authors::Name,
            ..ColumnLayout::default()
        };
        assert!(!columns.customized());
        columns.toggle(Date);
        assert!(columns.customized());
    }
}
