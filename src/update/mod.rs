//! Auto-update: the manifest format, the decision, the transport, the
//! install, and the state the UI shows.
//!
//! * [`manifest`] — the release format CI publishes and the client reads;
//! * [`check`] — the pure "is there something newer for *this* build" decision;
//! * [`source`] — the trait a transport implements ([`source::StaticSource`]
//!   is the test double), and [`github`] — the one this app ships: the
//!   `latest.json` asset of the newest GitHub release;
//! * [`install`] — download, verify (size + SHA-256), swap in place, relaunch;
//! * [`state`] — the entity the shell observes, with an explicit phase.
//!
//! [`crate::updater`] wires it up: when to check, the notification, and the
//! row in Settings → About. `docs/PACKAGING.md` covers releasing.

// `StaticSource` and parts of the manifest builder are only used by tests.
#![allow(dead_code)]

pub mod check;
pub mod github;
pub mod install;
pub mod manifest;
pub mod source;
pub mod state;

// Re-exported for the application that wires this up: `use crate::update::{…}`
// rather than reaching into `state`.
#[allow(unused_imports)]
pub use state::{UpdateEvent, UpdatePhase, UpdateState};
