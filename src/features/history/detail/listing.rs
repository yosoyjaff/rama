//! How the panel lists files: in git's order or by name, flat or as a
//! folder tree, and which folders of the tree are open.
//!
//! The tree is not built as nested folders. Each path gets a sort key in
//! which a folder name sorts before any file name at the same depth; in that
//! order every folder's files are contiguous, so one pass that compares each
//! path's folders with the previous path's yields the rows.

use std::collections::{HashMap, HashSet};

use crate::git::{DiffFileSummary, GitStatusEntry};

/// The last part of a path.
pub fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

/// The folder a path is in, or `""` at the top.
pub fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The panel's listing options, kept across commits.
#[derive(Debug, Default)]
pub struct Listing {
    pub as_tree: bool,
    pub by_name: bool,
    /// Every folder open, whatever was opened by hand.
    pub open_all: bool,
    /// Folders opened by hand. The rest start folded, so a large change
    /// reads first as its handful of folders.
    pub opened: HashSet<String>,
}

impl Listing {
    pub fn is_folded(&self, folder: &str) -> bool {
        !self.open_all && !self.opened.contains(folder)
    }

    pub fn flip_folder(&mut self, folder: String) {
        if !self.opened.remove(&folder) {
            self.opened.insert(folder);
        }
    }

    /// The files in listing order: by name (then path), ignoring case, or
    /// as git gave them.
    pub fn order<'a>(&self, files: &'a [DiffFileSummary]) -> Vec<&'a DiffFileSummary> {
        let mut order: Vec<_> = files.iter().collect();
        if self.by_name {
            order.sort_by_cached_key(|file| {
                (
                    file_name(&file.path).to_lowercase(),
                    file.path.to_lowercase(),
                )
            });
        }
        order
    }

    /// The rows of the tree, folded as the options say.
    pub fn tree<'a>(&self, files: &'a [DiffFileSummary]) -> Vec<Line<'a>> {
        tree_lines(files, |folder| self.is_folded(folder))
    }
}

/// A row of the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Line<'a> {
    Folder {
        /// The whole path, which is how it is folded.
        path: &'a str,
        depth: usize,
        /// Files anywhere under it.
        files: usize,
        folded: bool,
    },
    File {
        file: &'a DiffFileSummary,
        depth: usize,
    },
}

/// The folders a path is in, outermost first, each as a whole path.
fn folders_of(path: &str) -> Vec<&str> {
    path.match_indices('/').map(|(at, _)| &path[..at]).collect()
}

/// Sorts folders before files at every depth, each group by name ignoring
/// case; the name as written breaks ties, so `Src/` and `src/` stay apart.
fn tree_key(path: &str) -> Vec<(bool, String, &str)> {
    let mut parts: Vec<_> = path
        .split('/')
        .map(|part| (false, part.to_lowercase(), part))
        .collect();
    if let Some(last) = parts.last_mut() {
        last.0 = true;
    }
    parts
}

fn tree_lines<'a>(files: &'a [DiffFileSummary], is_folded: impl Fn(&str) -> bool) -> Vec<Line<'a>> {
    let mut order: Vec<&DiffFileSummary> = files.iter().collect();
    order.sort_by_cached_key(|file| tree_key(&file.path));

    let mut under: HashMap<&str, usize> = HashMap::new();
    for file in &order {
        for folder in folders_of(&file.path) {
            *under.entry(folder).or_default() += 1;
        }
    }

    let mut lines = Vec::new();
    let mut previous: Vec<&str> = Vec::new();
    for file in order {
        let folders = folders_of(&file.path);
        let kept = previous
            .iter()
            .zip(&folders)
            .take_while(|(a, b)| a == b)
            .count();
        // Down to the first folded folder; nothing inside it shows.
        let first_folded = folders.iter().position(|folder| is_folded(folder));
        let shown = first_folded.map_or(folders.len(), |at| at + 1);
        for (depth, &folder) in folders.iter().enumerate().take(shown).skip(kept) {
            lines.push(Line::Folder {
                path: folder,
                depth,
                files: under[folder],
                folded: first_folded == Some(depth),
            });
        }
        if first_folded.is_none() {
            lines.push(Line::File {
                file,
                depth: folders.len(),
            });
        }
        previous = folders;
    }
    lines
}

/// The working tree's changes on each side of the index.
#[derive(Debug, Default)]
pub struct Staging {
    pub unstaged: Vec<DiffFileSummary>,
    pub staged: Vec<DiffFileSummary>,
}

impl Staging {
    /// A file changed in both the index and the tree is on both sides,
    /// each with its own letter. An untracked file is unstaged and added.
    /// A conflicted file is on neither: it gets resolved, not staged.
    pub fn of(entries: &[GitStatusEntry]) -> Self {
        let summary = |entry: &GitStatusEntry, letters: &str| DiffFileSummary {
            path: entry.path.clone(),
            old_path: entry.old_path.clone(),
            status: letters.chars().next().unwrap_or('M').to_string(),
            additions: 0,
            deletions: 0,
            binary: false,
        };
        let mut staging = Self::default();
        for entry in entries {
            if entry.conflicted {
                continue;
            }
            if entry.untracked {
                staging.unstaged.push(summary(entry, "A"));
                continue;
            }
            if entry.is_staged() {
                staging.staged.push(summary(entry, &entry.index));
            }
            if entry.is_unstaged() {
                staging.unstaged.push(summary(entry, &entry.worktree));
            }
        }
        staging
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> DiffFileSummary {
        DiffFileSummary {
            path: path.into(),
            old_path: None,
            status: "M".into(),
            additions: 0,
            deletions: 0,
            binary: false,
        }
    }

    fn show(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                Line::Folder {
                    path,
                    depth,
                    files,
                    folded,
                } => {
                    let mark = if *folded { "+" } else { "-" };
                    format!("{depth}{mark}{}/ {files}", file_name(path))
                }
                Line::File { file, depth } => format!("{depth} {}", file_name(&file.path)),
            })
            .collect()
    }

    #[test]
    fn a_path_splits_into_folder_and_name() {
        assert_eq!(folder_of("src/ui/a.rs"), "src/ui");
        assert_eq!(file_name("src/ui/a.rs"), "a.rs");
        assert_eq!((folder_of("README"), file_name("README")), ("", "README"));
        assert_eq!(folders_of("a/b/c.rs"), ["a", "a/b"]);
    }

    #[test]
    fn by_name_puts_equal_names_side_by_side() {
        let files = [file("b/X.rs"), file("a/y.rs"), file("a/x.rs")];
        let paths = |listing: &Listing| -> Vec<String> {
            listing
                .order(&files)
                .into_iter()
                .map(|f| f.path.clone())
                .collect()
        };
        let mut listing = Listing::default();
        assert_eq!(paths(&listing), ["b/X.rs", "a/y.rs", "a/x.rs"]);
        listing.by_name = true;
        assert_eq!(paths(&listing), ["a/x.rs", "b/X.rs", "a/y.rs"]);
    }

    #[test]
    fn folders_come_before_files_at_every_depth() {
        let files = [
            file("src/main.rs"),
            file("README.md"),
            file("src/ui/a.rs"),
            file("src/ui/B.rs"),
            file("docs/x.md"),
        ];
        let lines = tree_lines(&files, |_| false);
        assert_eq!(
            show(&lines),
            [
                "0-docs/ 1",
                "1 x.md",
                "0-src/ 3",
                "1-ui/ 2",
                "2 a.rs",
                "2 B.rs",
                "1 main.rs",
                "0 README.md",
            ]
        );
    }

    #[test]
    fn a_folded_folder_keeps_its_row_and_count_only() {
        let files = [file("src/ui/a.rs"), file("src/b.rs"), file("top.rs")];
        let lines = tree_lines(&files, |folder| folder == "src/ui");
        assert_eq!(show(&lines), ["0-src/ 2", "1+ui/ 1", "1 b.rs", "0 top.rs"]);
        let lines = tree_lines(&files, |folder| folder == "src");
        assert_eq!(show(&lines), ["0+src/ 2", "0 top.rs"]);
    }

    #[test]
    fn folders_that_differ_only_in_case_stay_apart() {
        let files = [file("src/a"), file("Src/b"), file("src/c")];
        let lines = tree_lines(&files, |_| false);
        assert_eq!(show(&lines), ["0-Src/ 1", "1 b", "0-src/ 2", "1 a", "1 c"]);
    }

    #[test]
    fn folders_start_folded_until_opened() {
        let mut listing = Listing::default();
        assert!(listing.is_folded("src"));
        listing.flip_folder("src".into());
        assert!(!listing.is_folded("src"));
        listing.flip_folder("src".into());
        listing.open_all = true;
        assert!(!listing.is_folded("src"));
    }

    #[test]
    fn the_working_tree_splits_at_the_index() {
        let entry = |path: &str, index: &str, worktree: &str| GitStatusEntry {
            path: path.into(),
            index: index.into(),
            worktree: worktree.into(),
            ..GitStatusEntry::default()
        };
        let mut untracked = entry("new.txt", "?", "?");
        untracked.untracked = true;
        let mut clash = entry("clash.rs", "U", "U");
        clash.conflicted = true;
        let staging = Staging::of(&[
            entry("both.rs", "M", "M"),
            entry("idx.rs", "A", "."),
            untracked,
            clash,
        ]);
        let show = |list: &[DiffFileSummary]| {
            list.iter()
                .map(|f| format!("{}:{}", f.path, f.status))
                .collect::<Vec<_>>()
        };
        assert_eq!(show(&staging.unstaged), ["both.rs:M", "new.txt:A"]);
        assert_eq!(show(&staging.staged), ["both.rs:M", "idx.rs:A"]);
    }
}
