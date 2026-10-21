//! The glyphs this application ships, and the source that serves them.
//!
//! Each is taken unchanged from one of two published sets, through their
//! Iconify packages:
//!
//! * **Solar Icons**, Linear weight, by 480 Design, licensed under CC BY 4.0
//!   (<https://creativecommons.org/licenses/by/4.0/>); attribution: "Solar
//!   Icons by 480 Design". The `*-linear` icons, saved without the suffix.
//! * **Hugeicons** free icons (MIT): the git marks (`git-branch`,
//!   `git-commit`, `git-compare`), the `-01`/`-02` names, `layout-*`,
//!   `tick-02` (as `check`), `cancel-01` (as `close`), `plus-sign` and
//!   `minus-sign` (as `plus`/`minus`), `undo`, `redo`, `text-wrap`
//!   (as `wrap-text`), `arrow-expand-01` and `arrow-shrink-02` (as
//!   `expand-arrows`/`fold-vertical`), and the rest of the diff's and commit
//!   panel's controls. Its settings gear, `settings-01`, is that set's
//!   `configuration-01` with the wrench taken out.
//!
//! `github` is the GitHub mark from GPUI Kit's own icon bundle (Apache-2.0),
//! what a ref badge draws for a GitHub remote whose owner photo is not in yet.
//! The window buttons in `assets/icons/` are Rama's own.
//!
//! A glyph is referenced by constant, never by string: `icon(icons::TAG)`
//! cannot typo its way into an empty square, and the constant is what ties the
//! file on disk to the code that draws it.
//!
//! These are served by [`crate::assets`], ahead of the component library's own
//! bundle.

use std::borrow::Cow;

use gpui_kit::{Result, SharedString, Styled as _, Svg, svg};

/// `NAME = "file"` declares `NAME` as the glyph's asset path and embeds
/// `assets/icons/glyphs/file.svg` in the binary under that path.
macro_rules! glyphs {
    ($($name:ident = $file:literal,)+) => {
        $(pub const $name: &str = concat!("icons/glyphs/", $file, ".svg");)+

        /// Every glyph, by asset path.
        const EMBEDDED: &[(&str, &[u8])] = &[$(
            ($name, include_bytes!(concat!("../../assets/icons/glyphs/", $file, ".svg"))),
        )+];
    };
}

glyphs! {
    ALERT_02 = "alert-02",
    ALT_ARROW_DOWN = "alt-arrow-down",
    ALT_ARROW_RIGHT = "alt-arrow-right",
    ALT_ARROW_UP = "alt-arrow-up",
    ARCHIVE_04 = "archive-04",
    ARCHIVE_MINIMALISTIC = "archive-minimalistic",
    ARCHIVE_UP_MINIMALISTIC = "archive-up-minimalistic",
    ARROW_DOWN = "arrow-down",
    ARROW_DOWN_01 = "arrow-down-01",
    ARROW_DOWN_A_Z = "arrow-down-a-z",
    ARROW_RIGHT = "arrow-right",
    ARROW_UP = "arrow-up",
    ARROW_UP_01 = "arrow-up-01",
    CANCEL_01 = "cancel-01",
    CHAT_ROUND_LINE = "chat-round-line",
    CHECK = "check",
    CHECKLIST = "checklist",
    CLOCK_CIRCLE = "clock-circle",
    CLOSE = "close",
    CLOSE_CIRCLE = "close-circle",
    CLOUD = "cloud",
    COPY_01 = "copy-01",
    DANGER_TRIANGLE = "danger-triangle",
    DOCUMENT = "document",
    EXPAND_ARROWS = "expand-arrows",
    EYE = "eye",
    EYE_HUGE = "eye-huge",
    FILE_01 = "file-01",
    FLOPPY_DISK = "floppy-disk",
    FOLD_VERTICAL = "fold-vertical",
    FOLDER = "folder",
    FOLDER_01 = "folder-01",
    FOLDER_WITH_FILES = "folder-with-files",
    GIT_BRANCH = "git-branch",
    GIT_COMMIT = "git-commit",
    GIT_COMPARE = "git-compare",
    GITHUB = "github",
    HOME_01 = "home-01",
    INFO_CIRCLE = "info-circle",
    LAPTOP = "laptop",
    LAYOUT_ALIGN_LEFT = "layout-align-left",
    LAYOUT_TWO_COLUMN = "layout-two-column",
    LEFT_TO_RIGHT_LIST_BULLET = "left-to-right-list-bullet",
    LINK_SQUARE_02 = "link-square-02",
    LIST = "list",
    MAGNIFER = "magnifer",
    MINUS = "minus",
    PARAGRAPH = "paragraph",
    PEN = "pen",
    PEN_NEW_SQUARE = "pen-new-square",
    PENCIL_EDIT_01 = "pencil-edit-01",
    PLUS = "plus",
    REDO = "redo",
    REFRESH = "refresh",
    SETTINGS_01 = "settings-01",
    SIDEBAR_MINIMALISTIC_LEFT = "sidebar-minimalistic-left",
    SORT = "sort",
    SPLIT_COLUMNS = "split-columns",
    TAG = "tag",
    TEXT_ALIGN_JUSTIFY_LEFT = "text-align-justify-left",
    TRASH_BIN_MINIMALISTIC = "trash-bin-minimalistic",
    UNDO = "undo",
    WRAP_TEXT = "wrap-text",
}

/// One glyph by path, sized and tinted by the caller.
///
/// An `svg()` rather than a component icon on purpose: it is an
/// `InteractiveElement`, so a glyph can recolour with its container's hover.
pub fn icon(path: &'static str) -> Svg {
    svg().path(path).flex_none()
}

/// Serve one of the embedded glyphs, if the path names one.
pub fn load(path: &str) -> Result<Option<Cow<'static, [u8]>>> {
    Ok(EMBEDDED
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| Cow::Borrowed(*bytes)))
}

/// Every embedded glyph under `path`.
pub fn list(path: &str) -> Vec<SharedString> {
    EMBEDDED
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| name.starts_with(path))
        .map(SharedString::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_constant_resolves_to_an_embedded_file() {
        for (path, bytes) in EMBEDDED {
            assert!(!bytes.is_empty(), "{path} is empty");
            let svg = std::str::from_utf8(bytes).expect("UTF-8 svg");
            assert!(svg.contains("<svg"), "{path} is not an svg: {svg}");
            assert!(load(path).unwrap().is_some(), "{path} is not served");
        }
    }

    #[test]
    fn a_path_outside_the_set_is_not_served() {
        assert!(load("icons/glyphs/nothing-here.svg").unwrap().is_none());
        assert!(!list("icons/glyphs/").is_empty());
    }
}
