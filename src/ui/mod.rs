//! Application chrome: the presentation the shell and every feature share.
//!
//! Nothing here knows what the product is. Components take what they render
//! and report intent through callbacks; the shell owns the state they
//! describe. That is what keeps them reusable across windows and, later,
//! across a second product built on this base.

// This layer is the base's API surface: the title bar's slots exist for the
// application written next, and a binary crate has no downstream consumer to
// keep `dead_code` quiet. The allow stops here — `features` and `shell` are
// held to the normal rule.
#![allow(dead_code)]

pub mod avatar;
pub mod chrome;
pub mod icons;
pub mod lane_palette;
pub mod loaders;
pub mod motion;
pub mod popover;
pub mod splitter;
pub mod tab_strip;
pub mod text_input;
pub mod theme;
pub mod tip;
mod title_bar;
pub mod toast;
pub mod typography;
pub mod window_frame;

pub use title_bar::AppTitleBar;
