//! The diff viewer's sizes, in one place so the rows, the strips and the
//! scroll arithmetic agree on them.

// Row heights.
pub const LINE_HEIGHT: f32 = 21.0;
pub const BAND_HEIGHT: f32 = 28.0;
pub const SUMMARY_HEIGHT: f32 = 24.0;
/// A line in File View, which is set a touch smaller than a diff row.
pub const FILE_ROW_HEIGHT: f32 = 20.0;
/// Under a file's last line.
pub const TAIL_PAD: f32 = 8.0;

// Code.
pub const CODE_TEXT: f32 = 12.0;
/// Before the code in the unified layout, and in each split half.
pub const CODE_INSET: f32 = 12.0;
pub const HALF_CODE_INSET: f32 = 6.0;
/// After the longest line, once scrolled to the end.
pub const CODE_TAIL: f32 = 24.0;
pub const TAB_STOP: usize = 4;

// Columns.
/// A line-number column is never narrower than this.
pub const MIN_GUTTER: f32 = 36.0;
/// The `+`/`−` column; each split half has a narrower one.
pub const SIGN_WIDTH: f32 = 28.0;
pub const HALF_SIGN_WIDTH: f32 = 18.0;
/// The tinted edge of a changed line.
pub const EDGE_BAR: f32 = 3.0;
/// Between the split halves.
pub const HALF_RULE: f32 = 1.0;
/// The change marks down the right edge.
pub const RULER: f32 = 12.0;
