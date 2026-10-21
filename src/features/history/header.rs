//! The band over the rows: column names, the dividers that resize them, the
//! headers that drag to reorder, and the two small menus (which columns show,
//! and how authors are shown).
//!
//! Every change goes through [`ColumnLayout::change`] into the settings, so
//! the rows and the settings page see it in the same frame.

use gpui_kit::{
    AnyElement, Context, DragMoveEvent, ElementId, Hsla, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Point, SharedString, Window, div, prelude::*, px,
};

use super::columns::{
    ColumnLayout, Divider, DividerDrag, Edge, HeaderChip, HeaderDrag, Invisible, Reorder,
    SUBJECT_MIN,
};
use super::{HistoryView, branch_column};
use crate::settings::{Authors, ColumnId, SavePolicy};
use crate::ui::icons;
use crate::ui::popover;
use crate::ui::theme::{self, Theme};

const BAND_HEIGHT: f32 = 24.0;
/// The group the band's hover is read under, to show the menu button.
const BAND_GROUP: &str = "history-header-band";

/// The gestures the header has under way.
#[derive(Default)]
pub(super) struct HeaderGestures {
    pub divider: Option<Divider>,
    pub reorder: Option<Reorder>,
}

/// The band, built before the graph's width is known; the container query
/// adds the graph's slot.
pub(super) struct HeaderParts {
    columns: AnyElement,
    button: AnyElement,
    badges: bool,
    ink: Hsla,
}

impl HeaderParts {
    pub(super) fn assemble(self, graph_slot: AnyElement) -> AnyElement {
        let badges = self.badges.then(|| {
            div()
                .flex_none()
                .w(px(branch_column::REFS_COLUMN_WIDTH))
                .pl(px(8.0))
                .child("Branch")
        });
        div()
            .id(BAND_GROUP)
            .group(BAND_GROUP)
            .relative()
            .flex_none()
            .h(px(BAND_HEIGHT))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme::hairline(0.06))
            .text_size(px(9.5))
            .text_color(self.ink)
            .children(badges)
            .child(graph_slot)
            .child(div().flex_1().min_w(px(SUBJECT_MIN)).child("Commit"))
            .child(self.columns)
            .child(self.button)
            .into_any_element()
    }
}

impl HistoryView {
    /// The width beside the graph that other columns hold: the optional
    /// columns, and the badges column when it is open.
    pub(super) fn width_beside_graph(&self, cx: &Context<Self>) -> f32 {
        let badges = if self.branch_column_open {
            branch_column::REFS_COLUMN_WIDTH
        } else {
            0.0
        };
        ColumnLayout::of(cx).visible_width() + badges
    }

    pub(super) fn header_parts(&self, theme: &Theme, cx: &mut Context<Self>) -> HeaderParts {
        let layout = ColumnLayout::of(cx);
        HeaderParts {
            columns: self.render_column_strip(&layout, theme, cx),
            button: self.render_menu_button(theme, cx),
            badges: self.branch_column_open,
            ink: theme.text_faint,
        }
    }

    /// The optional columns' headers, each with the divider at its left.
    /// The strip is where dragged headers drop.
    fn render_column_strip(
        &self,
        layout: &ColumnLayout,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let visible = layout.visible();
        let lefts = std::iter::once(Edge::Subject).chain(visible.iter().map(|&c| Edge::Column(c)));
        let cells: Vec<AnyElement> = visible
            .iter()
            .zip(lefts)
            .enumerate()
            .map(|(slot, (&column, left))| {
                self.render_column_head(layout, column, left, slot, theme, cx)
            })
            .collect();
        div()
            .id("history-header-columns")
            .h_full()
            .flex()
            .flex_shrink(1.0)
            .on_drag_move::<HeaderDrag>(cx.listener(Self::follow_header_drag))
            .on_drop::<HeaderDrag>(cx.listener(|history, drag: &HeaderDrag, _, cx| {
                history.drop_header(drag.column, cx);
            }))
            .children(cells)
            .into_any_element()
    }

    /// One column's header in visible `slot`, after `left`.
    fn render_column_head(
        &self,
        layout: &ColumnLayout,
        column: ColumnId,
        left: Edge,
        slot: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = column.name();
        // While a header is dragged over this one, a bar shows which side it
        // would land on.
        let marker = self
            .gestures
            .reorder
            .filter(|r| r.over == slot && r.over != r.from)
            .map(|r| {
                let bar = div()
                    .absolute()
                    .top(px(3.0))
                    .bottom(px(3.0))
                    .w(px(2.0))
                    .rounded_full()
                    .bg(theme.accent);
                if r.from < r.over {
                    bar.right_0()
                } else {
                    bar.left_0()
                }
            });
        let author = column == ColumnId::Author;
        div()
            .id(SharedString::from(format!("history-head-{name}")))
            .relative()
            .h_full()
            .w(px(layout.width(column)))
            .min_w(px(Edge::Column(column).limits().0))
            .flex_shrink(1.0)
            .flex()
            .items_center()
            .when(author, |head| head.justify_center())
            .cursor_pointer()
            .when(author, |head| {
                head.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|history, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        history.open_author_menu(event.position, cx);
                    }),
                )
            })
            .on_drag(
                HeaderDrag {
                    column,
                    label: name.into(),
                },
                |drag, _, _, cx| {
                    cx.stop_propagation();
                    let label = drag.label.clone();
                    cx.new(|_| HeaderChip(label))
                },
            )
            .children(marker)
            .child(name)
            // Last: its thin strip must win over the header's own drag.
            .child(self.render_divider(left, Edge::Column(column), theme, cx))
            .into_any_element()
    }

    /// The divider between `left` and `right`, straddling the left edge of
    /// `right`. A double click puts every width back to its default.
    fn render_divider(
        &self,
        left: Edge,
        right: Edge,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lit = theme.border_strong.opacity(0.7);
        div()
            .id(SharedString::from(format!(
                "history-divider-{left:?}-{right:?}"
            )))
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-3.0))
            .w(px(6.0))
            .cursor_col_resize()
            .hover(move |style| style.bg(lit))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |history, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    let columns = ColumnLayout::of(cx).columns;
                    let x = f32::from(event.position.x);
                    history.gestures.divider = Some(Divider::grab(left, right, x, &columns));
                }),
            )
            .on_drag(DividerDrag, |_, _: Point<Pixels>, _, cx| {
                cx.stop_propagation();
                cx.new(|_| Invisible)
            })
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|history, event: &MouseUpEvent, _, cx| {
                    history.gestures.divider = None;
                    if event.click_count == 2 {
                        ColumnLayout::change(SavePolicy::Debounced, cx, |layout| {
                            layout.columns.reset_widths();
                        });
                    }
                }),
            )
            .into_any_element()
    }

    /// The pointer moved during a divider drag. The widths are saved once it
    /// rests, not on every frame.
    pub(super) fn on_column_resize(
        &mut self,
        event: &DragMoveEvent<DividerDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(divider) = self.gestures.divider else {
            return;
        };
        let x = f32::from(event.event.position.x);
        ColumnLayout::change(SavePolicy::Debounced, cx, |layout| {
            divider.dragged_to(x, &mut layout.columns);
        });
    }

    fn follow_header_drag(
        &mut self,
        event: &DragMoveEvent<HeaderDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = ColumnLayout::of(cx);
        let dragged = event.drag(cx).column;
        let Some(from) = layout.visible().iter().position(|&c| c == dragged) else {
            return;
        };
        let x = f32::from(event.event.position.x - event.bounds.left());
        let over = layout.landing(x, f32::from(event.bounds.size.width));
        let now = Some(Reorder { from, over });
        if self.gestures.reorder != now {
            self.gestures.reorder = now;
            cx.notify();
        }
    }

    fn drop_header(&mut self, dragged: ColumnId, cx: &mut Context<Self>) {
        let layout = ColumnLayout::of(cx);
        let onto = self
            .gestures
            .reorder
            .take()
            .and_then(|r| layout.visible().get(r.over).copied());
        cx.notify();
        if let Some(onto) = onto.filter(|&onto| onto != dragged) {
            let order = layout.moved(dragged, onto);
            ColumnLayout::change(SavePolicy::Debounced, cx, |layout| {
                layout.columns.order = order
            });
        }
    }

    /// At the band's right end, the button for the column menu: shown while
    /// the band is hovered or the menu is up.
    fn render_menu_button(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let up = self.popups.columns.is_open();
        div()
            .id("history-header-menu-button")
            .absolute()
            .top(px(2.0))
            .right(px(3.0))
            .size(px(20.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .cursor_pointer()
            .opacity(if up { 1.0 } else { 0.0 })
            .group_hover(BAND_GROUP, |style| style.opacity(1.0))
            .hover(|style| style.bg(theme::ink(0.08)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|history, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    history.open_column_menu(event.position, cx);
                }),
            )
            .child(
                icons::icon(icons::CHECKLIST)
                    .size(px(12.0))
                    .text_color(theme.text_muted),
            )
            .into_any_element()
    }

    pub(super) fn open_column_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.close_author_menu(cx);
        self.popups.columns.open(at);
        cx.notify();
    }

    pub(super) fn close_column_menu(&mut self, cx: &mut Context<Self>) {
        self.popups
            .columns
            .dismiss(cx, |history| &mut history.popups.columns);
    }

    fn open_author_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.close_column_menu(cx);
        self.popups.author.open(at);
        cx.notify();
    }

    pub(super) fn close_author_menu(&mut self, cx: &mut Context<Self>) {
        self.popups
            .author
            .dismiss(cx, |history| &mut history.popups.author);
    }

    /// Show or hide each column, and put everything back when changed.
    pub(super) fn render_column_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let layout = ColumnLayout::of(cx);
        let mut card = small_menu(theme, 132.0)
            .on_mouse_down_out(cx.listener(|history, _, _, cx| history.close_column_menu(cx)));
        for column in ColumnId::EVERY {
            let name = column.name();
            card = card.child(
                checked_line(
                    theme,
                    &format!("history-show-{name}"),
                    name,
                    layout.is_shown(column),
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    ColumnLayout::change(SavePolicy::Immediate, cx, |layout| layout.toggle(column));
                })),
            );
        }
        if layout.customized() {
            card = card
                .child(
                    div()
                        .h(px(1.0))
                        .mx(px(5.0))
                        .my(px(2.0))
                        .bg(theme::hairline(0.08)),
                )
                .child(
                    plain_line(theme, "history-columns-defaults")
                        .text_color(theme.text_muted)
                        .child("Reset")
                        .on_click(cx.listener(|history, _, _, cx| {
                            cx.stop_propagation();
                            ColumnLayout::change(SavePolicy::Immediate, cx, |layout| {
                                *layout = ColumnLayout {
                                    authors: layout.authors,
                                    ..ColumnLayout::default()
                                };
                            });
                            history.close_column_menu(cx);
                        })),
                );
        }
        card.into_any_element()
    }

    /// Authors as photos, or with "Name" ticked, as names.
    pub(super) fn render_author_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let names = ColumnLayout::of(cx).authors == Authors::Name;
        small_menu(theme, 116.0)
            .on_mouse_down_out(cx.listener(|history, _, _, cx| history.close_author_menu(cx)))
            .child(
                checked_line(theme, "history-author-names", "Name", names).on_click(cx.listener(
                    |history, _, _, cx| {
                        cx.stop_propagation();
                        ColumnLayout::change(SavePolicy::Immediate, cx, |layout| {
                            layout.authors = match layout.authors {
                                Authors::Photo => Authors::Name,
                                Authors::Name => Authors::Photo,
                            };
                        });
                        history.close_author_menu(cx);
                    },
                )),
            )
            .into_any_element()
    }
}

fn small_menu(theme: &Theme, width: f32) -> gpui_kit::Div {
    popover::popover_card(theme)
        .w(px(width))
        .p(px(3.0))
        .rounded(px(9.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
}

fn plain_line(theme: &Theme, id: &str) -> gpui_kit::Stateful<gpui_kit::Div> {
    popover::menu_row(theme, false, SharedString::from(id.to_owned()))
        .id(ElementId::Name(id.to_owned().into()))
        .px(px(7.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .text_size(px(11.5))
}

/// A menu line with a tick at the end while `on`.
fn checked_line(
    theme: &Theme,
    id: &str,
    label: &'static str,
    on: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let tick = on.then(|| {
        icons::icon(icons::CHECK)
            .size(px(10.0))
            .text_color(theme.text_muted)
    });
    plain_line(theme, id)
        .gap(px(0.0))
        .child(div().flex_1().child(label))
        .child(
            div()
                .flex_none()
                .w(px(12.0))
                .flex()
                .justify_end()
                .children(tick),
        )
}
