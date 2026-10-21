//! Preview: a file shown as what it is, not as its text.
//!
//! Markdown renders; an image or an SVG draws — the new version over the old
//! one.
//! A raster image's patch is "binary files differ" and its File View has no
//! text at all, so for those the preview is the only readable view, and the
//! viewer opens on it.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{
    AnyElement, Image, ImageFormat, IntoElement, ObjectFit, SharedString, div, img, prelude::*, px,
};

use super::view::DiffSource;
use crate::git::command::git_bytes;
use crate::ui::theme::{Theme, ink};
use crate::ui::typography::ui_rems;

/// What a file previews as, from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Markdown,
    Svg,
    Image(ImageFormat),
}

impl PreviewKind {
    pub fn of(path: &str) -> Option<Self> {
        let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        Some(match extension.as_str() {
            "md" | "markdown" | "mdx" => Self::Markdown,
            "svg" => Self::Svg,
            "png" => Self::Image(ImageFormat::Png),
            "jpg" | "jpeg" => Self::Image(ImageFormat::Jpeg),
            "gif" => Self::Image(ImageFormat::Gif),
            "webp" => Self::Image(ImageFormat::Webp),
            "bmp" => Self::Image(ImageFormat::Bmp),
            _ => return None,
        })
    }

    /// A raster image has no text to read: its preview is the view it opens
    /// on.
    pub fn opens_on_preview(self) -> bool {
        matches!(self, Self::Image(_))
    }

    fn format(self) -> Option<ImageFormat> {
        match self {
            Self::Markdown => None,
            Self::Svg => Some(ImageFormat::Svg),
            Self::Image(format) => Some(format),
        }
    }
}

/// Both sides of the file. A side is `None` where the file does not exist:
/// before an add, after a delete.
#[derive(Debug, Clone, Default)]
pub struct PreviewSides {
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

/// Read both sides of the file the source names. Blocking: run it on the
/// background executor.
pub fn read_sides(source: &DiffSource) -> PreviewSides {
    let root = Path::new(source.repo());
    let path = source.path();
    let show = |revision: &str| git_bytes(root, &["show", &format!("{revision}:{path}")]).ok();
    match source {
        DiffSource::CommitFile { sha, .. } => PreviewSides {
            before: show(&format!("{sha}^")),
            after: show(sha),
        },
        // Staged: the index against HEAD.
        DiffSource::WorkingTreeFile { staged: true, .. } => PreviewSides {
            before: show("HEAD"),
            after: show(""),
        },
        // Unstaged: the working copy against the index, or HEAD when the
        // index has never seen the file.
        DiffSource::WorkingTreeFile { staged: false, .. } | DiffSource::Conflict { .. } => {
            PreviewSides {
                before: show("").or_else(|| show("HEAD")),
                after: std::fs::read(root.join(path)).ok(),
            }
        }
    }
}

/// One side of an image comparison, decoded and measured.
pub struct ImageSide {
    image: Arc<Image>,
    /// Pixel size, read from the format's header. `None` for SVG, whose size
    /// is whatever it is drawn at.
    dimensions: Option<(u32, u32)>,
    bytes: usize,
}

impl ImageSide {
    fn new(format: ImageFormat, bytes: Vec<u8>) -> Self {
        let dimensions = image_dimensions(&bytes);
        let len = bytes.len();
        Self {
            image: Arc::new(Image::from_bytes(format, bytes)),
            dimensions,
            bytes: len,
        }
    }

    /// `320 × 200 · 12.4 KB`, or just the size.
    fn caption(&self) -> String {
        let size = human_size(self.bytes);
        match self.dimensions {
            Some((width, height)) => format!("{width} × {height} · {size}"),
            None => size,
        }
    }
}

/// What the preview holds once read: the rendered forms, ready to draw.
pub enum Preview {
    Markdown(SharedString),
    Images {
        before: Option<ImageSide>,
        after: Option<ImageSide>,
    },
}

impl Preview {
    pub fn build(kind: PreviewKind, sides: PreviewSides) -> Self {
        match kind.format() {
            None => Preview::Markdown(
                sides
                    .after
                    .or(sides.before)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned().into())
                    .unwrap_or_default(),
            ),
            Some(format) => {
                let image =
                    |bytes: Option<Vec<u8>>| bytes.map(|bytes| ImageSide::new(format, bytes));
                Preview::Images {
                    before: image(sides.before),
                    after: image(sides.after),
                }
            }
        }
    }
}

/// Draw a preview in the viewer's body. `single` draws only the file as it
/// stands — an image's File View — instead of the before/after comparison.
pub fn render(preview: &Preview, single: bool, theme: &Theme) -> AnyElement {
    match preview {
        Preview::Markdown(text) => div()
            .id("diff-preview-markdown")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .px(px(28.0))
            .py(px(20.0))
            .child(
                gpui_kit::component::text::TextView::markdown(
                    "diff-preview-markdown-text",
                    text.clone(),
                )
                .selectable(true)
                // A file in the repository decides where its links point;
                // only the web and mail leave the app.
                .on_link_click(|url, _, _, cx| {
                    if opens_outside(url) {
                        cx.open_url(url);
                    }
                }),
            )
            .into_any_element(),
        Preview::Images { before, after } => {
            // The new image first, the old one under
            // it, each under its own band, each at its own size. A deleted
            // file's only version is the old one.
            let sides: Vec<(&'static str, &ImageSide)> = if single {
                after
                    .as_ref()
                    .or(before.as_ref())
                    .map(|side| vec![("Image", side)])
                    .unwrap_or_default()
            } else {
                [
                    ("New image", after.as_ref()),
                    ("Old image", before.as_ref()),
                ]
                .into_iter()
                .filter_map(|(title, side)| side.map(|side| (title, side)))
                .collect()
            };
            div()
                .id("diff-preview-images")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(
                    sides
                        .into_iter()
                        .map(|(title, side)| image_side(title, side, theme)),
                )
                .into_any_element()
        }
    }
}

/// Past this an image with no stated size (an SVG) is fitted, not grown.
const UNSIZED_BOX: f32 = 360.0;

/// One image of the comparison: its band — which one it is, what it
/// measures — then the image at its own pixel size, left-aligned. Wider than
/// the pane, it shrinks to fit.
fn image_side(title: &'static str, side: &ImageSide, theme: &Theme) -> impl IntoElement {
    let image = img(side.image.clone())
        .max_w_full()
        .object_fit(ObjectFit::Contain);
    let image = match side.dimensions {
        Some((width, height)) => image.w(px(width as f32)).h(px(height as f32)),
        None => image.w(px(UNSIZED_BOX)).h(px(UNSIZED_BOX)),
    };
    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(theme.border.opacity(0.5))
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .py(px(4.0))
                .bg(ink(0.06))
                .border_b_1()
                .border_color(theme.border.opacity(0.5))
                .child(
                    div()
                        .font_family(theme.font_mono.clone())
                        .text_size(ui_rems(12.5))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(ui_rems(10.5))
                        .text_color(theme.text_faint)
                        .child(SharedString::from(side.caption())),
                ),
        )
        .child(div().p(px(12.0)).flex().flex_row().child(image))
}

/// `12.4 KB`, the way a file manager says it.
fn opens_outside(url: &str) -> bool {
    let url = url.trim_start().to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
}

fn human_size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Width and height from the header of a PNG, GIF, BMP, JPEG or WebP. Every
/// one of them states its size in the first few hundred bytes, so this reads
/// no pixels.
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |at: usize| Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let le16 = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as u32);
    let be32 = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le32 = |at: usize| Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let le24 = |at: usize| {
        let b = bytes.get(at..at + 3)?;
        Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if bytes.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if bytes.starts_with(b"BM") {
        // A negative height is a top-down bitmap: the size is its magnitude.
        return Some((le32(18)?.unsigned_abs(), le32(22)?.unsigned_abs()));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return match bytes.get(12..16)? {
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
                Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        // Walk the segments to the first start-of-frame, which carries the
        // size. Every segment but the markers without a body states its
        // length right after its marker.
        let mut at = 2;
        while at + 4 <= bytes.len() {
            if bytes[at] != 0xff {
                return None;
            }
            let marker = bytes[at + 1];
            let length = be16(at + 2)? as usize;
            let is_frame = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
            if is_frame {
                return Some((be16(at + 7)?, be16(at + 5)?));
            }
            at += 2 + length;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_preview_link_opens_only_on_the_web_or_in_mail() {
        for url in [
            "https://example.com/x",
            "HTTP://example.com",
            "mailto:a@b.c",
        ] {
            assert!(super::opens_outside(url), "{url}");
        }
        for url in [
            "file:///etc/passwd",
            "smb://host/share",
            "vscode://x",
            "javascript:x",
        ] {
            assert!(!super::opens_outside(url), "{url}");
        }
    }

    use super::*;

    #[test]
    fn a_file_previews_by_its_extension() {
        assert_eq!(
            PreviewKind::of("docs/README.md"),
            Some(PreviewKind::Markdown)
        );
        assert_eq!(PreviewKind::of("logo.SVG"), Some(PreviewKind::Svg));
        assert_eq!(
            PreviewKind::of("a/b/photo.jpeg"),
            Some(PreviewKind::Image(ImageFormat::Jpeg))
        );
        assert_eq!(PreviewKind::of("src/main.rs"), None);
        assert_eq!(PreviewKind::of("Makefile"), None);
    }

    #[test]
    fn sizes_read_the_way_a_file_manager_says_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(12_698), "12.4 KB");
        assert_eq!(human_size(3 * 1_048_576), "3.0 MB");
    }

    #[test]
    fn dimensions_come_from_each_formats_header() {
        // PNG: IHDR width/height, big-endian.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&320u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((320, 200)));
        // GIF: logical screen, little-endian.
        let gif = [
            b"GIF89a".as_slice(),
            &80u16.to_le_bytes(),
            &60u16.to_le_bytes(),
        ]
        .concat();
        assert_eq!(image_dimensions(&gif), Some((80, 60)));
        // JPEG: an APP0 segment, then SOF0 with height before width.
        let jpeg = [
            &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00][..],
            &[0xff, 0xc0, 0x00, 0x11, 0x08],
            &200u16.to_be_bytes(),
            &320u16.to_be_bytes(),
        ]
        .concat();
        assert_eq!(image_dimensions(&jpeg), Some((320, 200)));
        assert_eq!(image_dimensions(b"<svg/>"), None);
    }

    #[test]
    fn only_a_raster_image_opens_on_its_preview() {
        assert!(PreviewKind::Image(ImageFormat::Png).opens_on_preview());
        assert!(!PreviewKind::Svg.opens_on_preview());
        assert!(!PreviewKind::Markdown.opens_on_preview());
    }

    #[test]
    fn a_commit_reads_the_file_on_both_sides_of_it() {
        let repo = crate::git::test_repo::TestRepo::new();
        repo.write("pic.svg", "<svg>old</svg>");
        repo.git(&["add", "pic.svg"]);
        repo.git(&["commit", "-m", "old"]);
        repo.write("pic.svg", "<svg>new</svg>");
        repo.git(&["add", "pic.svg"]);
        repo.git(&["commit", "-m", "new"]);
        let sha = repo.git(&["rev-parse", "HEAD"]);

        let sides = read_sides(&DiffSource::CommitFile {
            repo: repo.path().to_string_lossy().into_owned(),
            sha,
            path: "pic.svg".into(),
        });
        assert_eq!(sides.before.as_deref(), Some(&b"<svg>old</svg>"[..]));
        assert_eq!(sides.after.as_deref(), Some(&b"<svg>new</svg>"[..]));
    }

    #[test]
    fn an_untracked_file_has_no_before() {
        let repo = crate::git::test_repo::TestRepo::new();
        repo.commit("first");
        repo.write("new.md", "# hi\n");

        let sides = read_sides(&DiffSource::WorkingTreeFile {
            repo: repo.path().to_string_lossy().into_owned(),
            path: "new.md".into(),
            staged: false,
        });
        assert!(sides.before.is_none());
        assert_eq!(sides.after.as_deref(), Some(&b"# hi\n"[..]));
    }
}
