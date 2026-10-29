//! What Rama remembers between launches, grouped by the part of the app it
//! belongs to, and the file it lives in.
//!
//! Views read it with [`read`] (or [`current`] for a copy) and change it with
//! [`update`]. A change shows at once; the file follows, straight away for a
//! toggle ([`SavePolicy::Immediate`]) or once a drag rests
//! ([`SavePolicy::Debounced`]) so a drag is not written every frame.
//!
//! Every group reads with defaults for what the file lacks and ignores what
//! it does not know, so files from older and newer builds both load. The key
//! names in the file are the field names below: renaming one forgets that
//! setting for everyone.

mod file;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{App, Global, Task};
use serde::{Deserialize, Serialize};

pub use file::{avatars_dir, settings_path};

/// How long a dragged value must rest before it is written.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// `value` kept inside `range`; `fallback` when it is not a number at all,
/// as a hand-edited file can have.
fn bounded(value: f32, range: (f32, f32), fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(range.0, range.1)
    } else {
        fallback
    }
}

// ── Look ────────────────────────────────────────────────────────────────────

/// Its own enum rather than the theme's, so renaming a theme colour cannot
/// change what a file means.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Accent {
    #[default]
    Indigo,
    Orange,
    Amber,
    Green,
    Cyan,
    Blue,
    Pink,
    Custom {
        red: u8,
        green: u8,
        blue: u8,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

/// The graph's lane colours by name; `ui::lane_palette` has the colours.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Palette {
    #[default]
    Rama,
    Kraken,
    Watchtower,
    Octo,
    Islands,
    Trident,
    Blueprint,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Look {
    pub mode: Mode,
    pub accent: Accent,
    pub palette: Palette,
}

// ── History ─────────────────────────────────────────────────────────────────

/// One of the optional columns beside each commit's subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnId {
    Author,
    Date,
    Sha,
}

impl ColumnId {
    pub const EVERY: [ColumnId; 3] = [ColumnId::Author, ColumnId::Date, ColumnId::Sha];

    /// How narrow and how wide it may be dragged.
    pub fn limits(self) -> (f32, f32) {
        match self {
            ColumnId::Author => (44.0, 220.0),
            ColumnId::Date => (68.0, 180.0),
            ColumnId::Sha => (58.0, 140.0),
        }
    }

    pub fn default_width(self) -> f32 {
        match self {
            ColumnId::Author | ColumnId::Date => 88.0,
            ColumnId::Sha => 74.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ColumnId::Author => "Author",
            ColumnId::Date => "Date",
            ColumnId::Sha => "SHA",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Column {
    pub shown: bool,
    pub width: f32,
}

impl Default for Column {
    fn default() -> Self {
        Self {
            shown: true,
            width: f32::NAN,
        }
    }
}

/// The optional columns. A hidden column keeps its place in `order`, so it
/// comes back where it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Columns {
    pub author: Column,
    pub date: Column,
    pub sha: Column,
    pub order: Vec<ColumnId>,
}

impl Default for Columns {
    fn default() -> Self {
        Self {
            author: Column::default(),
            date: Column::default(),
            sha: Column::default(),
            order: ColumnId::EVERY.to_vec(),
        }
        .sanitized()
    }
}

impl Columns {
    pub fn get(&self, id: ColumnId) -> &Column {
        match id {
            ColumnId::Author => &self.author,
            ColumnId::Date => &self.date,
            ColumnId::Sha => &self.sha,
        }
    }

    pub fn get_mut(&mut self, id: ColumnId) -> &mut Column {
        match id {
            ColumnId::Author => &mut self.author,
            ColumnId::Date => &mut self.date,
            ColumnId::Sha => &mut self.sha,
        }
    }

    /// Widths inside their limits (a missing one at its default), and every
    /// column in the order exactly once: a file edited by hand can repeat
    /// one or leave one out.
    pub fn sanitized(mut self) -> Self {
        for id in ColumnId::EVERY {
            let column = self.get_mut(id);
            column.width = bounded(column.width, id.limits(), id.default_width());
        }
        let mut order = Vec::with_capacity(ColumnId::EVERY.len());
        for id in self.order.iter().copied().chain(ColumnId::EVERY) {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        self.order = order;
        self
    }

    /// Every width back to its default, nothing else.
    pub fn reset_widths(&mut self) {
        for id in ColumnId::EVERY {
            self.get_mut(id).width = id.default_width();
        }
    }
}

/// How the author column shows who wrote a commit.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Authors {
    #[default]
    Photo,
    Name,
}

/// What the Pull button does with what it brings in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pull {
    /// Fast-forward when possible and merge otherwise, like `git pull`.
    #[default]
    Merge,
    FastForwardOnly,
    Rebase,
}

impl Pull {
    pub const EVERY: [Pull; 3] = [Pull::Merge, Pull::FastForwardOnly, Pull::Rebase];

    /// For the button, which is narrow.
    pub fn button_label(self) -> &'static str {
        match self {
            Pull::Merge => "Pull",
            Pull::FastForwardOnly => "Pull (ff)",
            Pull::Rebase => "Pull (rb)",
        }
    }

    /// For the menu, which has room to explain.
    pub fn menu_label(self) -> &'static str {
        match self {
            Pull::Merge => "Pull: merge if needed",
            Pull::FastForwardOnly => "Pull: fast-forward only",
            Pull::Rebase => "Pull: rebase",
        }
    }
}

impl From<Pull> for crate::git::PullMode {
    fn from(pull: Pull) -> Self {
        match pull {
            Pull::Merge => Self::FfPossible,
            Pull::FastForwardOnly => Self::FfOnly,
            Pull::Rebase => Self::Rebase,
        }
    }
}

/// The commit panel beside the history.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Panel {
    /// Files as a folder tree rather than a flat list.
    pub tree: bool,
    /// Files sorted by name rather than in git's order.
    pub by_name: bool,
    /// Its share of the pane's width.
    pub share: f32,
}

impl Panel {
    pub const SHARE: (f32, f32) = (0.15, 0.55);
    pub const DEFAULT_SHARE: f32 = 0.26;

    pub fn bounded_share(share: f32) -> f32 {
        bounded(share, Self::SHARE, Self::DEFAULT_SHARE)
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            tree: false,
            by_name: false,
            share: Self::DEFAULT_SHARE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct History {
    pub columns: Columns,
    pub authors: Authors,
    /// The branch badges column at the left.
    pub badges: bool,
    pub panel: Panel,
    pub pull: Pull,
    /// Merge with a merge commit even when a fast-forward would do.
    pub merge_commit: bool,
}

impl Default for History {
    fn default() -> Self {
        Self {
            columns: Columns::default(),
            authors: Authors::default(),
            badges: true,
            panel: Panel::default(),
            pull: Pull::default(),
            merge_commit: true,
        }
    }
}

// ── Diff ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Diff {
    /// The left half's share of a split diff.
    pub split_share: f32,
    pub wrap: bool,
    pub ignore_whitespace: bool,
}

impl Diff {
    /// Either half keeps room for its gutter and some code.
    pub const SPLIT_SHARE: (f32, f32) = (0.2, 0.8);
    pub const DEFAULT_SPLIT_SHARE: f32 = 0.5;

    pub fn bounded_split(share: f32) -> f32 {
        bounded(share, Self::SPLIT_SHARE, Self::DEFAULT_SPLIT_SHARE)
    }
}

impl Default for Diff {
    fn default() -> Self {
        Self {
            split_share: Self::DEFAULT_SPLIT_SHARE,
            wrap: false,
            ignore_whitespace: false,
        }
    }
}

// ── Everything ──────────────────────────────────────────────────────────────

/// The repositories open in tabs. Only the active one is read on launch.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tabs {
    pub open: Vec<String>,
    pub active: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub look: Look,
    pub history: History,
    pub diff: Diff,
    pub tabs: Tabs,
    /// The GitKraken import ran, so the home screen stops offering it.
    pub gitkraken_imported: bool,
}

impl Settings {
    /// Everything brought back inside its limits.
    pub fn sanitized(mut self) -> Self {
        self.history.columns = self.history.columns.sanitized();
        self.history.panel.share = Panel::bounded_share(self.history.panel.share);
        self.diff.split_share = Diff::bounded_split(self.diff.split_share);
        self
    }
}

// ── The store ───────────────────────────────────────────────────────────────

/// When a change reaches the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavePolicy {
    /// Once it has rested: for something being dragged.
    Debounced,
    /// Now: for a toggle.
    Immediate,
}

struct Store {
    settings: Settings,
    path: PathBuf,
    /// Bumped on every change; `saved` is the value last written.
    version: u64,
    saved: u64,
    waiting: Option<Task<()>>,
}

impl Global for Store {}

/// Load the file. Once, at startup.
pub fn init(cx: &mut App) {
    let path = settings_path();
    let settings = file::read(&path).unwrap_or_default().sanitized();
    cx.set_global(Store {
        settings,
        path,
        version: 0,
        saved: 0,
        waiting: None,
    });
}

/// Look at the settings without copying them.
pub fn read<R>(cx: &App, look: impl FnOnce(&Settings) -> R) -> R {
    match cx.try_global::<Store>() {
        Some(store) => look(&store.settings),
        None => look(&Settings::default()),
    }
}

/// A copy of the settings, for a view that reads many of them.
pub fn current(cx: &App) -> Settings {
    read(cx, Settings::clone)
}

/// Change the settings and save the change. False when `change` changed
/// nothing, in which case nothing is written either.
pub fn update(policy: SavePolicy, cx: &mut App, change: impl FnOnce(&mut Settings)) -> bool {
    let Some(store) = cx.try_global::<Store>() else {
        return false;
    };
    let mut next = store.settings.clone();
    change(&mut next);
    if next == store.settings {
        return false;
    }
    let store = cx.global_mut::<Store>();
    store.settings = next;
    store.version += 1;
    // A write still waiting would save something already stale.
    store.waiting = None;
    // Any window may be drawing from what changed.
    cx.refresh_windows();
    match policy {
        SavePolicy::Immediate => save(cx),
        SavePolicy::Debounced => {
            let waiting = cx.spawn(async move |cx| {
                cx.background_executor().timer(DEBOUNCE).await;
                cx.update(save);
            });
            cx.global_mut::<Store>().waiting = Some(waiting);
        }
    }
    true
}

/// Write the file in the background, unless it is already current.
fn save(cx: &mut App) {
    let Some(store) = cx.try_global::<Store>() else {
        return;
    };
    if store.saved == store.version {
        return;
    }
    let (settings, path, version) = (store.settings.clone(), store.path.clone(), store.version);
    cx.global_mut::<Store>().saved = version;
    cx.background_executor()
        .spawn(async move {
            let _ = file::write(&path, &settings);
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_range_values_come_back_in() {
        let mut settings = Settings::default();
        settings.history.columns.author.width = 5_000.0;
        settings.history.columns.date.width = -3.0;
        settings.history.columns.sha.width = f32::NAN;
        settings.history.panel.share = 0.99;
        settings.diff.split_share = f32::INFINITY;
        let fixed = settings.sanitized();
        let columns = &fixed.history.columns;
        assert_eq!(columns.author.width, ColumnId::Author.limits().1);
        assert_eq!(columns.date.width, ColumnId::Date.limits().0);
        assert_eq!(columns.sha.width, ColumnId::Sha.default_width());
        assert_eq!(fixed.history.panel.share, Panel::SHARE.1);
        assert_eq!(fixed.diff.split_share, Diff::DEFAULT_SPLIT_SHARE);
    }

    #[test]
    fn the_order_names_each_column_once() {
        use ColumnId::*;
        let columns = Columns {
            order: vec![Sha, Sha, Author],
            ..Columns::default()
        }
        .sanitized();
        assert_eq!(columns.order, [Sha, Author, Date]);
    }

    #[test]
    fn a_partial_file_fills_in_and_unknown_keys_pass() {
        let read: Settings = serde_json::from_str(
            r#"{"history":{"badges":false,"pull":"rebase","columns":{"date":{"shown":false}}},"someday":1}"#,
        )
        .unwrap();
        let read = read.sanitized();
        assert!(!read.history.badges);
        assert_eq!(read.history.pull, Pull::Rebase);
        assert!(!read.history.columns.date.shown);
        assert_eq!(
            read.history.columns.date.width,
            ColumnId::Date.default_width()
        );
        assert!(read.history.columns.author.shown);
        assert_eq!(read.diff, Diff::default());
    }

    #[test]
    fn resetting_widths_leaves_the_rest() {
        let mut columns = Columns::default();
        columns.sha.width = 100.0;
        columns.sha.shown = false;
        columns.reset_widths();
        assert_eq!(columns.sha.width, ColumnId::Sha.default_width());
        assert!(!columns.sha.shown);
    }
}
