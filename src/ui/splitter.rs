//! The divider between two panels that share a width: a hairline with a small grip on it, a wider
//! strip around it that takes the pointer, and a double-click that puts the
//! split back where it started.
//!
//! The splitter only starts a drag. The view that owns the split listens on
//! the element that holds both panels —
//! `.on_drag_move::<SplitterDrag>(…)` — and turns the pointer into a ratio
//! against that element's bounds (see [`ratio_from_pointer`]). The drag
//! carries the splitter's id, so two splits inside one another (the commit
//! panel beside a split diff) each move only on their own divider.

use gpui_kit::{
    App, AppContext as _, Context, IntoElement, MouseButton, Render, SharedString, Stateful,
    Window, div, linear_color_stop, linear_gradient, prelude::*, px,
};

use super::motion;
use super::theme::Theme;

/// What a splitter drags: its own id.
#[derive(Clone)]
pub struct SplitterDrag(pub SharedString);

/// The drag has no preview: the panels moving are the feedback.
struct SplitterGhost;

impl Render for SplitterGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

/// The width that takes the pointer, centred on the hairline. The hairline is
/// what reads; the strip is what a hand can hit.
const HIT_WIDTH: f32 = 7.0;

/// The grip on the hairline.
const GRIP_WIDTH: f32 = 4.0;
const GRIP_HEIGHT: f32 = 24.0;

/// A vertical divider for a row of panels. `grip` draws the handle on it —
/// panels have one; the split diff's divider is the bare
/// hairline. `on_reset` runs on a double-click.
pub fn splitter(
    id: impl Into<SharedString>,
    theme: &Theme,
    grip: bool,
    on_reset: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<gpui_kit::Div> {
    let id: SharedString = id.into();
    // The seam: the 1px border is all there is at rest; hovering fades
    // in a stronger line over it that thins out toward both ends.
    let fade_key = format!("splitter-{id}");
    let highlight = motion::fade_color(
        &fade_key,
        theme.border_strong.opacity(0.0),
        theme.border_strong,
    );
    let clear = highlight.opacity(0.0);
    div()
        .id(id.clone())
        .relative()
        .w(px(1.0))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.border)
        .when(grip, |line| {
            line.child(
                div()
                    .w(px(GRIP_WIDTH))
                    .h(px(GRIP_HEIGHT))
                    .flex_none()
                    .rounded(px(GRIP_WIDTH))
                    .bg(theme.border_strong),
            )
        })
        .child(
            div()
                .id(SharedString::from(format!("{id}-hit")))
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-(HIT_WIDTH - 1.0) / 2.0))
                .w(px(HIT_WIDTH))
                .cursor_col_resize()
                .on_hover(motion::on_hover_fade(fade_key))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px((HIT_WIDTH - 1.0) / 2.0))
                        .w(px(1.0))
                        .flex()
                        .flex_col()
                        .child(div().flex_1().bg(linear_gradient(
                            180.0,
                            linear_color_stop(clear, 0.0),
                            linear_color_stop(highlight, 1.0),
                        )))
                        .child(div().flex_1().bg(linear_gradient(
                            180.0,
                            linear_color_stop(highlight, 0.0),
                            linear_color_stop(clear, 1.0),
                        ))),
                )
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    // No text selection, no window drag, no focus move.
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_mouse_up(MouseButton::Left, move |event, window, cx| {
                    if event.click_count == 2 {
                        on_reset(window, cx);
                    }
                })
                .on_drag(SplitterDrag(id), |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| SplitterGhost)
                }),
        )
}

/// Where the pointer sits across `bounds`, from 0 at its left edge to 1 at
/// its right.
pub fn ratio_from_pointer(
    pointer_x: gpui_kit::Pixels,
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> f32 {
    let width = f32::from(bounds.size.width);
    if width <= 0.0 {
        return 0.5;
    }
    f32::from(pointer_x - bounds.left()) / width
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Bounds, point, size};

    #[test]
    fn the_ratio_is_measured_from_the_left_edge_of_the_split() {
        let bounds = Bounds::new(point(px(100.0), px(0.0)), size(px(400.0), px(10.0)));
        assert_eq!(ratio_from_pointer(px(100.0), bounds), 0.0);
        assert_eq!(ratio_from_pointer(px(300.0), bounds), 0.5);
        assert_eq!(ratio_from_pointer(px(500.0), bounds), 1.0);
    }

    #[test]
    fn an_unlaid_split_reads_as_centred() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(0.0)));
        assert_eq!(ratio_from_pointer(px(10.0), bounds), 0.5);
    }
}
