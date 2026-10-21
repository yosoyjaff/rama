//! Small pieces the history's rows are made of: how an author, a date and a
//! lane's colour are shown.

use chrono::DateTime;
use gpui_kit::{App, Hsla, ImageFormat, SharedString};

use super::detail::{Change, Tally};
use crate::git::GitStatusEntry;
use crate::settings;
use crate::ui::lane_palette::LanePalette;
use crate::ui::theme::Theme;

/// A commit's date as the date column shows it: `Mar 4, 2026`.
pub(super) fn format_date(value: &str) -> String {
    match DateTime::parse_from_rfc3339(value) {
        Ok(date) => date.format("%b %-d, %Y").to_string(),
        Err(_) => "—".to_string(),
    }
}

/// The graph's colours: the palette picked in the settings, for the current
/// appearance. Lane `n` always takes colour `n`, wrapping round, so loading
/// more history never recolours what is on screen.
pub(super) fn lane_palette(theme: &Theme, cx: &App) -> Vec<Hsla> {
    LanePalette::from(settings::read(cx, |s| s.look.palette))
        .colors(theme.appearance)
        .iter()
        .map(|&rgb| gpui_kit::rgb(rgb).into())
        .collect()
}

pub(super) fn lane_color(lane: usize, theme: &Theme, cx: &App) -> Hsla {
    let palette = lane_palette(theme, cx);
    palette[lane % palette.len()]
}

/// The name to show for an author.
pub fn history_author_name(name: &str) -> SharedString {
    match name.trim() {
        "" => "Unknown".into(),
        _ => SharedString::from(name.to_string()),
    }
}

/// What stands in for an author's photo.
pub(super) fn history_author_initial(name: &str) -> SharedString {
    crate::ui::avatar::author_initials(name).into()
}

/// An author's colour, from their email (or name).
pub fn history_author_tint(key: &str) -> u32 {
    crate::ui::avatar::author_tint_rgb(key)
}

/// The format of image bytes, told from how they start. A cached avatar's
/// file name does not say, and anything not recognised (an error page, a
/// cut-off download) is `None`, so the initial is shown instead.
pub(super) fn sniff_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    let webp = bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP";
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageFormat::Gif),
        _ if webp => Some(ImageFormat::Webp),
        _ => None,
    }
}

/// How many files of the working tree are added, changed, deleted and
/// renamed. An untracked file counts as added; otherwise the working tree's
/// letter decides, or the index's when the working tree has none.
pub(super) fn wip_status_counts(entries: &[GitStatusEntry]) -> Tally {
    let letter = |side: &str| side.trim().chars().next().filter(|&c| c != '.');
    entries
        .iter()
        .map(|entry| match entry.untracked {
            true => Change::Added,
            false => {
                let found = letter(&entry.worktree)
                    .or_else(|| letter(&entry.index))
                    .unwrap_or('M');
                Change::of(&found.to_string())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unnamed_author_is_unknown() {
        assert_eq!(history_author_name("  "), SharedString::from("Unknown"));
        assert_eq!(history_author_name("Ada"), SharedString::from("Ada"));
    }

    #[test]
    fn images_are_known_by_their_first_bytes() {
        assert_eq!(
            sniff_image_format(b"\x89PNG\r\n\x1a\n"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            sniff_image_format(&[0xFF, 0xD8, 0xFF, 0xDB]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(sniff_image_format(b"GIF87a"), Some(ImageFormat::Gif));
        assert_eq!(
            sniff_image_format(b"RIFF\0\0\0\0WEBPVP8L"),
            Some(ImageFormat::Webp)
        );
        assert_eq!(sniff_image_format(b"<!doctype html>"), None);
        assert_eq!(sniff_image_format(b""), None);
    }

    #[test]
    fn dates_read_as_month_day_year() {
        assert_eq!(format_date("2026-03-04T10:00:00+01:00"), "Mar 4, 2026");
        assert_eq!(format_date("yesterday"), "—");
    }

    #[test]
    fn the_working_tree_counts_each_file_once() {
        let entry = |index: &str, worktree: &str, untracked: bool| GitStatusEntry {
            index: index.into(),
            worktree: worktree.into(),
            untracked,
            ..GitStatusEntry::default()
        };
        let counts = wip_status_counts(&[
            entry("?", "?", true),
            entry(".", "M", false),
            entry("D", ".", false),
            entry("R", ".", false),
            entry("A", "M", false),
        ]);
        assert_eq!(Change::ORDER.map(|change| counts.get(change)), [2, 1, 1, 1]);
    }
}
