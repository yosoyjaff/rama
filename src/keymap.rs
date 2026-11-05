//! The application keymap.
//!
//! Nearly empty on purpose: the one binding the platform expects, and Copy. A
//! shortcut is a product decision, and a base that ships invisible ones leaves
//! the next application discovering behaviour it then has to remove.
//!
//! To add one:
//!
//! ```ignore
//! // src/actions.rs
//! gpui_kit::actions!(app, [OpenLogs]);
//!
//! // here
//! KeyBinding::new("ctrl-shift-o", OpenLogs, Some(CONTEXT)) // cmd- on macOS
//!
//! // and, in the view that owns the behaviour and declares the key context
//! .on_action(cx.listener(Self::open_logs))
//! ```
//!
//! `KeyBinding::new` does not resolve `secondary-` (it panics on it): only
//! `KeyBinding::load` knows the host. Bind `cmd-` on macOS and `ctrl-`
//! elsewhere, behind `cfg(target_os)`, as the Copy binding below does.
//!
//! Bindings are scoped to a key context. [`CONTEXT`] is the shell's, declared
//! on its root element; a view that owns its own commands declares its own, so
//! a shortcut only exists where the thing it acts on exists.

use gpui_kit::{App, KeyBinding};

#[cfg(target_os = "macos")]
use crate::actions::Quit;
use crate::features::diff;

/// The key context the shell's root element declares, and the scope an
/// application-wide binding belongs in.
pub const CONTEXT: &str = "App";

pub fn init(cx: &mut App) {
    // Command-Q is a macOS platform contract, not product chrome: the user
    // will press it whether or not the application binds it. Elsewhere,
    // quitting is the window's close button.
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);

    // Copy in the diff copies its selected lines. A text field inside it
    // (the merge editor's box) is a deeper context and keeps its own Copy.
    // `KeyBinding` does not parse `secondary`: name the platform's key.
    #[cfg(target_os = "macos")]
    const COPY: &str = "cmd-c";
    #[cfg(not(target_os = "macos"))]
    const COPY: &str = "ctrl-c";
    cx.bind_keys([KeyBinding::new(
        COPY,
        diff::CopySelection,
        Some(diff::KEY_CONTEXT),
    )]);
}
