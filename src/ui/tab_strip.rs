//! The tab strip: fixed-width chips for whatever the pane has open.
//!
//! One chip is 112×24 with three slots — a leading glyph, the title, and a
//! trailing slot that is empty at rest and holds the close ✕ on hover. The
//! slots are fixed so nothing shifts when the pointer crosses a chip, and the
//! widths are uniform so a strip of five tabs reads as five of the same thing
//! rather than five different things.
//!
//! The strip knows nothing about what a tab contains: it takes what to draw
//! and reports which chip was pressed.
//!
//! What does not fit scrolls, with nothing drawn for it — no bar, no shadow:
//! the wheel moves the chips, and the owner brings the active one into view
//! through the [`gpui_kit::ScrollHandle`] it passes in.

use gpui_kit::{
    AnyElement, App, ElementId, IntoElement, SharedString, Window, div, prelude::*, px,
};

use crate::ui::motion;
use crate::ui::theme::{Theme, wash};
use crate::ui::typography::ui_rems;

/// One chip's slot. Fixed, as uniform widths are what let a strip of tabs read
/// as one control instead of a row of differently-sized buttons.
pub const CHIP_WIDTH: f32 = 112.0;
/// The chip's height, matching the toolbar controls beside it.
pub const CHIP_HEIGHT: f32 = 24.0;
/// The gap between chips.
pub const CHIP_GAP: f32 = 4.0;
/// The leading and trailing slots inside a chip.
const SLOT_SIZE: f32 = 18.0;

/// What one chip says.
pub struct Tab {
    /// Stable across frames: the chip's identity for hover and click.
    pub id: ElementId,
    /// Group key for the hover swap inside the chip.
    pub group: SharedString,
    pub title: SharedString,
    /// A glyph path, or `None` for a chip whose title says it all.
    pub icon: Option<&'static str>,
    pub active: bool,
    /// Whether this tab can be closed. The pane's home tab cannot.
    pub closable: bool,
}

/// Draw the strip. `on_select` and `on_close` receive the tab's index;
/// `scroll` is the strip's own offset, which `scroll_to_item(index)` moves
/// just far enough to show a chip.
pub fn tab_strip(
    tabs: Vec<Tab>,
    scroll: &gpui_kit::ScrollHandle,
    on_select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_close: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let mut strip = div()
        .id("tab-strip")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(CHIP_GAP))
        .min_w_0()
        .overflow_x_scroll()
        .track_scroll(scroll);
    for (index, tab) in tabs.into_iter().enumerate() {
        strip = strip.child(chip(index, tab, on_select.clone(), on_close.clone()));
    }
    strip.into_any_element()
}

fn chip(
    index: usize,
    tab: Tab,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_close: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let group = tab.group.clone();
    let fade = tab.group.clone();
    let closable = tab.closable;
    div()
        .id(tab.id)
        .group(group.clone())
        .h(px(CHIP_HEIGHT))
        .w(px(CHIP_WIDTH))
        .flex_none()
        .pl(px(if tab.icon.is_some() { 4.0 } else { 9.0 }))
        .pr(px(4.0))
        .rounded(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .cursor_pointer()
        // Never `occlude()`: a blocking hitbox ends the hit test, and the
        // strip behind the chips would stop scrolling under the wheel.
        .when(tab.active, |chip| chip.bg(wash(0.10)))
        .when(!tab.active, |chip| {
            chip.bg(motion::fade_color(&fade, wash(0.0), wash(0.06)))
                .on_hover(motion::on_hover_fade(&fade))
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_select(index, window, cx);
        })
        .children(tab.icon.map(|path| {
            div()
                .flex_none()
                .size(px(SLOT_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .child(TabGlyph {
                    path,
                    active: tab.active,
                })
        }))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_size(ui_rems(11.5))
                .child(TabTitle {
                    title: tab.title,
                    active: tab.active,
                }),
        )
        // The trailing slot is permanently reserved, so a chip never changes
        // width when the pointer arrives: empty at rest, the ✕ on hover.
        .child(
            div()
                .id(("tab-strip-close", index))
                .flex_none()
                .size(px(SLOT_SIZE))
                .rounded(px(4.0))
                .when(closable, |slot| {
                    slot.hover(|style| style.bg(wash(0.12)))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            on_close(index, window, cx);
                        })
                        .child(
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(0.0)
                                .group_hover(group, |style| style.opacity(1.0))
                                .child(CloseGlyph),
                        )
                }),
        )
        .into_any_element()
}

/// The chip's leading glyph, tinted by the theme at render time.
#[derive(IntoElement)]
struct TabGlyph {
    path: &'static str,
    active: bool,
}

impl RenderOnce for TabGlyph {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        crate::ui::icons::icon(self.path)
            .size(px(12.0))
            .text_color(if self.active {
                theme.text_muted
            } else {
                theme.text_muted.opacity(0.7)
            })
    }
}

#[derive(IntoElement)]
struct TabTitle {
    title: SharedString,
    active: bool,
}

impl RenderOnce for TabTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .truncate()
            .text_color(if self.active {
                theme.text
            } else {
                theme.text_muted
            })
            .child(self.title)
    }
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
