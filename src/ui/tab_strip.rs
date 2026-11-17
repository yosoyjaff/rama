//! The tab strip: a plate for each tab the pane has open.
//!
//! A plate holds a tile with the tab's glyph and its title. Tabs keep their
//! full width while they fit; when the band runs short they all give way
//! together, the selected one only down to [`SELECTED_MIN_WIDTH`]. A title
//! too long for its plate fades out over its last few pixels rather than
//! ending in an ellipsis. On hover a close ✕ sits over the plate's right end.
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

use gpui_kit::{
    AnyElement, App, ContentMask, ElementId, IntoElement, SharedString, TextAlign, TextRun, Window,
    canvas, div, prelude::*, px,
};

use crate::ui::motion;
use crate::ui::theme::{Theme, hairline, wash};
use crate::ui::typography::ui_rems;

/// A tab's width while the band has room for every one.
pub const MAX_WIDTH: f32 = 224.0;
/// The selected tab shrinks with the others only down to this.
pub const SELECTED_MIN_WIDTH: f32 = 112.0;
/// Below this a tab is only its tile, and the strip scrolls instead.
pub const MIN_WIDTH: f32 = 40.0;
/// The plate's height, inside the title band.
pub const PLATE_HEIGHT: f32 = 27.0;
/// The gap between plates.
pub const GAP: f32 = 2.0;
/// The glyph's tile: a touch wider than tall.
const TILE_WIDTH: f32 = 24.0;
const TILE_HEIGHT: f32 = 20.0;
/// The tile sits as far from the plate's left edge as from its top.
const TILE_INSET: f32 = (PLATE_HEIGHT - TILE_HEIGHT) / 2.0;
/// Between the tile and the title.
const TILE_GAP: f32 = 8.0;
/// The close button, and the room the title leaves for it.
const CLOSE_SIZE: f32 = 20.0;
const CLOSE_ROOM: f32 = 26.0;
/// How far from its end an overflowing title starts to fade.
const FADE: f32 = 40.0;

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
    /// A glyph path for the tile, or `None` for a tab whose title says it all.
    pub icon: Option<&'static str>,
    pub active: bool,
    /// Whether this tab can be closed. The pane's home tab cannot.
    pub closable: bool,
    /// A field editing the title, drawn in its place.
    pub editor: Option<AnyElement>,
}

/// Draw the strip. `on_select`, `on_rename` (a double click) and `on_close`
/// receive the tab's index; `scroll` is the strip's own offset, which
/// [`reveal`] moves just far enough to show a plate. `trailing` follows the
/// last plate, as close to it as the plates are to each other. `column`
/// stacks the plates down a sidebar, each as wide as it, instead of across
/// the band.
pub fn tab_strip(
    tabs: Vec<Tab>,
    scroll: &gpui_kit::ScrollHandle,
    trailing: Option<AnyElement>,
    column: bool,
    on_select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_rename: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_close: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let mut strip = div()
        .id("tab-strip")
        .flex()
        .gap(px(GAP))
        .map(|strip| match column {
            true => strip.flex_col().size_full().min_h_0().overflow_y_scroll(),
            false => strip
                .flex_row()
                .items_center()
                .min_w_0()
                .overflow_x_scroll(),
        })
        .track_scroll(scroll);
    for (index, tab) in tabs.into_iter().enumerate() {
        strip = strip.child(plate(
            index,
            tab,
            column,
            on_select.clone(),
            on_rename.clone(),
            on_close.clone(),
        ));
    }
    strip
        .children(trailing.map(|trailing| div().flex_none().child(trailing)))
        .into_any_element()
}

fn plate(
    index: usize,
    tab: Tab,
    column: bool,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_rename: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_close: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let group = tab.group.clone();
    let fade = tab.group.clone();
    let closable = tab.closable;
    div()
        .id(tab.id)
        .group(group.clone())
        .relative()
        .h(px(PLATE_HEIGHT))
        .map(|plate| match column {
            // In a column every plate is the column's width.
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
        .overflow_hidden()
        .pl(px(if tab.icon.is_some() { TILE_INSET } else { 10.0 }))
        .pr(px(if closable { CLOSE_ROOM } else { 8.0 }))
        .rounded(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(TILE_GAP))
        .cursor_pointer()
        // Never `occlude()`: a blocking hitbox ends the hit test, and the
        // strip behind the plates would stop scrolling under the wheel.
        .when(tab.active, |plate| plate.bg(wash(0.10)))
        .when(!tab.active, |plate| {
            plate
                .bg(motion::fade_color(&fade, wash(0.04), wash(0.07)))
                .on_hover(motion::on_hover_fade(&fade))
        })
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            if event.click_count() == 2 {
                on_rename(index, window, cx);
            } else {
                on_select(index, window, cx);
            }
        })
        .children(tab.icon.map(|path| TabTile { path }))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .h(px(TILE_HEIGHT))
                .flex()
                .items_center()
                .text_size(ui_rems(13.0))
                .map(|slot| match tab.editor {
                    Some(editor) => slot.child(editor),
                    None => slot.child(TabTitle {
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
                    .top(px((PLATE_HEIGHT - CLOSE_SIZE) / 2.0))
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

/// The tab's glyph on a small raised tile.
#[derive(IntoElement)]
struct TabTile {
    path: &'static str,
}

impl RenderOnce for TabTile {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .flex_none()
            .w(px(TILE_WIDTH))
            .h(px(TILE_HEIGHT))
            .rounded(px(5.0))
            .bg(theme.surface_raised)
            .border_1()
            .border_color(hairline(0.10))
            .flex()
            .items_center()
            .justify_center()
            .child(
                crate::ui::icons::icon(self.path)
                    .size(px(14.0))
                    .text_color(theme.text),
            )
    }
}

/// The title, dim unless its tab is selected or under the pointer. Painted
/// by hand so an overflowing one fades out instead of ending in an ellipsis.
#[derive(IntoElement)]
struct TabTitle {
    title: SharedString,
    active: bool,
    group: SharedString,
}

impl RenderOnce for TabTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let lit = theme.text;
        div()
            .size_full()
            .text_color(if self.active { lit } else { theme.text_muted })
            .group_hover(self.group, move |style| style.text_color(lit))
            .child(faded_label(self.title))
    }
}

/// One line of text in the inherited style. Short text is left as is; text
/// wider than the box fades to nothing over its last [`FADE`] pixels.
fn faded_label(text: SharedString) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
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
            let line = window.text_system().shape_line(
                text.clone(),
                font_size,
                std::slice::from_ref(&run),
                None,
            );
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
