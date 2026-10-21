//! How things move: overlays appearing and leaving, panes and row groups
//! changing size, the loaders' loops, and hover colours fading.
//!
//! A movement is a [`Timing`], a length and a curve. Callers either hand
//! GPUI its [`Animation`] or ask it how far along it is since an instant
//! they kept, which is what a movement that must survive a remount needs.
//!
//! Reduced motion is GPUI's own `App::reduce_motion`: `with_animation` snaps
//! to the end under it, and nothing here keeps frames coming.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::{
    Animation, AnimationElement, App, ElementId, EntityId, Global, Hsla, IntoElement, Rgba,
    SharedString, Styled, Window, px,
};

pub use gpui_kit::AnimationExt;

/// A timing curve as CSS writes it, `cubic-bezier(x1, y1, x2, y2)`: a cubic
/// Bézier from (0, 0) to (1, 1) through two control points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    x: (f32, f32),
    y: (f32, f32),
}

impl Curve {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self {
            x: (x1, x2),
            y: (y1, y2),
        }
    }

    /// The curve's value where it is `x` across, from 0 to 1.
    ///
    /// Across only grows along a timing curve, so the Bézier parameter for
    /// `x` is found by bisection; 24 rounds get below what an `f32` can
    /// resolve on 0..1.
    pub fn at(&self, x: f32) -> f32 {
        let axis = |(c1, c2): (f32, f32), s: f32| {
            let r = 1.0 - s;
            3.0 * s * r * (r * c1 + s * c2) + s * s * s
        };
        if x <= 0.0 || x >= 1.0 {
            return x.clamp(0.0, 1.0);
        }
        let mut span = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let mid = (span.0 + span.1) / 2.0;
            span = match axis(self.x, mid) < x {
                true => (mid, span.1),
                false => (span.0, mid),
            };
        }
        // GPUI checks that eased values stay inside 0..=1, and rounding at
        // the end can overshoot by a hair.
        axis(self.y, (span.0 + span.1) / 2.0).clamp(0.0, 1.0)
    }
}

/// CSS `ease`.
pub const EASE: Curve = Curve::new(0.25, 0.1, 0.25, 1.0);
/// CSS `ease-out`.
pub const EASE_OUT: Curve = Curve::new(0.0, 0.0, 0.58, 1.0);
/// Material's standard curve: quick to start, long to settle.
const STANDARD: Curve = Curve::new(0.4, 0.0, 0.2, 1.0);

/// How long a movement takes and how it eases.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    base: Duration,
    curve: Curve,
}

impl Timing {
    pub const fn ms(ms: u64, curve: Curve) -> Self {
        Self {
            base: Duration::from_millis(ms),
            curve,
        }
    }

    /// How long it takes, stretched by [`speed_scale`].
    pub fn length(&self) -> Duration {
        self.base.mul_f32(speed_scale())
    }

    /// The eased value a `linear` share of the way through.
    pub fn ease(&self, linear: f32) -> f32 {
        match self.base.is_zero() {
            true => 1.0,
            false => self.curve.at(linear.clamp(0.0, 1.0)),
        }
    }

    /// The eased value once `start` is this far in the past.
    pub fn ease_since(&self, start: Instant, now: Instant) -> f32 {
        let length = self.length().as_secs_f32();
        match length > 0.0 {
            true => self.ease(now.saturating_duration_since(start).as_secs_f32() / length),
            false => 1.0,
        }
    }

    /// A GPUI animation that plays once.
    pub fn animation(&self) -> Animation {
        let timing = *self;
        Animation::new(self.length()).with_easing(move |linear| timing.ease(linear))
    }
}

/// A menu going away: quicker than it came, so it is out of the way.
pub const MENU_CLOSE: Timing = Timing::ms(90, EASE);
/// A pane changing width.
pub const RESIZE: Timing = Timing::ms(220, EASE_OUT);
/// A group of rows opening or shutting.
pub const COLLAPSE: Timing = Timing::ms(160, EASE_OUT);
/// One turn of the working indicator.
pub const SPINNER_TURN: Duration = Duration::from_millis(800);
const HOVER: Timing = Timing::ms(120, STANDARD);

// ── Overlays ────────────────────────────────────────────────────────────────
//
// GPUI cannot scale a div, so an overlay arrives as a fade with a couple of
// pixels of travel. The travel is a relative `top`, which is applied after
// layout: like a CSS transform, it never moves anything else.

/// What is appearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// Drops into place from just above, already partly visible.
    Menu,
    /// Rises into place from nothing.
    Dialog,
}

impl Overlay {
    /// (timing, opacity at the start, pixels of travel; up is negative).
    fn entrance(self) -> (Timing, f32, f32) {
        match self {
            Self::Menu => (Timing::ms(130, EASE), 0.35, -2.0),
            Self::Dialog => (Timing::ms(170, EASE), 0.0, 3.0),
        }
    }
}

/// `element` appearing as an `overlay`.
pub fn appear<E>(id: impl Into<ElementId>, overlay: Overlay, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    let (timing, start, travel) = overlay.entrance();
    element.with_animation(id, timing.animation(), move |el, t| {
        let left = 1.0 - t;
        el.relative()
            .opacity(start + (1.0 - start) * t)
            .top(px(travel * left))
    })
}

/// A menu leaving, `t` of the way out by the caller's clock. The animation
/// is only there to keep frames coming: its own clock starts over whenever
/// the element remounts, and replaying the exit would flash the menu back.
pub fn menu_leaving<E>(id: impl Into<ElementId>, t: f32, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    // Back the way it came in.
    let (_, _, travel) = Overlay::Menu.entrance();
    element.with_animation(id, MENU_CLOSE.animation(), move |el, _| {
        el.relative().opacity(1.0 - t).top(px(travel * t))
    })
}

// ── Loops ───────────────────────────────────────────────────────────────────
//
// A repeating `with_animation` asks for every display refresh while it is
// mounted: a small spinner would keep a 120 Hz screen busy. Loaders instead
// read one shared clock, which wakes the views that drew one at about 30 Hz
// and stops once none has for a moment.

/// How often a view showing a loop is woken.
const LOOP_TICK: Duration = Duration::from_millis(33);
/// How long a view is still woken after it last drew a loop.
const LOOP_LINGER: Duration = Duration::from_millis(300);

struct Ticker {
    epoch: Instant,
    /// The views that drew a loop, and until when to wake them.
    watching: HashMap<EntityId, Instant>,
    running: bool,
}

impl Global for Ticker {}

impl Default for Ticker {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            watching: HashMap::new(),
            running: false,
        }
    }
}

impl Ticker {
    /// Drop the views that stopped drawing, and wake the rest. False once
    /// nobody is left.
    fn tick(&mut self, now: Instant, cx: &mut App) -> bool {
        self.watching.retain(|_, until| *until > now);
        for &view in self.watching.keys() {
            cx.notify(view);
        }
        self.running = !self.watching.is_empty();
        self.running
    }
}

/// How far into a loop of `period` every loader is now, from 0 up to 1, and
/// a promise to redraw `view` for as long as it keeps asking. All loops
/// count from the same instant, so spinners on screen turn together. With
/// reduced motion it is always 0 and nothing is scheduled.
pub fn loop_phase(period: Duration, view: EntityId, cx: &mut App) -> f32 {
    if cx.reduce_motion() {
        return 0.0;
    }
    let ticker = cx.default_global::<Ticker>();
    ticker.watching.insert(view, Instant::now() + LOOP_LINGER);
    let phase = (ticker.epoch.elapsed().as_secs_f32() / period.as_secs_f32()).fract();
    if !ticker.running {
        ticker.running = true;
        cx.spawn(async move |cx| {
            let mut going = true;
            while going {
                cx.background_executor().timer(LOOP_TICK).await;
                going = cx.update(|cx| {
                    let mut ticker = cx.remove_global::<Ticker>();
                    let going = ticker.tick(Instant::now(), cx);
                    cx.set_global(ticker);
                    going
                });
            }
        })
        .detach();
    }
    phase
}

// ── Hover fades ─────────────────────────────────────────────────────────────
//
// `.hover()` swaps a style on the frame the pointer arrives. To fade
// instead, each element is known by a key: a listener notes when the
// pointer came or went, the element asks for its colour while it is built,
// and the window keeps redrawing until every fade has finished.
//
// The fades live in a thread-local, not a GPUI global, so the free
// functions that build rows can ask without a context. All of it runs on
// the UI thread.

/// A key's fade toward hovered or not, from the level it had then.
#[derive(Debug, Clone, Copy)]
struct Fade {
    hovered: bool,
    from: f32,
    since: Instant,
}

impl Fade {
    fn target(&self) -> f32 {
        if self.hovered { 1.0 } else { 0.0 }
    }

    fn level(&self, now: Instant) -> f32 {
        let t = HOVER.ease_since(self.since, now);
        self.from + (self.target() - self.from) * t
    }

    fn done(&self, now: Instant) -> bool {
        HOVER.ease_since(self.since, now) >= 1.0
    }
}

#[derive(Default)]
struct Fades(HashMap<SharedString, Fade>);

impl Fades {
    /// The pointer came to or left `key`. Turning back mid-way starts from
    /// where the fade had got to, so it never jumps; `at_once` skips it.
    fn turn(&mut self, key: &SharedString, hovered: bool, at_once: bool, now: Instant) {
        let level = match self.0.get(key) {
            Some(fade) => fade.level(now),
            // Leaving something never entered: nothing to fade.
            None if !hovered => return,
            None => 0.0,
        };
        let from = match at_once {
            true => f32::from(u8::from(hovered)),
            false => level,
        };
        self.0.insert(
            key.clone(),
            Fade {
                hovered,
                from,
                since: now,
            },
        );
    }

    fn level(&self, key: &str, now: Instant) -> f32 {
        self.0.get(key).map_or(0.0, |fade| fade.level(now))
    }

    /// Forget the fades that ended unhovered; true while any is moving.
    fn sweep(&mut self, now: Instant) -> bool {
        self.0.retain(|_, fade| fade.hovered || !fade.done(now));
        self.0.values().any(|fade| !fade.done(now))
    }
}

thread_local! {
    static FADES: RefCell<Fades> = RefCell::new(Fades::default());
}

/// The `on_hover` listener for an element that reads its colour with
/// [`fade_color`] under the same `key`.
pub fn on_hover_fade(
    key: impl Into<SharedString>,
) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let key = key.into();
    move |hovered, window, cx| {
        let at_once = cx.reduce_motion();
        FADES.with(|fades| {
            fades
                .borrow_mut()
                .turn(&key, *hovered, at_once, Instant::now())
        });
        window.refresh();
        redraw_while_fading(window);
    }
}

fn redraw_while_fading(window: &Window) {
    window.on_next_frame(|window, _| {
        if FADES.with(|fades| fades.borrow_mut().sweep(Instant::now())) {
            window.refresh();
            redraw_while_fading(window);
        }
    });
}

/// `rest`, or `hovered`, or between them as far as `key`'s fade has got.
pub fn fade_color(key: &str, rest: Hsla, hovered: Hsla) -> Hsla {
    let t = FADES.with(|fades| fades.borrow().level(key, Instant::now()));
    mix(rest, hovered, t)
}

/// `a` toward `b` by `t`, as browsers animate colours: per sRGB channel,
/// weighted by alpha, so fading in from transparent keeps the target's hue
/// rather than passing through grey.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    match t {
        t if t <= 0.0 => return a,
        t if t >= 1.0 => return b,
        _ => {}
    }
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let (wa, wb) = (a.a * (1.0 - t), b.a * t);
    let alpha = wa + wb;
    if alpha <= f32::EPSILON {
        return Hsla::from(Rgba { a: 0.0, ..b });
    }
    let channel = |x: f32, y: f32| (x * wa + y * wb) / alpha;
    Hsla::from(Rgba {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: alpha,
    })
}

// ── Switches ────────────────────────────────────────────────────────────────

/// `RAMA_MOTION_SCALE` (default 1) stretches every timing, to watch a
/// movement slowly while working on it. Read once.
pub fn speed_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("RAMA_MOTION_SCALE")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|scale| scale.is_finite())
            .map_or(1.0, |scale| scale.clamp(0.01, 100.0))
    })
}

/// Whether the user asked for less motion.
pub fn reduced_motion(cx: &App) -> bool {
    cx.reduce_motion()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_run_from_zero_to_one_without_falling_back() {
        for curve in [EASE, EASE_OUT, STANDARD] {
            let values: Vec<f32> = (0..=50).map(|i| curve.at(i as f32 / 50.0)).collect();
            assert_eq!((values[0], values[50]), (0.0, 1.0), "{curve:?}");
            assert!(
                values.windows(2).all(|pair| pair[0] <= pair[1]),
                "{curve:?}"
            );
        }
        assert!(EASE_OUT.at(0.5) > 0.5, "ease-out is ahead at half time");
    }

    #[test]
    fn evenly_placed_control_points_make_a_straight_line() {
        let straight = Curve::new(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0);
        for x in [0.1, 0.3, 0.6, 0.9] {
            assert!((straight.at(x) - x).abs() < 1e-4, "{x}");
        }
    }

    #[test]
    fn a_timing_clamps_and_an_instant_one_is_already_over() {
        assert_eq!(RESIZE.ease(-0.5), 0.0);
        assert_eq!(RESIZE.ease(1.5), 1.0);
        assert_eq!(Timing::ms(0, EASE).ease(0.0), 1.0);
        let start = Instant::now();
        assert_eq!(RESIZE.ease_since(start, start + RESIZE.length() * 2), 1.0);
    }

    #[test]
    fn turning_back_mid_fade_starts_where_it_was() {
        let key = SharedString::from("row");
        let start = Instant::now();
        let mut fades = Fades::default();
        fades.turn(&key, true, false, start);
        let middle = start + HOVER.length() / 2;
        let reached = fades.level("row", middle);
        assert!(reached > 0.0 && reached < 1.0);

        fades.turn(&key, false, false, middle);
        assert!((fades.level("row", middle) - reached).abs() < 1e-6);
        assert!(fades.sweep(middle), "still moving");

        let after = middle + HOVER.length() * 2;
        assert_eq!(fades.level("row", after), 0.0);
        assert!(!fades.sweep(after));
        assert!(fades.0.is_empty(), "a finished fade out is forgotten");
    }

    #[test]
    fn at_once_lands_immediately_and_leaving_unentered_is_ignored() {
        let now = Instant::now();
        let mut fades = Fades::default();
        fades.turn(&"gone".into(), false, false, now);
        assert!(fades.0.is_empty());
        fades.turn(&"row".into(), true, true, now);
        assert_eq!(fades.level("row", now), 1.0);
    }

    #[test]
    fn mixing_from_transparent_keeps_the_hue() {
        let clear = Hsla::from(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        });
        let white = Hsla::from(Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.2,
        });
        let half = Rgba::from(mix(clear, white, 0.5));
        assert!((half.r - 1.0).abs() < 1e-3 && (half.a - 0.1).abs() < 1e-3);
        assert_eq!(mix(clear, white, 0.0), clear);
        assert_eq!(mix(clear, white, 1.0), white);
    }
}
