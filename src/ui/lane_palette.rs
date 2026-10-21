//! The graph's lane colours: which palette paints the branches.
//!
//! A lane's colour is its identity — "the green branch" — so a palette is a
//! fixed ordered list, and lane N always takes entry N modulo its length. A
//! palette may carry a separate light-appearance list where the source it
//! follows has one; otherwise one list serves both.
//!
//! The palettes named after another client are *inspired by* it: colours
//! measured from its published sources or screenshots, under a name of
//! Rama's own. Rama has no affiliation with any of those projects.

use crate::settings::Palette;
use crate::ui::theme::Appearance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanePalette {
    /// Rama's own: the app's accent families, completed with GitHub Primer's
    /// purple, red and lime.
    Rama,
    /// The ramp this graph shipped with first.
    Kraken,
    /// Tower: macOS system colours. Blue, pink and green measured from its
    /// screenshots; the rest of the same system family.
    Watchtower,
    /// GitHub: Primer's display foreground colours.
    Octo,
    /// JetBrains IDEs and Rebased: the IntelliJ log graph's generated
    /// colours under the Islands theme (saturation 0.6, brightness 0.6 dark /
    /// 0.7 light).
    Islands,
    /// Fork: the lane colours visible in its published screenshots.
    Trident,
    /// VS Code Git Graph: the extension's default `git-graph.graph.colours`.
    Blueprint,
}

impl LanePalette {
    pub const ALL: [Self; 7] = [
        Self::Rama,
        Self::Kraken,
        Self::Watchtower,
        Self::Octo,
        Self::Islands,
        Self::Trident,
        Self::Blueprint,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rama => "Rama",
            Self::Kraken => "The Kraken",
            Self::Watchtower => "Watchtower",
            Self::Octo => "Octo",
            Self::Islands => "Islands",
            Self::Trident => "Trident",
            Self::Blueprint => "Blueprint",
        }
    }

    /// What the palette evokes, for its tooltip. `None` for Rama's own.
    pub fn inspired_by(self) -> Option<&'static str> {
        match self {
            Self::Rama => None,
            Self::Kraken => Some("Inspired by GitKraken"),
            Self::Watchtower => Some("Inspired by Tower"),
            Self::Octo => Some("Inspired by GitHub"),
            Self::Islands => Some("Inspired by JetBrains IDEs and Rebased"),
            Self::Trident => Some("Inspired by Fork"),
            Self::Blueprint => Some("Inspired by Git Graph for VS Code"),
        }
    }

    /// The lane colours, as `0xRRGGBB`, for this appearance.
    pub fn colors(self, appearance: Appearance) -> &'static [u32] {
        let light = appearance == Appearance::Light;
        match self {
            Self::Rama if light => &RAMA_LIGHT,
            Self::Rama => &RAMA_DARK,
            Self::Kraken => &KRAKEN,
            Self::Watchtower => &WATCHTOWER,
            Self::Octo if light => &OCTO_LIGHT,
            Self::Octo => &OCTO_DARK,
            Self::Islands if light => &ISLANDS_LIGHT,
            Self::Islands => &ISLANDS_DARK,
            Self::Trident => &TRIDENT,
            Self::Blueprint => &BLUEPRINT,
        }
    }
}

impl From<Palette> for LanePalette {
    fn from(choice: Palette) -> Self {
        match choice {
            Palette::Rama => Self::Rama,
            Palette::Kraken => Self::Kraken,
            Palette::Watchtower => Self::Watchtower,
            Palette::Octo => Self::Octo,
            Palette::Islands => Self::Islands,
            Palette::Trident => Self::Trident,
            Palette::Blueprint => Self::Blueprint,
        }
    }
}

impl From<LanePalette> for Palette {
    fn from(palette: LanePalette) -> Self {
        match palette {
            LanePalette::Rama => Self::Rama,
            LanePalette::Kraken => Self::Kraken,
            LanePalette::Watchtower => Self::Watchtower,
            LanePalette::Octo => Self::Octo,
            LanePalette::Islands => Self::Islands,
            LanePalette::Trident => Self::Trident,
            LanePalette::Blueprint => Self::Blueprint,
        }
    }
}

/// The accent families (`ui::theme::AccentColor`, dark primaries) in hue
/// order, then Primer's purple, red and lime.
const RAMA_DARK: [u32; 10] = [
    0x7c86ff, 0x00c8d8, 0xf46eb4, 0xff8918, 0x4bcb71, 0xf1b200, 0x4c9fff, 0xb687f7, 0xf27d83,
    0x7dae37,
];
/// The same families' light-appearance primaries, and Primer's light set.
const RAMA_LIGHT: [u32; 10] = [
    0x4f39f6, 0x006572, 0xa7016d, 0xae3400, 0x006c28, 0x825300, 0x0052cc, 0x783ae4, 0xc50d28,
    0x476c28,
];

/// A saturated ramp, the same on both backgrounds.
const KRAKEN: [u32; 10] = [
    0x15a0bf, // teal
    0x0669f7, // blue
    0x8e00c2, // violet
    0xc517b6, // magenta
    0xd90171, // pink
    0xcd0101, // red
    0xf25d2e, // orange
    0xf2ca33, // yellow
    0x7bd938, // green
    0x2ece9d, // mint
];

/// Measured: blue, pink, green. Then macOS system orange, purple, cyan,
/// yellow, indigo, red and mint.
const WATCHTOWER: [u32; 10] = [
    0x157efb, 0xfc3159, 0x33ca4a, 0xff9500, 0xaf52de, 0x32ade6, 0xffcc00, 0x5856d6, 0xff3b30,
    0x00c7be,
];

/// `@primer/primitives` `--display-*-fgColor`, dark theme.
const OCTO_DARK: [u32; 9] = [
    0x4da0ff, 0x41b445, 0xb687f7, 0xed8326, 0xe57bb2, 0x1cb0ab, 0xd3910d, 0xf27d83, 0x7dae37,
];
/// The same tokens, light theme.
const OCTO_LIGHT: [u32; 9] = [
    0x005fcc, 0x2b6e3f, 0x783ae4, 0xa24610, 0xb12f79, 0x106e75, 0x805900, 0xc50d28, 0x476c28,
];

/// IntelliJ's `DefaultColorGenerator` for colour ids 0–9.
const ISLANDS_DARK: [u32; 10] = [
    0x74993d, 0x3d8a99, 0x573d99, 0x3d5499, 0x3d9982, 0x743d99, 0x3d3d99, 0x3d993d, 0x4a993d,
    0x996b3d,
];
const ISLANDS_LIGHT: [u32; 10] = [
    0x88b247, 0x47a1b2, 0x6647b2, 0x4762b2, 0x47b298, 0x8847b2, 0x4747b2, 0x47b247, 0x57b247,
    0xb27d47,
];

/// The six lane colours Fork's screenshots show.
const TRIDENT: [u32; 6] = [0x2897cf, 0xee9211, 0x6fcd4b, 0xd64447, 0xf0bd2b, 0xf5cb14];

/// Git Graph's default `git-graph.graph.colours` (MIT).
const BLUEPRINT: [u32; 12] = [
    0x0085d9, 0xd9008f, 0x00d90a, 0xd98500, 0xa300d9, 0xff0000, 0x00d9cc, 0xe138e8, 0x85d900,
    0xdc5b23, 0x6f24d6, 0xffcc00,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_round_trips_through_the_settings_file() {
        for palette in LanePalette::ALL {
            assert_eq!(LanePalette::from(Palette::from(palette)), palette);
        }
    }

    #[test]
    fn every_palette_has_distinct_colours_for_both_appearances() {
        for palette in LanePalette::ALL {
            for appearance in [Appearance::Dark, Appearance::Light] {
                let colors = palette.colors(appearance);
                assert!(colors.len() >= 6, "{} is too short", palette.label());
                let unique: std::collections::HashSet<_> = colors.iter().collect();
                assert_eq!(
                    unique.len(),
                    colors.len(),
                    "{} repeats a colour",
                    palette.label()
                );
            }
        }
    }

    #[test]
    fn the_kraken_keeps_the_original_ramp() {
        assert_eq!(LanePalette::Kraken.colors(Appearance::Dark)[0], 0x15a0bf);
    }
}
