//! Tooltips: GPUI Kit's own, built from text. A `\n` in the text starts a
//! new line.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{AnyView, App, SharedString, Window};

/// What `.tooltip(…)` takes, showing `text`.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}
