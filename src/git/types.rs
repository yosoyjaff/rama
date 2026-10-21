//! The data the git layer hands to the views: commits and their refs, the
//! repository's state, file lists and patches. Plain values, filled by the
//! readers in this module and drawn by the features.

/// What sort of ref a badge on a commit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Branch,
    Remote,
    Tag,
}

/// A ref pointing at a commit, as its badge shows it (`main`,
/// `origin/main`, `v1.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefBadge {
    pub kind: RefKind,
    pub label: String,
}

/// Who wrote a commit, and when, as git records it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// RFC 3339, in the writer's own offset.
    pub when: String,
}

/// A commit in the graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    /// First parent first; a merge has more.
    pub parents: Vec<String>,
    pub subject: String,
    /// The message after the subject, shortened by the reader, often empty.
    /// It comes with the page so the detail panel needs no further read.
    pub body: String,
    pub author: Signature,
    pub refs: Vec<RefBadge>,
    /// A stash rather than a commit on a branch, drawn with its own mark and
    /// dashed lines.
    pub is_stash: bool,
}

/// A remote and the URL it fetches from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

/// What the toolbar and the menus need to know about the checkout. Read once,
/// with the first page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoState {
    /// The checked-out branch; `None` with HEAD detached.
    pub branch: Option<String>,
    /// The remote the branch tracks, or else the repository's default one.
    pub tracked: Option<GitRemote>,
    pub stash_count: usize,
    /// A merge, rebase, cherry-pick or revert under way, if any.
    pub operation: crate::git::conflicts::OperationStatus,
    pub remotes: Vec<GitRemote>,
}

/// A stretch of history, and what the pane draws around it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryPage {
    pub commits: Vec<Commit>,
    /// The commit at the tip of every branch, once each, for the branch-tips
    /// view. Only on the first page.
    pub tips: Vec<Commit>,
    pub head_sha: Option<String>,
    /// Where the next page starts, if there is one.
    pub next: Option<usize>,
    /// Only on the first page; later ones describe the same checkout.
    pub state: Option<RepoState>,
}

/// A file changed by a commit or in the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFileSummary {
    pub path: String,
    /// Where it was before a rename or copy.
    pub old_path: Option<String>,
    /// Git's status letter.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

/// A branch, remote branch or tag in the side panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub name: String,
    /// The branch the main checkout has out.
    pub current: bool,
    /// The linked worktree that has it out, if one does.
    pub worktree_path: Option<String>,
}

/// How a file is in conflict: which sides changed, added or deleted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    BothModified,
    BothAdded,
    BothDeleted,
    AddedByUs,
    AddedByThem,
    DeletedByUs,
    DeletedByThem,
}

/// The conflict an unmerged status pair (`UU`, `AA`, …) stands for.
pub fn conflict_kind(xy: &str) -> Option<ConflictKind> {
    use ConflictKind::*;
    [
        ("UU", BothModified),
        ("AA", BothAdded),
        ("DD", BothDeleted),
        ("AU", AddedByUs),
        ("UA", AddedByThem),
        ("DU", DeletedByUs),
        ("UD", DeletedByThem),
    ]
    .into_iter()
    .find_map(|(pair, kind)| (pair == xy).then_some(kind))
}

/// One path in `git status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatusEntry {
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    /// Status of the index against HEAD (git's first letter).
    pub index: String,
    /// Status of the working tree against the index (the second).
    pub worktree: String,
    pub untracked: bool,
    pub conflicted: bool,
    pub conflict_kind: Option<ConflictKind>,
    /// Lines added and removed against HEAD; zero for untracked and binary
    /// files.
    pub additions: u32,
    pub deletions: u32,
}

impl GitStatusEntry {
    fn changed(letter: &str) -> bool {
        !letter.is_empty() && letter != "."
    }

    /// Something of it is staged.
    pub fn is_staged(&self) -> bool {
        !self.untracked && Self::changed(&self.index)
    }

    /// It has changes not yet staged. Untracked files are their own list.
    pub fn is_unstaged(&self) -> bool {
        !self.untracked && Self::changed(&self.worktree)
    }
}

/// The working tree: every changed path, and where the branch stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    /// `None` with HEAD detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<GitStatusEntry>,
}

/// A patch for the viewer, with the list of files in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSnapshot {
    /// The branch it was read on, or `HEAD` when detached.
    pub branch: String,
    /// The commit it is against: the commit itself, or HEAD for the working
    /// tree. `None` in a repository without commits.
    pub head_sha: Option<String>,
    pub patch: String,
    pub files: Vec<DiffFileSummary>,
    pub additions: u32,
    pub deletions: u32,
    /// The patch was cut at its size limit, and the view says so.
    pub truncated: bool,
}
