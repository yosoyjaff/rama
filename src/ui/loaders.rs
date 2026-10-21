//! The "working" mark: six round dots in two columns, with a light going
//! round them clockwise.
//!
//! Each spinner is its own small view, so its frames repaint only the six
//! dots and not the row it sits in. Its clock is the shared loop clock in
//! [`crate::ui::motion`], which also stops it under reduced motion.

use gpui_kit::{
    App, AppContext, Context, Entity, Hsla, IntoElement, ParentElement, Render, RenderOnce,
    SharedString, StyleRefinement, Styled, Window, div, px,
};

use crate::ui::motion::{self, SPINNER_TURN};

/// The faintest a dot gets: the shape stays, only the light moves.
const FAINT: f32 = 0.1;
/// Where each dot is on the way round, by row and column: down the right
/// side, back up the left.
const ROUND: [[u8; 2]; 3] = [[0, 1], [5, 2], [4, 3]];

/// The spinner in the theme's three accent tones, one per row. `key` tells
/// spinners apart in the same window.
pub fn glyph_spinner(key: impl Into<SharedString>, dot: f32, tones: [Hsla; 3]) -> impl IntoElement {
    Spinner {
        key: key.into(),
        dot,
        tones,
    }
}

/// How lit a dot is, `t` of a turn after the light passed it: bright, then
/// fading to [`FAINT`] by just under half a turn, staying there, and lighting
/// up again just before the light comes round. The long dark stretch is what
/// makes it read as one moving light.
fn brightness(t: f32) -> f32 {
    match t.rem_euclid(1.0) {
        t if t < 0.45 => 1.0 - (1.0 - FAINT) * t / 0.45,
        t if t < 0.92 => FAINT,
        t => FAINT + (1.0 - FAINT) * (t - 0.92) / 0.08,
    }
}

#[derive(IntoElement)]
struct Spinner {
    key: SharedString,
    dot: f32,
    tones: [Hsla; 3],
}

struct Dots {
    dot: f32,
    tones: [Hsla; 3],
}

impl RenderOnce for Spinner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (dot, tones) = (self.dot, self.tones);
        // The same view across frames, kept in the element's state.
        let view: Entity<Dots> = window.with_global_id(self.key.into(), |id, window| {
            window.with_element_state(id, |kept: Option<Entity<Dots>>, _| {
                let view = kept.unwrap_or_else(|| cx.new(|_| Dots { dot, tones }));
                view.update(cx, |dots, cx| {
                    if dots.dot != dot || dots.tones != tones {
                        (dots.dot, dots.tones) = (dot, tones);
                        cx.notify();
                    }
                });
                (view.clone(), view)
            })
        });
        view.cached(StyleRefinement::default().w(px(dot * 2.5)).h(px(dot * 4.0)))
    }
}

impl Render for Dots {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let turn = motion::loop_phase(SPINNER_TURN, cx.entity_id(), cx);
        let (dot, gap) = (self.dot, self.dot / 2.0);
        let rows = ROUND.iter().zip(self.tones).map(|(places, tone)| {
            let dots = places.iter().map(move |&place| {
                div()
                    .size(px(dot))
                    .rounded_full()
                    .bg(tone)
                    .opacity(brightness(turn + f32::from(place) / 6.0))
            });
            div().flex().gap(px(gap)).children(dots)
        });
        div().flex().flex_col().gap(px(gap)).children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_dims_rests_and_lights_up_again() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(near(brightness(0.0), 1.0));
        assert!(near(brightness(0.45), FAINT));
        assert!(near(brightness(0.7), FAINT));
        assert!(near(brightness(1.0), 1.0));
        assert!(near(brightness(-0.55), brightness(0.45)));
        let fading = brightness(0.2);
        assert!(FAINT < fading && fading < 1.0);
    }

    #[test]
    fn every_dot_has_its_own_place_round_the_ring() {
        let mut places: Vec<u8> = ROUND.iter().flatten().copied().collect();
        places.sort();
        assert_eq!(places, [0, 1, 2, 3, 4, 5]);
    }
}
