//! The window's frame on Linux.
//!
//! Under client-side decorations — what GNOME on Wayland always gives, and
//! what this application asks for everywhere on Linux — gpui-kit's root wraps
//! the window in its own frame: a square border, a shadow and the resize
//! strips. The window composites with alpha
//! (`WindowBackgroundAppearance::Transparent`) so that shadow can show.
//!
//! The frame is square, so no layer rounds a window corner. [`corner_radius`]
//! stays the one place those layers ask, should the frame round again.

use gpui_kit::Window;

/// The radius a layer painting into a window corner must round that corner
/// with: 0, as gpui-kit's frame is square.
pub fn corner_radius(_window: &Window) -> f32 {
    0.0
}
