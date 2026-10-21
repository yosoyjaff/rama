//! Drawing the rows of a diff, one at a time and without state: each function
//! takes a piece of the model and the widths it has to line up with, and
//! returns the element. The unified and split layouts are built from the
//! same pieces.

use gpui_kit::{
    AnyElement, ContentMask, Div, Hsla, PathBuilder, SharedString, StyledText, TextRun, canvas,
    div, font, point, prelude::*, px,
};

use super::highlight::{HighlightSpan, runs_for_line};
use super::metrics::{
    BAND_HEIGHT, CODE_INSET, CODE_TEXT, EDGE_BAR, HALF_CODE_INSET, HALF_RULE, HALF_SIGN_WIDTH,
    LINE_HEIGHT, SIGN_WIDTH, SUMMARY_HEIGHT,
};
use super::model::{DiffHighlights, DiffLine, FileDiff, FileStatus, LineKind, section_name};
use super::sideways::{Fit, Span, Track};
use crate::ui::theme::{self, Theme};

/// The text size of the line numbers and bands, a step under the code's.
const SMALL_TEXT: f32 = 11.0;
/// How strongly an added or removed line is tinted.
const ROW_WASH: f32 = 0.055;
/// Gap between the stripes of a missing side.
const HATCH_GAP: f32 = 7.0;

/// A line under the file header saying something about the file as a whole:
/// "New file", "Binary file — contents not shown".
pub fn notice_row(notice: String, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .w_full()
        .h(px(SUMMARY_HEIGHT))
        .flex()
        .items_center()
        .px_4()
        .text_size(px(SMALL_TEXT))
        .text_color(theme.text_faint)
        .child(SharedString::from(notice))
        .into_any_element()
}

/// The band between hunks. The `-a,b +c,d` counts are left out, since the
/// gutters already number every line; only the function or section git
/// named, if any, is shown.
pub fn hunk_header_row(header: &str, theme: &Theme) -> AnyElement {
    let context = section_name(header);
    div()
        .flex_none()
        .w_full()
        .h(px(BAND_HEIGHT))
        .flex()
        .items_center()
        .px_4()
        .bg(theme.diff_hunk_bg)
        .font_family(theme.font_mono.clone())
        .text_size(px(SMALL_TEXT))
        .text_color(theme.text_faint)
        .when(!context.is_empty(), |band| {
            band.child(SharedString::from(context.to_owned()))
        })
        .into_any_element()
}

/// The colours a changed line is drawn in.
struct Tint {
    sign: &'static str,
    sign_color: Hsla,
    wash: Option<Hsla>,
    edge: Option<Hsla>,
    number: Hsla,
}

impl Tint {
    fn of(kind: LineKind, theme: &Theme) -> Self {
        let changed = |color: Hsla, sign| Tint {
            sign,
            sign_color: color,
            wash: Some(Hsla {
                a: ROW_WASH,
                ..color
            }),
            edge: Some(color.opacity(0.55)),
            number: color.opacity(0.9),
        };
        match kind {
            LineKind::Add => changed(theme.diff_add, "+"),
            LineKind::Del => changed(theme.diff_del, "−"),
            LineKind::Context | LineKind::Meta => Tint {
                sign: "·",
                sign_color: theme.text_faint.opacity(0.5),
                wash: None,
                edge: None,
                number: theme.text_faint.opacity(0.8),
            },
        }
    }

    /// A conflict line in its side's colour, brighter on the conflict being
    /// worked on.
    fn conflict(tone: Hsla, current: bool) -> Self {
        Tint {
            sign: "",
            sign_color: tone,
            wash: Some(tone.opacity(if current { 0.16 } else { 0.07 })),
            edge: Some(tone.opacity(0.55)),
            number: tone.opacity(0.9),
        }
    }
}

fn edge(color: Option<Hsla>) -> Div {
    div()
        .flex_none()
        .w(px(EDGE_BAR))
        .self_stretch()
        .when_some(color, |bar, color| bar.bg(color))
}

fn line_number(number: Option<u32>, width: f32, color: Hsla, theme: &Theme) -> Div {
    div()
        .flex_none()
        .w(px(width))
        .pr(px(8.0))
        .flex()
        .justify_end()
        .font_family(theme.font_mono.clone())
        .text_size(px(SMALL_TEXT))
        .line_height(px(LINE_HEIGHT))
        .text_color(color)
        .child(SharedString::from(
            number.map_or(String::new(), |n| n.to_string()),
        ))
}

fn sign_column(width: f32, color: Hsla, theme: &Theme) -> Div {
    div()
        .flex_none()
        .w(px(width))
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .font_family(theme.font_mono.clone())
        .text_size(px(CODE_TEXT))
        .line_height(px(LINE_HEIGHT))
        .text_color(color)
}

/// A row's height: exact, or at least one line when the code wraps.
fn line_height(row: Div, wrapped: bool) -> Div {
    match wrapped {
        true => row.min_h(px(LINE_HEIGHT)),
        false => row.h(px(LINE_HEIGHT)),
    }
}

/// The code of a line. This is the only part of a row that may be wider than
/// the pane: the gutters stay put, and only this scrolls sideways, on the
/// file's shared scroll handle. `width` is the file's full code width when it
/// scrolls, so every row scrolls the same distance.
fn code(
    text: String,
    runs: Vec<TextRun>,
    theme: &Theme,
    indent: f32,
    width: Fit,
    full_width: impl Fn(&Span) -> f32,
    scroll: Option<Track>,
) -> AnyElement {
    let wrapped = width == Fit::Wrap;
    let fixed = match width {
        Fit::Scroll(metrics) => Some(full_width(&metrics)),
        Fit::Clip | Fit::Wrap => None,
    };
    let text = div()
        .pl(px(indent))
        .font_family(theme.font_mono.clone())
        .text_size(px(CODE_TEXT))
        .line_height(px(LINE_HEIGHT))
        .map(|text| match wrapped {
            true => text.w_full().min_w_0().whitespace_normal(),
            false => text.whitespace_nowrap(),
        })
        // A child must not stretch the extent every row shares.
        .when_some(fixed, |text, width| {
            text.flex_none().w(px(width)).overflow_hidden()
        })
        .child(StyledText::new(text).with_runs(runs));
    let port = div()
        .flex_1()
        .min_w_0()
        .min_h(px(LINE_HEIGHT))
        .overflow_hidden()
        .child(text);
    match scroll {
        Some(scroll) if !wrapped => {
            let mut port = port
                .id(scroll.id)
                .overflow_x_scroll()
                .track_scroll(&scroll.handle);
            // GPUI would otherwise turn a vertical wheel into a sideways
            // scroll here, and the list underneath would never get it.
            port.style().restrict_scroll_to_axis = Some(true);
            port.into_any_element()
        }
        _ => port.into_any_element(),
    }
}

/// `\ No newline at end of file`: a remark about the line above, set past
/// the columns in small italics.
fn remark_row(text: &str, theme: &Theme, indent: f32) -> AnyElement {
    div()
        .flex_none()
        .w_full()
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .pl(px(indent))
        .italic()
        .text_size(px(10.5))
        .text_color(theme.text_faint)
        .child(SharedString::from(text.to_owned()))
        .into_any_element()
}

/// The text runs of a line: syntax colours where there are any, the plain
/// code colour elsewhere.
pub fn line_runs(
    line: &DiffLine,
    highlights: Option<&DiffHighlights>,
    theme: &Theme,
) -> Vec<TextRun> {
    let spans = highlights.map_or(&[][..], |h| h.spans(line));
    runs_for_line(&line.text, spans, &font(theme.font_mono.clone()), theme)
}

/// A line of the unified layout: the coloured edge, the old and new line
/// numbers, the sign and the code.
///
/// The old number is only shown on a removed line. On an unchanged line it
/// would repeat the new one all the way down the gutter.
pub fn diff_line_row(
    line: &DiffLine,
    spans: &[HighlightSpan],
    theme: &Theme,
    gutter_px: f32,
    code_width: Fit,
    scroll: Option<Track>,
) -> AnyElement {
    if line.kind == LineKind::Meta {
        let past_columns = EDGE_BAR + 2.0 * gutter_px + SIGN_WIDTH + 12.0;
        return remark_row(&line.text, theme, past_columns);
    }
    let tint = Tint::of(line.kind, theme);
    let old = line.old_no.filter(|_| line.kind == LineKind::Del);
    let new_color = match line.kind {
        LineKind::Add => tint.number,
        _ => theme.text_faint.opacity(0.8),
    };
    let runs = runs_for_line(&line.text, spans, &font(theme.font_mono.clone()), theme);
    line_height(div(), code_width == Fit::Wrap)
        .flex_none()
        .w_full()
        .flex()
        .items_start()
        .when_some(tint.wash, |row, wash| row.bg(wash))
        .child(edge(tint.edge))
        .child(line_number(old, gutter_px, tint.number, theme))
        .child(line_number(line.new_no, gutter_px, new_color, theme))
        .child(sign_column(SIGN_WIDTH, tint.sign_color, theme).child(tint.sign))
        .child(code(
            line.text.clone(),
            runs,
            theme,
            CODE_INSET,
            code_width,
            |m| m.unified(gutter_px),
            scroll,
        ))
        .into_any_element()
}

/// How a line of a merge conflict is drawn instead: in its side's colour,
/// stronger on the current conflict, with its take box in the sign column.
pub struct CellLook {
    pub tone: Hsla,
    pub strong: bool,
    pub marker: AnyElement,
}

/// One half of a split row: edge, its own side's line number, sign, code.
#[allow(clippy::too_many_arguments)]
pub fn split_line_cell_styled(
    line: &DiffLine,
    number: Option<u32>,
    runs: Vec<TextRun>,
    theme: &Theme,
    gutter_px: f32,
    code_width: Fit,
    scroll: Option<Track>,
    look: Option<CellLook>,
) -> Div {
    let (tint, take_box) = match look {
        Some(look) => (Tint::conflict(look.tone, look.strong), Some(look.marker)),
        None => (Tint::of(line.kind, theme), None),
    };
    let sign = sign_column(HALF_SIGN_WIDTH, tint.sign_color, theme);
    let sign = match take_box {
        Some(take_box) => sign.child(take_box),
        None => sign.child(tint.sign),
    };
    div()
        .flex_1()
        .min_w_0()
        .self_stretch()
        .overflow_hidden()
        .flex()
        .items_start()
        .when_some(tint.wash, |cell, wash| cell.bg(wash))
        .child(edge(tint.edge))
        .child(line_number(number, gutter_px, tint.number, theme))
        .child(sign)
        .child(code(
            line.text.clone(),
            runs,
            theme,
            HALF_CODE_INSET,
            code_width,
            |m| m.half(gutter_px),
            scroll,
        ))
}

/// The empty half of a row where only one side has a line: a faint plate,
/// quieter than either tint.
pub fn split_filler() -> Div {
    div().flex_1().min_w_0().self_stretch().bg(theme::ink(0.03))
}

/// The left half of a new file, which has no old version at all: hatched,
/// with the removal colour down its edge.
pub fn split_missing_side(theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .self_stretch()
        .flex()
        .child(edge(Some(theme.diff_del.opacity(0.55))))
        .child(hatching(theme.text_faint.opacity(0.22)))
}

/// Diagonal stripes, laid on the window's grid rather than each row's, so a
/// column of rows reads as one surface.
fn hatching(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (top, bottom) = (f32::from(bounds.top()), f32::from(bounds.bottom()));
            let (left, right) = (f32::from(bounds.left()), f32::from(bounds.right()));
            // Each stripe is a line x + y = k, for k on the grid.
            let mut k = ((left + top) / HATCH_GAP).floor() * HATCH_GAP;
            let mut stripes = PathBuilder::stroke(px(1.0));
            while k < right + bottom {
                stripes.move_to(point(px(k - top), px(top)));
                stripes.line_to(point(px(k - bottom), px(bottom)));
                k += HATCH_GAP;
            }
            if let Ok(path) = stripes.build() {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    window.paint_path(path, color);
                });
            }
        },
    )
    .flex_1()
    .self_stretch()
}

/// The left half of a row with no old line: hatched when the whole file is
/// new, the plain filler when only this line is.
pub fn empty_left(file: &FileDiff, theme: &Theme) -> AnyElement {
    match file.status {
        FileStatus::Added => split_missing_side(theme).into_any_element(),
        _ => split_filler().into_any_element(),
    }
}

/// The two halves of a split row and the divider between them. `ratio` is
/// the left half's share of the width.
pub fn split_row(left: AnyElement, right: AnyElement, ratio: f32, wrapped: bool) -> Div {
    let half = |share: f32, cell: AnyElement| {
        div()
            .flex_basis(px(0.0))
            .flex_grow(share)
            .min_w_0()
            .self_stretch()
            .flex()
            .child(cell)
    };
    // Both halves carry washes of their own, so the divider needs more
    // weight than a hairline between plain columns would.
    let divider = div()
        .flex_none()
        .w(px(HALF_RULE))
        .self_stretch()
        .bg(theme::hairline(0.14));
    line_height(div(), wrapped)
        .flex_none()
        .w_full()
        .flex()
        .items_stretch()
        .child(half(ratio, left))
        .child(divider)
        .child(half(1.0 - ratio, right))
}
