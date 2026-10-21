//! Scrolling code sideways.
//!
//! Every row of a file scrolls together, by the width of the file's longest
//! line. That width is counted in character columns when the file is read
//! ([`Reach`]), turned into pixels for the window's code font when drawn
//! ([`Span`]), and each row's code is told how to [`Fit`]. A split view has
//! two halves that scroll apart: with the divider off centre they overflow
//! by different amounts.

use gpui_kit::{ScrollHandle, SharedString, Window, font, px};
use unicode_width::UnicodeWidthChar as _;

use super::metrics::{CODE_INSET, CODE_TAIL, CODE_TEXT, HALF_CODE_INSET, TAB_STOP};
use super::model::FileDiff;
use crate::ui::theme::Theme;

/// How many monospace columns `text` fills: a tab runs to the next stop,
/// a wide character takes two.
fn columns(text: &str) -> usize {
    text.chars().fold(0, |at, c| {
        at + match c {
            '\t' => TAB_STOP - at % TAB_STOP,
            c => c.width().unwrap_or(0),
        }
    })
}

/// A file's longest line, in columns, and its gutter width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    columns: usize,
    gutter: f32,
}

impl Reach {
    pub fn of(file: &FileDiff) -> Self {
        let lines = file.hunks.iter().flat_map(|hunk| &hunk.lines);
        Self {
            columns: lines.map(|line| columns(&line.text)).max().unwrap_or(0),
            gutter: file.gutter(),
        }
    }

    /// In pixels, at the width of a column of the code font.
    pub fn in_pixels(self, theme: &Theme, window: &Window) -> Span {
        let text = window.text_system();
        let font_id = text.resolve_font(&font(theme.font_mono.clone()));
        let column = text
            .ch_advance(font_id, px(CODE_TEXT))
            .map_or(CODE_TEXT * 0.6, |advance| advance.as_f32());
        Span {
            text: self.columns as f32 * column,
            gutter: self.gutter,
        }
    }
}

/// A file's code width in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    text: f32,
    gutter: f32,
}

impl Span {
    /// How far a unified row scrolls when drawn with `gutter`-wide number
    /// columns. A row with narrower gutters scrolls further by the
    /// difference, so the whole file moves as one.
    pub fn unified(self, gutter: f32) -> f32 {
        self.text + CODE_INSET + CODE_TAIL + 2.0 * (self.gutter - gutter)
    }

    /// The same for one split half, which has one gutter. Both halves use
    /// it, so they line up even where one side is empty.
    pub fn half(self, gutter: f32) -> f32 {
        self.text + HALF_CODE_INSET + CODE_TAIL + (self.gutter - gutter)
    }
}

/// How a row's code meets the pane's edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// Cut off.
    Clip,
    /// As wide as the file's longest line, scrolled sideways.
    Scroll(Span),
    /// Broken onto more lines.
    Wrap,
}

/// The scroll a row's code follows, and the id of the element that does.
#[derive(Clone)]
pub struct Track {
    pub handle: ScrollHandle,
    pub id: SharedString,
}

/// Which split half; the unified layout uses the left one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Left,
    Right,
}

/// What a file keeps for sideways scrolling, outside its rows.
pub struct Sideways {
    pub reach: Reach,
    pub left: ScrollHandle,
    pub right: ScrollHandle,
}

impl Sideways {
    pub fn new(file: &FileDiff) -> Self {
        Self {
            reach: Reach::of(file),
            left: ScrollHandle::new(),
            right: ScrollHandle::new(),
        }
    }

    /// The track for list row `row`, in `half`, for the cell called `cell`.
    pub fn track(&self, row: usize, half: Half, cell: &str) -> Track {
        let handle = match half {
            Half::Left => self.left.clone(),
            Half::Right => self.right.clone(),
        };
        Track {
            handle,
            id: format!("diff-code-{row}-{cell}").into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_reach_the_next_stop_and_wide_glyphs_count_twice() {
        assert_eq!(columns("xy\tz"), 5);
        assert_eq!(columns("\t"), 4);
        assert_eq!(columns("漢字"), 4);
    }

    #[test]
    fn narrower_gutters_scroll_further() {
        let span = Span {
            text: 100.0,
            gutter: 40.0,
        };
        assert_eq!(span.unified(40.0) + 20.0, span.unified(30.0));
        assert_eq!(span.half(40.0) + 10.0, span.half(30.0));
    }
}
