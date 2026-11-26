//! The tab strip: a plate for each tab the pane has open.
//!
//! A plate holds the tab's glyph and its title. Tabs keep their
//! full width while they fit; when the band runs short they all give way
//! together, the selected one only down to [`SELECTED_MIN_WIDTH`]. A title
//! too long for its plate fades out over its last few pixels rather than
//! ending in an ellipsis, and shows in full in a tooltip. On hover a close ✕
//! sits over the plate's right end.
//!
//! The strip knows nothing about what a tab contains: it takes what to draw
//! and reports which plate was pressed.
//!
//! The same plates can instead stack down a column, as a sidebar.
//!
//! When even the narrowest plates do not fit, the strip scrolls, with nothing
//! drawn for it — no bar, no shadow: the wheel moves the plates, and the
//! owner brings the active one into view with [`reveal`], on the
//! [`gpui_kit::ScrollHandle`] it passes in.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyElement, App, AppContext as _, ContentMask, ElementId, Entity, FontWeight, Hsla,
    IntoElement, MouseButton, Pixels, Point, ShapedLine, SharedString, TextAlign, TextRun, Window,
    canvas, div, prelude::*, px,
};

use crate::ui::motion;
use crate::ui::theme::{Theme, wash};
use crate::ui::typography::ui_rems;

/// A tab's width while the band has room for every one.
pub const MAX_WIDTH: f32 = 224.0;
/// The selected tab shrinks with the others only down to this.
pub const SELECTED_MIN_WIDTH: f32 = 112.0;
/// Below this a tab is only its glyph, and the strip scrolls instead.
pub const MIN_WIDTH: f32 = 40.0;
/// The plate's height, inside the title band.
pub const PLATE_HEIGHT: f32 = 27.0;
/// A row's height when the tabs run down a column.
pub const ROW_HEIGHT: f32 = 28.0;
/// The gap between plates.
pub const GAP: f32 = 2.0;
/// The title's line.
const TITLE_HEIGHT: f32 = 20.0;
/// Between the glyph and the title.
const GLYPH_GAP: f32 = 8.0;
/// The close button, and the room the title leaves for it.
const CLOSE_SIZE: f32 = 20.0;
const CLOSE_ROOM: f32 = 26.0;
/// How far from its end an overflowing title starts to fade.
const FADE: f32 = 40.0;
/// Between a group plate's dot, name and chevron.
const GROUP_GAP: f32 = 6.0;
/// A group's wash while a dragged tab is over it.
const DROP_WASH: f32 = 0.16;
/// The bar on the side of a tab where a dragged one would land.
const DROP_MARK: f32 = 2.0;
/// How far a drop zone reaches past its plate's edge: to put the bar in the
/// middle of the gap between two plates.
const MARK_OUTSET: f32 = (GAP + DROP_MARK) / 2.0;
/// How much faster a group plate's tab title shrinks than its name.
const GROUP_TITLE_SHRINK: f32 = 8.0;
/// A column's section label, and the space above one after the first.
pub const SECTION_HEIGHT: f32 = 28.0;
const SECTION_GAP: f32 = 8.0;
/// Where a group's tabs start down a column, under its name.
const NESTED_INDENT: f32 = 16.0;

/// Scroll the strip just far enough to show plate `index`, and no further.
///
/// Plates change width as the band does, so where one sits comes from the
/// last layout. A plate opened since then has none yet; tabs open at the
/// end, so the strip goes to its end, which layout clamps to the last plate.
/// Before the first frame there is no viewport at all, and the move waits
/// for one.
pub fn reveal(scroll: &gpui_kit::ScrollHandle, index: usize, window: &Window) {
    let viewport = scroll.bounds();
    if viewport.size.width <= px(0.0) {
        let scroll = scroll.clone();
        window.on_next_frame(move |window, _| {
            if scroll.bounds().size.width > px(0.0) {
                reveal(&scroll, index, window);
            }
        });
        return;
    }
    // The strip runs across the band or down a column. The plates fill the
    // other axis, so it is always in view there and only the run moves.
    let mut offset = scroll.offset();
    match scroll.bounds_for_item(index) {
        Some(plate) => {
            offset.x = reveal_offset(
                f32::from(plate.left() - viewport.left()),
                f32::from(plate.right() - viewport.left()),
                f32::from(offset.x),
                f32::from(viewport.size.width),
            )
            .into();
            offset.y = reveal_offset(
                f32::from(plate.top() - viewport.top()),
                f32::from(plate.bottom() - viewport.top()),
                f32::from(offset.y),
                f32::from(viewport.size.height),
            )
            .into();
        }
        None => {
            offset.x = (-f32::MAX).into();
            offset.y = (-f32::MAX).into();
        }
    }
    scroll.set_offset(offset);
}

/// The offset along one axis that shows a plate spanning `left..right` (in the
/// strip's unscrolled coordinates) in a viewport `viewport` wide, from
/// offset `current`: unchanged if it is already in full view, else the plate
/// lands flush against the nearer edge.
fn reveal_offset(left: f32, right: f32, current: f32, viewport: f32) -> f32 {
    if left + current < 0.0 {
        -left
    } else if right + current > viewport {
        (viewport - right).min(0.0)
    } else {
        current
    }
}

/// What one tab says.
pub struct Tab {
    /// Stable across frames: the tab's identity for hover and click.
    pub id: ElementId,
    /// Group key for the hover swap inside the tab.
    pub group: SharedString,
    pub title: SharedString,
    /// A glyph path, or `None` for a tab whose title says it all.
    pub icon: Option<&'static str>,
    pub active: bool,
    /// Whether this tab can be closed. The pane's home tab cannot.
    pub closable: bool,
    /// A field editing the title, drawn in its place.
    pub editor: Option<AnyElement>,
    /// Listed under a group's heading, down a column: set in from the
    /// loose tabs.
    pub nested: bool,
}

/// A tab group. Down a column it is a heading over its tabs that a click
/// folds; across the band it is one plate standing for all of them, which
/// the owner opens as a dropdown.
pub struct GroupLabel {
    pub id: ElementId,
    pub name: SharedString,
    pub color: Hsla,
    /// Folded, down a column.
    pub collapsed: bool,
    /// The tab on screen is one of the group's.
    pub active: bool,
    /// That tab's title, shown after the group's name across the band.
    pub current: Option<SharedString>,
    /// A field editing the name, drawn in its place.
    pub editor: Option<AnyElement>,
}

/// A column's section label — Groups, Projects — with its button.
pub struct Section {
    pub label: SharedString,
    pub action: Option<AnyElement>,
}

/// One thing in the strip.
pub enum StripItem {
    Tab(Tab),
    Group(GroupLabel),
    Section(Section),
}

pub type Handler = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// A tab dropped: what was dragged, and where it landed.
pub type DropHandler = Rc<dyn Fn(&TabDrag, DropAt, &mut Window, &mut App)>;

/// Where a dragged tab was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAt {
    /// The strip around the plates.
    Strip,
    /// The group at this position: into it.
    On(usize),
    /// Beside the tab at this position — after it when `after`, before it
    /// otherwise — in its group.
    Beside { tab: usize, after: bool },
    /// Just before the group at this position, outside it: the left half of
    /// its plate across the band, the way between two groups that touch.
    Before(usize),
    /// Just after the group at this position: only for a group dragged.
    After(usize),
}
pub type MenuHandler = Rc<dyn Fn(usize, Point<Pixels>, &mut Window, &mut App)>;

/// What the strip reports, each with the item's position in the strip. A
/// group's `select` is its click; it has no `rename` or `close`.
#[derive(Clone)]
pub struct StripEvents {
    pub select: Handler,
    /// A double click.
    pub rename: Handler,
    pub close: Handler,
    /// A right click, or a group's options button, with where it landed in
    /// the window.
    pub menu: MenuHandler,
    /// A tab dropped on a plate or on the strip.
    pub drop: DropHandler,
}

/// What a tab carries while it is dragged: where it came from, and what the
/// chip under the pointer shows.
pub struct TabDrag {
    pub from: DragFrom,
    title: SharedString,
    icon: Option<&'static str>,
    /// A group's colour, shown as its dot.
    color: Option<Hsla>,
}

/// Where a dragged tab came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragFrom {
    /// A plate, by its position in the strip.
    Strip(usize),
    /// Somewhere the owner drew it — a dropdown row — by the owner's own
    /// number for it.
    Owner(usize),
    /// A whole group, by its plate's or heading's position in the strip.
    Group(usize),
}

impl DragFrom {
    fn is_group(self) -> bool {
        matches!(self, DragFrom::Group(_))
    }
}

impl TabDrag {
    pub fn new(from: DragFrom, title: SharedString, icon: Option<&'static str>) -> Self {
        Self {
            from,
            title,
            icon,
            color: None,
        }
    }

    /// The group at strip position `position`, dragged whole.
    fn group(position: usize, name: SharedString, color: Hsla) -> Self {
        Self {
            from: DragFrom::Group(position),
            title: name,
            icon: None,
            color: Some(color),
        }
    }

    /// The chip that follows the pointer: what `on_drag` builds.
    pub fn chip(&self, cx: &mut App) -> Entity<TabDragChip> {
        let (title, icon, color) = (self.title.clone(), self.icon, self.color);
        cx.new(|_| TabDragChip { title, icon, color })
    }
}

/// Drawn under the pointer while a tab is dragged: its glyph and title.
pub struct TabDragChip {
    title: SharedString,
    icon: Option<&'static str>,
    color: Option<Hsla>,
}

impl Render for TabDragChip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .h(px(PLATE_HEIGHT))
            .max_w(px(MAX_WIDTH))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(GLYPH_GAP))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.surface_raised)
            .shadow_md()
            .opacity(0.9)
            .text_size(ui_rems(13.0))
            .text_color(theme.text)
            .children(self.icon.map(|path| TabGlyph { path }))
            .children(
                self.color
                    .map(|color| div().flex_none().size(px(8.0)).rounded_full().bg(color)),
            )
            .child(div().min_w_0().truncate().child(self.title.clone()))
    }
}

/// Draw the strip. `scroll` is the strip's own offset, which [`reveal`]
/// moves just far enough to show an item. `trailing` follows the last plate,
/// as close to it as the plates are to each other. `column` stacks the plates
/// down a sidebar, each as wide as it, instead of across the band.
/// `dragging` says a drag is under way: across the band the strip then
/// stretches over the free width, where a tab dropped leaves its group.
pub fn tab_strip(
    items: Vec<StripItem>,
    scroll: &gpui_kit::ScrollHandle,
    trailing: Option<AnyElement>,
    column: bool,
    dragging: bool,
    events: StripEvents,
) -> AnyElement {
    let drop = events.drop.clone();
    let mut strip = div()
        .id("tab-strip")
        .flex()
        .gap(px(GAP))
        .map(|strip| match column {
            true => strip
                .flex_col()
                .flex_1()
                .w_full()
                .min_h_0()
                .overflow_y_scroll(),
            false => strip
                .flex_row()
                .items_center()
                .min_w_0()
                .overflow_x_scroll(),
        })
        .track_scroll(scroll);
    // Around the plates, a dropped tab leaves its group.
    let around = drop.clone();
    strip =
        strip.on_drop(move |drag: &TabDrag, window, cx| around(drag, DropAt::Strip, window, cx));
    for (index, item) in items.into_iter().enumerate() {
        strip = strip.child(match item {
            StripItem::Tab(tab) => plate(index, tab, column, dragging, events.clone()),
            StripItem::Group(label) if column => {
                group_heading(index, label, dragging, events.clone())
            }
            StripItem::Group(label) => group_plate(index, label, dragging, events.clone()),
            StripItem::Section(section) => section_label(index, section),
        });
    }
    let strip = strip.children(trailing.map(|trailing| div().flex_none().child(trailing)));
    if column {
        // A column already runs the sidebar's length.
        return strip.into_any_element();
    }
    // The same wrapper either way, so the strip keeps its place in the tree.
    div()
        .h_full()
        .min_w_0()
        .flex()
        .items_center()
        .when(dragging, |zone| {
            zone.flex_1()
                .on_drop(move |drag: &TabDrag, window, cx| drop(drag, DropAt::Strip, window, cx))
        })
        .child(strip)
        .into_any_element()
}

/// A column's section label. One after the first sits a gap below what is
/// above it.
fn section_label(index: usize, section: Section) -> AnyElement {
    div()
        .h(px(SECTION_HEIGHT))
        .when(index > 0, |label| label.mt(px(SECTION_GAP)))
        .flex_none()
        // The label lines up with the rows' glyphs; its button sits where
        // their close buttons do, 4px from the edge.
        .pl(px(8.0))
        .pr(px(4.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .text_size(ui_rems(12.0))
        .font_weight(FontWeight::MEDIUM)
        .child(SectionText(section.label))
        .children(section.action)
        .into_any_element()
}

#[derive(IntoElement)]
struct SectionText(SharedString);

impl RenderOnce for SectionText {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex_1()
            .text_color(Theme::of(cx).text_muted)
            .child(self.0)
    }
}

/// A group down a column: a chevron and its name. Folded, the chevron gives
/// way to the group's colour until the pointer comes over it; an options
/// button shows on hover.
fn group_heading(
    index: usize,
    label: GroupLabel,
    dragging: bool,
    events: StripEvents,
) -> AnyElement {
    let StripEvents {
        select,
        rename,
        menu,
        drop,
        ..
    } = events;
    let drag = label
        .editor
        .is_none()
        .then(|| TabDrag::group(index, label.name.clone(), label.color));
    let options_menu = menu.clone();
    let key = SharedString::from(format!("tab-group-heading-{index}"));
    let collapsed = label.collapsed;
    let chevron = HeadingChevron {
        collapsed,
        key: key.clone(),
    };
    let mark = div()
        .size(px(16.0))
        .flex_none()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .when(collapsed, |mark| {
            mark.child(
                div()
                    .size(px(8.0))
                    .rounded_full()
                    .bg(label.color)
                    .group_hover(key.clone(), |style| style.invisible()),
            )
        })
        .child(chevron);
    let name = match label.editor {
        Some(editor) => div().flex_1().min_w_0().child(editor).into_any_element(),
        None => HeadingName {
            id: label.id.clone(),
            name: label.name,
            key: key.clone(),
        }
        .into_any_element(),
    };
    div()
        .id(label.id)
        .group(key.clone())
        .relative()
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex_none()
        .pl(px(6.0))
        .pr(px(8.0))
        .rounded(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GLYPH_GAP))
        .cursor_pointer()
        .text_size(ui_rems(13.0))
        .font_weight(FontWeight::MEDIUM)
        .opacity(0.65)
        .when(!collapsed, |heading| heading.bg(wash(0.05)))
        .hover(|style| style.bg(wash(0.08)).opacity(1.0))
        .child(mark)
        .child(name)
        .child(
            div()
                .id(("tab-group-options", index))
                .absolute()
                .right(px(4.0))
                .top(px((ROW_HEIGHT - CLOSE_SIZE) / 2.0))
                .size(px(CLOSE_SIZE))
                .rounded(px(4.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .invisible()
                .group_hover(key, |style| style.visible())
                .hover(|style| style.bg(wash(0.12)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    options_menu(index, event.position(), window, cx);
                })
                .children((0..3).map(|_| OptionsDot)),
        )
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            menu(index, event.position, window, cx);
        })
        // Dragged, the group moves.
        .when_some(drag, |heading, drag| {
            heading.on_drag(drag, |drag, _, _, cx| {
                cx.stop_propagation();
                drag.chip(cx)
            })
        })
        // A tab dragged here joins the group. (Down a column the groups
        // have a section of their own: there is no between.) A group
        // dragged here goes above or below it, by the half it is over.
        .when(dragging, |heading| {
            heading
                .child(group_half(index, false, true, drop.clone()))
                .child(group_half(index, true, true, drop.clone()))
        })
        .drag_over::<TabDrag>(|style, drag, _, _| match drag.from.is_group() {
            true => style,
            false => style.bg(wash(DROP_WASH)),
        })
        .on_drop(move |drag: &TabDrag, window, cx| drop(drag, DropAt::On(index), window, cx))
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            // Each click of a double one folds or opens the group: the second
            // undoes the first, and the name is edited as a tab's would be.
            select(index, window, cx);
            if event.click_count() == 2 {
                rename(index, window, cx);
            }
        })
        .into_any_element()
}

/// A group across the band: one plate for all its tabs, its colour, its
/// name and, while one of them is on screen, that tab's title. A click opens
/// the dropdown that lists them.
fn group_plate(index: usize, label: GroupLabel, dragging: bool, events: StripEvents) -> AnyElement {
    let StripEvents {
        select,
        rename,
        menu,
        drop,
        ..
    } = events;
    let key = SharedString::from(format!("tab-group-plate-{index}"));
    let lit = label.active;
    let drag = label
        .editor
        .is_none()
        .then(|| TabDrag::group(index, label.name.clone(), label.color));
    div()
        .id(label.id.clone())
        .group(key.clone())
        .relative()
        .h(px(PLATE_HEIGHT))
        .flex_none()
        .max_w(px(MAX_WIDTH))
        .pl(px(10.0))
        .pr(px(8.0))
        .rounded(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GROUP_GAP))
        .cursor_pointer()
        .text_size(ui_rems(13.0))
        .when(lit, |plate| plate.bg(wash(0.10)))
        .when(!lit, |plate| {
            plate
                .bg(motion::fade_color(&key, wash(0.04), wash(0.07)))
                .on_hover(motion::on_hover_fade(&key))
        })
        .child(
            div()
                .flex_none()
                .size(px(8.0))
                .rounded_full()
                .bg(label.color),
        )
        .child(GroupPlateText {
            id: label.id.clone(),
            name: label.name,
            current: label.current,
            editor: label.editor,
            lit,
            key,
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            menu(index, event.position, window, cx);
        })
        // Dragged, the group moves.
        .when_some(drag, |plate, drag| {
            plate.on_drag(drag, |drag, _, _, cx| {
                cx.stop_propagation();
                drag.chip(cx)
            })
        })
        // A tab dragged here joins the group; over the left half it goes
        // just before it instead. A group goes before it or after it.
        .when(dragging, |plate| {
            plate
                .child(before_group(index, drop.clone()))
                .child(group_half(index, true, false, drop.clone()))
        })
        .drag_over::<TabDrag>(|style, drag, _, _| match drag.from.is_group() {
            true => style,
            false => style.bg(wash(DROP_WASH)),
        })
        .on_drop(move |drag: &TabDrag, window, cx| drop(drag, DropAt::On(index), window, cx))
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            // The first click of a double one opened the dropdown; the second
            // edits what the plate shows, as a tab's double click does.
            if event.click_count() == 2 {
                rename(index, window, cx);
            } else {
                select(index, window, cx);
            }
        })
        .into_any_element()
}

/// A group heading's name, cut with an ellipsis when it is too long and
/// then shown in full in a tooltip.
#[derive(IntoElement)]
struct HeadingName {
    id: ElementId,
    name: SharedString,
    key: SharedString,
}

impl RenderOnce for HeadingName {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = Arc::new(self.id);
        let overflows = overflow_state(&id, window, cx);
        let tip = overflows
            .read(cx)
            .then(|| crate::ui::tip::tip(self.name.clone()));
        div()
            .id(ElementId::NamedChild(id, "name".into()))
            .relative()
            .flex_1()
            .min_w_0()
            .truncate()
            .group_hover(self.key, |style| style.pr(px(CLOSE_ROOM)))
            .when_some(tip, |name, tip| name.tooltip(tip))
            .child(self.name.clone())
            .child(overflow_probe(self.name, overflows))
    }
}

/// One line of text that takes the rest of a row, cut with an ellipsis when
/// it does not fit and then shown in full in a tooltip. `id` comes from the
/// row's own: it keys what last frame measured.
#[derive(IntoElement)]
pub struct TruncatedLabel {
    id: ElementId,
    text: SharedString,
}

impl TruncatedLabel {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
        }
    }
}

impl RenderOnce for TruncatedLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = Arc::new(self.id);
        let overflows = overflow_state(&id, window, cx);
        let tip = overflows
            .read(cx)
            .then(|| crate::ui::tip::tip(self.text.clone()));
        div()
            .id(ElementId::NamedChild(id, "label".into()))
            .relative()
            .flex_1()
            .min_w_0()
            .truncate()
            .when_some(tip, |label, tip| label.tooltip(tip))
            .child(self.text.clone())
            .child(overflow_probe(self.text, overflows))
    }
}

/// A half of the group at strip position `index` that only a dragged group
/// marks: the far half (`after`) or the near one — down a `column` the
/// bottom or top of its heading, across the band the right of its plate.
/// It lets the pointer through to the group, which a dragged tab joins.
fn group_half(index: usize, after: bool, column: bool, drop: DropHandler) -> impl IntoElement {
    let half = gpui_kit::relative(0.5);
    let out = px(-MARK_OUTSET);
    let zone = div().absolute();
    let zone = match (column, after) {
        (false, _) => zone.top_0().bottom_0().right(out).w(half),
        (true, false) => zone.left_0().right_0().top(out).h(half),
        (true, true) => zone.left_0().right_0().bottom(out).h(half),
    };
    let id = match after {
        true => ("tab-group-after", index),
        false => ("tab-group-above", index),
    };
    zone.id(id)
        .drag_over::<TabDrag>(move |style, drag, _, cx| {
            if !drag.from.is_group() {
                return style;
            }
            let style = match (column, after) {
                (false, _) => style.border_r(px(DROP_MARK)),
                (true, false) => style.border_t(px(DROP_MARK)),
                (true, true) => style.border_b(px(DROP_MARK)),
            };
            style.border_color(Theme::of(cx).accent)
        })
        .on_drop(move |drag: &TabDrag, window, cx| {
            let at = match (drag.from.is_group(), after) {
                (false, _) => DropAt::On(index),
                (true, true) => DropAt::After(index),
                (true, false) => DropAt::Before(index),
            };
            drop(drag, at, window, cx)
        })
}

/// The left half of the group plate at strip position `index` while a tab
/// is dragged: dropped there, the tab goes just before the group, outside
/// it — the way between two groups that touch. It blocks the plate under it,
/// so only one of the two lights; the wheel still reaches the strip.
fn before_group(index: usize, drop: DropHandler) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        // In the gap, where the tab before it draws the same bar.
        .left(px(-MARK_OUTSET))
        .w(gpui_kit::relative(0.5))
        .id(("tab-group-before", index))
        .block_mouse_except_scroll()
        .drag_over::<TabDrag>(|style, _, _, cx| {
            style
                .border_l(px(DROP_MARK))
                .border_color(Theme::of(cx).accent)
        })
        .on_drop(move |drag: &TabDrag, window, cx| drop(drag, DropAt::Before(index), window, cx))
}

/// One half of the tab at strip position `index` while a tab is dragged:
/// the far half (`after`) or the near one, across the band left and right,
/// down a `column` top and bottom. Dropped there, the tab lands on that side.
/// The half under the pointer shows a bar on its outer edge. It holds no
/// layout, so the bar moves nothing; it blocks the plate under it, but not
/// the wheel.
fn beside_tab(index: usize, after: bool, column: bool, drop: DropHandler) -> impl IntoElement {
    let half = gpui_kit::relative(0.5);
    // Out past the edge, so the bar sits in the middle of the gap: the far
    // half of one tab and the near half of the next mean the same place,
    // and draw it in the same place.
    let out = px(-MARK_OUTSET);
    let zone = div().absolute();
    let zone = match (column, after) {
        (false, false) => zone.top_0().bottom_0().left(out).w(half),
        (false, true) => zone.top_0().bottom_0().right(out).w(half),
        (true, false) => zone.left_0().right_0().top(out).h(half),
        (true, true) => zone.left_0().right_0().bottom(out).h(half),
    };
    let id = match after {
        true => ("tab-drop-after", index),
        false => ("tab-drop-before", index),
    };
    zone.id(id)
        .block_mouse_except_scroll()
        .drag_over::<TabDrag>(move |style, _, _, cx| {
            let style = match (column, after) {
                (false, false) => style.border_l(px(DROP_MARK)),
                (false, true) => style.border_r(px(DROP_MARK)),
                (true, false) => style.border_t(px(DROP_MARK)),
                (true, true) => style.border_b(px(DROP_MARK)),
            };
            style.border_color(Theme::of(cx).accent)
        })
        .on_drop(move |drag: &TabDrag, window, cx| {
            drop(drag, DropAt::Beside { tab: index, after }, window, cx)
        })
}

/// A group heading's chevron. Folded, it hides until the pointer comes
/// over the heading, where the group's colour stands for it.
#[derive(IntoElement)]
struct HeadingChevron {
    collapsed: bool,
    key: SharedString,
}

impl RenderOnce for HeadingChevron {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .when(self.collapsed, |slot| {
                slot.invisible()
                    .group_hover(self.key, |style| style.visible())
            })
            .child(
                crate::ui::icons::icon(if self.collapsed {
                    crate::ui::icons::ALT_ARROW_RIGHT
                } else {
                    crate::ui::icons::ALT_ARROW_DOWN
                })
                .size(px(12.0))
                .text_color(theme.text),
            )
    }
}

/// One of the three dots on a group heading's options button.
#[derive(IntoElement)]
struct OptionsDot;

impl RenderOnce for OptionsDot {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .size(px(3.0))
            .rounded_full()
            .bg(Theme::of(cx).text_muted)
    }
}

/// A group plate's name and the title of its tab on screen. The title gives
/// way first; whichever is cut, a tooltip shows both in full.
#[derive(IntoElement)]
struct GroupPlateText {
    id: ElementId,
    name: SharedString,
    current: Option<SharedString>,
    editor: Option<AnyElement>,
    lit: bool,
    key: SharedString,
}

impl RenderOnce for GroupPlateText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if let Some(editor) = self.editor {
            return div().w(px(120.0)).child(editor).into_any_element();
        }
        let id = Arc::new(self.id);
        let name_overflows = overflow_state(
            &Arc::new(ElementId::NamedChild(id.clone(), "name".into())),
            window,
            cx,
        );
        let current_overflows = overflow_state(
            &Arc::new(ElementId::NamedChild(id.clone(), "current".into())),
            window,
            cx,
        );
        let cut = *name_overflows.read(cx) || *current_overflows.read(cx);
        let tip = cut.then(|| {
            let text = match &self.current {
                Some(current) if !self.name.trim().is_empty() => {
                    format!("{}\n{current}", self.name)
                }
                Some(current) => current.to_string(),
                None => self.name.to_string(),
            };
            crate::ui::tip::tip(text)
        });
        let theme = Theme::of(cx);
        let (rest, lit) = (theme.text_muted, theme.text);
        div()
            .id(ElementId::NamedChild(id, "text".into()))
            .min_w_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GROUP_GAP))
            .text_color(if self.lit { lit } else { rest })
            .group_hover(self.key, move |style| style.text_color(lit))
            .when_some(tip, |text, tip| text.tooltip(tip))
            .child(
                div()
                    .relative()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.name.clone())
                    .child(overflow_probe(self.name, name_overflows)),
            )
            .children(self.current.map(|current| {
                div()
                    .relative()
                    .min_w_0()
                    // The title gives way well before the name does.
                    .flex_shrink(GROUP_TITLE_SHRINK)
                    .truncate()
                    .text_color(rest)
                    .child(current.clone())
                    .child(overflow_probe(current, current_overflows))
            }))
            .child(
                crate::ui::icons::icon(crate::ui::icons::ALT_ARROW_DOWN)
                    .flex_none()
                    .size(px(10.0))
                    .text_color(rest),
            )
            .into_any_element()
    }
}

fn plate(index: usize, tab: Tab, column: bool, dragging: bool, events: StripEvents) -> AnyElement {
    let StripEvents {
        select: on_select,
        rename: on_rename,
        close: on_close,
        menu: on_menu,
        drop: on_drop,
    } = events;
    // A project tab can be dragged onto a group; Home cannot, nor a tab
    // whose name is being edited.
    let drag = (tab.closable && tab.editor.is_none())
        .then(|| TabDrag::new(DragFrom::Strip(index), tab.title.clone(), tab.icon));
    let group = tab.group.clone();
    let fade = tab.group.clone();
    let closable = tab.closable;
    // Across the band every tab is a plate, so a row of them reads as one
    // control. Down a column only the selected one is lit, as in any
    // sidebar list: plates there would read as all selected.
    let (rest, hover) = match column {
        true => (0.0, 0.06),
        false => (0.04, 0.07),
    };
    let height = if column { ROW_HEIGHT } else { PLATE_HEIGHT };
    div()
        .id(tab.id.clone())
        .group(group.clone())
        .relative()
        .h(px(height))
        .map(|plate| match column {
            // In a column every row is the column's width.
            true => plate.w_full().flex_none(),
            // Across the band every plate starts at the full width and gives
            // way at the same rate, so a crowded band splits evenly.
            false => plate
                .w(px(MAX_WIDTH))
                .flex_shrink(1.0)
                .min_w(px(if tab.active {
                    SELECTED_MIN_WIDTH
                } else {
                    MIN_WIDTH
                })),
        })
        .pl(px(match (column, tab.nested) {
            (true, true) => NESTED_INDENT,
            (true, false) => 8.0,
            (false, _) => 10.0,
        }))
        .pr(px(if closable { CLOSE_ROOM } else { 8.0 }))
        .rounded(px(if column { 8.0 } else { 6.0 }))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GLYPH_GAP))
        .cursor_pointer()
        // Never `occlude()`: a blocking hitbox ends the hit test, and the
        // strip behind the plates would stop scrolling under the wheel.
        .when(tab.active, |plate| plate.bg(wash(0.10)))
        .when(!tab.active, |plate| {
            plate
                .bg(motion::fade_color(&fade, wash(rest), wash(hover)))
                .on_hover(motion::on_hover_fade(&fade))
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            on_menu(index, event.position, window, cx);
        })
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            if event.click_count() == 2 {
                on_rename(index, window, cx);
            } else {
                on_select(index, window, cx);
            }
        })
        .when_some(drag, |plate, drag| {
            plate.on_drag(drag, |drag, _, _, cx| {
                cx.stop_propagation();
                drag.chip(cx)
            })
        })
        // A tab dropped here lands beside this one, on the side under the
        // pointer, and takes its group.
        .when(dragging, |plate| {
            plate
                .child(beside_tab(index, false, column, on_drop.clone()))
                .child(beside_tab(index, true, column, on_drop.clone()))
        })
        .children(tab.icon.map(|path| TabGlyph { path }))
        .child(
            div()
                .min_w_0()
                .flex_1()
                // The plate does not clip — a drop bar hangs past its edge —
                // so its title does.
                .overflow_hidden()
                .h(px(TITLE_HEIGHT))
                .flex()
                .items_center()
                .text_size(ui_rems(13.0))
                .map(|slot| match tab.editor {
                    Some(editor) => slot.child(editor),
                    None => slot.child(TabTitle {
                        id: tab.id,
                        title: tab.title,
                        active: tab.active,
                        group: group.clone(),
                    }),
                }),
        )
        // Over the plate, not beside the title: it shows on hover and takes
        // no room from the title the rest of the time.
        .when(closable, |plate| {
            plate.child(
                div()
                    .id(("tab-strip-close", index))
                    .absolute()
                    .right(px(4.0))
                    .top(px((height - CLOSE_SIZE) / 2.0))
                    .size(px(CLOSE_SIZE))
                    .rounded(px(4.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.0)
                    .group_hover(group, |style| style.opacity(1.0))
                    .hover(|style| style.bg(wash(0.12)))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        on_close(index, window, cx);
                    })
                    .child(CloseGlyph),
            )
        })
        .into_any_element()
}

/// The tab's glyph, bare: a tile around it is all a narrow tab
/// would show.
#[derive(IntoElement)]
struct TabGlyph {
    path: &'static str,
}

impl RenderOnce for TabGlyph {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        crate::ui::icons::icon(self.path)
            .flex_none()
            .size(px(14.0))
            .text_color(Theme::of(cx).text_muted)
    }
}

/// The title, dim unless its tab is selected or under the pointer. Painted
/// by hand so an overflowing one fades out instead of ending in an ellipsis;
/// that one also shows in full in a tooltip.
#[derive(IntoElement)]
struct TabTitle {
    /// The tab's id, which keys whether its title overflowed last frame.
    id: ElementId,
    title: SharedString,
    active: bool,
    group: SharedString,
}

impl RenderOnce for TabTitle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = Arc::new(self.id);
        let overflows = overflow_state(&key, window, cx);
        let theme = Theme::of(cx);
        let lit = theme.text;
        let tip = overflows
            .read(cx)
            .then(|| crate::ui::tip::tip(self.title.clone()));
        div()
            .id(ElementId::NamedChild(key, "title".into()))
            .size_full()
            .text_color(if self.active { lit } else { theme.text_muted })
            .group_hover(self.group, move |style| style.text_color(lit))
            .when_some(tip, |title, tip| title.tooltip(tip))
            .child(faded_label(self.title, overflows))
    }
}

/// Whether `id`'s text overflowed its box last frame, as [`overflow_probe`]
/// or [`faded_label`] measured it.
fn overflow_state(id: &Arc<ElementId>, window: &mut Window, cx: &mut App) -> Entity<bool> {
    window.use_keyed_state(
        ElementId::NamedChild(id.clone(), "overflows".into()),
        cx,
        |_, _| false,
    )
}

/// `text` shaped as one line in the inherited style, and the run it used.
fn shape(text: &SharedString, window: &mut Window) -> (ShapedLine, TextRun, Pixels) {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line =
        window
            .text_system()
            .shape_line(text.clone(), font_size, std::slice::from_ref(&run), None);
    (line, run, font_size)
}

/// Tell `overflows` whether a line `width` wide fits a box `room` wide.
/// Measured after layout, so the tooltip it decides comes a frame later.
fn record_overflow(overflows: &Entity<bool>, width: Pixels, room: Pixels, cx: &mut App) {
    // Half a pixel of slack: a box sized to its text can come out a hair
    // narrower than the text measured again.
    let fits = room <= px(0.0) || width <= room + px(0.5);
    overflows.update(cx, |overflows, cx| {
        if *overflows == fits {
            *overflows = !fits;
            cx.notify();
        }
    });
}

/// Measures `text` against the box it is laid over — a child of the element
/// that truncates it — and records whether it was cut.
fn overflow_probe(text: SharedString, overflows: Entity<bool>) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let (line, _, _) = shape(&text, window);
            record_overflow(&overflows, line.width(), bounds.size.width, cx);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// One line of text in the inherited style. Short text is left as is; text
/// wider than the box fades to nothing over its last [`FADE`] pixels.
/// `overflows` learns which of the two it was, for the next frame.
fn faded_label(text: SharedString, overflows: Entity<bool>) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let (line, run, font_size) = shape(&text, window);
            record_overflow(&overflows, line.width(), bounds.size.width, cx);
            if line.width() <= bounds.size.width || bounds.size.width <= px(0.0) {
                return line;
            }
            // One run per character, each dimmed by how close its centre
            // sits to the box's end.
            let runs: Vec<TextRun> = text
                .char_indices()
                .map(|(ix, ch)| {
                    let centre =
                        (line.x_for_index(ix) + line.x_for_index(ix + ch.len_utf8())) / 2.0;
                    let mut run = run.clone();
                    run.len = ch.len_utf8();
                    run.color.a *= fade_alpha(f32::from(bounds.size.width), f32::from(centre));
                    run
                })
                .collect();
            window
                .text_system()
                .shape_line(text.clone(), font_size, &runs, None)
        },
        |bounds, line, window, cx| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                let _ = line.paint(
                    bounds.origin,
                    bounds.size.height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            });
        },
    )
    .size_full()
}

/// The opacity of a glyph centred at `x` in a box `width` wide: full until
/// the last [`FADE`] pixels, then down to nothing at the edge.
fn fade_alpha(width: f32, x: f32) -> f32 {
    ((width - x) / FADE).clamp(0.0, 1.0)
}

#[derive(IntoElement)]
struct CloseGlyph;

impl RenderOnce for CloseGlyph {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        crate::ui::icons::icon(crate::ui::icons::CLOSE)
            .size(px(12.0))
            .text_color(Theme::of(cx).text_muted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plate_already_in_view_does_not_move_the_strip() {
        assert_eq!(reveal_offset(80.0, 192.0, 0.0, 600.0), 0.0);
        assert_eq!(reveal_offset(900.0, 940.0, -500.0, 600.0), -500.0);
    }

    #[test]
    fn a_plate_past_the_right_edge_lands_against_it() {
        let offset = reveal_offset(2000.0, 2112.0, 0.0, 600.0);
        assert_eq!(2112.0 + offset, 600.0);
    }

    #[test]
    fn a_plate_past_the_left_edge_lands_against_it() {
        assert_eq!(reveal_offset(120.0, 232.0, -900.0, 600.0), -120.0);
    }

    #[test]
    fn a_title_fades_only_over_its_last_pixels() {
        assert_eq!(fade_alpha(200.0, 100.0), 1.0);
        assert_eq!(fade_alpha(200.0, 180.0), 0.5);
        assert_eq!(fade_alpha(200.0, 230.0), 0.0);
    }
}
