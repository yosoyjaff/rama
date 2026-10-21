//! The window's own frame on Linux, where nobody else draws one.
//!
//! Under client-side decorations — what GNOME on Wayland always gives, and
//! what this application asks for everywhere on Linux — the compositor draws
//! no border, no corners and no resize handles. This module is that frame:
//!
//! * a **corner radius** for a floating window, so it rounds against the
//!   desktop the way a native one does. The window composites with alpha
//!   (`WindowBackgroundAppearance::Transparent`) and the corners outside the
//!   curve are left unpainted. GPUI's content masks are rectangles —
//!   `.rounded()` never clips children — so every layer that paints a
//!   background into a window corner rounds that corner itself with
//!   [`corner_radius`];
//! * invisible **resize strips** along every free edge.
//!
//! A maximized or tiled window sits flush with the screen: square corners,
//! and no strip on an edge the compositor has taken. Server decorations and
//! the other platforms hand the frame back to the system: radius 0, no strips.
//!
//! Physical window geometry, so in pixels — the same exception the title bar
//! makes.

use gpui_kit::{
    AnyElement, Decorations, InteractiveElement as _, IntoElement, MouseButton, ResizeEdge,
    Styled as _, Window, div, px,
};

/// Corner radius of a floating Linux window.
const CORNER_RADIUS: f32 = 10.0;
/// Thickness of a resize strip along an edge.
const EDGE: f32 = 6.0;
/// Side of the square resize handle at a corner.
const CORNER: f32 = 14.0;

/// Which edges are free — not taken by the compositor — as `(top, bottom,
/// left, right)`. All false under server decorations and off Linux.
fn free_edges(window: &Window) -> (bool, bool, bool, bool) {
    if !cfg!(target_os = "linux") {
        return (false, false, false, false);
    }
    match window.window_decorations() {
        Decorations::Client { tiling } => {
            (!tiling.top, !tiling.bottom, !tiling.left, !tiling.right)
        }
        Decorations::Server => (false, false, false, false),
    }
}

/// The radius a layer painting into a window corner must round that corner
/// with, this frame: [`CORNER_RADIUS`] while the window floats, 0 otherwise.
pub fn corner_radius(window: &Window) -> f32 {
    let (top, bottom, left, right) = free_edges(window);
    if top && bottom && left && right {
        CORNER_RADIUS
    } else {
        0.0
    }
}

/// The invisible strips that turn a press at a free edge into a compositor
/// resize. Paint them last, so they sit above everything at the edges.
pub fn resize_edges(window: &Window) -> Vec<AnyElement> {
    let (top, bottom, left, right) = free_edges(window);
    let strip = |element: gpui_kit::Div, edge: ResizeEdge| {
        element
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.start_window_resize(edge);
            })
            .into_any_element()
    };
    let mut out = Vec::new();
    // The cardinal edges stop short of the corners, which have their own
    // squares with the diagonal cursors.
    if top {
        out.push(strip(
            div()
                .absolute()
                .top_0()
                .left(px(CORNER))
                .right(px(CORNER))
                .h(px(EDGE))
                .cursor_ns_resize(),
            ResizeEdge::Top,
        ));
    }
    if bottom {
        out.push(strip(
            div()
                .absolute()
                .bottom_0()
                .left(px(CORNER))
                .right(px(CORNER))
                .h(px(EDGE))
                .cursor_ns_resize(),
            ResizeEdge::Bottom,
        ));
    }
    if left {
        out.push(strip(
            div()
                .absolute()
                .left_0()
                .top(px(CORNER))
                .bottom(px(CORNER))
                .w(px(EDGE))
                .cursor_ew_resize(),
            ResizeEdge::Left,
        ));
    }
    if right {
        out.push(strip(
            div()
                .absolute()
                .right_0()
                .top(px(CORNER))
                .bottom(px(CORNER))
                .w(px(EDGE))
                .cursor_ew_resize(),
            ResizeEdge::Right,
        ));
    }
    if top && left {
        out.push(strip(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size(px(CORNER))
                .cursor_nwse_resize(),
            ResizeEdge::TopLeft,
        ));
    }
    if top && right {
        out.push(strip(
            div()
                .absolute()
                .top_0()
                .right_0()
                .size(px(CORNER))
                .cursor_nesw_resize(),
            ResizeEdge::TopRight,
        ));
    }
    if bottom && left {
        out.push(strip(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .size(px(CORNER))
                .cursor_nesw_resize(),
            ResizeEdge::BottomLeft,
        ));
    }
    if bottom && right {
        out.push(strip(
            div()
                .absolute()
                .bottom_0()
                .right_0()
                .size(px(CORNER))
                .cursor_nwse_resize(),
            ResizeEdge::BottomRight,
        ));
    }
    out
}
