//! Interface text sizes.
//!
//! Sizes are authored as "pixels at the default font size" — the number a
//! design is measured in — and expressed in rems, so the whole interface
//! scales together when the window's rem size changes. A raw `px` text size
//! would ignore that and stay the one thing on screen that does not grow.

use gpui_kit::{Rems, rems};

/// The design's base font size: the denominator every authored size is
/// measured against.
pub const BASE_FONT_SIZE: f32 = 16.0;

/// A text size authored in pixels at [`BASE_FONT_SIZE`].
pub const fn ui_rems(pixels_at_default: f32) -> Rems {
    rems(pixels_at_default / BASE_FONT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_authored_size_is_its_share_of_the_base() {
        assert_eq!(ui_rems(BASE_FONT_SIZE), rems(1.0));
        assert_eq!(ui_rems(12.0), rems(0.75));
    }
}
