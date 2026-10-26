//! Drawing the graph: the lines, painted once over the whole list, and each
//! row's commit node, with the button that folds a branch.

use super::row::ROW_HEIGHT;
use std::time::Duration;

use gpui_kit::{
    AnyElement, App, Bounds, Context, Div, Hsla, MouseButton, ObjectFit, PathBuilder, Pixels,
    SharedString, Window, canvas, div, img, point, prelude::*, px,
};

use super::cells::{history_author_initial, history_author_name, lane_palette};
use super::lanes::{Mark, Row, branch_ref_key};
use super::rail::{dashes, path};
use super::{HistoryView, Scope};
use crate::ui::icons;
use crate::ui::theme::Theme;
use crate::ui::tip::tip;

// A commit's node is its author's photo inside a ring of the lane's colour,
// the ring as thick as the lines. Merges and the working tree get a dot.
const PHOTO: f32 = 18.0;
const RING: f32 = 2.0;
const NODE: f32 = PHOTO + 2.0 * RING;
pub(super) const NODE_HALF: f32 = NODE / 2.0;
const LINE: f32 = RING;
const MERGE_DOT: f32 = 12.0;
const STASH_GLYPH: f32 = 12.0;
/// How far a line runs into the next row, so rows join without a seam.
const SEAM: f32 = 0.75;

/// The square outline of a stash node as dots: `side` wide, a dot of
/// `size` every `2 × size`.
fn dotted_square(side: f32, size: f32) -> Vec<(f32, f32)> {
    let count = ((side - size) / (2.0 * size)).round() as usize;
    let mut dots = Vec::with_capacity(4 * count);
    for i in 0..=count {
        let along = i as f32 * 2.0 * size;
        dots.extend([(along, 0.0), (along, side - size)]);
        if (1..count).contains(&i) {
            dots.extend([(0.0, along), (side - size, along)]);
        }
    }
    dots
}

impl HistoryView {
    /// The lines of the whole graph, painted over the list in one pass per
    /// colour. Each row's lines go where the list says that row is now, so
    /// they follow scrolling and rows animating their height.
    pub(super) fn graph_paths(&self, theme: &Theme, cx: &App) -> AnyElement {
        let palette = lane_palette(theme, cx);
        let rows = self.graph.rows.clone();
        let list = self.list.clone();
        let spread = self.sizing.shown.clone();
        // Lanes start after the branch badges.
        let lane_start = self.lane_origin_x();
        // The tips overview has no lines between rows, even as a rail.
        let rail_allowed = self.scope == Scope::Everything;
        canvas(
            |_, _, _| (),
            move |view: Bounds<Pixels>, _, window: &mut Window, _| {
                let spread = spread.get();
                let rail = spread.is_rail() && rail_allowed;
                // The rows on screen and where each one is.
                let on_screen: Vec<(usize, Bounds<Pixels>)> = (0..rows.len())
                    .filter_map(|i| Some((i, list.bounds_for_item(i)?)))
                    .filter(|(_, b)| b.bottom() >= view.top() && b.top() <= view.bottom())
                    .filter(|(_, b)| f32::from(b.size.height) > 0.5)
                    .collect();
                for (shade, color) in palette.iter().enumerate() {
                    let mut builder = PathBuilder::stroke(px(LINE));
                    let mut any = false;
                    for &(i, bounds) in &on_screen {
                        let row = &rows[i];
                        let height = f32::from(bounds.size.height);
                        let origin = point(bounds.origin.x + px(lane_start), bounds.origin.y);
                        if rail {
                            // One line from this node's middle to the next one's.
                            if row.lane % palette.len() != shade {
                                continue;
                            }
                            let x = origin.x + px(spread.x(0));
                            let to = list.bounds_for_item(i + 1).map_or(bounds.bottom(), |next| {
                                next.origin.y + next.size.height / 2.0
                            });
                            builder.move_to(point(x, origin.y + px(height / 2.0)));
                            builder.line_to(point(x, to));
                            any = true;
                            continue;
                        }
                        // Shrinking rows shrink their overlap too.
                        let seam = SEAM * (height / ROW_HEIGHT).min(1.0);
                        let at = |(x, y): (f32, f32)| point(origin.x + px(x), origin.y + px(y));
                        for &line in row
                            .lines
                            .iter()
                            .filter(|line| line.lane() % palette.len() == shade)
                        {
                            let points = path(line, row.lane, spread, height, seam);
                            let strokes = match line.dashed() {
                                true => dashes(&points),
                                false => vec![points],
                            };
                            for stroke in strokes {
                                let Some((&first, rest)) = stroke.split_first() else {
                                    continue;
                                };
                                builder.move_to(at(first));
                                for &next in rest {
                                    builder.line_to(at(next));
                                }
                            }
                            any = true;
                        }
                    }
                    if any && let Ok(path) = builder.build() {
                        window.paint_path(path, *color);
                    }
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    /// The graph's cell on row `index`: the commit's node, and on a row with
    /// a branch, the button that folds it (shown on hover, or always while
    /// folded).
    pub(super) fn graph_cell(
        &mut self,
        index: usize,
        row: Row,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spread = self.sizing.shown.get();
        let palette = lane_palette(theme, cx);
        let color = palette[row.lane % palette.len()];
        let commit = self.shown_rows.get(index).cloned();
        let x = spread.x(row.lane);
        // Merges and the working tree are smaller than a commit, which is as
        // big as an author's photo.
        let radius = match row.mark {
            Mark::Merge | Mark::WorkingTree => MERGE_DOT / 2.0,
            Mark::Commit | Mark::Stash => NODE_HALF,
        };
        // An opaque disc in the background colour, so lines passing under
        // stop at its edge, ringed in the lane's colour.
        let disc = div()
            .absolute()
            .left(px(x - radius))
            .top(px(ROW_HEIGHT / 2.0 - radius))
            .size(px(2.0 * radius))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded_full()
            .border(px(RING))
            .border_color(color)
            .bg(theme.bg);
        let node = if row.mark == Mark::Merge {
            disc.bg(color)
        } else if row.mark == Mark::Stash {
            stash_node(disc, 2.0 * radius, color)
        } else if row.mark == Mark::WorkingTree {
            disc.border_dashed()
        } else {
            // A commit: the author's photo, or their initials on a tint.
            let photo = commit.as_ref().and_then(|c| {
                let email = c.author.email.trim().to_ascii_lowercase();
                self.photos.authors.get(&email).cloned()
            });
            match photo {
                Some(photo) => disc.child(
                    img(photo)
                        .flex_none()
                        .size(px(PHOTO))
                        .rounded_full()
                        .object_fit(ObjectFit::Cover),
                ),
                None => disc.child(
                    div()
                        .flex_none()
                        .size(px(PHOTO))
                        .rounded_full()
                        .bg(color.opacity(0.22))
                        .font_family(theme.font_sans.clone())
                        .text_size(px(10.0))
                        .text_center()
                        // A line as tall as the disc centres the letter.
                        .line_height(px(PHOTO))
                        .text_color(color)
                        .child(history_author_initial(&history_author_name(
                            commit.as_ref().map_or("", |c| c.author.name.as_str()),
                        ))),
                ),
            }
        };
        let fold = commit
            .as_ref()
            .filter(|_| self.scope == Scope::Everything)
            .and_then(|c| c.refs.iter().find(|r| branch_ref_key(r).is_some()).cloned())
            .map(|branch| self.fold_button(&row.sha, branch, x, color, theme, cx));
        div()
            .id(SharedString::from(format!("history-node-{}", row.sha)))
            .relative()
            .group("history-graph-tip")
            .flex_none()
            .w(px(spread.width()))
            .h(px(ROW_HEIGHT))
            .child(node)
            .children(fold)
            .into_any_element()
    }

    fn fold_button(
        &self,
        sha: &str,
        branch: crate::git::RefBadge,
        node_x: f32,
        color: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = branch_ref_key(&branch).unwrap_or_default();
        let folded = self.folds.closed.contains(&key);
        let hidden = self.folds.hidden.get(&key).copied().unwrap_or(0);
        let hint: SharedString = match (folded, hidden) {
            (false, _) => format!("Collapse {}", branch.label),
            (true, 0) => format!("Expand {}", branch.label),
            (true, n) => format!("Expand {} ({n} hidden)", branch.label),
        }
        .into();
        let history = cx.entity();
        div()
            .id(SharedString::from(format!("history-fold-{sha}")))
            .absolute()
            .left(px(node_x + NODE_HALF + 3.0))
            .top(px((ROW_HEIGHT - 16.0) / 2.0))
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(color.opacity(0.32))
            .bg(theme.bg.opacity(0.96))
            .cursor_pointer()
            .opacity(if folded { 1.0 } else { 0.0 })
            .group_hover("history-graph-tip", |style| style.opacity(1.0))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let branch = branch.clone();
                history.update(cx, |history, cx| history.toggle_fold(branch, cx));
            })
            .child(
                icons::icon(if folded {
                    icons::EXPAND_ARROWS
                } else {
                    icons::FOLD_VERTICAL
                })
                .size(px(9.0))
                .text_color(color.opacity(0.9)),
            )
            .tooltip(tip(hint))
            .tooltip_show_delay(Duration::from_millis(250))
            .into_any_element()
    }
}

/// A stash's node: a dotted square with the stash mark inside. GPUI has no
/// dotted border, so the dots are drawn one by one.
fn stash_node(disc: Div, side: f32, color: Hsla) -> Div {
    let dot = RING;
    disc.rounded(px(0.0))
        .border(px(0.0))
        .children(dotted_square(side, dot).into_iter().map(|(x, y)| {
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .size(px(dot))
                .rounded_full()
                .bg(color)
        }))
        .child(
            icons::icon(icons::ARCHIVE_04)
                .size(px(STASH_GLYPH))
                .text_color(color),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stash_square_is_dotted_evenly_on_all_four_sides() {
        let dots = dotted_square(22.0, 2.0);
        // Six dots along the top and the bottom, four more down each side.
        assert_eq!(dots.len(), 6 * 2 + 4 * 2);
        let mut top: Vec<f32> = dots.iter().filter(|d| d.1 == 0.0).map(|d| d.0).collect();
        top.sort_by(f32::total_cmp);
        assert_eq!(top, [0.0, 4.0, 8.0, 12.0, 16.0, 20.0]);
        assert!(
            dots.iter()
                .all(|&(x, y)| (0.0..=20.0).contains(&x) && (0.0..=20.0).contains(&y))
        );
    }
}
