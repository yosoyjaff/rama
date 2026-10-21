//! What happened to a file, and how many files had each thing happen.

use gpui_kit::Hsla;

use crate::git::DiffFileSummary;
use crate::ui::icons;
use crate::ui::theme::Theme;

/// What happened to a file. A copy counts as a rename: either way the file
/// has an ancestor under another path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
}

impl Change {
    /// The order counts are written in.
    pub const ORDER: [Change; 4] = [
        Change::Modified,
        Change::Added,
        Change::Deleted,
        Change::Renamed,
    ];

    /// From whatever git called it. Its words (`added`, `copied`, …) and
    /// its letters (`A`, `C100`, …) begin with the same letter, so the
    /// first one decides; anything unknown, like a type change or an
    /// unmerged path, is a modification.
    pub fn of(status: &str) -> Self {
        match status.chars().next().map(|c| c.to_ascii_uppercase()) {
            Some('A') => Self::Added,
            Some('D') => Self::Deleted,
            Some('R' | 'C') => Self::Renamed,
            _ => Self::Modified,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
        }
    }

    /// As a past participle, after a count.
    pub fn word(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Modified => icons::PEN,
            Self::Added => icons::PLUS,
            Self::Deleted => icons::MINUS,
            Self::Renamed => icons::PEN_NEW_SQUARE,
        }
    }

    pub fn color(self, theme: &Theme) -> Hsla {
        match self {
            Self::Modified => theme.warning,
            Self::Added => theme.success,
            Self::Deleted => theme.danger,
            Self::Renamed => theme.busy,
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// How many files had each [`Change`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally([usize; 4]);

impl Tally {
    pub fn of_files(files: &[DiffFileSummary]) -> Self {
        files.iter().map(|file| Change::of(&file.status)).collect()
    }

    pub fn get(&self, change: Change) -> usize {
        self.0[change.slot()]
    }

    pub fn total(&self) -> usize {
        self.0.iter().sum()
    }

    /// The changes that happened at least once, with their counts, in
    /// [`Change::ORDER`].
    pub fn present(self) -> impl Iterator<Item = (Change, usize)> {
        Change::ORDER
            .into_iter()
            .map(move |change| (change, self.get(change)))
            .filter(|&(_, n)| n > 0)
    }
}

impl FromIterator<Change> for Tally {
    fn from_iter<I: IntoIterator<Item = Change>>(changes: I) -> Self {
        let mut tally = Self::default();
        for change in changes {
            tally.0[change.slot()] += 1;
        }
        tally
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_words_and_letters_read_the_same() {
        use Change::*;
        for (said, change) in [
            ("added", Added),
            ("A", Added),
            ("deleted", Deleted),
            ("D", Deleted),
            ("renamed", Renamed),
            ("R087", Renamed),
            ("copied", Renamed),
            ("C100", Renamed),
            ("modified", Modified),
            ("unmerged", Modified),
            ("T", Modified),
            ("", Modified),
        ] {
            assert_eq!(Change::of(said), change, "{said:?}");
        }
    }

    #[test]
    fn a_tally_lists_only_what_happened_in_a_fixed_order() {
        let tally: Tally = [
            Change::Renamed,
            Change::Added,
            Change::Modified,
            Change::Modified,
        ]
        .into_iter()
        .collect();
        let present: Vec<_> = tally.present().collect();
        assert_eq!(
            present,
            [
                (Change::Modified, 2),
                (Change::Added, 1),
                (Change::Renamed, 1)
            ]
        );
        assert_eq!(tally.get(Change::Deleted), 0);
    }
}
