//! Toasts: the receipt an action leaves behind, in the component library's
//! own notification stack.
//!
//! An action that changes something the window does not otherwise show —
//! a pull, a push, a stash — says so here, and gets out of the way. Every
//! toast leaves on its own; hovering one holds it, so git's words on a
//! failure can still be read to the end.
//!
//! [`show`] takes `&mut App` rather than a window because the code that
//! learns the outcome (an entity update, a background task's completion)
//! rarely holds one. The toast goes to the active window, after the current
//! update, so it never collides with the window borrow it was raised from.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::{Anchor, App};

/// What the toast reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// It worked. Leaves on its own.
    Success,
    /// It did not. Leaves on its own too; hovering holds it.
    Error,
}

/// The longest receipt that still reads as a title. Anything longer — git's
/// own output, a list of conflicted files — is a message under one.
const TITLE_MAX: usize = 60;

/// Raise a toast in the active window.
pub fn show(kind: ToastKind, text: impl Into<String>, cx: &mut App) {
    let note = notification(kind, text.into());
    cx.defer(move |cx| {
        let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
            return;
        };
        window
            .update(cx, |_, window, cx| window.push_notification(note, cx))
            .ok();
    });
}

fn notification(kind: ToastKind, text: String) -> Notification {
    let text = text.trim().to_string();
    let short = !text.contains('\n') && text.chars().count() <= TITLE_MAX;
    match kind {
        // A short receipt is the whole toast; a long one keeps a heading so
        // the stack stays scannable.
        ToastKind::Success if short => Notification::new().title(text),
        ToastKind::Success => Notification::new().title("Done").message(text),
        ToastKind::Error => Notification::new().title("Action failed").message(text),
    }
    .with_type(match kind {
        ToastKind::Success => NotificationType::Success,
        ToastKind::Error => NotificationType::Error,
    })
    // Bottom left, over the sidebar's empty tail: clear of the action bar
    // and the search at the top right, and of the graph's rows.
    .placement(Anchor::BottomLeft)
}
