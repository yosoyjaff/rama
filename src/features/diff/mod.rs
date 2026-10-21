//! The diff viewer: a patch, its rows, and the view that scrolls them.
//!
//! * [`model`] — parsing a patch and flattening it into rows. Pure.
//! * [`render`] — drawing one row. No state.
//! * [`highlight`] — syntax colours: a small lexer by file extension.
//! * [`preview`] — Markdown rendered, images and SVG drawn.
//! * [`view`] — the entity that reads a diff and scrolls it.

// The parse and the row model arrived whole; the view draws most of it, but
// not yet all. What is unused is named and deliberate: the sticky file header
// (the band that pins the current file while its body scrolls) and
// `DiffHighlights`, which colours by whole-file line numbers. Each is a view
// feature with its model already in place.
#![allow(dead_code)]

pub mod highlight;
pub mod metrics;
pub mod model;
pub mod preview;
pub mod render;
pub mod sideways;
pub mod view;

pub use view::DiffView;
