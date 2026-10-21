//! How an author is shown without a photo: their initials, in a colour of
//! their own that is the same wherever they appear.

use gpui_kit::Hsla;

/// Soft colours that read on both the dark and the light background.
const TINTS: [u32; 8] = [
    0x7aa2f7, 0xbb9af7, 0x9ece6a, 0xe0af68, 0xf7768e, 0x73daca, 0xff9e64, 0xb4f9f8,
];

/// The first letter of a name, or of its first and last words, in capitals;
/// `?` for no name.
pub fn author_initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let capital = |word: &str| {
        word.chars()
            .next()
            .into_iter()
            .flat_map(char::to_uppercase)
            .collect::<String>()
    };
    match words.as_slice() {
        [] => "?".into(),
        [only] => capital(only),
        [first, .., last] => capital(first) + &capital(last),
    }
}

/// The colour for `key` (an email, or a name when there is none), as RGB.
pub fn author_tint_rgb(key: &str) -> u32 {
    // FNV-1a over the lowercase key, so the same address always lands on
    // the same colour whatever its case.
    let hash = key
        .trim()
        .to_lowercase()
        .bytes()
        .fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
    TINTS[hash as usize % TINTS.len()]
}

pub fn author_tint(key: &str) -> Hsla {
    gpui_kit::rgb(author_tint_rgb(key)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_the_first_and_last_words() {
        assert_eq!(author_initials("ada"), "A");
        assert_eq!(author_initials("  Grace Brewster Hopper "), "GH");
        assert_eq!(author_initials("élodie dupont"), "ÉD");
        assert_eq!(author_initials(""), "?");
    }

    #[test]
    fn a_person_keeps_one_colour() {
        assert_eq!(
            author_tint_rgb("Ana@Example.org"),
            author_tint_rgb(" ana@example.org")
        );
        let spread: std::collections::HashSet<u32> = (0..50)
            .map(|n| author_tint_rgb(&format!("dev{n}@x.io")))
            .collect();
        assert!(spread.len() > 4);
    }
}
