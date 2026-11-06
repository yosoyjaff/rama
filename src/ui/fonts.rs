//! The interface typefaces, shipped inside the binary.
//!
//! Rama draws its text in Geist and its code in Geist Mono (Vercel, SIL Open
//! Font License 1.1, `assets/fonts/OFL.txt`) on every platform, instead of
//! whatever the system has: DejaVu Sans on most Linux desktops, which reads
//! wide and heavy at interface sizes, Helvetica on macOS, Segoe UI on Windows.
//! One face everywhere is also one set of metrics, so a layout measured on one
//! system holds on the others.
//!
//! The four weights the views ask for are shipped as static files; a weight
//! without its own file would be synthesised, and a faux bold is visibly
//! smeared.

use std::borrow::Cow;

use gpui_kit::App;

/// The interface family.
pub const SANS: &str = "Geist";
/// The code family: the diff, the editor, SHAs.
pub const MONO: &str = "Geist Mono";

const FILES: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/GeistMono-Medium.ttf"),
    include_bytes!("../../assets/fonts/GeistMono-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/GeistMono-Bold.ttf"),
];

/// Register the faces and make them GPUI Kit's defaults, so its components
/// (inputs, menus, the code editor) draw in them too. After `gpui_kit::init`;
/// [`super::theme::init`] calls it.
pub fn init(cx: &mut App) {
    let files = FILES.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(error) = cx.text_system().add_fonts(files) {
        // Not fatal: GPUI falls back to the system's faces.
        eprintln!("rama: could not load the bundled fonts: {error:#}");
        return;
    }
    gpui_kit::component::Theme::update(cx, |theme| {
        theme.font_family = SANS.into();
        theme.mono_font_family = MONO.into();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn the_bundled_families_resolve_to_themselves(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let text = cx.text_system();
            for family in [SANS, MONO] {
                let id = text.resolve_font(&gpui_kit::font(family));
                let resolved = text.get_font_for_id(id).expect("font is loaded").family;
                assert_eq!(resolved.as_ref(), family);
            }
        });
    }

    /// Changing the light or dark appearance reloads GPUI Kit's theme file;
    /// the faces must survive it.
    #[gpui_kit::test]
    fn the_families_survive_a_theme_change(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            for mode in [
                gpui_kit::component::ThemeMode::Light,
                gpui_kit::component::ThemeMode::Dark,
            ] {
                gpui_kit::component::Theme::change(mode, None, cx);
                let theme = gpui_kit::component::Theme::global(cx);
                assert_eq!(theme.font_family.as_ref(), SANS);
                assert_eq!(theme.mono_font_family.as_ref(), MONO);
            }
        });
    }
}
