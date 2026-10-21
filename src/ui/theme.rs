//! Rama's colours, as named roles.
//!
//! Views ask for a role (`text_muted`, `surface_raised`) and never for a
//! colour; which colour fills each role, in the dark and in the light
//! appearance, is decided only here.
//!
//! The values are written in oklch, where equal steps of lightness look like
//! equal steps, and turned into GPUI's `Hsla` when a theme is built. The light
//! appearance is not the dark one inverted. In the dark, planes stack by
//! getting lighter; in the light the content is already white, so borders and
//! shadows separate the planes instead.
//!
//! GPUI Kit's components keep their own theme. [`Theme::of`] follows its
//! dark-or-light mode, so the two always agree.

use std::cell::Cell;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Global, Hsla, SharedString, hsla};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        self == Self::Dark
    }
}

/// The colour interactive things are drawn in. Every accent role comes from
/// the one picked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AccentColor {
    #[default]
    Indigo,
    Orange,
    Amber,
    Green,
    Cyan,
    Blue,
    Pink,
    /// Picked with the colour picker, kept as sRGB so it is saved exactly.
    Custom {
        red: u8,
        green: u8,
        blue: u8,
    },
}

/// A point of the Tailwind v4 palette, in oklch: lightness, chroma, hue.
type Step = (f32, f32, f32);

/// The three steps an accent family is drawn from: on the dark, the 400 as
/// the accent and the 600 under text; on white, the 700 for both, which
/// keeps light text on it readable.
struct Family {
    step_400: Step,
    step_600: Step,
    step_700: Step,
}

impl AccentColor {
    /// The ready-made families, in the order the settings show them.
    pub const ALL: [Self; 7] = [
        Self::Indigo,
        Self::Orange,
        Self::Amber,
        Self::Green,
        Self::Cyan,
        Self::Blue,
        Self::Pink,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Indigo => "Indigo",
            Self::Orange => "Orange",
            Self::Amber => "Amber",
            Self::Green => "Green",
            Self::Cyan => "Cyan",
            Self::Blue => "Blue",
            Self::Pink => "Pink",
            Self::Custom { .. } => "Custom",
        }
    }

    /// The colour of its swatch in the settings.
    pub fn swatch(self, appearance: Appearance) -> Hsla {
        self.pair(appearance).0
    }

    fn family(self) -> Option<Family> {
        let family = |step_400, step_600, step_700| Family {
            step_400,
            step_600,
            step_700,
        };
        Some(match self {
            Self::Indigo => family(
                (0.673, 0.182, 276.935),
                (0.511, 0.262, 276.966),
                (0.457, 0.240, 277.023),
            ),
            Self::Orange => family(
                (0.750, 0.183, 55.934),
                (0.646, 0.222, 41.116),
                (0.553, 0.195, 38.402),
            ),
            Self::Amber => family(
                (0.828, 0.189, 84.429),
                (0.666, 0.179, 58.318),
                (0.555, 0.163, 48.998),
            ),
            Self::Green => family(
                (0.792, 0.209, 151.711),
                (0.627, 0.194, 149.214),
                (0.527, 0.154, 150.069),
            ),
            Self::Cyan => family(
                (0.789, 0.154, 211.530),
                (0.609, 0.126, 221.723),
                (0.520, 0.105, 223.128),
            ),
            Self::Blue => family(
                (0.707, 0.165, 254.624),
                (0.546, 0.245, 262.881),
                (0.488, 0.243, 264.376),
            ),
            Self::Pink => family(
                (0.718, 0.202, 349.761),
                (0.592, 0.249, 0.584),
                (0.525, 0.223, 3.958),
            ),
            Self::Custom { .. } => return None,
        })
    }

    /// The accent and its stronger partner, for fills that carry text.
    fn pair(self, appearance: Appearance) -> (Hsla, Hsla) {
        let step = |(l, c, h): Step| oklch(l, c, h);
        if let Some(family) = self.family() {
            return match appearance {
                Appearance::Dark => (step(family.step_400), step(family.step_600)),
                Appearance::Light => (step(family.step_700), step(family.step_700)),
            };
        }
        let Self::Custom { red, green, blue } = self else {
            unreachable!("every preset has a family");
        };
        // A picked colour stands for the 400; the deeper steps darken it.
        let picked = from_rgb(red, green, blue);
        let deeper = |factor: f32, ceiling: f32| Hsla {
            l: (picked.l * factor).min(ceiling),
            ..picked
        };
        match appearance {
            Appearance::Dark => (picked, deeper(0.8, 0.85)),
            Appearance::Light => (deeper(0.7, 0.6), deeper(0.7, 0.6)),
        }
    }
}

/// The working indicator's three rows: the accent lifted, the accent, and
/// its strong partner (deepened on white), so the light going round reads as
/// one colour with depth.
fn indicator_tones(accent: Hsla, strong: Hsla, appearance: Appearance) -> [Hsla; 3] {
    let lifted = Hsla {
        l: (accent.l + 0.12).min(0.88),
        s: accent.s * 0.75,
        ..accent
    };
    let deep = match appearance {
        Appearance::Dark => strong,
        Appearance::Light => Hsla {
            l: (strong.l - 0.08).max(0.2),
            ..strong
        },
    };
    [lifted, accent, deep]
}

/// Every role, resolved for one appearance and accent.
#[derive(Debug, Clone)]
pub struct Theme {
    pub appearance: Appearance,

    /// The content plane: darkest in the dark, white in the light.
    pub bg: Hsla,
    /// Side panels and strips. It recedes from the content in both
    /// appearances: lighter than it in the dark, greyer in the light.
    pub surface: Hsla,
    /// Opaque chips and pills.
    pub surface_raised: Hsla,
    /// Dialogs.
    pub surface_dialog: Hsla,
    /// Menus and popovers, the top plane.
    pub surface_overlay: Hsla,
    /// Behind a row or button under the pointer.
    pub element_hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,

    pub text: Hsla,
    /// Secondary labels, times.
    pub text_muted: Hsla,
    /// Placeholders, disabled text.
    pub text_faint: Hsla,
    /// Text on a [`Self::text`]-coloured plate.
    pub on_solid: Hsla,

    pub accent: Hsla,
    /// For accent fills with [`Self::on_accent`] text on them.
    pub accent_strong: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    /// For error text in a line of copy.
    pub danger_muted: Hsla,
    /// The fill of a destructive button.
    pub danger_strong: Hsla,
    pub warning: Hsla,
    pub warning_muted: Hsla,
    pub success: Hsla,
    /// Something in progress.
    pub busy: Hsla,
    /// The working indicator's rows, lightest first.
    pub indicator: [Hsla; 3],

    pub input_bg: Hsla,
    /// The fixed code palette, shared by the diff and the editor.
    pub syntax: Syntax,
    pub diff_add: Hsla,
    pub diff_del: Hsla,
    /// Behind the band between hunks.
    pub diff_hunk_bg: Hsla,

    pub font_sans: SharedString,
    pub font_mono: SharedString,
}

/// The colours code is drawn in, one role per token kind.
///
/// Unlike every other role these do **not** follow the accent: the same file
/// reads the same whatever colour the chrome is. Only the appearance swaps the
/// table, so the text stays legible on the dark and on the light plane. The
/// values are the VS Code Dark+ and Light+ token colours, which is what
/// GitKraken ships inside its editor too.
#[derive(Debug, Clone)]
pub struct Syntax {
    /// Everything the tokenizer left alone.
    pub plain: Hsla,
    pub comment: Hsla,
    pub keyword: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    /// Type and class names.
    pub type_name: Hsla,
    /// Calls and definitions.
    pub function: Hsla,
    pub punctuation: Hsla,
}

impl Syntax {
    fn of(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Dark => Self {
                plain: hex(0xD4D4D4),
                comment: hex(0x6A9955),
                keyword: hex(0x569CD6),
                string: hex(0xCE9178),
                number: hex(0xB5CEA8),
                type_name: hex(0x4EC9B0),
                function: hex(0xDCDCAA),
                punctuation: hex(0xD4D4D4),
            },
            Appearance::Light => Self {
                plain: hex(0x000000),
                comment: hex(0x008000),
                keyword: hex(0x0000FF),
                string: hex(0xA31515),
                number: hex(0x098658),
                type_name: hex(0x267F99),
                function: hex(0x795E26),
                punctuation: hex(0x000000),
            },
        }
    }
}

/// The greys of one appearance, before they become a [`Theme`].
struct Planes {
    bg: Hsla,
    surface: Hsla,
    raised: Hsla,
    dialog: Hsla,
    overlay: Hsla,
    hover: Hsla,
    border: Hsla,
    border_strong: Hsla,
    input: Hsla,
    text: Hsla,
    text_muted: Hsla,
    text_faint: Hsla,
    on_solid: Hsla,
}

/// The status colours of one appearance.
struct Status {
    success: Hsla,
    danger: Hsla,
    danger_muted: Hsla,
    danger_strong: Hsla,
    warning: Hsla,
    warning_muted: Hsla,
    hunk: Hsla,
}

impl Theme {
    pub fn dark() -> Self {
        Self::build(Appearance::Dark, AccentColor::default())
    }

    pub fn light() -> Self {
        Self::build(Appearance::Light, AccentColor::default())
    }

    /// Greys follow Tailwind v4's neutral scale and the status colours its
    /// green, rose and yellow families, at the steps that read on each
    /// background. See-through roles come from [`ink_in`] and [`wash_in`],
    /// the same as the free helpers below.
    fn build(appearance: Appearance, accent_color: AccentColor) -> Self {
        let (accent, strong) = accent_color.pair(appearance);
        let indicator = indicator_tones(accent, strong, appearance);
        let ink = |alpha| ink_in(appearance, alpha);
        let wash = |alpha| wash_in(appearance, alpha);
        let common = |planes: Planes, status: Status| Self {
            appearance,
            bg: planes.bg,
            surface: planes.surface,
            surface_raised: planes.raised,
            surface_dialog: planes.dialog,
            surface_overlay: planes.overlay,
            element_hover: planes.hover,
            border: planes.border,
            border_strong: planes.border_strong,
            text: planes.text,
            text_muted: planes.text_muted,
            text_faint: planes.text_faint,
            on_solid: planes.on_solid,
            accent,
            accent_strong: strong,
            on_accent: neutral(0.985),
            danger: status.danger,
            danger_muted: status.danger_muted,
            danger_strong: status.danger_strong,
            warning: status.warning,
            warning_muted: status.warning_muted,
            success: status.success,
            busy: accent,
            indicator,
            input_bg: planes.input,
            syntax: Syntax::of(appearance),
            diff_add: status.success,
            diff_del: status.danger,
            diff_hunk_bg: status.hunk,
            font_sans: system_font(false).into(),
            font_mono: system_font(true).into(),
        };
        match appearance {
            // In the dark, each plane up the stack is a step lighter.
            Appearance::Dark => common(
                Planes {
                    bg: neutral(0.13),
                    surface: neutral(0.165),
                    raised: neutral(0.25),
                    dialog: neutral(0.19),
                    overlay: neutral(0.225),
                    hover: wash(0.10),
                    border: ink(0.08),
                    border_strong: ink(0.15),
                    input: ink(0.03),
                    text: neutral(0.93),
                    text_muted: neutral(0.72),
                    text_faint: neutral(0.57),
                    on_solid: neutral(0.16),
                },
                Status {
                    success: oklch(0.792, 0.209, 151.711),
                    danger: oklch(0.712, 0.194, 13.428),
                    danger_muted: oklch(0.810, 0.117, 11.638),
                    danger_strong: oklch(0.586, 0.253, 17.585),
                    warning: oklch(0.852, 0.199, 91.936),
                    warning_muted: oklch(0.945, 0.129, 101.54),
                    hunk: oklch(0.707, 0.165, 254.624).opacity(0.06),
                },
            ),
            // On white the planes are told apart by grey steps and lines.
            Appearance::Light => common(
                Planes {
                    bg: neutral(1.0),
                    // Any closer to white and the window reads as one sheet.
                    surface: neutral(0.965),
                    // Chips sit on white with no border: a real grey.
                    raised: neutral(0.935),
                    dialog: neutral(1.0),
                    overlay: neutral(1.0),
                    hover: wash(0.06),
                    border: ink(0.10),
                    border_strong: ink(0.18),
                    input: neutral(1.0),
                    // Not black, which is harsher on white than the dark
                    // appearance's text is on its background.
                    text: neutral(0.27),
                    text_muted: neutral(0.45),
                    // Dark enough for the grey chrome too.
                    text_faint: neutral(0.54),
                    on_solid: neutral(0.985),
                },
                Status {
                    success: oklch(0.527, 0.154, 150.069),
                    danger: oklch(0.586, 0.253, 17.585),
                    danger_muted: oklch(0.514, 0.222, 16.935),
                    danger_strong: oklch(0.514, 0.222, 16.935),
                    warning: oklch(0.554, 0.135, 66.442),
                    warning_muted: oklch(0.476, 0.114, 61.907),
                    hunk: oklch(0.546, 0.245, 262.881).opacity(0.07),
                },
            ),
        }
    }

    /// The theme for the appearance the window is in now.
    pub fn of(cx: &App) -> &Theme {
        let dark = cx.theme().is_dark();
        // The free helpers below have no `cx`; keep them on the same side.
        SEEN.set(if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        });
        let themes = cx.global::<Themes>();
        if dark { &themes.dark } else { &themes.light }
    }
}

/// Both appearances, built once.
struct Themes {
    dark: Theme,
    light: Theme,
}

impl Global for Themes {}

pub fn init(cx: &mut App) {
    set_accent(AccentColor::default(), cx);
}

pub fn set_accent(accent: AccentColor, cx: &mut App) {
    cx.set_global(Themes {
        dark: Theme::build(Appearance::Dark, accent),
        light: Theme::build(Appearance::Light, accent),
    });
}

/// Apply the appearance and accent from the settings, to Rama's theme and to
/// GPUI Kit's at once: moving only one leaves half the window behind.
pub fn apply_settings(cx: &mut App) {
    use gpui_kit::component::ThemeMode;
    let settings = crate::settings::current(cx);
    set_accent(
        crate::features::settings_page::accent_color(settings.look.accent),
        cx,
    );
    let mode = match settings.look.mode {
        crate::settings::Mode::Dark => ThemeMode::Dark,
        crate::settings::Mode::Light => ThemeMode::Light,
    };
    gpui_kit::component::Theme::change(mode, None, cx);
    let themes = cx.global::<Themes>();
    let current = if mode.is_dark() {
        &themes.dark
    } else {
        &themes.light
    }
    .clone();
    paint_editor(&current, cx);
}

/// Give GPUI Kit's code editor Rama's colours. Its own (a near-black plane,
/// a bright current-line bar) look foreign inside Rama's panels, the merge
/// editor's result most of all.
fn paint_editor(theme: &Theme, cx: &mut App) {
    let kit = gpui_kit::component::Theme::global_mut(cx);
    let mut colors = (*kit.highlight_theme).clone();
    let style = &mut colors.style;
    style.editor_background = Some(theme.bg);
    style.editor_gutter_background = Some(theme.bg);
    style.editor_active_line = Some(wash_in(theme.appearance, 0.05));
    style.editor_foreground = Some(theme.text);
    style.editor_line_number = Some(theme.text_faint);
    kit.highlight_theme = Arc::new(colors);
}

// ── See-through colours without a theme at hand ─────────────────────────────
//
// Much of the chrome is built in free functions with no `cx`. These follow
// the appearance [`Theme::of`] last saw on this thread, which is the UI
// thread in the app and each test's own thread under `cargo test`. Their
// alpha is always given as meant in the dark; the light appearance adjusts.

thread_local! {
    static SEEN: Cell<Appearance> = const { Cell::new(Appearance::Dark) };
}

fn appearance() -> Appearance {
    SEEN.get()
}

/// Paint: white on the dark, black on the light.
fn ink_in(appearance: Appearance, alpha: f32) -> Hsla {
    match appearance {
        Appearance::Dark => hsla(0.0, 0.0, 1.0, alpha),
        Appearance::Light => hsla(0.0, 0.0, 0.0, alpha),
    }
}

/// A tint: a grey just short of white or black, so it reads as a tint and
/// not as paint.
fn wash_in(appearance: Appearance, alpha: f32) -> Hsla {
    match appearance {
        Appearance::Dark => hsla(0.0, 0.0, 0.93, alpha),
        Appearance::Light => hsla(0.0, 0.0, 0.08, alpha),
    }
}

/// A fill: white over the dark, black over the light.
pub fn ink(alpha: f32) -> Hsla {
    ink_in(appearance(), alpha)
}

/// A line. On a light field a hairline needs a third more ink to show.
pub fn hairline(alpha: f32) -> Hsla {
    match appearance() {
        Appearance::Dark => ink_in(Appearance::Dark, alpha),
        Appearance::Light => ink_in(Appearance::Light, (alpha * 1.35).min(0.5)),
    }
}

/// A hover or pressed tint: softer than [`ink`], so it reads as a tint and
/// not as paint.
pub fn wash(alpha: f32) -> Hsla {
    wash_in(appearance(), alpha)
}

/// The dimming behind a dialog. Black in both appearances, but about half as
/// strong in the light, where the dark's strength would black the window out.
pub fn scrim(alpha_dark: f32) -> Hsla {
    let alpha = match appearance() {
        Appearance::Dark => alpha_dark,
        Appearance::Light => alpha_dark * 0.55,
    };
    hsla(0.0, 0.0, 0.0, alpha)
}

// ── Colour maths ────────────────────────────────────────────────────────────

/// A grey of the given oklch lightness. Made directly as a grey, so rounding
/// in the conversion cannot add a trace of colour.
fn neutral(lightness: f32) -> Hsla {
    hsla(0.0, 0.0, oklab_to_srgb(lightness, 0.0, 0.0)[0], 1.0)
}

fn from_rgb(red: u8, green: u8, blue: u8) -> Hsla {
    let channel = |c: u8| f32::from(c) / 255.0;
    let (h, s, l) = rgb_to_hsl(channel(red), channel(green), channel(blue));
    hsla(h, s, l, 1.0)
}

/// A fixed colour written the way the palette it comes from is written, `0xRRGGBB`.
fn hex(value: u32) -> Hsla {
    from_rgb(
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    )
}

/// A colour from oklch: lightness 0–1, chroma, hue in degrees.
fn oklch(lightness: f32, chroma: f32, hue: f32) -> Hsla {
    let (sin, cos) = hue.to_radians().sin_cos();
    let [r, g, b] = oklab_to_srgb(lightness, chroma * cos, chroma * sin);
    let (h, s, l) = rgb_to_hsl(r, g, b);
    hsla(h, s, l, 1.0)
}

/// Oklab to gamma-encoded sRGB, each channel clipped to 0–1. The matrices
/// are the ones Oklab is defined by.
fn oklab_to_srgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let cube = |x: f32| x * x * x;
    let lms = [
        cube(l + 0.396_337_78 * a + 0.215_803_76 * b),
        cube(l - 0.105_561_346 * a - 0.063_854_17 * b),
        cube(l - 0.089_484_18 * a - 1.291_485_5 * b),
    ];
    const TO_LINEAR_SRGB: [[f32; 3]; 3] = [
        [4.076_741_7, -3.307_711_6, 0.230_969_93],
        [-1.268_438, 2.609_757_4, -0.341_319_4],
        [-0.004_196_086_3, -0.703_418_6, 1.707_614_7],
    ];
    TO_LINEAR_SRGB.map(|row| {
        let linear = (row[0] * lms[0] + row[1] * lms[1] + row[2] * lms[2]).clamp(0.0, 1.0);
        match linear <= 0.003_130_8 {
            true => linear * 12.92,
            false => 1.055 * linear.powf(1.0 / 2.4) - 0.055,
        }
    })
}

/// sRGB channels (0–1) to hue, saturation and lightness, each 0–1 as GPUI
/// takes them.
pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let high = r.max(g).max(b);
    let low = r.min(g).min(b);
    let lightness = (high + low) / 2.0;
    let spread = high - low;
    if spread < f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = spread / (1.0 - (2.0 * lightness - 1.0).abs());
    let sector = if high == r {
        ((g - b) / spread).rem_euclid(6.0)
    } else if high == g {
        (b - r) / spread + 2.0
    } else {
        (r - g) / spread + 4.0
    };
    (sector / 6.0, saturation, lightness)
}

/// The platform's interface or monospace font.
fn system_font(mono: bool) -> &'static str {
    match (cfg!(target_os = "macos"), cfg!(target_os = "windows"), mono) {
        (true, _, false) => "Helvetica",
        (true, _, true) => "Menlo",
        (_, true, false) => "Segoe UI",
        (_, true, true) => "Consolas",
        (_, _, false) => "DejaVu Sans",
        (_, _, true) => "DejaVu Sans Mono",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast between two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |color: Hsla| {
            let rgb = gpui_kit::Rgba::from(color);
            let linear = |c: f32| match c <= 0.040_45 {
                true => c / 12.92,
                false => ((c + 0.055) / 1.055).powf(2.4),
            };
            0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    fn both() -> [Theme; 2] {
        [Theme::dark(), Theme::light()]
    }

    #[test]
    fn body_and_muted_text_meet_wcag_on_the_content_plane() {
        for theme in both() {
            let mode = theme.appearance;
            assert!(contrast(theme.text, theme.bg) >= 7.0, "{mode:?}");
            assert!(contrast(theme.text_muted, theme.bg) >= 4.5, "{mode:?}");
            assert!(
                contrast(theme.on_accent, theme.accent_strong) >= 3.0,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn side_panels_step_away_from_the_content() {
        let [dark, light] = both();
        assert!(dark.surface.l > dark.bg.l, "lighter in the dark");
        assert!(light.surface.l < light.bg.l, "greyer in the light");
    }

    #[test]
    fn presets_and_a_picked_colour_build_a_coloured_accent() {
        let picked = AccentColor::Custom {
            red: 200,
            green: 40,
            blue: 90,
        };
        let accents = AccentColor::ALL.into_iter().chain([picked]);
        for (accent, appearance) in
            accents.flat_map(|accent| [(accent, Appearance::Dark), (accent, Appearance::Light)])
        {
            let theme = Theme::build(appearance, accent);
            assert!(theme.accent.s > 0.0, "{accent:?} {appearance:?}");
            assert_eq!(theme.busy, theme.accent);
            assert_eq!(theme.indicator[1], theme.accent);
        }
    }

    #[test]
    fn the_free_helpers_follow_the_last_appearance_seen() {
        SEEN.set(Appearance::Light);
        assert_eq!(ink(0.2), hsla(0.0, 0.0, 0.0, 0.2));
        SEEN.set(Appearance::Dark);
        assert_eq!(ink(0.2), hsla(0.0, 0.0, 1.0, 0.2));
    }

    #[test]
    fn a_neutral_is_grey_and_oklch_lands_where_expected() {
        let mid = neutral(0.5);
        assert_eq!(mid.s, 0.0);
        assert!((0.35..0.45).contains(&mid.l), "{mid:?}");
        assert_eq!(
            oklab_to_srgb(1.0, 0.0, 0.0).map(|c| (c * 1000.0).round()),
            [1000.0; 3]
        );
        let red = oklch(0.628, 0.258, 29.2);
        assert!(red.h < 0.02 || red.h > 0.98, "{red:?} should be red");
    }

    #[test]
    fn the_syntax_palette_holds_the_code_colours() {
        let dark = Theme::dark().syntax;
        assert_eq!(dark.plain, hex(0xD4D4D4));
        assert_eq!(dark.comment, hex(0x6A9955));
        assert_eq!(dark.keyword, hex(0x569CD6));
        assert_eq!(dark.string, hex(0xCE9178));
        assert_eq!(dark.number, hex(0xB5CEA8));
        assert_eq!(dark.type_name, hex(0x4EC9B0));
        let light = Theme::light().syntax;
        assert_eq!(light.plain, hex(0x000000));
        assert_eq!(light.keyword, hex(0x0000FF));
        assert_eq!(light.string, hex(0xA31515));
    }

    #[test]
    fn rgb_to_hsl_finds_the_primaries() {
        assert_eq!(rgb_to_hsl(1.0, 0.0, 0.0), (0.0, 1.0, 0.5));
        let (h, s, l) = rgb_to_hsl(0.0, 0.0, 1.0);
        assert!((h - 2.0 / 3.0).abs() < 1e-6 && s == 1.0 && l == 0.5);
        assert_eq!(rgb_to_hsl(0.3, 0.3, 0.3), (0.0, 0.0, 0.3));
    }

    #[gpui_kit::test]
    fn the_editor_takes_ramas_colours(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            let theme = Theme::dark();
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            paint_editor(&theme, cx);
            let style = &gpui_kit::component::Theme::global(cx).highlight_theme.style;
            assert_eq!(style.editor_background, Some(theme.bg));
            assert_eq!(style.editor_foreground, Some(theme.text));
            assert_eq!(style.editor_line_number, Some(theme.text_faint));
        });
    }
}
