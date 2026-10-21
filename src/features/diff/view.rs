//! The diff viewer: one file's patch, virtualized by line.
//!
//! It reads ONE file at a time — a
//! commit's version against its parent, or the working tree's staged or
//! unstaged side — with the whole file as context, so the patch can be
//! scrolled out of a hunk and kept reading. Prev/next walks the files of the
//! panel it was opened from.
//!
//! Two strips sit above the body. The first names the file and holds the
//! whole-file Stage/Unstage and the ✕. The second switches between the patch
//! and the whole file, walks the files, and holds the layout, whitespace and
//! blame toggles.
//!
//! Every row of the body is one hunk header, one diff line or one pad — the
//! flat model in [`super::model`]. Only the visible slice is ever built. Two
//! layouts share that model: *unified* stacks the old side above the new,
//! *split* pairs each hunk's deletions against its additions in two columns.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Empty, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ListAlignment, ListState, Pixels, Render, SharedString, Task, Window, canvas, div,
    list, prelude::*, px,
};

use super::highlight::{HighlightSpan, HighlightedDocument, highlight_document};
use super::metrics::RULER;
use super::model::{
    ConflictLine, DiffMode, DiffRow, FileDiff, FileStatus, MarkTone, hunk_spans, layout_rows,
    read_patch, ruler_marks,
};
use super::preview::{Preview, PreviewKind};
use super::render::{
    CellLook, diff_line_row, empty_left, hunk_header_row, notice_row, split_filler,
    split_line_cell_styled, split_missing_side, split_row,
};
use super::sideways::{Fit, Half, Sideways};
use crate::git::FileDiffSide;
use crate::git::blame::BlameResult;
use crate::git::conflict_file::{self, ConflictSegment, ConflictTakes, Side};
use crate::settings::{self, SavePolicy};
use crate::ui::chrome::{EDGE_INSET, SEGMENT_INNER_RADIUS, SEGMENT_RADIUS};
use crate::ui::splitter::{SplitterDrag, ratio_from_pointer, splitter};
use crate::ui::text_input::TextInput;
use crate::ui::theme::{Theme, wash};
use crate::ui::typography::ui_rems;
use crate::ui::{avatar, icons, motion};

/// Width the blame gutter takes in File View.
const BLAME_WIDTH: f32 = 250.0;

/// Height of each of the two strips above the body.
const STRIP_HEIGHT: f32 = 32.0;

/// The divider between the two sides of a split.
const SPLIT_SPLITTER: &str = "diff-split-splitter";

/// The strip along the bottom of the code that holds its horizontal bars.
const HORIZONTAL_BAR_HEIGHT: f32 = 10.0;

/// The small square controls of the second strip.
const TOOL_SIZE: f32 = 20.0;

/// What the viewer is reading: always one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    /// One file of a commit, against the commit's first parent.
    CommitFile {
        repo: String,
        sha: String,
        path: String,
    },
    /// One file of the working tree: the index against HEAD when `staged`,
    /// the working copy against the index otherwise.
    WorkingTreeFile {
        repo: String,
        path: String,
        staged: bool,
    },
    /// A conflicted file of the working tree, read as a patch of its two
    /// sides — A (ours) old, B (theirs) new — with the merge editor's take
    /// boxes, output and Save Output on top of the viewer.
    Conflict { repo: String, path: String },
}

impl DiffSource {
    /// The checkout this patch was read from.
    pub fn repo(&self) -> &str {
        match self {
            Self::CommitFile { repo, .. }
            | Self::WorkingTreeFile { repo, .. }
            | Self::Conflict { repo, .. } => repo,
        }
    }

    /// The file this source reads.
    pub fn path(&self) -> &str {
        match self {
            Self::CommitFile { path, .. }
            | Self::WorkingTreeFile { path, .. }
            | Self::Conflict { path, .. } => path,
        }
    }

    /// Identity of what was read: a new key means a new read and a list
    /// reset, and it is what keeps a second click on the same row a no-op.
    pub fn key(&self) -> String {
        match self {
            Self::CommitFile { repo, sha, path } => format!("{repo}|{sha}|{path}"),
            Self::WorkingTreeFile { repo, path, staged } => {
                format!("{repo}|working-tree|{staged}|{path}")
            }
            Self::Conflict { repo, path } => format!("{repo}|conflict|{path}"),
        }
    }

    /// The same source moved to another file of the same list.
    fn with_file(&self, file: &DiffNavFile) -> Self {
        match self {
            Self::CommitFile { repo, sha, .. } => Self::CommitFile {
                repo: repo.clone(),
                sha: sha.clone(),
                path: file.path.clone(),
            },
            Self::WorkingTreeFile { repo, .. } => Self::WorkingTreeFile {
                repo: repo.clone(),
                path: file.path.clone(),
                staged: file.staged,
            },
            Self::Conflict { repo, .. } => Self::Conflict {
                repo: repo.clone(),
                path: file.path.clone(),
            },
        }
    }
}

/// One entry of the list prev/next walks: a file of the panel the patch was
/// opened from, in that panel's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffNavFile {
    pub path: String,
    /// Which side of the working tree it sits on. Always false for a commit.
    pub staged: bool,
}

pub enum DiffViewEvent {
    /// The ✕: put the graph back.
    Close,
    /// The Stage/Unstage button moved the file: the working tree changed.
    WorkingTreeChanged,
    /// Save Output wrote a conflicted file and marked it resolved.
    ConflictResolved,
}

/// The merge editor's state over a conflict source.
struct ConflictState {
    segments: Vec<ConflictSegment>,
    /// For each line of the patch's one hunk: the conflict line it is.
    refs: Vec<Option<ConflictLine>>,
    takes: ConflictTakes,
    /// The conflict the stepper and the brighter tint are on.
    current: usize,
    /// What Save Output writes, rebuilt from the takes.
    output: Vec<String>,
    output_list: ListState,
    output_width: SeenWidth,
    /// The manual output buffer: "edit output directly".
    /// `Some` while the user is typing their own resolution: Save Output
    /// writes the box, and the takes freeze, since applying one would
    /// overwrite what was typed.
    edit: Option<Entity<TextInput>>,
    saving: bool,
    save_task: Option<Task<()>>,
}

/// Last path segment.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Hand a list its rows all at once, with one height for all of them.
///
/// A plain `reset` leaves every row `Unmeasured`, so the list only ever knows
/// the height of what it has drawn: the scrollbar is sized from the viewport
/// plus the overdraw — the same sliver for a file of twenty lines and for one
/// of fifty thousand. The hint makes the extent proportional to the row count
/// from the first frame. The rows a diff lays out are near-uniform (bands and
/// summaries are the exceptions) and adjust as they are drawn, so an
/// approximation is what [`super::metrics`] is for.
fn reset_list(list: &ListState, len: usize, height: f32) {
    list.reset_with_uniform_height(len, px(height));
}

/// The width a list was last laid out at; see [`keep_row_hint`].
type SeenWidth = Rc<Cell<Pixels>>;

/// Put a list's height hint back after GPUI drops it.
///
/// A list that changes width — its first layout included — forgets every
/// row's height, hint and all, so the scrollbar is sized again from the rows
/// drawn since: the thumb reaches the bottom after a few ticks, then jumps
/// back up and down as more rows are measured. Laid after the list, this
/// notices the new width and hands the unmeasured rows the hint again; the
/// rows already drawn keep their measured height.
fn keep_row_hint(list: &ListState, height: f32, seen: &SeenWidth) -> impl IntoElement {
    let list = list.clone();
    let seen = seen.clone();
    canvas(
        move |_, window, _| {
            let width = list.viewport_bounds().size.width;
            if seen.replace(width) != width {
                list.with_uniform_item_height(px(height));
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
}

/// Everything before the last segment, with its trailing slash.
fn file_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(ix) => &path[..=ix],
        None => "",
    }
}

/// How the patch is laid out: one per layout button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffLayout {
    /// Only the hunks, with git's three lines of context around each.
    Hunk,
    /// The whole file in one column, changes interleaved.
    Inline,
    /// The whole file, old and new side by side.
    Split,
}

/// The parse of one patch, plus the per-file state that outlives a scroll.
struct Parsed {
    files: Arc<Vec<FileDiff>>,
    /// Indexed like `files`, so horizontal scrolling and the measured code
    /// width survive virtualization.
    sideways: Vec<Sideways>,
    /// Syntax colours, indexed like `files`, then `[hunk][line]`.
    spans: Vec<Vec<Vec<Vec<HighlightSpan>>>>,
    /// The whole file came as context: its one hunk IS the file, so the
    /// `@@` band above it says nothing and is not drawn.
    full: bool,
    truncated: bool,
    utf8: bool,
}

pub struct DiffView {
    source: Option<DiffSource>,
    /// The files prev/next walks.
    files: Vec<DiffNavFile>,
    /// File View: the whole file as it stands at the source's revision,
    /// instead of the patch.
    view_file: bool,
    /// The Preview button of a Markdown or SVG file: the file as what it is
    /// instead of its text. An image needs no button — its File View and
    /// Diff View are its pictures (see [`Self::showing_preview`]).
    preview: bool,
    preview_data: Option<Preview>,
    preview_task: Option<Task<()>>,
    /// Side by side is how every file opens: comparing is why the panel
    /// exists, and the other two are the narrow-pane fallbacks.
    layout: DiffLayout,
    /// Blame annotates a file's lines, so it only draws in File View.
    blame_on: bool,
    blame: Option<BlameResult>,
    blame_task: Option<Task<()>>,
    stage_busy: bool,
    stage_task: Option<Task<()>>,
    file_rows: Vec<(u32, String)>,
    /// Syntax colours of `file_rows`, one entry per row.
    file_highlights: Option<Arc<HighlightedDocument>>,
    file_list: ListState,
    file_list_width: SeenWidth,
    file_loading: bool,
    file_error: Option<SharedString>,
    file_task: Option<Task<()>>,
    parsed: Option<Parsed>,
    rows: Vec<DiffRow>,
    list: ListState,
    list_width: SeenWidth,
    /// Set while the source is a conflict.
    conflict: Option<ConflictState>,
    /// What sides A and B are: "Commit abc1234 on main".
    conflict_labels: (SharedString, SharedString),
    loading: bool,
    error: Option<SharedString>,
    task: Option<Task<()>>,
    focus_handle: FocusHandle,
}

impl EventEmitter<DiffViewEvent> for DiffView {}

impl DiffView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            source: None,
            files: Vec::new(),
            view_file: false,
            preview: false,
            preview_data: None,
            preview_task: None,
            layout: DiffLayout::Split,
            blame_on: false,
            blame: None,
            blame_task: None,
            stage_busy: false,
            stage_task: None,
            file_rows: Vec::new(),
            file_highlights: None,
            file_list: ListState::new(0, ListAlignment::Top, px(1024.0)),
            file_list_width: SeenWidth::default(),
            file_loading: false,
            file_error: None,
            file_task: None,
            parsed: None,
            rows: Vec::new(),
            // A generous overdraw: a diff line is 21px, so this is about two
            // screens of them, which is what keeps a fast scroll from
            // outrunning the list.
            list: ListState::new(0, ListAlignment::Top, px(1024.0)),
            list_width: SeenWidth::default(),
            conflict: None,
            conflict_labels: (SharedString::default(), SharedString::default()),
            loading: false,
            error: None,
            task: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Read exactly what the given source describes.
    pub fn show(&mut self, source: DiffSource, cx: &mut Context<Self>) {
        if self.source.as_ref() == Some(&source) {
            return;
        }
        let kind = PreviewKind::of(source.path());
        let is_conflict = matches!(source, DiffSource::Conflict { .. });
        self.source = Some(source);
        self.conflict = None;
        // The merge editor is the patch and nothing else: no File View, no
        // preview, no blame over a file full of markers.
        if is_conflict {
            self.view_file = false;
            self.preview = false;
            self.blame_on = false;
        }
        self.parsed = None;
        self.rows.clear();
        self.list.reset(0);
        self.preview_data = None;
        // Only a Markdown or SVG file can stay on its Preview button.
        if !kind.is_some_and(|kind| !kind.opens_on_preview()) {
            self.preview = false;
        }
        if self.view_file && !self.showing_preview() {
            self.load_file_view(cx);
        }
        self.load_blame(cx);
        self.reload(cx);
    }

    /// Name the two sides of a conflict source.
    pub fn set_conflict_labels(&mut self, ours: String, theirs: String, cx: &mut Context<Self>) {
        self.conflict_labels = (ours.into(), theirs.into());
        cx.notify();
    }

    fn is_conflict(&self) -> bool {
        matches!(self.source, Some(DiffSource::Conflict { .. }))
    }

    /// What is being read.
    pub fn source(&self) -> Option<&DiffSource> {
        self.source.as_ref()
    }

    /// The files prev/next walks — the panel's list, which changes as the
    /// working tree does.
    pub fn set_files(&mut self, files: Vec<DiffNavFile>, cx: &mut Context<Self>) {
        if self.files != files {
            self.files = files;
            cx.notify();
        }
    }

    /// Read the patch again — what is shown was read before the repository
    /// moved, or the whitespace toggle changed what git is asked for.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if let DiffSource::Conflict { repo, path } = &source {
            self.reload_conflict(repo.clone(), path.clone(), cx);
            return;
        }
        let key = source.key();
        let ignore_whitespace = settings::current(cx).diff.ignore_whitespace;
        let context = (self.layout == DiffLayout::Hunk).then_some(3);
        // The preview is read from the same revisions: it moves with them.
        if self.showing_preview() {
            self.load_preview(cx);
        }
        self.loading = true;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let root = Path::new(source.repo());
                    let side = match &source {
                        DiffSource::CommitFile { sha, .. } => FileDiffSide::Commit(sha),
                        DiffSource::WorkingTreeFile { staged: true, .. } => FileDiffSide::Staged,
                        DiffSource::WorkingTreeFile { staged: false, .. }
                        | DiffSource::Conflict { .. } => FileDiffSide::Unstaged,
                    };
                    crate::git::capture_file_diff(
                        root,
                        side,
                        source.path(),
                        ignore_whitespace,
                        context,
                    )
                })
                .await;
            this.update(cx, |view, cx| {
                // A newer read landed first, or the file moved on.
                if view.source.as_ref().map(DiffSource::key).as_deref() != Some(key.as_str()) {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(read) => view.apply(read, cx),
                    Err(error) => {
                        view.parsed = None;
                        view.rows.clear();
                        view.list.reset(0);
                        view.error = Some(error.to_string().into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Read the conflicted working copy and lay it out as a patch of its
    /// two sides.
    fn reload_conflict(&mut self, repo: String, path: String, cx: &mut Context<Self>) {
        let key = self
            .source
            .as_ref()
            .map(DiffSource::key)
            .unwrap_or_default();
        self.loading = true;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let read_path = path.clone();
            let bytes = cx
                .background_executor()
                .spawn(async move { std::fs::read(Path::new(&repo).join(&read_path)) })
                .await;
            this.update(cx, |view, cx| {
                if view.source.as_ref().map(DiffSource::key).as_deref() != Some(key.as_str()) {
                    return;
                }
                view.loading = false;
                let text = match bytes {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(text) => text,
                        Err(_) => {
                            view.error =
                                Some("Binary file: resolve it by taking one side whole".into());
                            cx.notify();
                            return;
                        }
                    },
                    Err(error) => {
                        view.error = Some(error.to_string().into());
                        cx.notify();
                        return;
                    }
                };
                let segments = conflict_file::parse_conflict_file(&text);
                let (file, refs) = FileDiff::from_conflict(&path, &segments);
                let takes = conflict_file::empty_takes(&segments);
                let output = conflict_file::output_lines(&segments, &takes);
                let output_list = ListState::new(0, ListAlignment::Top, px(1024.0));
                reset_list(&output_list, output.len(), super::metrics::LINE_HEIGHT);
                view.conflict = Some(ConflictState {
                    segments,
                    refs,
                    takes,
                    current: 0,
                    output,
                    output_list,
                    output_width: SeenWidth::default(),
                    edit: None,
                    saving: false,
                    save_task: None,
                });
                let sideways = vec![Sideways::new(&file)];
                let spans = vec![hunk_spans(&file)];
                view.parsed = Some(Parsed {
                    files: Arc::new(vec![file]),
                    sideways,
                    spans,
                    full: true,
                    truncated: false,
                    utf8: true,
                });
                view.rebuild_rows(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// The conflicts as runs of rows — one per conflict, in order — which is
    /// what the stepper and the ruler walk.
    fn conflict_runs(&self) -> Vec<super::model::RulerMark> {
        match (&self.conflict, &self.parsed) {
            (Some(_), Some(parsed)) => ruler_marks(&self.rows, &parsed.files),
            _ => Vec::new(),
        }
    }

    fn refresh_conflict_output(state: &mut ConflictState) {
        state.output = conflict_file::output_lines(&state.segments, &state.takes);
        reset_list(
            &state.output_list,
            state.output.len(),
            super::metrics::LINE_HEIGHT,
        );
    }

    /// Take or untake one line. Taking a line is also saying which conflict
    /// you are on, so the stepper and the tint follow the click.
    fn conflict_toggle(&mut self, line: ConflictLine, cx: &mut Context<Self>) {
        let Some(state) = self.conflict.as_mut() else {
            return;
        };
        // While the output is edited by hand a take would overwrite the
        // typed text, so the takes hold still until the box closes.
        if state.edit.is_some() {
            return;
        }
        state.current = line.conflict;
        state.takes =
            conflict_file::toggle_line(state.takes.clone(), line.conflict, line.side, line.line);
        Self::refresh_conflict_output(state);
        cx.notify();
    }

    /// The box over a side: take all of it, or none.
    fn conflict_take_all(&mut self, side: Side, cx: &mut Context<Self>) {
        let Some(state) = self.conflict.as_mut() else {
            return;
        };
        if state.edit.is_some() {
            return;
        }
        let taken = !conflict_file::side_taken(&state.takes, side);
        state.takes = conflict_file::set_all_taken(state.takes.clone(), side, taken);
        Self::refresh_conflict_output(state);
        cx.notify();
    }

    /// "Edit output directly": the output becomes a text box
    /// preloaded with what the takes produced, and Save Output writes what
    /// is in it. Closing the box returns to the derived output.
    fn toggle_output_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.conflict.as_mut() else {
            return;
        };
        if state.edit.take().is_some() {
            cx.notify();
            return;
        }
        let text = state.output.join("\n");
        let input = cx.new(|cx| {
            let mut input = TextInput::code_editor("Edit the resolution", window, cx);
            input.set_text(text, window, cx);
            input
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        if let Some(state) = self.conflict.as_mut() {
            state.edit = Some(input);
        }
        cx.notify();
    }

    /// Step `delta` through the conflicts, wrapping, and bring the new one
    /// into view.
    fn conflict_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let runs = self.conflict_runs();
        let Some(state) = self.conflict.as_mut() else {
            return;
        };
        if runs.is_empty() {
            return;
        }
        state.current = (state.current as isize + delta).rem_euclid(runs.len() as isize) as usize;
        if let Some(run) = runs.get(state.current) {
            self.list.scroll_to_reveal_item(run.start);
        }
        cx.notify();
    }

    /// Save Output: write what was taken — or what was typed over it — to
    /// the file and mark it resolved.
    fn save_output(&mut self, cx: &mut Context<Self>) {
        let Some(DiffSource::Conflict { repo, path }) = self.source.clone() else {
            return;
        };
        let Some(state) = self.conflict.as_mut() else {
            return;
        };
        if state.saving {
            return;
        }
        let edited = state
            .edit
            .as_ref()
            .map(|input| input.read(cx).text().to_string());
        let text = edited.unwrap_or_else(|| state.output.join("\n")) + "\n";
        state.saving = true;
        cx.notify();
        state.save_task = Some(cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let root = Path::new(&repo);
                    std::fs::write(root.join(&path), text)?;
                    crate::git::conflicts::mark_resolved(root, std::slice::from_ref(&path))
                })
                .await;
            this.update(cx, |view, cx| {
                if let Some(state) = view.conflict.as_mut() {
                    state.saving = false;
                }
                match outcome {
                    Ok(()) => cx.emit(DiffViewEvent::ConflictResolved),
                    Err(error) => view.error = Some(error.to_string().into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// One line of the output: the patch's unified context row — the bar's
    /// place, the number, the code.
    fn render_output_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let Some(state) = &self.conflict else {
            return Empty.into_any_element();
        };
        let digits = (state.output.len().max(1).ilog10() + 1) as f32;
        let gutter = (digits * 6.6 + 8.0 + 6.0).max(super::metrics::MIN_GUTTER);
        let text = state
            .output
            .get(ix)
            .map(|line| line.replace('\t', "    "))
            .unwrap_or_default();
        let mono = gpui_kit::font(theme.font_mono.clone());
        let runs = super::highlight::runs_for_line(&text, &[], &mono, &theme);
        div()
            .h(px(super::metrics::LINE_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .items_start()
            .child(div().w(px(super::metrics::EDGE_BAR)).flex_none())
            .child(
                div()
                    .w(px(gutter))
                    .flex_none()
                    .font_family(theme.font_mono.clone())
                    .text_size(px(11.0))
                    .line_height(px(super::metrics::LINE_HEIGHT))
                    .text_color(theme.text_faint.opacity(0.8))
                    .flex()
                    .justify_end()
                    .pr(px(8.0))
                    .child(SharedString::from((ix + 1).to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(super::metrics::CODE_INSET))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .font_family(theme.font_mono.clone())
                    .text_size(px(super::metrics::CODE_TEXT))
                    .line_height(px(super::metrics::LINE_HEIGHT))
                    .child(gpui_kit::StyledText::new(SharedString::from(text)).with_runs(runs)),
            )
            .into_any_element()
    }

    /// Over each column of a conflict: the box that takes or untakes the
    /// whole side, sitting over the boxes it governs, its A/B badge and what
    /// the side is.
    fn render_side_header(&self, side: Side, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = &self.conflict else {
            return Empty.into_any_element();
        };
        let gutter = self
            .parsed
            .as_ref()
            .and_then(|parsed| parsed.files.first())
            .map(FileDiff::gutter)
            .unwrap_or(super::metrics::MIN_GUTTER);
        let (badge, label, key) = match side {
            Side::Ours => (
                "A",
                self.conflict_labels.0.clone(),
                "conflict-take-all-ours",
            ),
            Side::Theirs => (
                "B",
                self.conflict_labels.1.clone(),
                "conflict-take-all-theirs",
            ),
        };
        let taken = conflict_file::side_taken(&state.takes, side);
        let tone = side_tone(side, theme);
        div()
            .flex_1()
            .min_w_0()
            .h(px(STRIP_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .child(div().w(px(super::metrics::EDGE_BAR + gutter)).flex_none())
            .child(
                div()
                    .w(px(super::metrics::HALF_SIGN_WIDTH))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        take_box(SharedString::from(key), taken, theme)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.conflict_take_all(side, cx);
                            }))
                            .tooltip(tooltip(if taken { "Untake all" } else { "Take all" })),
                    ),
            )
            .child(
                div()
                    .ml(px(super::metrics::HALF_CODE_INSET))
                    .size(px(16.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(tone.opacity(0.7))
                    .text_size(ui_rems(10.0))
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(tone)
                    .child(badge),
            )
            .child(
                div()
                    .ml(px(6.0))
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .child(label),
            )
            .into_any_element()
    }

    fn apply(&mut self, read: crate::git::FilePatch, cx: &mut Context<Self>) {
        let files = read_patch(&read.patch);
        let sideways = files.iter().map(Sideways::new).collect();
        let spans = files.iter().map(hunk_spans).collect();
        self.parsed = Some(Parsed {
            files: Arc::new(files),
            sideways,
            spans,
            full: read.full,
            truncated: read.truncated,
            utf8: read.utf8,
        });
        self.rebuild_rows(cx);
    }

    /// File View: read the whole file at the source's revision — the commit's
    /// blob, the index for a staged file, the working copy otherwise.
    fn load_file_view(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.file_loading = true;
        self.file_error = None;
        self.file_rows.clear();
        self.file_highlights = None;
        self.file_list.reset(0);
        cx.notify();
        self.file_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let root = Path::new(source.repo());
                    let path = source.path();
                    let read = match &source {
                        // Bytes, not text: a binary file is said to be one
                        // below, rather than failing to decode.
                        DiffSource::CommitFile { sha, .. } => crate::git::command::git_bytes(
                            root,
                            &["show", &format!("{sha}:{path}")],
                        ),
                        DiffSource::WorkingTreeFile { staged: true, .. } => {
                            crate::git::command::git_bytes(root, &["show", &format!(":{path}")])
                        }
                        DiffSource::WorkingTreeFile { staged: false, .. }
                        | DiffSource::Conflict { .. } => {
                            std::fs::read(root.join(path)).map_err(anyhow::Error::from)
                        }
                    };
                    // Coloured here, off the main thread, while the bytes
                    // are at hand.
                    read.map(|bytes| {
                        let colours = std::str::from_utf8(&bytes)
                            .ok()
                            .filter(|_| !bytes.iter().take(8000).any(|byte| *byte == 0))
                            .and_then(|text| highlight_document(path, text.lines()));
                        (bytes, colours)
                    })
                })
                .await;
            this.update(cx, |view, cx| {
                view.file_loading = false;
                match result {
                    Ok((bytes, colours)) => {
                        view.file_highlights = colours.map(Arc::new);
                        // git's own binary heuristic — a NUL in the prologue —
                        // plus anything that is not text at all.
                        let text = std::str::from_utf8(&bytes).ok();
                        let binary =
                            text.is_none() || bytes.iter().take(8000).any(|byte| *byte == 0);
                        let text = text.unwrap_or_default();
                        view.file_rows = if binary {
                            Vec::new()
                        } else {
                            text.lines()
                                .enumerate()
                                .map(|(ix, line)| (ix as u32 + 1, line.to_string()))
                                .collect()
                        };
                        if binary {
                            view.file_error = Some("Binary file — no text to show.".into());
                        }
                    }
                    Err(error) => view.file_error = Some(error.to_string().into()),
                }
                reset_list(
                    &view.file_list,
                    view.file_rows.len(),
                    super::metrics::FILE_ROW_HEIGHT,
                );
                cx.notify();
            })
            .ok();
        }));
    }

    /// `git blame` of the file, while the gutter is on in File View.
    fn load_blame(&mut self, cx: &mut Context<Self>) {
        self.blame = None;
        self.blame_task = None;
        if !self.blame_on || !self.view_file {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        self.blame_task = Some(cx.spawn(async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        crate::git::blame::blame(Path::new(source.repo()), source.path())
                    })
                    .await;
            this.update(cx, |view, cx| {
                view.blame = result.ok();
                cx.notify();
            })
            .ok();
        }));
    }

    /// Read both sides of the file and build its preview.
    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(kind) = PreviewKind::of(source.path()) else {
            return;
        };
        let key = source.key();
        self.preview_data = None;
        cx.notify();
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let sides = cx
                .background_executor()
                .spawn(async move { super::preview::read_sides(&source) })
                .await;
            this.update(cx, |view, cx| {
                if view.source.as_ref().map(DiffSource::key).as_deref() != Some(key.as_str()) {
                    return;
                }
                view.preview_data = Some(Preview::build(kind, sides));
                cx.notify();
            })
            .ok();
        }));
    }

    fn kind(&self) -> Option<PreviewKind> {
        if self.is_conflict() {
            return None;
        }
        PreviewKind::of(self.source.as_ref()?.path())
    }

    /// A raster image, whose views are pictures: Diff View stacks the new
    /// image over the old one, File View shows the new one alone.
    fn is_image(&self) -> bool {
        self.kind().is_some_and(PreviewKind::opens_on_preview)
    }

    /// Whether the body draws a preview rather than text.
    fn showing_preview(&self) -> bool {
        self.is_image() || (self.preview && self.kind().is_some())
    }

    /// Show the file's preview.
    fn show_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview {
            return;
        }
        self.preview = true;
        self.view_file = false;
        self.load_blame(cx);
        self.load_preview(cx);
    }

    /// Flip the panel between the patch and the whole file.
    fn set_view_file(&mut self, view_file: bool, cx: &mut Context<Self>) {
        if self.view_file == view_file && !self.preview {
            return;
        }
        self.preview = false;
        self.view_file = view_file;
        // An image's two views are the same pictures, already read.
        if self.is_image() {
            if self.preview_data.is_none() {
                self.load_preview(cx);
            }
            cx.notify();
            return;
        }
        if view_file {
            self.load_file_view(cx);
        }
        self.load_blame(cx);
        cx.notify();
    }

    /// Blame rides with File View: turning it on goes there.
    fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        // An image has no lines to annotate.
        if self.is_image() {
            return;
        }
        self.blame_on = !self.blame_on;
        self.preview = false;
        if !self.view_file {
            self.view_file = true;
            self.load_file_view(cx);
        }
        self.load_blame(cx);
        cx.notify();
    }

    /// Hunk asks git for a different read; Inline and Split lay out the
    /// same one.
    fn set_layout(&mut self, layout: DiffLayout, cx: &mut Context<Self>) {
        if self.layout == layout {
            return;
        }
        let reread = (self.layout == DiffLayout::Hunk) != (layout == DiffLayout::Hunk);
        self.layout = layout;
        if reread {
            self.reload(cx);
        } else {
            self.rebuild_rows(cx);
        }
    }

    /// Open the working copy in the system's editor for its type.
    fn edit_file(&mut self, cx: &mut Context<Self>) {
        if let Some(source) = &self.source {
            cx.open_with_system(&Path::new(source.repo()).join(source.path()));
        }
    }

    /// The patch's rows, with the bars that scroll them: one down the side,
    /// and one along the bottom per column of code — per side in a split, since each side overflows by its
    /// own amount once the divider moves. A split also gets the divider
    /// itself, laid over the rows' own hairline.
    fn render_code(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = settings::current(cx);
        let split = self.is_split();
        let ratio = settings.diff.split_share;
        let sideways = self
            .parsed
            .as_ref()
            .and_then(|parsed| parsed.sideways.first())
            .filter(|_| !settings.diff.wrap)
            .map(|state| (state.left.clone(), state.right.clone()));
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .relative()
            .on_drag_move(
                cx.listener(|_, event: &gpui_kit::DragMoveEvent<SplitterDrag>, _, cx| {
                    if event.drag(cx).0.as_ref() != SPLIT_SPLITTER {
                        return;
                    }
                    let ratio = ratio_from_pointer(event.event.position.x, event.bounds);
                    settings::update(SavePolicy::Debounced, cx, |settings| {
                        settings.diff.split_share = settings::Diff::bounded_split(ratio);
                    });
                    cx.notify();
                }),
            )
            .child(
                list(self.list.clone(), cx.processor(Self::render_row))
                    .with_sizing_behavior(gpui_kit::ListSizingBehavior::Auto)
                    .size_full(),
            )
            .child(keep_row_hint(
                &self.list,
                super::metrics::LINE_HEIGHT,
                &self.list_width,
            ))
            .vertical_scrollbar(&self.list)
            .when_some(sideways, |code, (left, right)| {
                let bar = |grow: f32| {
                    div()
                        .flex_grow(grow)
                        .flex_basis(px(0.0))
                        .min_w_0()
                        .h_full()
                        .relative()
                };
                code.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(HORIZONTAL_BAR_HEIGHT))
                        .flex()
                        .flex_row()
                        .map(|row| {
                            if split {
                                row.child(bar(ratio).horizontal_scrollbar(&left))
                                    .child(div().w(px(super::metrics::HALF_RULE)).flex_none())
                                    .child(bar(1.0 - ratio).horizontal_scrollbar(&right))
                            } else {
                                row.child(bar(1.0).horizontal_scrollbar(&left))
                            }
                        }),
                )
            })
            .when(split, |code| {
                code.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui_kit::relative(ratio))
                        .child(splitter(SPLIT_SPLITTER, theme, false, |_, cx| {
                            settings::update(SavePolicy::Immediate, cx, |settings| {
                                settings.diff.split_share = settings::Diff::DEFAULT_SPLIT_SHARE;
                            });
                            cx.refresh_windows();
                        })),
                )
            })
            .into_any_element()
    }

    /// Scroll the patch so row `ix` is on screen — a click on the ruler.
    fn reveal_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.list.scroll_to_reveal_item(ix);
        cx.notify();
    }

    fn toggle_whitespace(&mut self, cx: &mut Context<Self>) {
        settings::update(SavePolicy::Immediate, cx, |settings| {
            settings.diff.ignore_whitespace = !settings.diff.ignore_whitespace;
        });
        self.reload(cx);
    }

    /// Where the file on screen sits in the list prev/next walks.
    fn current_index(&self) -> Option<usize> {
        let path = self.source.as_ref()?.path();
        self.files.iter().position(|file| file.path == path)
    }

    /// Step `delta` ±1 through the panel's files.
    fn step_file(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(index) = self.current_index() else {
            return;
        };
        let next = index as isize + delta;
        let Some(file) = usize::try_from(next).ok().and_then(|ix| self.files.get(ix)) else {
            return;
        };
        let moved = source.with_file(file);
        self.show(moved, cx);
    }

    /// The one button above the patch: stage the whole file, or take it back
    /// out of the index. It also flips the side the patch is read from, so
    /// the change stays on screen instead of vanishing into the other list.
    fn toggle_stage(&mut self, cx: &mut Context<Self>) {
        if self.stage_busy {
            return;
        }
        let Some(DiffSource::WorkingTreeFile { repo, path, staged }) = self.source.clone() else {
            return;
        };
        self.stage_busy = true;
        cx.notify();
        self.stage_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let repo = repo.clone();
                    let path = path.clone();
                    async move {
                        let root = Path::new(&repo);
                        if staged {
                            crate::git::status::unstage(root, &[path])
                        } else {
                            crate::git::status::stage(root, &[path])
                        }
                    }
                })
                .await;
            this.update(cx, |view, cx| {
                view.stage_busy = false;
                match result {
                    Ok(()) => {
                        view.show(
                            DiffSource::WorkingTreeFile {
                                repo,
                                path,
                                staged: !staged,
                            },
                            cx,
                        );
                        cx.emit(DiffViewEvent::WorkingTreeChanged);
                    }
                    Err(error) => {
                        view.error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    /// Whether the rows pair the two sides in columns.
    fn is_split(&self) -> bool {
        self.layout == DiffLayout::Split || self.is_conflict()
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let mode = if self.is_split() {
            DiffMode::Split
        } else {
            DiffMode::Unified
        };
        let Some(parsed) = &self.parsed else {
            self.rows.clear();
            self.list.reset(0);
            return;
        };
        let (rows, _) = layout_rows(&parsed.files, mode, |_| false);
        self.rows = rows;
        reset_list(&self.list, self.rows.len(), super::metrics::LINE_HEIGHT);
        cx.notify();
    }

    /// One line of File View: the blame cell when it is on, its number, and
    /// the text beside it.
    fn render_file_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let (number, text) = self
            .file_rows
            .get(ix)
            .cloned()
            .unwrap_or_else(|| (0, String::new()));
        let blame_cell = self.blame_on.then(|| self.render_blame_cell(ix, &theme));
        let spans = self
            .file_highlights
            .as_ref()
            .and_then(|document| document.lines.get(ix))
            .map_or(&[][..], Vec::as_slice);
        let runs = super::highlight::runs_for_line(
            &text,
            spans,
            &gpui_kit::font(theme.font_mono.clone()),
            &theme,
        );
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(super::metrics::FILE_ROW_HEIGHT))
            .font_family(theme.font_mono.clone())
            .text_size(ui_rems(11.0))
            .children(blame_cell)
            .child(
                div()
                    .w(px(44.0))
                    .flex_none()
                    .pr(px(8.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(number.to_string())),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .whitespace_nowrap()
                    .child(gpui_kit::StyledText::new(text).with_runs(runs)),
            )
            .into_any_element()
    }

    /// One row of the blame gutter. The author's mark, the subject and the
    /// date print only on the first line of each run — repeating them down a
    /// 40-line block is noise. A row blame has no entry for keeps its slot, so
    /// the code column stays aligned down the file.
    fn render_blame_cell(&self, ix: usize, theme: &Theme) -> AnyElement {
        let slot = div().w(px(BLAME_WIDTH)).h_full().flex_none();
        let Some(blame) = &self.blame else {
            return slot.into_any_element();
        };
        let Some(commit) = blame
            .lines
            .get(ix)
            .and_then(|commit_ix| blame.commits.get(*commit_ix))
        else {
            return slot.into_any_element();
        };
        let first = ix == 0 || blame.lines.get(ix - 1) != blame.lines.get(ix);
        if !first {
            return slot.into_any_element();
        }
        let tint_key = if commit.author_email.is_empty() {
            &commit.author
        } else {
            &commit.author_email
        };
        slot.flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .pl(px(4.0))
            .overflow_hidden()
            .child(
                div()
                    .size(px(14.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.0))
                    .bg(avatar::author_tint(tint_key))
                    .text_size(ui_rems(9.0))
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(theme.bg)
                    .child(SharedString::from(avatar::author_initials(&commit.author))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_muted.opacity(0.85))
                    .child(SharedString::from(commit.summary.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .pr(px(6.0))
                    .text_size(ui_rems(10.0))
                    .text_color(theme.text_muted.opacity(0.55))
                    .child(SharedString::from(compact_date(commit.authored_at))),
            )
            .into_any_element()
    }

    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.build_row(ix, window, cx)
            .unwrap_or_else(|| Empty.into_any_element())
    }

    /// List row `ix`; `None` for a row that draws nothing.
    fn build_row(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = *self.rows.get(ix)?;
        let parsed = self.parsed.as_ref()?;
        let file = parsed.files.get(row.file())?;
        let theme = Theme::of(cx).clone();
        let (wrap, ratio) = settings::read(cx, |s| (s.diff.wrap, s.diff.split_share));
        let sideways = parsed.sideways.get(row.file())?;
        let fit = match wrap {
            true => Fit::Wrap,
            false => Fit::Scroll(sideways.reach.in_pixels(&theme, window)),
        };
        match row {
            // The title strip already names the file, and a folding body is
            // drawn by the fold animation; both rows only hold their place.
            DiffRow::FileHeader { .. } | DiffRow::FoldingBody { .. } => None,
            DiffRow::Notice { notice, .. } => {
                let text = file.summary().into_iter().nth(notice as usize)?;
                Some(notice_row(text, &theme))
            }
            // Shown whole, the file is its one hunk: a band would mark a gap
            // that is not there.
            DiffRow::HunkHeader { .. } if parsed.full => None,
            DiffRow::HunkHeader { hunk, .. } => Some(hunk_header_row(
                &file.hunks.get(hunk as usize)?.header,
                &theme,
            )),
            DiffRow::Line { hunk, line, .. } => {
                let line_ix = line;
                let line = file.hunks.get(hunk as usize)?.lines.get(line as usize)?;
                let track = sideways.track(ix, Half::Left, "unified");
                let spans = parsed
                    .spans
                    .get(row.file())
                    .and_then(|file| file.get(hunk as usize))
                    .and_then(|hunk| hunk.get(line_ix as usize))
                    .map_or(&[][..], Vec::as_slice);
                Some(diff_line_row(
                    line,
                    spans,
                    &theme,
                    file.gutter(),
                    fit,
                    Some(track),
                ))
            }
            DiffRow::SplitLine {
                hunk, left, right, ..
            } => {
                let lines = &file.hunks.get(hunk as usize)?.lines;
                let spans = parsed
                    .spans
                    .get(row.file())
                    .and_then(|file| file.get(hunk as usize))
                    .map_or(&[][..], Vec::as_slice);
                let half = |half: Half, at: Option<u32>, cx: &mut Context<Self>| {
                    self.split_half(ix, file, lines, spans, half, at, fit, sideways, &theme, cx)
                };
                let (left, right) = (half(Half::Left, left, cx), half(Half::Right, right, cx));
                Some(split_row(left, right, ratio, wrap).into_any_element())
            }
            DiffRow::BodyPad { .. } => {
                Some(div().h(px(super::metrics::TAIL_PAD)).into_any_element())
            }
        }
    }

    /// One half of a split row: hunk line `at` on `half`'s side, or the
    /// filler for a side with nothing there.
    #[allow(clippy::too_many_arguments)]
    fn split_half(
        &self,
        ix: usize,
        file: &FileDiff,
        lines: &[super::model::DiffLine],
        spans: &[Vec<HighlightSpan>],
        half: Half,
        at: Option<u32>,
        fit: Fit,
        sideways: &Sideways,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(line) = at.and_then(|at| lines.get(at as usize)) else {
            return match half {
                // A new file has no old side: one note says so, rather than
                // every row washing against a blank column.
                Half::Left if file.status == FileStatus::Added => empty_left(file, theme),
                Half::Left => split_missing_side(theme).into_any_element(),
                Half::Right => split_filler().into_any_element(),
            };
        };
        let (number, cell) = match half {
            Half::Left => (line.old_no, "left"),
            Half::Right => (line.new_no, "right"),
        };
        let look = at.and_then(|at| self.conflict_look(at, theme, cx));
        split_line_cell_styled(
            line,
            number,
            super::highlight::runs_for_line(
                &line.text,
                at.and_then(|at| spans.get(at as usize))
                    .map_or(&[][..], Vec::as_slice),
                &gpui_kit::font(theme.font_mono.clone()),
                theme,
            ),
            theme,
            file.gutter(),
            fit,
            Some(sideways.track(ix, half, cell)),
            look,
        )
        .into_any_element()
    }

    /// How a conflict's line looks: its side's colour, and its take box in
    /// place of the patch sign. `None` outside a conflict.
    fn conflict_look(&self, at: u32, theme: &Theme, cx: &mut Context<Self>) -> Option<CellLook> {
        let state = self.conflict.as_ref()?;
        let line = state.refs.get(at as usize).copied().flatten()?;
        let taken = state.takes.is_taken(line.conflict, line.side, line.line);
        let side = match line.side {
            Side::Ours => "ours",
            Side::Theirs => "theirs",
        };
        let take = take_box(
            SharedString::from(format!("conflict-take-{side}-{at}")),
            taken,
            theme,
        )
        .on_click(cx.listener(move |view, _, _, cx| view.conflict_toggle(line, cx)))
        .tooltip(tooltip(if taken {
            "Don't take this line"
        } else {
            "Take this line"
        }));
        Some(CellLook {
            tone: side_tone(line.side, theme),
            strong: line.conflict == state.current,
            marker: take.into_any_element(),
        })
    }

    /// Strip 1: the change mark and the path — folder quiet, name bright —
    /// then the encoding, the whole-file Stage/Unstage while it is a
    /// working-tree file, and the ✕.
    fn render_title_strip(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let path = self
            .source
            .as_ref()
            .map(|source| source.path().to_string())
            .unwrap_or_default();
        let staged = match &self.source {
            Some(DiffSource::WorkingTreeFile { staged, .. }) => Some(*staged),
            _ => None,
        };
        let status = self
            .parsed
            .as_ref()
            .and_then(|parsed| parsed.files.first())
            .map(|file| file.status);
        let mark = status.map(|status| match status {
            FileStatus::Added => ("+", theme.success),
            FileStatus::Deleted => ("−", theme.danger),
            FileStatus::Renamed => ("R", theme.accent),
            FileStatus::Modified => ("M", theme.warning),
        });
        let conflicts = self.conflict.as_ref().map(|_| self.conflict_runs().len());
        let saving = self.conflict.as_ref().is_some_and(|state| state.saving);
        // A conflict's mark is the warning, and its count stands where the
        // encoding does.
        let mark = if self.is_conflict() { None } else { mark };
        let encoding = match conflicts {
            Some(1) => Some("1 conflict".to_string()),
            Some(count) => Some(format!("{count} conflicts")),
            None => self
                .parsed
                .as_ref()
                .map(|parsed| if parsed.utf8 { "UTF-8" } else { "Binary" }.to_string()),
        };
        let is_conflict = self.is_conflict();
        let busy = self.stage_busy;
        div()
            .h(px(STRIP_HEIGHT + 8.0))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .bg(theme.surface)
            .child(
                div()
                    .id("diff-file-name")
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(6.0))
                    .text_size(ui_rems(13.0))
                    .when(is_conflict, |row| {
                        row.child(
                            icons::icon(icons::ALERT_02)
                                .size(px(14.0))
                                .flex_none()
                                .text_color(theme.warning),
                        )
                    })
                    .when_some(mark, |row, (mark, tone)| {
                        row.child(
                            div()
                                .flex_none()
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .text_color(tone)
                                .child(mark),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(
                                gpui_kit::StyledText::new(SharedString::from(path.clone()))
                                    .with_highlights([(
                                        0..file_dir(&path).len(),
                                        gpui_kit::HighlightStyle {
                                            color: Some(theme.text_muted),
                                            ..Default::default()
                                        },
                                    )]),
                            )
                            .text_color(theme.text),
                    )
                    .tooltip(tooltip(path.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .when_some(encoding, |strip, encoding| {
                        strip.child(
                            div()
                                .text_size(ui_rems(10.0))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(encoding)),
                        )
                    })
                    // The merge editor's one action, where the patch has its
                    // Stage File: the diff's own bordered button, in the
                    // accent.
                    .when(is_conflict, |strip| {
                        strip.child(
                            div()
                                .id("diff-save-output")
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(5.0))
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(theme.accent.opacity(0.7))
                                .bg(theme.accent.opacity(0.14))
                                .text_size(ui_rems(11.0))
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .when(saving, |button| button.opacity(0.45))
                                .when(!saving, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|style| style.bg(theme.accent.opacity(0.24)))
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.save_output(cx)),
                                        )
                                })
                                .tooltip(tooltip(
                                    "Write the output to the file and mark it resolved",
                                ))
                                .child(
                                    icons::icon(icons::FLOPPY_DISK)
                                        .size(px(12.0))
                                        .text_color(theme.text),
                                )
                                .child("Save Output"),
                        )
                    })
                    .when_some(staged, |strip, staged| {
                        let (label, hint, tone) = if staged {
                            (
                                "Unstage File",
                                "Remove this file from the index",
                                theme.text_muted,
                            )
                        } else {
                            ("Stage File", "Add this file to the index", theme.success)
                        };
                        let border = if staged {
                            theme.border.opacity(0.6)
                        } else {
                            theme.success.opacity(0.6)
                        };
                        let (fill, hover) = if staged {
                            (wash(0.0), wash(0.10))
                        } else {
                            (theme.success.opacity(0.10), theme.success.opacity(0.20))
                        };
                        strip.child(
                            div()
                                .id("diff-stage-file")
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(border)
                                .bg(fill)
                                .text_size(ui_rems(11.0))
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(tone)
                                .when(busy, |button| button.opacity(0.4))
                                .when(!busy, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(hover))
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.toggle_stage(cx);
                                        }))
                                })
                                .tooltip(tooltip(hint))
                                .child(label),
                        )
                    })
                    .child(
                        div()
                            .id("diff-close")
                            .size(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .bg(motion::fade_color("diff-close", wash(0.0), wash(0.10)))
                            .on_hover(motion::on_hover_fade("diff-close"))
                            .tooltip(tooltip("Close the diff and go back to the graph"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DiffViewEvent::Close)))
                            .child(
                                icons::icon(icons::CANCEL_01)
                                    .size(px(15.0))
                                    .text_color(theme.text_muted),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Strip 2: "Edit This File" on the left; the file's side of the index
    /// and [File View | Diff View] in the middle; prev/next, the three
    /// layouts, whitespace and blame on the right.
    fn render_tool_strip(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let ignore_whitespace = settings::current(cx).diff.ignore_whitespace;
        let index = self.current_index();
        let prev_ready = index.is_some_and(|ix| ix > 0);
        let next_ready = index.is_some_and(|ix| ix + 1 < self.files.len());
        let is_image = self.is_image();
        let in_diff = !self.view_file && !self.showing_preview();
        // An image's pictures ARE its two views: the Preview button is for
        // the files that also have text worth reading.
        let previewable = self.kind().is_some() && !is_image;
        let file_active = self.view_file && !self.preview;
        let diff_active = !self.view_file && !self.preview;
        let staged = match &self.source {
            Some(DiffSource::WorkingTreeFile { staged, .. }) => Some(*staged),
            _ => None,
        };
        let is_conflict = self.is_conflict();
        let conflict_count = self.conflict_runs().len();
        let conflict_current = self
            .conflict
            .as_ref()
            .map(|state| state.current)
            .unwrap_or(0);

        // A bordered group of halves; the active one wears the wash, and the
        // ends round their own outer side so the wash follows the curve.
        let group = || {
            div()
                .flex_none()
                .flex()
                .flex_row()
                .rounded(px(SEGMENT_RADIUS))
                .border_1()
                .border_color(theme.border.opacity(0.55))
        };
        #[derive(Clone, Copy)]
        enum End {
            First,
            Middle,
            Last,
        }
        let segment = |id: &'static str, end: End, active: bool, enabled: bool| {
            div()
                .id(id)
                .when(matches!(end, End::First), |segment| {
                    segment.rounded_l(px(SEGMENT_INNER_RADIUS))
                })
                .when(matches!(end, End::Last), |segment| {
                    segment.rounded_r(px(SEGMENT_INNER_RADIUS))
                })
                .when(!matches!(end, End::First), |segment| {
                    segment
                        .border_l_1()
                        .border_color(theme.border.opacity(0.55))
                })
                .h(px(TOOL_SIZE + 2.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(4.0))
                .text_size(ui_rems(11.0))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(if active { theme.text } else { theme.text_muted })
                .when(active, |segment| segment.bg(theme.accent.opacity(0.18)))
                .when(!enabled, |segment| segment.opacity(0.4))
                .when(enabled && !active, |segment| {
                    segment
                        .cursor_pointer()
                        .hover(|style| style.bg(wash(0.08)).text_color(theme.text))
                })
        };
        // An svg does not inherit its container's text colour: it is set here.
        let glyph = |path: &'static str, size: f32, active: bool| {
            icons::icon(path).size(px(size)).text_color(if active {
                theme.text
            } else {
                theme.text_muted
            })
        };
        let nav =
            |id: &'static str, end: End, path: &'static str, ready: bool, delta: isize, hint| {
                segment(id, end, false, ready)
                    .w(px(TOOL_SIZE + 12.0))
                    .when(ready, |button| {
                        button
                            .on_click(cx.listener(move |view, _, _, cx| view.step_file(delta, cx)))
                    })
                    .tooltip(tooltip(SharedString::from(hint)))
                    .child(glyph(path, 13.0, false))
            };
        let layout = |id: &'static str, end: End, which: DiffLayout, path: &'static str, hint| {
            let usable = in_diff && (!is_conflict || which == DiffLayout::Split);
            let active = in_diff
                && if is_conflict {
                    which == DiffLayout::Split
                } else {
                    self.layout == which
                };
            segment(id, end, active, usable)
                .w(px(TOOL_SIZE + 12.0))
                .when(usable, |segment| {
                    segment.on_click(cx.listener(move |view, _, _, cx| view.set_layout(which, cx)))
                })
                .tooltip(tooltip(SharedString::from(hint)))
                .child(glyph(path, 13.0, active))
        };

        div()
            .h(px(STRIP_HEIGHT + 8.0))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .bg(theme.surface)
            // Left. A commit's version is history and cannot be edited: the
            // button is only offered for the working tree.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .when(staged.is_some() || is_conflict, |left| {
                        left.child(
                            div()
                                .id("diff-edit-file")
                                .flex_none()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(5.0))
                                .h(px(TOOL_SIZE + 2.0))
                                .px(px(8.0))
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(theme.accent.opacity(0.7))
                                .text_size(ui_rems(11.0))
                                .text_color(theme.text)
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.accent.opacity(0.12)))
                                .on_click(cx.listener(|view, _, _, cx| view.edit_file(cx)))
                                .tooltip(tooltip("Open the working copy in your editor"))
                                .child(glyph(icons::PENCIL_EDIT_01, 12.0, true))
                                .child("Edit This File"),
                        )
                    }),
            )
            // Middle: which side of the index the patch is, and the view.
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(16.0))
                    // Which side of the index the file is on — for a
                    // conflicted file, that it is in neither yet.
                    .when(is_conflict, |middle| {
                        middle.child(
                            div()
                                .h(px(TOOL_SIZE + 2.0))
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .rounded(px(4.0))
                                .bg(theme.warning.opacity(0.18))
                                .text_size(ui_rems(11.0))
                                .text_color(theme.text)
                                .child("Conflicted"),
                        )
                    })
                    .when_some(staged, |middle, staged| {
                        middle.child(
                            div()
                                .h(px(TOOL_SIZE + 2.0))
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .rounded(px(4.0))
                                .bg(theme.accent.opacity(0.18))
                                .text_size(ui_rems(11.0))
                                .text_color(theme.text)
                                .child(if staged { "Staged" } else { "Unstaged" }),
                        )
                    })
                    .child(
                        group()
                            .child(
                                segment(
                                    "diff-view-file",
                                    End::First,
                                    file_active,
                                    true,
                                )
                                    .px(px(10.0))
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.set_view_file(true, cx)),
                                    )
                                    .tooltip(tooltip("Show the whole file as it stands now"))
                                    .child("File View"),
                            )
                            .child(
                                segment(
                                    "diff-view-diff",
                                    if previewable { End::Middle } else { End::Last },
                                    diff_active,
                                    true,
                                )
                                .px(px(10.0))
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.set_view_file(false, cx)),
                                )
                                .tooltip(tooltip("Show only what changed, as a patch"))
                                .child("Diff View"),
                            )
                            .when(previewable, |group| {
                                group.child(
                                    segment("diff-view-preview", End::Last, self.preview, true)
                                        .px(px(10.0))
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.show_preview(cx)),
                                        )
                                        .tooltip(tooltip(
                                            "Show the file as it looks: Markdown rendered, images drawn",
                                        ))
                                        .child("Preview"),
                                )
                            }),
                    ),
            )
            // Right: walk the files, lay the patch out, and annotate it.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .gap(px(12.0))
                    // The merge editor's own stepper, beside the files'.
                    .when(is_conflict && conflict_count > 0, |right| {
                        right
                            .child(
                                div()
                                    .text_size(ui_rems(11.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(format!(
                                        "Conflict {} of {conflict_count}",
                                        conflict_current + 1
                                    ))),
                            )
                            .child(
                                group()
                                    .child(
                                        segment("diff-conflict-prev", End::First, false, true)
                                            .w(px(TOOL_SIZE + 12.0))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.conflict_step(-1, cx)
                                            }))
                                            .tooltip(tooltip("Prev Conflict"))
                                            .child(glyph(icons::ARROW_UP_01, 13.0, false)),
                                    )
                                    .child(
                                        segment("diff-conflict-next", End::Last, false, true)
                                            .w(px(TOOL_SIZE + 12.0))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.conflict_step(1, cx)
                                            }))
                                            .tooltip(tooltip("Next Conflict"))
                                            .child(glyph(icons::ARROW_DOWN_01, 13.0, false)),
                                    ),
                            )
                    })
                    .child(
                        group()
                            .child(nav(
                                "diff-prev-file",
                                End::First,
                                icons::ARROW_UP_01,
                                prev_ready,
                                -1,
                                "Prev File",
                            ))
                            .child(nav(
                                "diff-next-file",
                                End::Last,
                                icons::ARROW_DOWN_01,
                                next_ready,
                                1,
                                "Next File",
                            )),
                    )
                    // The layouts only mean something for a patch: File View
                    // shows one file, with no second side and no hunks. A
                    // conflict's two sides only read side by side, so there
                    // the other two stay in place, disabled.
                    .child(
                        group()
                            .child(layout(
                                "diff-layout-hunk",
                                End::First,
                                DiffLayout::Hunk,
                                icons::TEXT_ALIGN_JUSTIFY_LEFT,
                                "Hunk view — only the changes, with a little context",
                            ))
                            .child(layout(
                                "diff-layout-inline",
                                End::Middle,
                                DiffLayout::Inline,
                                icons::LEFT_TO_RIGHT_LIST_BULLET,
                                "Inline view — the whole file, changes interleaved",
                            ))
                            .child(layout(
                                "diff-layout-split",
                                End::Last,
                                DiffLayout::Split,
                                icons::LAYOUT_TWO_COLUMN,
                                "Split view — old and new side by side",
                            )),
                    )
                    .child(
                        group().child(
                            segment(
                                "diff-ignore-whitespace",
                                End::First,
                                ignore_whitespace && !is_conflict,
                                !is_conflict,
                            )
                            .rounded_r(px(SEGMENT_INNER_RADIUS))
                            .w(px(TOOL_SIZE + 12.0))
                            .when(!is_conflict, |button| {
                                button.on_click(
                                    cx.listener(|view, _, _, cx| view.toggle_whitespace(cx)),
                                )
                            })
                            .tooltip(tooltip(if ignore_whitespace {
                                "Showing changes ignoring whitespace (active)"
                            } else {
                                "Ignore whitespace changes"
                            }))
                            .child(glyph(
                                icons::PARAGRAPH,
                                13.0,
                                ignore_whitespace,
                            )),
                        ),
                    )
                    .child({
                        // Blame annotates a file's lines, so it rides with
                        // File View.
                        let active = self.blame_on && self.view_file;
                        group().child(
                            segment("diff-blame", End::First, active, true)
                                .rounded_r(px(SEGMENT_INNER_RADIUS))
                                .px(px(10.0))
                                .on_click(cx.listener(|view, _, _, cx| view.toggle_blame(cx)))
                                .tooltip(tooltip(
                                    "Annotate each line with the commit that last changed it",
                                ))
                                .child("Blame"),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The change ruler down the patch's right edge: one mark per run of
    /// changed rows, placed at the fraction of the list it sits at. A click
    /// scrolls there.
    fn render_ruler(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let total = self.rows.len().max(1) as f32;
        let runs = self
            .parsed
            .as_ref()
            .map(|parsed| ruler_marks(&self.rows, &parsed.files))
            .unwrap_or_default();
        div()
            .w(px(RULER))
            .h_full()
            .flex_none()
            .relative()
            .bg(wash(0.04))
            .border_l_1()
            .border_color(theme.border.opacity(0.4))
            .children(runs.into_iter().map(|run| {
                // A merge's changed rows are its conflicts: one colour.
                let color = if self.is_conflict() {
                    theme.warning
                } else {
                    match run.tone {
                        MarkTone::Added => theme.success,
                        MarkTone::Removed => theme.danger,
                        MarkTone::Mixed => theme.warning,
                    }
                };
                let start = run.start;
                div()
                    .id(("diff-ruler-mark", start))
                    .absolute()
                    .left(px(2.0))
                    .right(px(2.0))
                    .top(gpui_kit::relative(start as f32 / total))
                    .h(gpui_kit::relative(run.len as f32 / total))
                    .min_h(px(2.0))
                    .bg(color.opacity(0.7))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| view.reveal_row(start, cx)))
            }))
            .into_any_element()
    }

    fn render_placeholder(&self, message: &str, theme: &Theme) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(ui_rems(11.0))
            .text_color(theme.text_muted)
            .child(SharedString::from(message.to_string()))
            .into_any_element()
    }

    fn render_error(&self, error: SharedString, theme: &Theme) -> AnyElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .px(px(20.0))
            .text_size(ui_rems(12.0))
            .text_color(theme.danger)
            .child(error)
            .into_any_element()
    }
}

/// A conflict side's colour: A the accent, B the warning amber.
fn side_tone(side: Side, theme: &Theme) -> gpui_kit::Hsla {
    match side {
        Side::Ours => theme.accent,
        Side::Theirs => theme.warning,
    }
}

/// A conflict line's take box.
fn take_box(id: SharedString, checked: bool, theme: &Theme) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .size(px(12.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .border_color(if checked {
            theme.accent
        } else {
            theme.text_muted.opacity(0.6)
        })
        .when(checked, |checkbox| checkbox.bg(theme.accent))
        .cursor_pointer()
        .when(checked, |checkbox| {
            checkbox.child(
                icons::icon(icons::CHECK)
                    .size(px(9.0))
                    .text_color(theme.on_accent),
            )
        })
}

use crate::ui::tip::tip as tooltip;

/// `Mon DD  HH:MM` this year, `Mon DD YYYY`
/// before it, in local time.
fn compact_date(seconds: i64) -> String {
    use chrono::{Datelike as _, Local, TimeZone as _};
    if seconds == 0 {
        return String::new();
    }
    let Some(date) = Local.timestamp_opt(seconds, 0).single() else {
        return String::new();
    };
    if date.year() == Local::now().year() {
        date.format("%b %d  %H:%M").to_string()
    } else {
        date.format("%b %d %Y").to_string()
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // With no panel beside it the patch reaches the window's
        // bottom-right corner; its left one lies over a pane of the same
        // colour, so both round.
        let corner = crate::ui::window_frame::corner_radius(window);
        let theme = Theme::of(cx).clone();
        let title = self.render_title_strip(&theme, cx);
        let tools = self.render_tool_strip(&theme, cx);
        let truncated = self.parsed.as_ref().is_some_and(|parsed| parsed.truncated);

        let body: AnyElement = if self.showing_preview() {
            match &self.preview_data {
                Some(preview) => {
                    super::preview::render(preview, self.is_image() && self.view_file, &theme)
                }
                None => self.render_placeholder("Loading…", &theme),
            }
        } else if self.view_file {
            if self.file_loading {
                self.render_placeholder("Loading…", &theme)
            } else if let Some(error) = self.file_error.clone() {
                self.render_error(error, &theme)
            } else if self.file_rows.is_empty() {
                self.render_placeholder("No content available.", &theme)
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(
                        list(self.file_list.clone(), cx.processor(Self::render_file_row))
                            .with_sizing_behavior(gpui_kit::ListSizingBehavior::Auto)
                            .size_full(),
                    )
                    .child(keep_row_hint(
                        &self.file_list,
                        super::metrics::FILE_ROW_HEIGHT,
                        &self.file_list_width,
                    ))
                    .vertical_scrollbar(&self.file_list)
                    .into_any_element()
            }
        } else if let Some(error) = self.error.clone() {
            self.render_error(error, &theme)
        } else if self.loading && self.rows.is_empty() {
            self.render_placeholder("Loading…", &theme)
        } else if self.rows.is_empty() {
            self.render_placeholder("No content available.", &theme)
        } else {
            let ruler = self.render_ruler(&theme, cx);
            let code = self.render_code(&theme, cx);
            let patch = div()
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_row()
                .child(code)
                .child(ruler);
            match self
                .conflict
                .as_ref()
                .map(|state| (state.output_list.clone(), state.output_width.clone()))
            {
                None => patch.into_any_element(),
                // The merge editor: the two sides named over the patch, and
                // what Save Output writes under it.
                Some((output_list, output_width)) => {
                    let split_ratio = settings::current(cx).diff.split_share;
                    let ours = self.render_side_header(Side::Ours, &theme, cx);
                    let theirs = self.render_side_header(Side::Theirs, &theme, cx);
                    let editing = self.conflict.as_ref().and_then(|state| state.edit.clone());
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_row()
                                .border_b_1()
                                .border_color(theme.border.opacity(0.4))
                                .bg(theme.surface)
                                .child(
                                    div()
                                        .flex_grow(split_ratio)
                                        .flex_basis(px(0.0))
                                        .min_w_0()
                                        .flex()
                                        .child(ours),
                                )
                                .child(
                                    div()
                                        .w(px(super::metrics::HALF_RULE))
                                        .self_stretch()
                                        .flex_none()
                                        .bg(crate::ui::theme::hairline(0.14)),
                                )
                                .child(
                                    div()
                                        .flex_grow(1.0 - split_ratio)
                                        .flex_basis(px(0.0))
                                        .min_w_0()
                                        .flex()
                                        .child(theirs),
                                )
                                .child(div().w(px(RULER)).flex_none()),
                        )
                        .child(patch)
                        .child(
                            div()
                                .flex_grow(0.8)
                                .flex_shrink(1.0)
                                .flex_basis(px(0.0))
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .border_t_1()
                                .border_color(theme.border.opacity(0.5))
                                .child(
                                    div()
                                        .h(px(STRIP_HEIGHT))
                                        .flex_none()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(12.0))
                                        .px(px(12.0))
                                        .border_b_1()
                                        .border_color(theme.border.opacity(0.4))
                                        .bg(theme.surface)
                                        .child(
                                            div()
                                                .text_size(ui_rems(11.0))
                                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                                .text_color(theme.text)
                                                .child("Output"),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(ui_rems(10.0))
                                                .text_color(theme.text_muted)
                                                .child(if editing.is_some() {
                                                    "what Save Output writes to the file — as you typed it"
                                                } else {
                                                    "what Save Output writes to the file"
                                                }),
                                        )
                                        .child(div().flex_1().min_w_0())
                                        // "Edit output directly":
                                        // the takes freeze while the box is
                                        // open, and Save Output writes what
                                        // is in it.
                                        .child(
                                            div()
                                                .id("diff-edit-output")
                                                .flex_none()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .gap(px(5.0))
                                                .px(px(8.0))
                                                .py(px(3.0))
                                                .rounded(px(4.0))
                                                .border_1()
                                                .border_color(theme.border.opacity(0.7))
                                                .text_size(ui_rems(11.0))
                                                .when(editing.is_some(), |button| {
                                                    button.bg(crate::ui::theme::ink(0.08))
                                                        .text_color(theme.text)
                                                })
                                                .cursor_pointer()
                                                .hover(|style| {
                                                    style.bg(crate::ui::theme::ink(0.05))
                                                })
                                                .on_click(cx.listener(
                                                    move |view, _, window, cx| {
                                                        view.toggle_output_edit(window, cx)
                                                    },
                                                ))
                                                .child(
                                                    icons::icon(if editing.is_some() {
                                                        icons::CHECK
                                                    } else {
                                                        icons::PEN
                                                    })
                                                    .size(px(11.0))
                                                    .text_color(theme.text_muted),
                                                )
                                                .child(if editing.is_some() {
                                                    "Done Editing"
                                                } else {
                                                    "Edit Output"
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .relative()
                                        .when(editing.is_none(), |area| {
                                            area.child(
                                                list(
                                                    output_list.clone(),
                                                    cx.processor(Self::render_output_row),
                                                )
                                                .size_full(),
                                            )
                                            .child(keep_row_hint(
                                                &output_list,
                                                super::metrics::LINE_HEIGHT,
                                                &output_width,
                                            ))
                                            .vertical_scrollbar(&output_list)
                                        })
                                        .when_some(editing, |area, input| {
                                            area.child(
                                                div()
                                                    .size_full()
                                                    .p(px(8.0))
                                                    .child(input),
                                            )
                                        }),
                                ),
                        )
                        .into_any_element()
                }
            }
        };

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .rounded_b(px(corner))
            .bg(theme.bg)
            .child(title)
            .child(tools)
            .child(body)
            .when(
                truncated && !self.view_file && !self.showing_preview(),
                |element| {
                    element.child(
                        div()
                            .w_full()
                            .flex_none()
                            .px(px(EDGE_INSET))
                            .py(px(4.0))
                            .bg(theme.warning.opacity(0.10))
                            .text_size(ui_rems(11.0))
                            .text_color(theme.warning)
                            .child("The patch was too large to read in full."),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain `reset` leaves every row unmeasured, so a list has nothing to
    /// size itself from and the scrollbar is drawn from the viewport alone.
    #[test]
    fn a_reset_without_a_hint_leaves_the_list_unsized() {
        let list = ListState::new(0, ListAlignment::Top, px(1024.0));
        list.reset(50_000);
        assert_eq!(list.max_offset_for_scrollbar().y, px(0.0));
    }

    /// The fix: the extent is the row count times the row height from the
    /// first frame, whatever the file's length.
    #[test]
    fn a_reset_with_a_hint_sizes_the_list_from_its_rows() {
        let list = ListState::new(0, ListAlignment::Top, px(1024.0));
        reset_list(&list, 50_000, super::super::metrics::LINE_HEIGHT);
        assert_eq!(
            list.max_offset_for_scrollbar().y,
            px(50_000.0 * super::super::metrics::LINE_HEIGHT),
        );
    }

    /// Rows of one height, in a pane whose width the test sets.
    struct Pane {
        list: ListState,
        seen: SeenWidth,
        width: f32,
        keep_hint: bool,
    }

    impl Render for Pane {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(self.width))
                .h(px(400.0))
                .relative()
                .child(
                    list(self.list.clone(), |_, _, _| {
                        div()
                            .h(px(super::super::metrics::LINE_HEIGHT))
                            .into_any_element()
                    })
                    .size_full(),
                )
                .when(self.keep_hint, |pane| {
                    pane.child(keep_row_hint(
                        &self.list,
                        super::super::metrics::LINE_HEIGHT,
                        &self.seen,
                    ))
                })
        }
    }

    /// How far a list of 50 000 rows scrolls once laid out at a new width.
    fn extent_after_resize(keep_hint: bool, cx: &mut gpui_kit::TestAppContext) -> Pixels {
        let list = ListState::new(0, ListAlignment::Top, px(1024.0));
        reset_list(&list, 50_000, super::super::metrics::LINE_HEIGHT);
        let (pane, cx) = cx.add_window_view(|_, _| Pane {
            list: list.clone(),
            seen: SeenWidth::default(),
            width: 600.0,
            keep_hint,
        });
        let draw = |cx: &mut gpui_kit::VisualTestContext| {
            cx.run_until_parked();
            cx.update(|window, cx| {
                _ = window.draw(cx);
            });
            cx.run_until_parked();
        };
        draw(cx);
        pane.update(cx, |pane, cx| {
            pane.width = 500.0;
            cx.notify();
        });
        draw(cx);
        list.max_offset_for_scrollbar().y
    }

    /// GPUI drops every row's height when a list changes width, so the
    /// scrollbar shrinks to the rows drawn since: a few screens, not 50 000
    /// rows.
    #[gpui_kit::test]
    fn a_new_width_drops_the_hint(cx: &mut gpui_kit::TestAppContext) {
        assert!(extent_after_resize(false, cx) < px(5_000.0));
    }

    /// The fix: the hint comes back, and with it the whole extent.
    #[gpui_kit::test]
    fn keep_row_hint_restores_the_extent_after_a_new_width(cx: &mut gpui_kit::TestAppContext) {
        let full = 50_000.0 * super::super::metrics::LINE_HEIGHT;
        let extent = extent_after_resize(true, cx);
        assert!(extent > px(full - 1_000.0), "{extent:?}");
    }
}
