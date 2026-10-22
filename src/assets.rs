//! The application's asset source.
//!
//! GPUI Kit's bundled Lucide icons are the fallback; anything this crate ships
//! in `assets/` under the *same path* wins. That is how the window chrome gets
//! its own glyphs without a second icon enum: `IconName::Close` still resolves
//! to `icons/close.svg`, and this source decides which file that is.
//!
//! The caption glyphs are overridden because weight is the whole point of
//! window chrome. Lucide draws a 24×24 box with the mark filling two thirds of
//! it; a caption button wants a mark that occupies about 40% of its box, drawn
//! in a hairline, so it reads as system chrome rather than as a toolbar icon.
//! They are drawn at a 16×16 viewBox with a 6.5px mark and a 1.25 stroke.
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

/// The app icon, as drawn on the About page.
pub const APP_ICON: &str = "app-icon/128x128.png";
const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/app-icon/128x128.png");

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
        for (path, _) in OVERRIDES {
            let bytes = Assets.load(path).unwrap().expect("override is served");
            let svg = std::str::from_utf8(&bytes).expect("UTF-8 svg");
            // A small mark in a 16px box at a hairline stroke, the size of
            // the platform's own caption buttons; the bundle's are 24px
            // boxes filled two thirds of the way.
            assert!(
                svg.contains(r#"viewBox="0 0 16 16""#),
                "{path} is not the 16px chrome glyph: {svg}"
            );
            let stroke: f32 = svg
                .split(r#"stroke-width=""#)
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .and_then(|width| width.parse().ok())
                .unwrap_or_else(|| panic!("{path} has no stroke width: {svg}"));
            assert!(
                (1.0..=1.5).contains(&stroke),
                "{path} is not a hairline: {svg}"
            );
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
