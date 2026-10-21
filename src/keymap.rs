//! The application keymap.
//!
//! Empty on purpose, except for the one binding the platform expects. A
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
//! KeyBinding::new("secondary-shift-o", OpenLogs, Some(CONTEXT))
//!
//! // and, in the view that owns the behaviour and declares the key context
//! .on_action(cx.listener(Self::open_logs))
//! ```
//!
//! `secondary` is GPUI's portable modifier: Command on macOS, Control
//! everywhere else. Prefer it over branching on the platform, and reserve an
//! explicit `cmd`/`ctrl` for a binding that genuinely differs.
//!
//! Bindings are scoped to a key context. [`CONTEXT`] is the shell's, declared
//! on its root element; a view that owns its own commands declares its own, so
//! a shortcut only exists where the thing it acts on exists.

use gpui_kit::App;

#[cfg(target_os = "macos")]
use crate::actions::Quit;
#[cfg(target_os = "macos")]
use gpui_kit::KeyBinding;

/// The key context the shell's root element declares, and the scope an
/// application-wide binding belongs in.
pub const CONTEXT: &str = "App";

pub fn init(cx: &mut App) {
    // Command-Q is a macOS platform contract, not product chrome: the user
    // will press it whether or not the application binds it. Elsewhere,
    // quitting is the window's close button.
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);

    let _ = cx;
}
