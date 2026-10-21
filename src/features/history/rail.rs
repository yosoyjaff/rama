//! How the graph is laid across its column: how far apart the lanes are at
//! the pane's width, and the path each line takes across its row.
//!
//! Everything about the column follows from one number, the gap between
//! lane centres. At full size it is [`LANE_GAP`]; a narrow pane squeezes it;
//! squeezed past reading, it drops to zero and every lane is drawn as one
//! rail. Animating between lanes and the rail is animating the gap.
//!
//! A line that changes lane runs straight down its lane, turns a short
//! circular corner onto the commit's row, and runs flat along the row (or the
//! reverse, for a line leaving the commit). Lines are point chains, which the
//! painter strokes whole or cuts into dashes.

use std::cell::Cell;
use std::f32::consts::FRAC_PI_2;
use std::rc::Rc;

use super::columns::SUBJECT_MIN;
use super::lanes::Line;
use super::node::NODE_HALF;

/// Between lane centres at full size.
const LANE_GAP: f32 = 22.0;
/// Before the first lane: its centre lines up with the header's text, and
/// the HEAD ring clears the pane's edge.
const LEAD: f32 = crate::ui::chrome::EDGE_INSET * 2.0 - NODE_HALF;
const TAIL: f32 = 12.0;
/// The column never shrinks below this to make room for the subjects.
const NARROWEST: f32 = 48.0;
/// The most of the pane the graph may take.
const MOST_OF_PANE: f32 = 0.34;
/// Squeezed widths move in steps this big, so dragging a divider does not
/// make the lanes shimmer.
const STEP: f32 = 2.0;
/// Below this gap the lanes become a rail; they come back only above the
/// second, so a width right at the edge does not flicker.
const RAIL_UNDER: f32 = 4.0;
const LANES_OVER: f32 = 5.0;
/// The radius of the corner a line turns onto its row with.
const CORNER: f32 = 8.0;
/// Straight pieces a corner is drawn with; at this size they read as a curve.
const CORNER_PIECES: usize = 8;
/// Length of a dash, and of the gap after it.
pub(super) const DASH: f32 = 2.0;

/// The column's width without its lanes: the room either side and the first
/// node.
fn edges() -> f32 {
    LEAD + TAIL + 2.0 * NODE_HALF
}

/// How the lanes are spread across the graph column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Spread {
    pub lanes: usize,
    /// Between lane centres; zero is the rail.
    pub gap: f32,
    /// Device pixels per point, which lane centres are rounded to.
    pub scale: f32,
}

impl Spread {
    /// Every lane at the full gap.
    pub fn full(lanes: usize) -> Self {
        Self {
            lanes: lanes.max(1),
            gap: LANE_GAP,
            scale: 1.0,
        }
    }

    /// The lanes fitted into `width`, if they need more.
    fn within(lanes: usize, width: f32) -> Self {
        let full = Self::full(lanes);
        if full.lanes == 1 || width >= full.width() {
            return full;
        }
        Self {
            gap: (width.max(edges()) - edges()) / (full.lanes - 1) as f32,
            ..full
        }
    }

    pub fn width(self) -> f32 {
        edges() + (self.lanes - 1) as f32 * self.gap
    }

    /// All lanes on one rail: there is no room to tell them apart.
    pub fn is_rail(self) -> bool {
        self.lanes > 1 && self.gap == 0.0
    }

    /// The centre of `lane`, on the device pixel grid.
    pub fn x(self, lane: usize) -> f32 {
        let x = LEAD + NODE_HALF + lane as f32 * self.gap;
        (x * self.scale).round() / self.scale
    }

    /// `t` of the way to `to`.
    pub fn toward(self, to: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            gap: self.gap + (to.gap - self.gap) * t,
            ..to
        }
    }

    /// The spread for a pane `pane` wide with `others` taken by the other
    /// columns, after `before`. The subjects come first: the graph takes at
    /// most a third of the pane and never their minimum. Squeezed widths
    /// move in whole steps, and a change of less than a step keeps `before`.
    fn for_pane(lanes: usize, pane: f32, others: f32, before: Self, scale: f32) -> Self {
        let full = Self::full(lanes);
        let room = (pane - others - SUBJECT_MIN)
            .min(pane * MOST_OF_PANE)
            .max(NARROWEST);
        let scale = scale.max(1.0);
        if full.width() <= NARROWEST || full.width() <= room {
            return Self { scale, ..full };
        }
        let wanted = Self::within(lanes, room);
        let rail_under = if before.is_rail() {
            LANES_OVER
        } else {
            RAIL_UNDER
        };
        if wanted.lanes > 1 && wanted.gap < rail_under {
            return Self {
                gap: 0.0,
                scale,
                ..full
            };
        }
        let stepped = Self::within(lanes, (room / STEP).floor() * STEP);
        let unchanged = before.lanes == stepped.lanes
            && !before.is_rail()
            && (before.width() - stepped.width()).abs() < STEP;
        Self {
            scale,
            ..if unchanged { before } else { stepped }
        }
    }
}

/// An animated change between lanes and the rail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Morph {
    pub from: Spread,
    pub to: Spread,
    /// Names the animation, so a new morph starts from the beginning.
    pub epoch: usize,
}

/// The column's spread, shared by the rows, their lines and the header.
/// `shown` is what is drawn; `target` is what the pane's width calls for,
/// which a morph eases `shown` towards.
#[derive(Clone)]
pub(super) struct Sizing {
    /// The most lanes this checkout has needed so far.
    pub lanes: usize,
    pub shown: Rc<Cell<Spread>>,
    target: Rc<Cell<Spread>>,
    morph: Rc<Cell<Option<Morph>>>,
}

impl Sizing {
    pub fn new() -> Self {
        let start = Spread::full(1);
        Self {
            lanes: 0,
            shown: Rc::new(Cell::new(start)),
            target: Rc::new(Cell::new(start)),
            morph: Rc::new(Cell::new(None)),
        }
    }

    /// Fit the column to a pane `pane` wide with `others` already taken.
    /// Going to or from the rail eases over (when `animate`); anything else
    /// is immediate. Returns the morph under way, if any.
    pub fn settle(&self, pane: f32, others: f32, scale: f32, animate: bool) -> Option<Morph> {
        let before = self.target.get();
        let next = Spread::for_pane(self.lanes, pane, others, before, scale);
        self.target.set(next);
        let running = self
            .morph
            .get()
            .filter(|morph| self.shown.get() != morph.to);
        match running {
            Some(morph) if morph.to.is_rail() == next.is_rail() => Some(morph),
            _ if animate && next.is_rail() != before.is_rail() => {
                let morph = Morph {
                    from: self.shown.get(),
                    to: next,
                    epoch: self.morph.get().map_or(0, |m| m.epoch.wrapping_add(1)),
                };
                self.morph.set(Some(morph));
                Some(morph)
            }
            _ => {
                self.morph.set(None);
                self.shown.set(next);
                None
            }
        }
    }
}

/// Points of the quarter circle of radius `r` around `centre`, from angle
/// `start` through a quarter turn in direction `turn` (±1), the first point
/// left out.
fn quarter(centre: (f32, f32), r: f32, start: f32, turn: f32) -> impl Iterator<Item = (f32, f32)> {
    (1..=CORNER_PIECES).map(move |i| {
        let angle = start + turn * FRAC_PI_2 * i as f32 / CORNER_PIECES as f32;
        (centre.0 + r * angle.cos(), centre.1 + r * angle.sin())
    })
}

/// The path of `line` across a row `height` tall whose commit sits in lane
/// `at`, in the row's coordinates. Lines reach `seam` past the top and
/// bottom so neighbouring rows join without a gap.
pub(super) fn path(
    line: Line,
    at: usize,
    spread: Spread,
    height: f32,
    seam: f32,
) -> Vec<(f32, f32)> {
    let middle = height / 2.0;
    let (top, bottom) = (-seam, height + seam);
    let node = spread.x(at);
    let lane = spread.x(line.lane());
    let straight = (lane - node).abs() <= f32::EPSILON;
    match line {
        Line::Past { .. } => vec![(lane, top), (lane, bottom)],
        Line::Into { .. } if straight => vec![(lane, top), (lane, middle)],
        Line::Out { .. } if straight => vec![(lane, middle), (lane, bottom)],
        Line::Into { .. } => {
            // Down the lane, round the corner, along the row to the node.
            let toward = (node - lane).signum();
            let r = CORNER.min(spread.gap).min(middle).min((node - lane).abs());
            let centre = (lane + toward * r, middle - r);
            let start = if toward > 0.0 {
                std::f32::consts::PI
            } else {
                0.0
            };
            let mut points = vec![(lane, top), (lane, middle - r)];
            points.extend(quarter(centre, r, start, -toward));
            points.push((node, middle));
            points
        }
        Line::Out { .. } => {
            // Along the row from the node, round the corner, down the lane.
            let away = (lane - node).signum();
            let r = CORNER.min(spread.gap).min(middle).min((lane - node).abs());
            let centre = (lane - away * r, middle + r);
            let mut points = vec![(node, middle), (lane - away * r, middle)];
            points.extend(quarter(centre, r, -FRAC_PI_2, away));
            points.push((lane, bottom));
            points
        }
    }
}

/// The dashes of a point chain, `DASH` on and `DASH` off, measured along the
/// chain so the rhythm carries on round corners.
pub(super) fn dashes(points: &[(f32, f32)]) -> Vec<Vec<(f32, f32)>> {
    let mut dashes: Vec<Vec<(f32, f32)>> = Vec::new();
    // Whether the last dash is still being drawn when a segment ends.
    let mut open = false;
    let mut walked = 0.0_f32;
    for pair in points.windows(2) {
        let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
        let length = (bx - ax).hypot(by - ay);
        if length <= f32::EPSILON {
            continue;
        }
        let point_at = |d: f32| {
            let t = (d - walked) / length;
            (ax + (bx - ax) * t, ay + (by - ay) * t)
        };
        let end = walked + length;
        let mut d = walked;
        while d < end {
            // Dashes are the even stretches of the pattern.
            let stretch = (d / DASH).floor();
            let boundary = (stretch + 1.0) * DASH;
            let until = boundary.min(end);
            if stretch as i64 % 2 == 0 {
                if !open {
                    dashes.push(vec![point_at(d)]);
                }
                dashes.last_mut().expect("a dash").push(point_at(until));
                open = until < boundary;
            } else {
                open = false;
            }
            d = until;
        }
        walked = end;
    }
    dashes
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 28.0;

    fn length(points: &[(f32, f32)]) -> f32 {
        points
            .windows(2)
            .map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1))
            .sum()
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn a_line_past_the_commit_is_straight() {
        let spread = Spread::full(3);
        let line = Line::Past {
            lane: 1,
            dashed: false,
        };
        let x = spread.x(1);
        assert_eq!(path(line, 0, spread, ROW, 0.5), [(x, -0.5), (x, ROW + 0.5)]);
    }

    #[test]
    fn an_arrival_bends_onto_the_row_and_ends_at_the_node() {
        let spread = Spread::full(4);
        let line = Line::Into {
            lane: 3,
            dashed: false,
        };
        let points = path(line, 0, spread, ROW, 0.0);
        assert!(close(points[0], (spread.x(3), 0.0)));
        assert!(close(*points.last().unwrap(), (spread.x(0), ROW / 2.0)));
        // The corner is the same small radius however far the line goes,
        // and every point lies between the lane and the node.
        assert!(close(
            points[2 + CORNER_PIECES - 1],
            (spread.x(3) - CORNER, ROW / 2.0)
        ));
        assert!(points.iter().all(|&(x, y)| {
            x >= spread.x(0) - 1e-3 && x <= spread.x(3) + 1e-3 && y <= ROW / 2.0 + 1e-3
        }));
    }

    #[test]
    fn a_departure_runs_along_the_row_then_down_its_lane() {
        let spread = Spread::full(3);
        let line = Line::Out {
            lane: 2,
            dashed: false,
        };
        let points = path(line, 0, spread, ROW, 0.0);
        assert!(close(points[0], (spread.x(0), ROW / 2.0)));
        assert!(close(*points.last().unwrap(), (spread.x(2), ROW)));
        assert!(close(
            points[points.len() - 2],
            (spread.x(2), ROW / 2.0 + CORNER)
        ));
        assert!(points.iter().all(|&(x, y)| {
            x >= spread.x(0) - 1e-3 && x <= spread.x(2) + 1e-3 && y >= ROW / 2.0 - 1e-3
        }));
    }

    #[test]
    fn leftward_lines_bend_the_other_way() {
        let spread = Spread::full(3);
        let into = path(
            Line::Into {
                lane: 0,
                dashed: false,
            },
            2,
            spread,
            ROW,
            0.0,
        );
        assert!(close(*into.last().unwrap(), (spread.x(2), ROW / 2.0)));
        let out = path(
            Line::Out {
                lane: 0,
                dashed: false,
            },
            2,
            spread,
            ROW,
            0.0,
        );
        assert!(close(*out.last().unwrap(), (spread.x(0), ROW)));
        assert!(
            out.iter()
                .all(|&(x, _)| x <= spread.x(2) + 1e-3 && x >= spread.x(0) - 1e-3)
        );
    }

    #[test]
    fn squeezed_lanes_shrink_the_corner() {
        let spread = Spread::within(10, 60.0);
        assert!(spread.gap < CORNER);
        let points = path(
            Line::Into {
                lane: 1,
                dashed: false,
            },
            0,
            spread,
            ROW,
            0.0,
        );
        assert!(close(*points.last().unwrap(), (spread.x(0), ROW / 2.0)));
    }

    #[test]
    fn dashes_keep_their_rhythm_round_corners() {
        let chain = [(0.0, 0.0), (0.0, 5.0), (5.0, 5.0)];
        let found = dashes(&chain);
        assert_eq!(found.len(), 3);
        for dash in &found[..2] {
            assert!((length(dash) - DASH).abs() < 1e-4);
        }
        assert_eq!(found[1].len(), 3, "the second dash bends round the corner");
    }

    #[test]
    fn a_roomy_pane_keeps_the_full_spread() {
        let spread = Spread::for_pane(3, 1_200.0, 240.0, Spread::full(3), 1.0);
        assert_eq!(spread, Spread::full(3));
    }

    #[test]
    fn a_narrow_pane_squeezes_the_lanes_before_the_subjects() {
        let spread = Spread::for_pane(12, 600.0, 240.0, Spread::full(12), 1.0);
        assert!(spread.gap > 0.0 && spread.gap < LANE_GAP);
        assert!(spread.width() <= 600.0 * MOST_OF_PANE + 0.01);
    }

    #[test]
    fn the_rail_comes_in_when_lanes_blur_and_goes_a_little_later() {
        let roomy = Spread::full(5);
        // 4.5 apart: still readable on the way in, not enough to leave.
        let room = edges() + 4.0 * 4.5;
        let pane = 1_000.0;
        let others = pane - SUBJECT_MIN - room;
        assert!(!Spread::for_pane(5, pane, others, roomy, 1.0).is_rail());
        let rail = Spread { gap: 0.0, ..roomy };
        assert!(Spread::for_pane(5, pane, others, rail, 1.0).is_rail());
        assert!(Spread::for_pane(40, 300.0, 0.0, roomy, 1.0).is_rail());
        assert!(!Spread::full(1).is_rail());
    }

    #[test]
    fn small_resizes_keep_the_spread_and_lanes_sit_on_device_pixels() {
        let before = Spread::for_pane(8, 101.0 / MOST_OF_PANE, 0.0, Spread::full(8), 2.0);
        let after = Spread::for_pane(8, 101.6 / MOST_OF_PANE, 0.0, before, 2.0);
        assert_eq!(before, after);
        for lane in 0..8 {
            assert_eq!((after.x(lane) * 2.0).fract(), 0.0);
        }
    }

    #[test]
    fn a_morph_reaches_the_rail_only_at_its_end_and_leaves_it_at_once() {
        let full = Spread::full(4);
        let rail = Spread { gap: 0.0, ..full };
        assert!(!full.toward(rail, 0.5).is_rail());
        assert!(full.toward(rail, 1.0).is_rail());
        assert!(rail.toward(full, 0.0).is_rail());
        assert!(!rail.toward(full, 0.1).is_rail());
        assert!((full.toward(rail, 0.5).gap - LANE_GAP / 2.0).abs() < 1e-4);
    }
}
