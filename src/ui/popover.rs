//! Floating surfaces: menus, dropdowns and modal dialogs, with the pieces that
//! go inside them (rows, separators, dialog text, buttons).
//!
//! A menu stays mounted for a moment after it is dismissed, so it can fade
//! out; a [`Layer`] holds its state through that and then lets it go.
//!
//! Every layer is `deferred` (painted above the rest of the window) and
//! `anchored` (kept inside the window). Layers `occlude`, because GPUI
//! hit-tests by paint order: without it, a click on a menu row would also
//! land on whatever is drawn underneath.

use std::time::{Duration, Instant};

use gpui_kit::{
    Anchor, AnyElement, Context, DispatchPhase, ElementId, FontWeight, Hsla, IntoElement,
    MouseDownEvent, Pixels, Point, SharedString, Size, anchored, canvas, deferred, div, point,
    prelude::*, px, white,
};

use crate::ui::motion;
use crate::ui::theme::{self, Theme, hairline, ink};
use crate::ui::typography::ui_rems;

// ---------------------------------------------------------------------------
// Lifetime
// ---------------------------------------------------------------------------

/// What a floating layer shows, and whether it is on its way out. GPUI drops
/// an element the frame its state goes, so a dismissed layer keeps its value
/// while it fades: input asks [`Self::is_open`] or [`Self::live_mut`], which
/// already treat it as gone; drawing asks [`Self::shown`], which still has it.
pub struct Layer<T> {
    value: Option<T>,
    /// When it was dismissed, while it fades.
    left_at: Option<Instant>,
}

impl<T> Default for Layer<T> {
    fn default() -> Self {
        Self {
            value: None,
            left_at: None,
        }
    }
}

impl<T> Layer<T> {
    pub fn open(&mut self, value: T) {
        self.value = Some(value);
        self.left_at = None;
    }

    pub fn is_open(&self) -> bool {
        self.value.is_some() && self.left_at.is_none()
    }

    /// The value to draw: open, or fading out.
    pub fn shown(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// The value to act on: only while open.
    pub fn live_mut(&mut self) -> Option<&mut T> {
        match self.left_at {
            None => self.value.as_mut(),
            Some(_) => None,
        }
    }

    /// When the fade-out began, while it plays.
    pub fn leaving_since(&self) -> Option<Instant> {
        self.left_at.filter(|_| self.value.is_some())
    }

    /// Start the fade-out and let the value go once it has played. `field`
    /// finds this layer again on the view that owns it. Does nothing unless
    /// open.
    pub fn dismiss<V: 'static>(&mut self, cx: &mut Context<V>, field: fn(&mut V) -> &mut Layer<T>)
    where
        T: 'static,
    {
        if !self.leave(Instant::now()) {
            return;
        }
        let wait = exit_span() + Duration::from_millis(20);
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(wait).await;
            view.update(cx, |view, cx| {
                field(view).forget_if_faded(Instant::now());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn leave(&mut self, now: Instant) -> bool {
        let open = self.is_open();
        if open {
            self.left_at = Some(now);
        }
        open
    }

    /// Drop the value once its fade-out is over. One opened again meanwhile
    /// has no `left_at` and stays.
    fn forget_if_faded(&mut self, now: Instant) {
        if self
            .left_at
            .is_some_and(|left| now.saturating_duration_since(left) >= exit_span())
        {
            *self = Self::default();
        }
    }
}

fn exit_span() -> Duration {
    motion::MENU_CLOSE.length()
}

/// How far a fade-out that began at `since` has got, eased. Read off the
/// wall clock each frame, so it only goes forward.
fn exit_progress(since: Instant) -> f32 {
    motion::MENU_CLOSE.ease_since(since, Instant::now())
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

/// The press that dismisses a menu is handled while it is captured. This
/// stops the same press once it bubbles back, so whatever lies under the
/// menu does not act on it as well.
fn swallow_dismissing_press(content: AnyElement) -> AnyElement {
    let guard = canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && !bounds.contains(&event.position) {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .inset_0();
    div()
        .relative()
        .child(guard)
        .child(content)
        .into_any_element()
}

/// The layer's motion: in when it opens, out once it is leaving. A leaving
/// menu gets a cover over it so its rows take no more clicks, and the exit
/// runs under its own id: reusing the entrance's would pick up its finished
/// clock and skip straight to the end.
fn with_motion(id: SharedString, closing: Option<Instant>, layer: gpui_kit::Div) -> AnyElement {
    match closing.map(exit_progress) {
        None => motion::appear(id, motion::Overlay::Menu, layer).into_any_element(),
        Some(t) => {
            let covered = layer.relative().child(div().absolute().inset_0().occlude());
            motion::menu_leaving(SharedString::from(format!("{id}-out")), t, covered)
                .into_any_element()
        }
    }
}

fn floating(
    id: SharedString,
    closing: Option<Instant>,
    layer: gpui_kit::Div,
) -> gpui_kit::Anchored {
    anchored()
        .anchor(Anchor::TopLeft)
        .snap_to_window_with_margin(px(8.0))
        .child(with_motion(id, closing, layer))
}

/// A menu at a point in the window: the context menus.
pub fn menu_at(
    id: impl Into<SharedString>,
    position: Point<Pixels>,
    content: AnyElement,
    closing: Option<Instant>,
) -> AnyElement {
    let layer = div().occlude().child(swallow_dismissing_press(content));
    deferred(floating(id.into(), closing, layer).position(position))
        .priority(1)
        .into_any_element()
}

/// A dropdown hanging 6px under the element it is a child of.
pub fn anchored_menu_below(
    id: impl Into<SharedString>,
    content: AnyElement,
    closing: Option<Instant>,
) -> AnyElement {
    let layer = div()
        .occlude()
        .pt(px(6.0))
        .child(swallow_dismissing_press(content));
    // A zero-size box at the trigger's bottom-left corner is the origin;
    // without it the layer would follow the trigger's own flex alignment.
    div()
        .absolute()
        .left_0()
        .bottom_0()
        .size_0()
        .child(deferred(floating(id.into(), closing, layer)).priority(1))
        .into_any_element()
}

/// A dialog in the middle of the window, over a dimmed backdrop that takes
/// every click. `viewport` is the window's size: an anchored layer is only
/// as big as its contents, so the backdrop needs it spelled out.
pub fn modal(id: impl Into<ElementId>, viewport: Size<Pixels>, card: AnyElement) -> AnyElement {
    let backdrop = div()
        .occlude()
        .w(viewport.width)
        .h(viewport.height)
        .bg(theme::scrim(0.6))
        .flex()
        .items_center()
        .justify_center()
        .child(motion::appear(
            id,
            motion::Overlay::Dialog,
            div().child(card),
        ));
    deferred(anchored().position(point(px(0.0), px(0.0))).child(backdrop))
        .priority(2)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Menu pieces
// ---------------------------------------------------------------------------

/// The card a menu's rows sit on.
pub fn popover_card(theme: &Theme) -> gpui_kit::Div {
    popover_surface(theme).overflow_hidden()
}

/// Between a menu card's edge and its rows.
const CARD_PADDING: f32 = 4.0;

/// [`popover_card`] without clipping, for a menu whose submenus hang
/// outside its edge. The card's padding keeps its rows off the corners.
pub fn popover_surface(theme: &Theme) -> gpui_kit::Div {
    div()
        .p(px(CARD_PADDING))
        .rounded(px(12.0))
        .border_1()
        .border_color(hairline(0.10))
        .bg(theme.surface_overlay)
        .shadow_lg()
        .text_size(ui_rems(13.0))
        .text_color(theme.text)
}

/// A clickable row. `active` marks the current choice; otherwise the row
/// washes in on hover under `fade_key`, which must be unique in the window
/// and stable from frame to frame. The caller gives it an id and a click.
pub fn menu_row(theme: &Theme, active: bool, fade_key: impl Into<SharedString>) -> gpui_kit::Div {
    let selected = theme.element_hover;
    let row = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .text_size(ui_rems(13.0))
        .cursor_pointer();
    if active {
        return row.bg(selected).text_color(theme.text);
    }
    let key = fade_key.into();
    let mut row = row
        .bg(motion::fade_color(&key, theme::wash(0.0), selected))
        .text_color(motion::fade_color(
            &key,
            theme.text.opacity(0.9),
            theme.text,
        ));
    // The caller's `.id()` is what makes the row able to keep hover state,
    // so the listener goes on through `interactivity` rather than `on_hover`.
    row.interactivity().on_hover(motion::on_hover_fade(key));
    row
}

/// A line between groups of rows, edge to edge across the card's padding.
pub fn menu_separator() -> gpui_kit::Div {
    div()
        .h(px(1.0))
        .my(px(CARD_PADDING))
        .mx(px(-CARD_PADDING))
        .bg(hairline(0.07))
}

/// The magnifier at the start of a search field. The glyph's lens sits off
/// its own box's centre, so it is nudged half a pixel.
pub fn palette_search_icon(theme: &Theme) -> gpui_kit::Div {
    div()
        .size(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            crate::ui::icons::icon(crate::ui::icons::MAGNIFER)
                .size(px(16.0))
                .relative()
                .left(px(0.5))
                .top(px(0.5))
                .text_color(theme.text_muted),
        )
}

// ---------------------------------------------------------------------------
// Dialog pieces
// ---------------------------------------------------------------------------

pub fn dialog_card(theme: &Theme) -> gpui_kit::Div {
    div()
        .w(px(360.0))
        .p(px(20.0))
        .flex()
        .flex_col()
        .rounded(px(16.0))
        .border_1()
        .border_color(hairline(0.10))
        .bg(theme.surface_dialog)
        .shadow_lg()
        .text_color(theme.text)
}

pub fn dialog_title(theme: &Theme, title: &str) -> gpui_kit::Div {
    div()
        .text_size(ui_rems(15.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.text)
        .child(SharedString::from(title.to_owned()))
}

pub fn dialog_body(theme: &Theme, text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .text_size(ui_rems(13.0))
        .line_height(px(19.0))
        .text_color(theme.text_muted)
        .child(text.into())
}

/// The frame around a text input in a dialog.
pub fn dialog_field(input: AnyElement) -> gpui_kit::Div {
    div()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(hairline(0.08))
        .bg(ink(0.04))
        .text_size(ui_rems(14.0))
        .child(input)
}

fn button(label: &str) -> gpui_kit::Div {
    div()
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .text_size(ui_rems(13.0))
        .cursor_pointer()
        .child(SharedString::from(label.to_owned()))
}

fn solid_button(label: &str, fill: Hsla, text: Hsla) -> gpui_kit::Div {
    button(label)
        .bg(fill)
        .text_color(text)
        .font_weight(FontWeight::MEDIUM)
        .hover(|style| style.opacity(0.9))
}

/// A quiet button that washes in on hover under `fade_key` (see
/// [`menu_row`]). The caller gives it an id and a click.
pub fn btn_ghost(theme: &Theme, label: &str, fade_key: impl Into<SharedString>) -> gpui_kit::Div {
    let key = fade_key.into();
    let mut button = button(label)
        .bg(motion::fade_color(&key, theme::wash(0.0), ink(0.06)))
        .text_color(motion::fade_color(&key, theme.text_muted, theme.text));
    button.interactivity().on_hover(motion::on_hover_fade(key));
    button
}

/// The button that does what the dialog is for.
pub fn btn_primary(theme: &Theme, label: &str) -> gpui_kit::Div {
    solid_button(label, theme.text, theme.on_solid)
}

/// The button that does what the dialog is for, when that destroys something.
pub fn btn_danger(theme: &Theme, label: &str) -> gpui_kit::Div {
    solid_button(label, theme.danger_strong, white())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dismissed_layer_is_drawn_but_takes_no_input() {
        let mut layer = Layer::default();
        assert!(layer.shown().is_none());
        layer.open(7);
        assert!(layer.is_open() && layer.leaving_since().is_none());
        assert_eq!(layer.live_mut().copied(), Some(7));

        assert!(layer.leave(Instant::now()));
        assert!(!layer.is_open());
        assert!(layer.live_mut().is_none());
        assert_eq!(layer.shown().copied(), Some(7));
        assert!(layer.leaving_since().is_some());
        assert!(
            !layer.leave(Instant::now()),
            "only the first dismissal counts"
        );
    }

    #[test]
    fn a_closed_layer_ignores_dismissal() {
        let mut layer: Layer<u8> = Layer::default();
        assert!(!layer.leave(Instant::now()));
        assert!(layer.shown().is_none());
    }

    #[test]
    fn the_value_goes_only_after_the_fade_and_unless_reopened() {
        let start = Instant::now();
        let mut layer = Layer::default();
        layer.open(1);
        layer.leave(start);
        layer.forget_if_faded(start);
        assert!(layer.shown().is_some(), "still fading");
        layer.forget_if_faded(start + exit_span() * 2);
        assert!(layer.shown().is_none());

        layer.open(1);
        layer.leave(start);
        layer.open(2);
        layer.forget_if_faded(start + exit_span() * 2);
        assert_eq!(layer.shown().copied(), Some(2));
    }

    #[test]
    fn exit_progress_runs_from_zero_to_one() {
        assert!(exit_progress(Instant::now()) < 0.5);
        assert_eq!(exit_progress(Instant::now() - exit_span() * 2), 1.0);
    }
}
