//! The application's asset source.
//!
//! GPUI Kit's bundled Lucide icons are the fallback; anything this crate ships
//! in `assets/` under the *same path* wins. That is how the window chrome gets
//! its own glyphs without a second icon enum: `IconName::Close` still resolves
//! to `icons/close.svg`, and this source decides which file that is.
//!
//! The caption glyphs are overridden so the window buttons match the rest of
//! the chrome: minimize, maximize and close are 24px drawings shown at 14px
//! (Hugeicons' `square` for maximize, MIT), and restore is Rama's own 16px
//! two-window mark.
//!
//! The product's own glyph set lives in [`crate::ui::icons`] and is served
//! here too, ahead of the bundle.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

/// Embedded at compile time: four files, and the binary owns its chrome.
const OVERRIDES: &[(&str, &[u8])] = &[
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
    (
        "icons/window-minimize.svg",
        include_bytes!("../assets/icons/window-minimize.svg"),
    ),
    (
        "icons/window-maximize.svg",
        include_bytes!("../assets/icons/window-maximize.svg"),
    ),
    (
        "icons/window-restore.svg",
        include_bytes!("../assets/icons/window-restore.svg"),
    ),
];

/// The app icon, as drawn on the About page: the 256px file, so it stays
/// sharp at its size on a 2× display.
pub const APP_ICON: &str = "app-icon/256x256.png";
const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/app-icon/256x256.png");

/// The app icon's mark alone, without its tile, for the About page: light
/// for the dark appearance and dark for the light one, both 256px.
pub const MARK_LIGHT: &str = "app-icon/mark-light.png";
pub const MARK_DARK: &str = "app-icon/mark-dark.png";
const MARK_LIGHT_BYTES: &[u8] = include_bytes!("../assets/app-icon/mark-light.png");
const MARK_DARK_BYTES: &[u8] = include_bytes!("../assets/app-icon/mark-dark.png");

/// This application's assets, with GPUI Kit's bundled icons behind them.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OVERRIDES.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(APP_ICON_BYTES)));
        }
        if path == MARK_LIGHT {
            return Ok(Some(Cow::Borrowed(MARK_LIGHT_BYTES)));
        }
        if path == MARK_DARK {
            return Ok(Some(Cow::Borrowed(MARK_DARK_BYTES)));
        }
        if let Some(bytes) = crate::ui::icons::load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        // The overrides replace files the bundle already lists, so its listing
        // covers them; the product's own glyphs are additional.
        let mut all = gpui_kit::assets::Assets.list(path)?;
        all.extend(crate::ui::icons::list(path));
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The override only works because the paths collide with the bundle's.
    /// If GPUI Kit ever renames one, the caption silently falls back to a
    /// Lucide glyph at twice the visual weight — which is exactly the bug
    /// these overrides exist to fix.
    #[test]
    fn every_override_shadows_a_path_the_bundle_also_has() {
        for (path, _) in OVERRIDES {
            assert!(
                gpui_kit::assets::Assets.load(path).unwrap().is_some(),
                "{path} is not in the bundle any more, so overriding it does nothing"
            );
        }
    }

    #[test]
    fn the_caption_glyphs_are_the_applications_own() {
        for (path, bytes) in OVERRIDES {
            let served = Assets.load(path).unwrap().expect("override is served");
            assert_eq!(served.as_ref(), *bytes, "{path} comes from the bundle");
            let svg = std::str::from_utf8(bytes).expect("UTF-8 svg");
            assert!(svg.contains("stroke-width"), "{path} is not a line glyph");
        }
    }

    /// The icon names the title bar asks for must resolve to the overridden
    /// paths, or the override never runs.
    #[test]
    fn the_icon_names_the_title_bar_uses_resolve_to_the_overrides() {
        use gpui_kit::assets::IconName;

        for name in [
            IconName::Close,
            IconName::WindowMinimize,
            IconName::WindowMaximize,
            IconName::WindowRestore,
        ] {
            let path = name.path();
            assert!(
                OVERRIDES.iter().any(|(name, _)| *name == path.as_ref()),
                "{path} is not overridden, so this glyph comes from the bundle"
            );
        }
    }
}
