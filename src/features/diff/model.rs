//! A patch as data, and the flat list of rows the viewer scrolls.
//!
//! [`read_patch`] turns `git diff` text into [`FileDiff`]s. [`layout_rows`]
//! lays those out as one list (a row per file header, summary line, hunk
//! band and code line) which the view virtualizes, so only what is on screen
//! is ever built; a folded file contributes its header and nothing else.
//! [`ruler_marks`] summarizes the list for the change ruler beside it.

use std::ops::Range;
use std::sync::Arc;

use super::highlight::{HighlightSpan, HighlightedDocument, Highlighter, Language};
use super::metrics::MIN_GUTTER;
use crate::git::conflict_file::{ConflictSegment, Side};

/// One column (unified) or two side by side (split).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffMode {
    #[default]
    Unified,
    Split,
}

// ── The patch ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
    /// `\ No newline at end of file`, or another remark from git.
    Meta,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    /// The whole `@@ … @@` line.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    /// Where the file is after the change.
    pub path: String,
    /// Where it was before, for a rename.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    /// Things the header said that deserve a line, like a mode change.
    pub notices: Vec<String>,
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
    /// The largest line number on either side; it sizes the gutters.
    pub max_line: u32,
}

impl FileDiff {
    fn new(path: String, old_path: Option<String>) -> Self {
        Self {
            path,
            old_path,
            status: FileStatus::Modified,
            binary: false,
            notices: Vec::new(),
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
            max_line: 0,
        }
    }

    /// Append to the last hunk, keeping the counts in step.
    fn add_line(&mut self, line: DiffLine) {
        match line.kind {
            LineKind::Add => self.additions += 1,
            LineKind::Del => self.deletions += 1,
            LineKind::Context | LineKind::Meta => {}
        }
        for n in [line.old_no, line.new_no].into_iter().flatten() {
            self.max_line = self.max_line.max(n);
        }
        if let Some(hunk) = self.hunks.last_mut() {
            hunk.lines.push(line);
        }
    }

    /// What the file shows above its hunks: what happened to it as a whole.
    pub fn summary(&self) -> Vec<String> {
        let what = match self.status {
            FileStatus::Added => Some("New file".to_owned()),
            FileStatus::Deleted => Some("Deleted file".to_owned()),
            FileStatus::Renamed => Some(format!(
                "Renamed from {}",
                self.old_path.as_deref().unwrap_or("?")
            )),
            FileStatus::Modified => None,
        };
        let binary = self
            .binary
            .then(|| "Binary file — contents not shown".to_owned());
        what.into_iter()
            .chain(binary)
            .chain(self.notices.iter().cloned())
            .collect()
    }

    /// The width of one line-number column: room for the largest number in
    /// the 11px gutter font (about 6.6px a digit) plus 14px of margin, and
    /// never under [`MIN_GUTTER`].
    pub fn gutter(&self) -> f32 {
        let digits = self.max_line.max(1).to_string().len() as f32;
        (14.0 + 6.6 * digits).max(MIN_GUTTER)
    }

    /// Whether hunk `index` is drawn under a band. A band marks skipped
    /// lines, so the first hunk only gets one if git named its section;
    /// otherwise a file shown whole would open on an empty grey strip.
    fn has_band(&self, index: usize) -> bool {
        index > 0
            || self
                .hunks
                .first()
                .is_some_and(|hunk| !section_name(&hunk.header).is_empty())
    }

    /// The split layout faces old and new; a deleted file has only old, so
    /// it keeps one column. A new file keeps two, the empty left saying
    /// there was nothing before.
    fn splits(&self, mode: DiffMode) -> bool {
        mode == DiffMode::Split && self.status != FileStatus::Deleted
    }
}

/// Syntax colours for the two sides of a file, when there are any.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiffHighlights {
    pub old: Option<Arc<HighlightedDocument>>,
    pub new: Option<Arc<HighlightedDocument>>,
}

impl DiffHighlights {
    /// The spans for `line`, from its own side. Unchanged lines use the new
    /// side when it is highlighted, and the old one otherwise.
    pub fn spans(&self, line: &DiffLine) -> &[HighlightSpan] {
        let (document, number) = match line.kind {
            LineKind::Meta => return &[],
            LineKind::Add => (self.new.as_deref(), line.new_no),
            LineKind::Del => (self.old.as_deref(), line.old_no),
            LineKind::Context => match self.new.as_deref() {
                Some(new) => (Some(new), line.new_no),
                None => (self.old.as_deref(), line.old_no),
            },
        };
        let (Some(document), Some(number)) = (document, number) else {
            return &[];
        };
        number
            .checked_sub(1)
            .and_then(|index| document.lines.get(index as usize))
            .map_or(&[], Vec::as_slice)
    }
}

/// Syntax spans for every line of `file`'s hunks, indexed `[hunk][line]`;
/// empty when its language is not known. Each side is read as its own text —
/// the old side through context and removed lines, the new side through
/// context and added lines — so a comment opened on one side does not leak
/// into the other. Context lines take the new side's reading.
pub fn hunk_spans(file: &FileDiff) -> Vec<Vec<Vec<HighlightSpan>>> {
    let Some(language) = Language::of(&file.path) else {
        return Vec::new();
    };
    let (mut old, mut new) = (Highlighter::new(language), Highlighter::new(language));
    file.hunks
        .iter()
        .map(|hunk| {
            old.reset();
            new.reset();
            hunk.lines
                .iter()
                .map(|line| match line.kind {
                    LineKind::Meta => Vec::new(),
                    LineKind::Del => old.line(&line.text),
                    LineKind::Add => new.line(&line.text),
                    LineKind::Context => {
                        old.line(&line.text);
                        new.line(&line.text)
                    }
                })
                .collect()
        })
        .collect()
}

// ── Reading a patch ─────────────────────────────────────────────────────────

/// Read `git diff` output. Lines it does not recognize are passed over, and
/// a hunk cut short keeps the lines it had.
pub fn read_patch(patch: &str) -> Vec<FileDiff> {
    let mut reader = PatchReader::default();
    for raw in patch.lines() {
        reader.feed(raw);
    }
    reader.files
}

#[derive(Default)]
struct PatchReader {
    files: Vec<FileDiff>,
    /// Inside a hunk: the next line number on the old and the new side.
    next: Option<(u32, u32)>,
}

impl PatchReader {
    fn feed(&mut self, raw: &str) {
        if let Some(paths) = raw.strip_prefix("diff --git ") {
            let (before, after) = git_paths(paths);
            let renamed = (before != after).then_some(before);
            self.files.push(FileDiff::new(after, renamed));
            self.next = None;
            return;
        }
        let Some(file) = self.files.last_mut() else {
            return;
        };
        if raw.starts_with("@@") {
            if let Some(start) = hunk_starts(raw) {
                file.hunks.push(Hunk {
                    header: raw.to_owned(),
                    lines: Vec::new(),
                });
                self.next = Some(start);
            }
            return;
        }
        if let Some((old, new)) = &mut self.next {
            let mut chars = raw.chars();
            let marker = chars.next();
            let text = chars.as_str().to_owned();
            let line = match marker {
                None | Some(' ') => {
                    *old += 1;
                    *new += 1;
                    (LineKind::Context, Some(*old - 1), Some(*new - 1), text)
                }
                Some('-') => {
                    *old += 1;
                    (LineKind::Del, Some(*old - 1), None, text)
                }
                Some('+') => {
                    *new += 1;
                    (LineKind::Add, None, Some(*new - 1), text)
                }
                Some('\\') => (LineKind::Meta, None, None, text.trim().to_owned()),
                // Anything else closes the hunk and is a header line.
                Some(_) => {
                    self.next = None;
                    return read_header(file, raw);
                }
            };
            let (kind, old_no, new_no, text) = line;
            file.add_line(DiffLine {
                kind,
                old_no,
                new_no,
                text,
            });
            return;
        }
        read_header(file, raw);
    }
}

/// What a header line says about the file, if it is one this viewer uses.
fn read_header(file: &mut FileDiff, raw: &str) {
    let rest = |prefix: &str| raw.strip_prefix(prefix).map(str::trim);
    if raw.starts_with("new file mode") || rest("--- ") == Some("/dev/null") {
        file.status = FileStatus::Added;
    } else if raw.starts_with("deleted file mode") || rest("+++ ") == Some("/dev/null") {
        file.status = FileStatus::Deleted;
    } else if let Some(from) = rest("rename from ") {
        file.status = FileStatus::Renamed;
        file.old_path = Some(from.to_owned());
    } else if let Some(to) = rest("rename to ") {
        file.status = FileStatus::Renamed;
        file.path = to.to_owned();
    } else if let Some(mode) = rest("new mode ") {
        file.notices.push(format!("Mode changed to {mode}"));
    } else if let Some(after) = rest("+++ ") {
        // A rename already has its path from the rename lines.
        if file.old_path.is_none() {
            file.path = drop_side(after).to_owned();
        }
    } else if raw.starts_with("Binary files") || raw.starts_with("GIT binary patch") {
        file.binary = true;
    }
}

/// `a/x` → `x`, `b/x` → `x`.
fn drop_side(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

/// The two paths of a `diff --git` line. Git quotes a path with unusual
/// characters; an unquoted one with spaces in it is split at the last ` b/`.
fn git_paths(paths: &str) -> (String, String) {
    let clean = |part: &str| drop_side(&unquote(part.trim())).to_owned();
    let split = paths.rfind(" b/").or_else(|| paths.rfind(" \"b/"));
    match split {
        Some(at) => (clean(&paths[..at]), clean(&paths[at + 1..])),
        None => (clean(paths), clean(paths)),
    }
}

/// A path as git wrote it, without the quotes and backslash escapes.
fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped || c != '\\' {
            out.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    out
}

/// The first line numbers in `@@ -old[,count] +new[,count] @@`.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let ranges = header.strip_prefix("@@")?.split("@@").next()?;
    let first = |sign: char| {
        ranges
            .split_whitespace()
            .find_map(|word| word.strip_prefix(sign))
            .and_then(|range| range.split(',').next())
            .and_then(|n| n.parse().ok())
    };
    Some((first('-')?, first('+')?))
}

/// The section git names after a hunk's second `@@`, often a function;
/// empty when it named none.
pub fn section_name(header: &str) -> &str {
    let after = header.strip_prefix("@@").unwrap_or(header);
    match after.split_once("@@") {
        Some((_, tail)) => tail.trim(),
        None => "",
    }
}

// ── Side by side ────────────────────────────────────────────────────────────

/// A split row: the hunk lines shown left and right, by index in the hunk.
type Facing = (Option<u32>, Option<u32>);

/// The rows of a hunk laid side by side, as a lazy sequence, so taking a
/// few rows of a huge hunk costs a few rows of work.
///
/// An unchanged line faces itself. An edit is a run of removed lines and
/// then added ones: they face each other in order, and the longer side runs
/// on alone. A removal after an addition opens a new edit (`-a +b -c +d` is
/// two). A `\ No newline` remark sticks to the side it follows and does not
/// close the edit, so an edited last line stays one edit, its two remarks
/// facing each other after it.
struct Faces<'a> {
    lines: std::iter::Enumerate<std::slice::Iter<'a, DiffLine>>,
    edit: Edit,
    /// Rows ready to hand out, front first.
    ready: std::collections::VecDeque<Facing>,
    /// The kind of the last non-remark line.
    after: LineKind,
}

#[derive(Default)]
struct Edit {
    old: Vec<u32>,
    new: Vec<u32>,
    old_remarks: Vec<u32>,
    new_remarks: Vec<u32>,
}

impl Edit {
    fn is_empty(&self) -> bool {
        self.old.is_empty()
            && self.new.is_empty()
            && self.old_remarks.is_empty()
            && self.new_remarks.is_empty()
    }

    /// Whether anything has come after the removed lines.
    fn past_removals(&self) -> bool {
        !(self.new.is_empty() && self.old_remarks.is_empty() && self.new_remarks.is_empty())
    }

    fn drain_into(&mut self, out: &mut std::collections::VecDeque<Facing>) {
        let mut zip = |left: &mut Vec<u32>, right: &mut Vec<u32>| {
            let rows = left.len().max(right.len());
            out.extend((0..rows).map(|i| (left.get(i).copied(), right.get(i).copied())));
            left.clear();
            right.clear();
        };
        zip(&mut self.old, &mut self.new);
        zip(&mut self.old_remarks, &mut self.new_remarks);
    }
}

impl Iterator for Faces<'_> {
    type Item = Facing;

    fn next(&mut self) -> Option<Facing> {
        loop {
            if let Some(row) = self.ready.pop_front() {
                return Some(row);
            }
            let Some((i, line)) = self.lines.next() else {
                if self.edit.is_empty() {
                    return None;
                }
                self.edit.drain_into(&mut self.ready);
                continue;
            };
            let i = i as u32;
            match line.kind {
                LineKind::Context => {
                    self.edit.drain_into(&mut self.ready);
                    self.ready.push_back((Some(i), Some(i)));
                }
                LineKind::Del => {
                    if self.edit.past_removals() {
                        self.edit.drain_into(&mut self.ready);
                    }
                    self.edit.old.push(i);
                }
                LineKind::Add => {
                    if !self.edit.new_remarks.is_empty() {
                        self.edit.drain_into(&mut self.ready);
                    }
                    self.edit.new.push(i);
                }
                LineKind::Meta => match self.after {
                    LineKind::Del => self.edit.old_remarks.push(i),
                    LineKind::Add => self.edit.new_remarks.push(i),
                    _ => {
                        self.edit.old_remarks.push(i);
                        self.edit.new_remarks.push(i);
                    }
                },
            }
            if line.kind != LineKind::Meta {
                self.after = line.kind;
            }
        }
    }
}

/// The hunk's lines as split rows, lazily.
fn faces(lines: &[DiffLine]) -> Faces<'_> {
    Faces {
        lines: lines.iter().enumerate(),
        edit: Edit::default(),
        ready: Default::default(),
        after: LineKind::Context,
    }
}

/// The first `limit` split rows of a hunk. A fold animation draws a few rows
/// of what may be an enormous hunk; this only does the work for those.
pub fn split_rows(lines: &[DiffLine], limit: usize) -> Vec<Facing> {
    faces(lines).take(limit).collect()
}

// ── Rows ────────────────────────────────────────────────────────────────────

/// One row of the scrolling list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRow {
    FileHeader {
        file: u32,
    },
    Notice {
        file: u32,
        notice: u32,
    },
    HunkHeader {
        file: u32,
        hunk: u32,
    },
    Line {
        file: u32,
        hunk: u32,
        line: u32,
        /// The line's position counting across all the file's hunks.
        flat: u32,
    },
    /// Two lines facing each other in the split layout.
    SplitLine {
        file: u32,
        hunk: u32,
        left: Option<u32>,
        right: Option<u32>,
    },
    /// The space under a file's last line.
    BodyPad {
        file: u32,
    },
    /// A whole body standing in as one row while it folds open or shut.
    FoldingBody {
        file: u32,
    },
}

impl DiffRow {
    pub fn file(self) -> usize {
        let (Self::FileHeader { file }
        | Self::Notice { file, .. }
        | Self::HunkHeader { file, .. }
        | Self::Line { file, .. }
        | Self::SplitLine { file, .. }
        | Self::BodyPad { file }
        | Self::FoldingBody { file }) = self;
        file as usize
    }
}

/// Every file as rows, and the range of rows each file covers, its header
/// first. A file `folded` says yes to is its header alone.
pub fn layout_rows(
    files: &[FileDiff],
    mode: DiffMode,
    mut folded: impl FnMut(usize) -> bool,
) -> (Vec<DiffRow>, Vec<Range<usize>>) {
    let mut rows = Vec::new();
    let spans = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let start = rows.len();
            let id = index as u32;
            rows.push(DiffRow::FileHeader { file: id });
            if !folded(index) {
                body_rows(file, id, mode, &mut rows);
            }
            start..rows.len()
        })
        .collect();
    (rows, spans)
}

fn body_rows(file: &FileDiff, id: u32, mode: DiffMode, rows: &mut Vec<DiffRow>) {
    let summary = file.summary().len() as u32;
    rows.extend((0..summary).map(|notice| DiffRow::Notice { file: id, notice }));
    let mut before = 0u32;
    for (index, hunk) in file.hunks.iter().enumerate() {
        let h = index as u32;
        if file.has_band(index) {
            rows.push(DiffRow::HunkHeader { file: id, hunk: h });
        }
        if file.splits(mode) {
            rows.extend(faces(&hunk.lines).map(|(left, right)| DiffRow::SplitLine {
                file: id,
                hunk: h,
                left,
                right,
            }));
        } else {
            let count = hunk.lines.len() as u32;
            rows.extend((0..count).map(|line| DiffRow::Line {
                file: id,
                hunk: h,
                line,
                flat: before + line,
            }));
        }
        before += hunk.lines.len() as u32;
    }
    rows.push(DiffRow::BodyPad { file: id });
}

// ── The change ruler ────────────────────────────────────────────────────────

/// The colour of a ruler mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkTone {
    Added,
    Removed,
    /// Both added and removed lines.
    Mixed,
}

/// A mark on the ruler: a run of rows that show a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RulerMark {
    pub start: usize,
    pub len: usize,
    pub tone: MarkTone,
}

/// The ruler's marks, counted in rows. The ruler stands for the list's
/// scroll range, headers and bands included, so a mark sits exactly where
/// scrolling would bring its rows.
pub fn ruler_marks(rows: &[DiffRow], files: &[FileDiff]) -> Vec<RulerMark> {
    let kind = |file: u32, hunk: u32, line: Option<u32>| -> Option<LineKind> {
        let hunk = files.get(file as usize)?.hunks.get(hunk as usize)?;
        hunk.lines.get(line? as usize).map(|l| l.kind)
    };
    // Per row: (shows an addition, shows a removal).
    let flags: Vec<(bool, bool)> = rows
        .iter()
        .map(|row| match *row {
            DiffRow::Line {
                file, hunk, line, ..
            } => {
                let k = kind(file, hunk, Some(line));
                (k == Some(LineKind::Add), k == Some(LineKind::Del))
            }
            DiffRow::SplitLine {
                file,
                hunk,
                left,
                right,
            } => (
                kind(file, hunk, right) == Some(LineKind::Add),
                kind(file, hunk, left) == Some(LineKind::Del),
            ),
            _ => (false, false),
        })
        .collect();

    let changed = |&(added, removed): &(bool, bool)| added || removed;
    let mut marks = Vec::new();
    let mut start = 0;
    for run in flags.chunk_by(|a, b| changed(a) == changed(b)) {
        if changed(&run[0]) {
            let added = run.iter().any(|f| f.0);
            let removed = run.iter().any(|f| f.1);
            marks.push(RulerMark {
                start,
                len: run.len(),
                tone: match (added, removed) {
                    (true, true) => MarkTone::Mixed,
                    (true, false) => MarkTone::Added,
                    _ => MarkTone::Removed,
                },
            });
        }
        start += run.len();
    }
    marks
}

// ── Conflicts ───────────────────────────────────────────────────────────────

/// The conflict, side and line of that side a patch line stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictLine {
    pub conflict: usize,
    pub side: Side,
    pub line: usize,
}

impl FileDiff {
    /// A conflicted file as a one-hunk patch: ours is the old side and
    /// theirs the new, so the split layout sets them face to face and what
    /// both share is context. Beside it, each hunk line's conflict line, if
    /// it is one.
    pub fn from_conflict(
        path: &str,
        segments: &[ConflictSegment],
    ) -> (FileDiff, Vec<Option<ConflictLine>>) {
        let mut file = FileDiff::new(path.to_owned(), None);
        file.hunks.push(Hunk {
            header: String::new(),
            lines: Vec::new(),
        });
        let mut origins = Vec::new();
        let (mut ours, mut theirs) = (0u32, 0u32);
        let mut conflict_ix = 0;
        for segment in segments {
            let (shared, sides): (&[String], [(&[String], Side); 2]) = match segment {
                ConflictSegment::Stable(shared) => {
                    (shared, [(&[], Side::Ours), (&[], Side::Theirs)])
                }
                ConflictSegment::Conflict { ours, theirs } => {
                    (&[], [(ours, Side::Ours), (theirs, Side::Theirs)])
                }
            };
            for text in shared {
                ours += 1;
                theirs += 1;
                file.add_line(DiffLine {
                    kind: LineKind::Context,
                    old_no: Some(ours),
                    new_no: Some(theirs),
                    text: text.clone(),
                });
                origins.push(None);
            }
            for (lines, side) in sides {
                for (line, text) in lines.iter().enumerate() {
                    let (kind, old_no, new_no) = match side {
                        Side::Ours => {
                            ours += 1;
                            (LineKind::Del, Some(ours), None)
                        }
                        Side::Theirs => {
                            theirs += 1;
                            (LineKind::Add, None, Some(theirs))
                        }
                    };
                    file.add_line(DiffLine {
                        kind,
                        old_no,
                        new_no,
                        text: text.clone(),
                    });
                    origins.push(Some(ConflictLine {
                        conflict: conflict_ix,
                        side,
                        line,
                    }));
                }
            }
            if matches!(segment, ConflictSegment::Conflict { .. }) {
                conflict_ix += 1;
            }
        }
        file.hunks[0].header = format!("@@ -1,{ours} +1,{theirs} @@");
        (file, origins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "\
diff --git a/src/app.rs b/src/app.rs
index 3333333..4444444 100644
--- a/src/app.rs
+++ b/src/app.rs
@@ -7,4 +7,5 @@ fn main()
 first
-before
+after
+more
 last
diff --git a/README b/README
new file mode 100644
--- /dev/null
+++ b/README
@@ -0,0 +1,2 @@
+alpha
+beta
";

    fn kinds(lines: &[DiffLine]) -> Vec<LineKind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn files_hunks_and_line_numbers_come_out_of_a_patch() {
        let files = read_patch(PATCH);
        assert_eq!(files.len(), 2);
        let app = &files[0];
        assert_eq!(
            (app.path.as_str(), app.status),
            ("src/app.rs", FileStatus::Modified)
        );
        assert_eq!((app.additions, app.deletions, app.max_line), (2, 1, 10));
        use LineKind::*;
        let lines = &app.hunks[0].lines;
        assert_eq!(kinds(lines), [Context, Del, Add, Add, Context]);
        let numbers: Vec<_> = lines.iter().map(|l| (l.old_no, l.new_no)).collect();
        assert_eq!(
            numbers,
            [
                (Some(7), Some(7)),
                (Some(8), None),
                (None, Some(8)),
                (None, Some(9)),
                (Some(9), Some(10)),
            ]
        );
        assert_eq!(lines[1].text, "before");
        let readme = &files[1];
        assert_eq!(readme.status, FileStatus::Added);
        assert_eq!(readme.summary(), ["New file"]);
    }

    #[test]
    fn header_lines_mark_renames_binaries_deletions_and_modes() {
        let files = read_patch(
            "diff --git a/one/x.rs b/two/x.rs\n\
             similarity index 88%\n\
             rename from one/x.rs\n\
             rename to two/x.rs\n\
             diff --git a/pic.png b/pic.png\n\
             Binary files a/pic.png and b/pic.png differ\n\
             diff --git a/old.txt b/old.txt\n\
             deleted file mode 100644\n\
             --- a/old.txt\n\
             +++ /dev/null\n\
             @@ -1 +0,0 @@\n\
             -gone\n\
             diff --git a/build.sh b/build.sh\n\
             old mode 100644\n\
             new mode 100755\n",
        );
        assert_eq!(
            (
                files[0].status,
                files[0].path.as_str(),
                files[0].old_path.as_deref()
            ),
            (FileStatus::Renamed, "two/x.rs", Some("one/x.rs"))
        );
        assert_eq!(files[0].summary(), ["Renamed from one/x.rs"]);
        assert!(files[1].binary && files[1].hunks.is_empty());
        assert_eq!(
            (files[2].status, files[2].deletions),
            (FileStatus::Deleted, 1)
        );
        assert_eq!(files[3].summary(), ["Mode changed to 100755"]);
    }

    #[test]
    fn paths_with_quotes_or_spaces_split_as_git_means() {
        assert_eq!(
            git_paths("\"a/two words.md\" \"b/two words.md\""),
            ("two words.md".into(), "two words.md".into())
        );
        assert_eq!(git_paths("a/p q b/p q"), ("p q".into(), "p q".into()));
        assert_eq!(unquote("\"say \\\"hi\\\"\""), "say \"hi\"");
    }

    #[test]
    fn hunk_headers_give_start_lines_and_section() {
        assert_eq!(hunk_starts("@@ -20,2 +22,4 @@ impl X"), Some((20, 22)));
        assert_eq!(hunk_starts("@@ -5 +6 @@"), Some((5, 6)));
        assert_eq!(hunk_starts("@@ nonsense @@"), None);
        assert_eq!(section_name("@@ -20,2 +22,4 @@ impl X"), "impl X");
        assert_eq!(section_name("@@ -0,0 +1 @@"), "");
    }

    #[test]
    fn an_unmarked_line_closes_the_hunk() {
        let files = read_patch("diff --git a/f b/f\n@@ -1 +1 @@\n-x\n+y\nrename from q\n");
        assert_eq!(files[0].hunks[0].lines.len(), 2);
        assert_eq!(files[0].status, FileStatus::Renamed);
    }

    fn lines(spec: &str) -> Vec<DiffLine> {
        spec.chars()
            .map(|c| DiffLine {
                kind: match c {
                    ' ' => LineKind::Context,
                    '-' => LineKind::Del,
                    '+' => LineKind::Add,
                    _ => LineKind::Meta,
                },
                old_no: None,
                new_no: None,
                text: String::new(),
            })
            .collect()
    }

    fn split(spec: &str) -> Vec<Facing> {
        split_rows(&lines(spec), usize::MAX)
    }

    #[test]
    fn replaced_lines_face_their_replacements() {
        assert_eq!(
            split(" -++ "),
            [
                (Some(0), Some(0)),
                (Some(1), Some(2)),
                (None, Some(3)),
                (Some(4), Some(4))
            ]
        );
        assert_eq!(split("--+"), [(Some(0), Some(2)), (Some(1), None)]);
        assert_eq!(split("-+-+"), [(Some(0), Some(1)), (Some(2), Some(3))]);
    }

    #[test]
    fn no_newline_remarks_stay_with_their_side() {
        assert_eq!(split("-\\+\\"), [(Some(0), Some(2)), (Some(1), Some(3))]);
        assert_eq!(split(" \\"), [(Some(0), Some(0)), (Some(1), Some(1))]);
    }

    #[test]
    fn a_limit_takes_only_the_first_rows() {
        let many = format!(" {}{}", "-".repeat(400), "+".repeat(400));
        assert_eq!(
            split_rows(&lines(&many), 3),
            [
                (Some(0), Some(0)),
                (Some(1), Some(401)),
                (Some(2), Some(402))
            ]
        );
    }

    #[test]
    fn a_folded_file_is_its_header() {
        let files = read_patch(PATCH);
        let (open, spans) = layout_rows(&files, DiffMode::Unified, |_| false);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].end, open.len());
        // Header, band (git named the section), five lines, pad.
        assert_eq!(spans[0].len(), 8);
        let (folded, spans) = layout_rows(&files, DiffMode::Unified, |i| i == 0);
        assert_eq!(spans[0], 0..1);
        assert_eq!(folded[0], DiffRow::FileHeader { file: 0 });
        // Header, summary, two lines, pad: no band without a section name.
        assert_eq!(spans[1].len(), 5);
    }

    #[test]
    fn a_deleted_file_stays_one_column_when_split() {
        let files =
            read_patch("diff --git a/d b/d\ndeleted file mode 100644\n@@ -1,2 +0,0 @@\n-x\n-y\n");
        let (rows, _) = layout_rows(&files, DiffMode::Split, |_| false);
        assert!(rows.iter().any(|r| matches!(r, DiffRow::Line { .. })));
        assert!(!rows.iter().any(|r| matches!(r, DiffRow::SplitLine { .. })));
    }

    #[test]
    fn the_ruler_marks_runs_of_changed_rows() {
        let files = read_patch(PATCH);
        let (rows, _) = layout_rows(&files, DiffMode::Unified, |_| false);
        assert_eq!(
            ruler_marks(&rows, &files),
            [
                RulerMark {
                    start: 3,
                    len: 3,
                    tone: MarkTone::Mixed
                },
                RulerMark {
                    start: 10,
                    len: 2,
                    tone: MarkTone::Added
                },
            ]
        );
        let (split, _) = layout_rows(&files, DiffMode::Split, |_| false);
        assert_eq!(ruler_marks(&split, &files)[0].tone, MarkTone::Mixed);
    }

    #[test]
    fn the_gutter_widens_for_long_numbers() {
        let mut file = FileDiff::new("f".into(), None);
        assert_eq!(file.gutter(), MIN_GUTTER);
        file.max_line = 1_000_000;
        assert!(file.gutter() > MIN_GUTTER);
    }

    #[test]
    fn a_conflict_reads_ours_against_theirs() {
        let segments = vec![
            ConflictSegment::Stable(vec!["head".into()]),
            ConflictSegment::Conflict {
                ours: vec!["o1".into(), "o2".into()],
                theirs: vec!["t1".into()],
            },
            ConflictSegment::Stable(vec!["foot".into()]),
            ConflictSegment::Conflict {
                ours: vec![],
                theirs: vec!["t2".into()],
            },
        ];
        let (file, origins) = FileDiff::from_conflict("c.txt", &segments);
        use LineKind::*;
        let lines = &file.hunks[0].lines;
        assert_eq!(kinds(lines), [Context, Del, Del, Add, Context, Add]);
        assert_eq!((lines[4].old_no, lines[4].new_no), (Some(4), Some(3)));
        assert_eq!(file.hunks[0].header, "@@ -1,4 +1,4 @@");
        assert_eq!(origins[0], None);
        assert_eq!(
            origins[3],
            Some(ConflictLine {
                conflict: 0,
                side: Side::Theirs,
                line: 0
            })
        );
        assert_eq!(origins[5].map(|o| o.conflict), Some(1));
        assert_eq!(
            split_rows(lines, usize::MAX),
            [
                (Some(0), Some(0)),
                (Some(1), Some(3)),
                (Some(2), None),
                (Some(4), Some(4)),
                (None, Some(5)),
            ]
        );
    }
}
