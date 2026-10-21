//! The measurements a surface's chrome is built from.
//!
//! One place for the numbers that make a toolbar read as a toolbar: how big a
//! control is, how round, how big the glyph inside it, and how far the whole
//! strip sits from the edge. They are physical geometry, in pixels, like the
//! rest of the chrome — a 24px button is 24px whatever the text scale is.

/// The side of a square toolbar control.
pub const CONTROL_SIZE: f32 = 24.0;
/// Its corner radius.
pub const CONTROL_RADIUS: f32 = 6.0;
/// The outer corner of a segmented control — two or more halves in one
/// border, like [File View | Diff View].
pub const SEGMENT_RADIUS: f32 = 8.0;
/// The corner of a half at the group's ends: the outer radius less the
/// border, so the active half's wash follows the curve instead of painting a
/// square corner over it.
pub const SEGMENT_INNER_RADIUS: f32 = SEGMENT_RADIUS - 1.0;
/// The glyph inside it. Smaller than the control by design: the padding is
/// what makes the hit target read as a button.
pub const ICON_SIZE: f32 = 14.0;
/// The gap between a strip's content and the edge of its surface.
pub const EDGE_INSET: f32 = 8.0;
/// The height of a surface's header band.
pub const HEADER_HEIGHT: f32 = 44.0;
