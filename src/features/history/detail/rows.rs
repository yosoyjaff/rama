//! What both panes draw the same way: the bar of listing options, a file's
//! row, a folder's row, and the tooltips.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, HighlightStyle, SharedString, StyledText, div,
    prelude::*, px,
};

use super::listing::{Line, file_name, folder_of};
use super::{Change, DetailPanel, PanelEvent, Shown};
use crate::git::DiffFileSummary;
use crate::ui::chrome::{SEGMENT_INNER_RADIUS, SEGMENT_RADIUS};
use crate::ui::icons;
use crate::ui::theme::{self, Theme};
pub(super) use crate::ui::tip::tip;
use crate::ui::typography::ui_rems;

pub(super) const LINE_HEIGHT: f32 = 26.0;
/// How far each level of the tree steps in.
const STEP_IN: f32 = 12.0;

/// The lists files are shown in. Each keeps its own element ids, since a
/// file can be in two at once, and opens its files its own way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum List {
    Commit,
    Unstaged,
    Staged,
}

impl List {
    fn key(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Unstaged => "unstaged",
            Self::Staged => "staged",
        }
    }
}

/// A band across the panel with a faint rule under it.
pub(super) fn band(theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme.border.opacity(0.4))
}

/// A path with its folder dimmed, so the name stands out.
pub(super) fn dimmed_folder(path: &str, theme: &Theme) -> StyledText {
    let folder = path.len() - file_name(path).len();
    let dim = HighlightStyle {
        color: Some(theme.text_muted),
        ..HighlightStyle::default()
    };
    StyledText::new(SharedString::from(path.to_owned())).with_highlights([(0..folder, dim)])
}

impl DetailPanel {
    /// What clicking a file of `list` opens.
    fn opening(&self, list: List, path: String) -> Option<PanelEvent> {
        match (list, &self.shown) {
            (List::Commit, Shown::Commit(commit)) => Some(PanelEvent::OpenCommitFile {
                commit: commit.clone(),
                path,
            }),
            (List::Commit, _) => None,
            (List::Unstaged | List::Staged, _) => Some(PanelEvent::OpenTreeFile {
                path,
                staged: list == List::Staged,
            }),
        }
    }

    /// The rows of `files` in `list`: one per file, or the folder tree.
    pub(super) fn file_rows(
        &self,
        list: List,
        files: &[DiffFileSummary],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.listing.as_tree {
            return self
                .listing
                .order(files)
                .into_iter()
                .map(|file| self.file_row(list, file, 0, true, theme, cx))
                .collect();
        }
        self.listing
            .tree(files)
            .into_iter()
            .map(|line| match line {
                Line::File { file, depth } => self.file_row(list, file, depth, false, theme, cx),
                folder => self.folder_row(list, folder, theme, cx),
            })
            .collect()
    }

    /// A folder: whether it is open, its name, and how many files are
    /// under it. Folders are structure, not change, so they stay grey.
    fn folder_row(
        &self,
        list: List,
        line: Line,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Line::Folder {
            path,
            depth,
            files,
            folded,
        } = line
        else {
            return div().into_any_element();
        };
        let grey = theme.text_muted;
        let chevron = match folded {
            true => icons::ALT_ARROW_RIGHT,
            false => icons::ALT_ARROW_DOWN,
        };
        let folder = path.to_owned();
        div()
            .id(SharedString::from(format!(
                "panel-{}-dir-{path}",
                list.key()
            )))
            .h(px(LINE_HEIGHT))
            .flex()
            .items_center()
            .gap(px(4.0))
            .pl(px(6.0 + depth as f32 * STEP_IN))
            .pr(px(8.0))
            .cursor_pointer()
            .hover(|style| style.bg(theme::ink(0.04)))
            .child(
                icons::icon(chevron)
                    .size(px(11.0))
                    .flex_none()
                    .text_color(grey.opacity(0.7)),
            )
            .child(
                icons::icon(icons::FOLDER_01)
                    .size(px(13.0))
                    .flex_none()
                    .text_color(grey.opacity(0.75)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(grey)
                    .child(SharedString::from(file_name(path).to_owned())),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(ui_rems(10.5))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(files.to_string())),
            )
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                panel.listing.flip_folder(folder.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    /// A file: what happened to it, its name (after its folder, in the flat
    /// list), and how many lines came and went.
    fn file_row(
        &self,
        list: List,
        file: &DiffFileSummary,
        depth: usize,
        with_folder: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let change = Change::of(&file.status);
        let name: AnyElement = match with_folder && !folder_of(&file.path).is_empty() {
            true => dimmed_folder(&file.path, theme).into_any_element(),
            false => SharedString::from(file_name(&file.path).to_owned()).into_any_element(),
        };
        let hint = match &file.old_path {
            Some(old) => format!("{old} → {}", file.path),
            None => file.path.clone(),
        };
        let lines = |count: u32, sign: &str, color| {
            (count > 0).then(|| {
                div()
                    .text_color(color)
                    .child(SharedString::from(format!("{sign}{count}")))
            })
        };
        let counts = (!file.binary).then(|| {
            div()
                .flex_none()
                .flex()
                .gap(px(4.0))
                .text_size(ui_rems(11.0))
                .children(lines(file.additions, "+", theme.success))
                .children(lines(file.deletions, "−", theme.danger))
        });
        let path = file.path.clone();
        div()
            .id(SharedString::from(format!(
                "panel-{}-file-{}",
                list.key(),
                file.path
            )))
            .h(px(LINE_HEIGHT))
            .flex()
            .items_center()
            .gap(px(5.0))
            .pl(px(18.0 + depth as f32 * STEP_IN))
            .pr(px(8.0))
            .cursor_pointer()
            .hover(|style| style.bg(theme::ink(0.04)))
            .child(
                icons::icon(change.icon())
                    .size(px(12.0))
                    .flex_none()
                    .text_color(change.color(theme)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(13.0))
                    .text_color(theme.text)
                    .child(name),
            )
            .children(counts)
            .tooltip(tip(hint))
            .on_click(cx.listener(move |panel, _, _, cx| {
                cx.stop_propagation();
                if let Some(event) = panel.opening(list, path.clone()) {
                    cx.emit(event);
                }
            }))
            .into_any_element()
    }

    /// Sorting, list or tree, and opening every folder.
    pub(super) fn listing_bar(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (lit, dim) = (theme.text, theme.text_muted);
        let by_name = self.listing.by_name;
        let sort = div()
            .id("panel-by-name")
            .flex_none()
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(SEGMENT_RADIUS))
            .cursor_pointer()
            .when(by_name, |button| button.bg(theme::ink(0.07)))
            .when(!by_name, |button| {
                button.hover(|style| style.bg(theme::ink(0.04)))
            })
            .child(
                icons::icon(icons::ARROW_DOWN_A_Z)
                    .size(px(13.0))
                    .flex_none()
                    .text_color(if by_name { lit } else { dim }),
            )
            .tooltip(tip("Sort by file name"))
            .on_click(cx.listener(|panel, _, _, cx| panel.flip_by_name(cx)));

        let modes = [
            (
                false,
                "Path",
                icons::LEFT_TO_RIGHT_LIST_BULLET,
                "One row per file, with its folder",
            ),
            (true, "Tree", icons::FOLDER_01, "Files under their folders"),
        ];
        let segments = modes.into_iter().map(|(as_tree, label, icon, hint)| {
            let on = self.listing.as_tree == as_tree;
            div()
                .id(SharedString::from(format!("panel-listing-{label}")))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(8.0))
                .py(px(3.0))
                // Only the outer corners round, so the lit half does not
                // paint a square corner over the frame's curve.
                .when(as_tree, |half| half.rounded_r(px(SEGMENT_INNER_RADIUS)))
                .when(!as_tree, |half| half.rounded_l(px(SEGMENT_INNER_RADIUS)))
                .cursor_pointer()
                .text_size(px(10.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on { lit } else { dim })
                .when(on, |half| half.bg(theme::ink(0.07)))
                .when(!on, |half| half.hover(move |style| style.text_color(lit)))
                .child(icons::icon(icon).size(px(11.0)).flex_none())
                .child(label)
                .tooltip(tip(hint))
                .on_click(cx.listener(move |panel, _, _, cx| {
                    if panel.listing.as_tree != as_tree {
                        panel.set_tree(as_tree, cx);
                    }
                }))
        });
        let mode = div()
            .flex_none()
            .flex()
            .rounded(px(SEGMENT_RADIUS))
            .border_1()
            .border_color(theme.border.opacity(0.55))
            .children(segments);

        let open_all = Checkbox::new("panel-open-all")
            .label("View all files")
            .checked(self.listing.open_all)
            .xsmall()
            .text_size(px(10.0))
            .text_color(dim)
            .on_click(cx.listener(|panel, checked: &bool, _, cx| {
                panel.listing.open_all = *checked;
                cx.notify();
            }));

        band(theme)
            .justify_between()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(6.0))
            .child(sort)
            .child(mode)
            .child(open_all)
            .into_any_element()
    }
}
