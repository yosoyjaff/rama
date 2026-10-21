//! The product's capabilities, one module each.
//!
//! A feature owns its state, its view, its actions and its keybindings. The
//! shell mounts it and knows nothing else about it.
//!
//! [`workspace`] is the window's content: a tab per open project, each one a
//! [`history`] screen, with [`welcome`] behind them. [`diff`] is the patch
//! viewer and [`settings_page`] the preferences screen: the commit graph, the rows beside
//! it, the detail of the selected commit and the diff under it.
//!
//! Split one of these into its own crate when it grows its own tests,
//! dependencies and release cadence; until then a module is the same boundary
//! with less ceremony.

pub mod diff;
pub mod history;
pub mod settings_page;
pub mod welcome;
pub mod workspace;
