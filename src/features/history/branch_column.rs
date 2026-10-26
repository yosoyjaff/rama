//! The column of branch and tag labels to the left of the graph.
//!
//! The labels go before the lanes so a name always starts at the same x and
//! the column reads as a legend for the graph beside it. Each row shows one
//! label plus a `+N` for the rest; hovering the row opens a layer over the
//! graph listing every label at full length.
//!
//! Refs that share a short name — `main`, `origin/main`, `fork/main` — are
//! one label with an icon per ref.

use std::cmp::Reverse;

use crate::git::{Commit, RefBadge, RefKind};
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, FontWeight, Hsla, MouseButton, MouseDownEvent,
    ObjectFit, SharedString, div, img, prelude::*, px,
};

use super::HistoryView;
use super::actions::{Aim, BranchAim};
use crate::ui::icons;
use crate::ui::theme::Theme;
use crate::ui::tip::tip;

/// Width of the column. Fixed so the graph — and every node in it — starts
/// at the same x on every row.
pub const REFS_COLUMN_WIDTH: f32 = 160.0;

/// Height of one label, and the gap from the column's left edge.
const LABEL_HEIGHT: f32 = 22.0;
const LABEL_INSET: f32 = 3.0;
const LABEL_PAD_X: f32 = 5.0;
const LABEL_TEXT: f32 = 12.0;
/// Between the name and its icons, and between icons.
const ICON_GAP: f32 = 5.0;
const ICON: f32 = 11.0;
const REMOTE_PHOTO: f32 = 14.0;
/// After the check of the current branch.
const CHECK_GAP: f32 = 6.0;
/// The `+N` chip and its distance from the label.
const MORE_HEIGHT: f32 = 18.0;
const MORE_GAP: f32 = 5.0;
/// Fill of a label, as a share of its lane colour: at rest, hovered, and
/// for the checked-out branch.
const FILL: f32 = 0.25;
const FILL_HOVER: f32 = 0.45;
const FILL_CURRENT: f32 = 0.50;
/// The line from a label to its node, faint so it never reads as a lane.
const LEADER: f32 = 0.22;

/// One ref inside a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadgeRef {
    Local { name: String },
    Remote { remote: String, branch: String },
    Tag { name: String },
}

impl BadgeRef {
    /// Locals, then remotes, then tags.
    fn kind_order(&self) -> u8 {
        match self {
            Self::Local { .. } => 0,
            Self::Remote { .. } => 1,
            Self::Tag { .. } => 2,
        }
    }

    /// Which ref of a label a click means: the local, then `origin`, then
    /// `upstream`, then any remote, then the tag.
    fn click_order(&self) -> u8 {
        match self {
            Self::Local { .. } => 0,
            Self::Remote { remote, .. } if remote == "origin" => 1,
            Self::Remote { remote, .. } if remote == "upstream" => 2,
            Self::Remote { .. } => 3,
            Self::Tag { .. } => 4,
        }
    }

    fn tooltip_line(&self) -> String {
        match self {
            Self::Local { name } => format!("{name} (local)"),
            Self::Remote { remote, branch } => format!("{remote} → {branch} (remote)"),
            Self::Tag { name } => format!("{name} (tag)"),
        }
    }
}

/// One label: the refs of a commit that share a short name. With no refs it
/// is a detached HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefGroup {
    /// The short name, without a remote.
    pub name: String,
    pub refs: Vec<BadgeRef>,
    /// Holds the checked-out branch.
    pub current: bool,
}

impl RefGroup {
    pub fn detached() -> Self {
        Self {
            name: "HEAD".into(),
            refs: Vec::new(),
            current: true,
        }
    }

    pub fn is_detached(&self) -> bool {
        self.refs.is_empty()
    }

    fn has_local(&self) -> bool {
        self.refs
            .iter()
            .any(|r| matches!(r, BadgeRef::Local { .. }))
    }

    /// What a click or the context menu acts on. `None` when detached.
    pub fn menu_target(&self) -> Option<Aim> {
        let chosen = self.refs.iter().min_by_key(|r| r.click_order())?;
        Some(match chosen {
            BadgeRef::Local { name } => Aim::Branch(BranchAim {
                badge: name.clone(),
                name: name.clone(),
                remote: None,
            }),
            BadgeRef::Remote { remote, branch } => Aim::Branch(BranchAim {
                badge: format!("{remote}/{branch}"),
                name: branch.clone(),
                remote: Some(remote.clone()),
            }),
            BadgeRef::Tag { name } => Aim::Tag(name.clone()),
        })
    }

    pub fn tooltip_lines(&self) -> Vec<String> {
        if self.is_detached() {
            return vec!["HEAD is detached here".into()];
        }
        let current = self.current.then(|| format!("{} (current)", self.name));
        current
            .into_iter()
            .chain(self.refs.iter().map(BadgeRef::tooltip_line))
            .collect()
    }
}

/// A commit's refs as labels: the checked-out branch first, then labels with
/// a local, then remote-only, then tags; more refs before fewer; then by
/// name. A HEAD with no refs gets the detached label.
pub fn group_refs(refs: &[RefBadge], is_head: bool, current_branch: Option<&str>) -> Vec<RefGroup> {
    let mut badges: Vec<RefGroup> = Vec::new();
    for r in refs {
        let label = r.label.as_str();
        let (short, member) = match r.kind {
            RefKind::Branch => (label, BadgeRef::Local { name: label.into() }),
            RefKind::Tag => (label, BadgeRef::Tag { name: label.into() }),
            RefKind::Remote => {
                let (remote, branch) = label.split_once('/').unwrap_or(("origin", label));
                let member = BadgeRef::Remote {
                    remote: remote.into(),
                    branch: branch.into(),
                };
                (branch, member)
            }
        };
        match badges.iter_mut().find(|b| b.name == short) {
            Some(badge) => badge.refs.push(member),
            None => badges.push(RefGroup {
                name: short.into(),
                refs: vec![member],
                current: false,
            }),
        }
    }
    for badge in &mut badges {
        badge.refs.sort_by_key(BadgeRef::kind_order);
        badge.current = current_branch.is_some_and(|current| {
            badge
                .refs
                .iter()
                .any(|r| matches!(r, BadgeRef::Local { name } if name == current))
        });
    }
    badges.sort_by(|a, b| {
        let key = |b: &RefGroup| {
            (
                Reverse(b.current),
                b.refs[0].kind_order(),
                Reverse(b.refs.len()),
            )
        };
        key(a).cmp(&key(b)).then_with(|| a.name.cmp(&b.name))
    });
    if badges.is_empty() && is_head {
        badges.push(RefGroup::detached());
    }
    badges
}

/// How many characters of a name a label shows at rest. Labels carrying a
/// check or a laptop icon have less room; a fixed count keeps every label in
/// the column the same shape.
pub fn name_room(has_check: bool, has_local: bool) -> usize {
    if !has_check {
        15
    } else if has_local {
        7
    } else {
        11
    }
}

/// `name` cut to `max` characters with an ellipsis. Counts characters, not
/// bytes.
pub fn clip_name(name: &str, max: usize) -> String {
    match name.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &name[..cut]),
        None => name.to_string(),
    }
}

/// Which part of a row's label zone the pointer is over.
#[derive(Debug, Clone, Copy)]
enum ZonePart {
    Cell,
    Layer,
}

/// The pointer over one row's labels: the cell, or the layer it opened. The
/// layer stays open while the pointer is on either, so moving from the cell
/// onto the layer covering it keeps it up.
#[derive(Debug, Default)]
pub(super) struct BadgeHover {
    row: Option<String>,
    on_cell: bool,
    on_layer: bool,
}

impl BadgeHover {
    fn is_open(&self, sha: &str) -> bool {
        self.row.as_deref() == Some(sha) && (self.on_cell || self.on_layer)
    }

    /// Record a hover change; true when that opened or closed the layer.
    fn update(&mut self, sha: &str, part: ZonePart, hovered: bool) -> bool {
        let before = self.is_open(sha);
        if self.row.as_deref() != Some(sha) {
            if !hovered {
                return false;
            }
            *self = Self {
                row: Some(sha.to_string()),
                ..Self::default()
            };
        }
        match part {
            ZonePart::Cell => self.on_cell = hovered,
            ZonePart::Layer => self.on_layer = hovered,
        }
        before != self.is_open(sha)
    }
}

impl HistoryView {
    pub(super) fn ref_groups(&self, commit: &Commit) -> Vec<RefGroup> {
        group_refs(
            &commit.refs,
            self.head_sha.as_deref() == Some(commit.sha.as_str()),
            self.repo.as_ref().and_then(|repo| repo.branch.as_deref()),
        )
    }

    fn hover_refs(&mut self, sha: &str, part: ZonePart, hovered: bool, cx: &mut Context<Self>) {
        if self.badge_hover.update(sha, part, hovered) {
            cx.notify();
        }
    }

    /// One row's cell. `node_x` is the node's offset into the graph, so the
    /// leader line reaches it; `tone` is the colour of its lane.
    pub(super) fn render_refs_cell(
        &self,
        sha: &str,
        badges: Vec<RefGroup>,
        node_x: f32,
        tone: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cell = div()
            .id(ElementId::Name(format!("history-ref-zone-{sha}").into()))
            .relative()
            .flex_none()
            .w(px(REFS_COLUMN_WIDTH))
            .h_full()
            .pl(px(LABEL_INSET))
            .flex()
            .items_center();
        let Some(first) = badges.first().cloned() else {
            return cell.into_any_element();
        };
        let open = self.badge_hover.is_open(sha);
        let more = badges.len() - 1;

        // Clipped on its own so the leader may overhang the column while a
        // long name may not.
        let shown = div()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(MORE_GAP + LABEL_INSET))
            .child(self.render_group(sha, first, tone, false, theme, cx))
            .when(more > 0 && !open, |row| {
                row.child(render_more(more, tone, theme))
            });

        let leader = div()
            .flex_1()
            .min_w(px(4.0))
            .ml(px(4.0))
            .mr(px(-node_x))
            .h(px(1.0))
            .bg(tone.opacity(LEADER));

        let layer = open.then(|| self.render_group_layer(sha, badges, tone, theme, cx));

        let row_sha = sha.to_string();
        cell.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.hover_refs(&row_sha, ZonePart::Cell, *hovered, cx);
        }))
        .child(shown)
        .child(leader)
        .children(layer)
        .into_any_element()
    }

    /// Every label of the row stacked over the graph at full length, as wide
    /// as the longest. It occludes so the rows beneath ignore the pointer.
    fn render_group_layer(
        &self,
        sha: &str,
        badges: Vec<RefGroup>,
        tone: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let layer_sha = sha.to_string();
        let items = badges.into_iter().map(|badge| {
            // An opaque base, so the graph does not show through the tint.
            div()
                .flex()
                .rounded(px(2.0))
                .bg(theme.bg)
                .child(self.render_group(sha, badge, tone, true, theme, cx))
        });
        let items: Vec<_> = items.collect();
        gpui_kit::deferred(
            div()
                .id(ElementId::Name(
                    format!("history-ref-zone-layer-{sha}").into(),
                ))
                .absolute()
                .top(px((super::row::ROW_HEIGHT - LABEL_HEIGHT) / 2.0))
                .left(px(LABEL_INSET))
                .occlude()
                .flex()
                .flex_col()
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.hover_refs(&layer_sha, ZonePart::Layer, *hovered, cx);
                }))
                .children(items),
        )
        .priority(1)
    }

    /// The icon for one ref: a laptop for a local branch, a tag for a tag,
    /// and for a remote its owner's photo once loaded — until then GitHub's
    /// mark on GitHub, a cloud elsewhere.
    fn render_ref_mark(&self, r: &BadgeRef, color: Hsla) -> AnyElement {
        let glyph = |path: &'static str| {
            icons::icon(path)
                .size(px(ICON))
                .text_color(color)
                .into_any_element()
        };
        let remote = match r {
            BadgeRef::Local { .. } => return glyph(icons::LAPTOP),
            BadgeRef::Tag { .. } => return glyph(icons::TAG),
            BadgeRef::Remote { remote, .. } => remote,
        };
        if let Some(photo) = self.photos.remotes.get(remote) {
            return img(photo.clone())
                .flex_none()
                .size(px(REMOTE_PHOTO))
                .rounded_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element();
        }
        let github = self
            .repo
            .as_ref()
            .and_then(|repo| repo.remotes.iter().find(|known| &known.name == remote))
            .and_then(|known| crate::git::forge::hosted(&known.url))
            .is_some_and(|hosted| hosted.forge == crate::git::forge::Forge::GitHub);
        glyph(if github { icons::GITHUB } else { icons::CLOUD })
    }

    /// One label: a check for the current branch, the name, an icon per
    /// ref. `whole` shows the full name (the hover layer).
    fn render_group(
        &self,
        sha: &str,
        badge: RefGroup,
        tone: Hsla,
        whole: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = theme.text;
        let current = badge.current;
        let check = current && !badge.is_detached();
        let name = if whole {
            badge.name.clone()
        } else {
            clip_name(&badge.name, name_room(check, badge.has_local()))
        };
        let lines: Vec<SharedString> = badge
            .tooltip_lines()
            .into_iter()
            .map(SharedString::from)
            .collect();
        let (rest, hover) = if current {
            (FILL_CURRENT, FILL_CURRENT)
        } else {
            (FILL, FILL_HOVER)
        };
        let place = if whole { "whole" } else { "inline" };

        let icons_row = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(ICON_GAP))
            .when(badge.is_detached(), |row| {
                row.child(
                    icons::icon(icons::GIT_BRANCH)
                        .size(px(ICON))
                        .text_color(color),
                )
            })
            .children(badge.refs.iter().map(|r| self.render_ref_mark(r, color)));

        let label = div()
            .id(ElementId::Name(
                format!("history-ref-{sha}-{}-{place}", badge.name).into(),
            ))
            .h(px(LABEL_HEIGHT))
            .min_w_0()
            .when(whole, |label| label.flex_none().flex_grow_1())
            .px(px(LABEL_PAD_X))
            .flex()
            .items_center()
            .rounded(px(2.0))
            .bg(tone.opacity(rest))
            .hover(move |s| s.bg(tone.opacity(hover)))
            .text_size(px(LABEL_TEXT))
            .text_color(color)
            .when(check, |label| {
                label.child(
                    icons::icon(icons::CHECK)
                        .size(px(ICON))
                        .mr(px(CHECK_GAP))
                        .text_color(color),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .mr(px(ICON_GAP))
                    .whitespace_nowrap()
                    .when(!whole, |n| n.truncate())
                    .when(current, |n| n.font_weight(FontWeight::MEDIUM))
                    .child(SharedString::from(name)),
            )
            .child(icons_row)
            .tooltip(tip(lines.join("\n")))
            .tooltip_show_delay(std::time::Duration::from_millis(300));

        let Some(target) = badge.menu_target() else {
            return label.into_any_element();
        };
        let (click_sha, menu_sha) = (sha.to_string(), sha.to_string());
        let menu_target = target.clone();
        label
            .cursor_pointer()
            // The row opens its commit on click; a label is about a ref.
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if let Aim::Tag(_) = target {
                    // A tag checkout detaches HEAD: go through the menu,
                    // which says so.
                    this.open_context_menu(event.position(), click_sha.clone(), target.clone(), cx);
                } else {
                    this.switch_to_badge(target.clone(), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_context_menu(
                        event.position,
                        menu_sha.clone(),
                        menu_target.clone(),
                        cx,
                    );
                }),
            )
            .into_any_element()
    }
}

/// The `+N` chip for labels the cell does not show.
fn render_more(count: usize, tone: Hsla, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .h(px(MORE_HEIGHT))
        .px(px(3.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.0))
        .bg(tone.opacity(FILL))
        .text_size(px(LABEL_TEXT))
        .text_color(theme.text)
        .child(SharedString::from(format!("+{count}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use RefKind::{Branch, Remote, Tag};

    fn refs(list: &[(RefKind, &str)]) -> Vec<RefBadge> {
        list.iter()
            .map(|(kind, label)| RefBadge {
                kind: *kind,
                label: label.to_string(),
            })
            .collect()
    }

    fn names(badges: &[RefGroup]) -> Vec<&str> {
        badges.iter().map(|b| b.name.as_str()).collect()
    }

    fn on(remote: &str, branch: &str) -> BadgeRef {
        BadgeRef::Remote {
            remote: remote.into(),
            branch: branch.into(),
        }
    }

    #[test]
    fn refs_with_one_short_name_share_a_label_local_first() {
        let badges = group_refs(
            &refs(&[
                (Remote, "origin/main"),
                (Branch, "main"),
                (Remote, "fork/main"),
            ]),
            true,
            Some("main"),
        );
        assert_eq!(
            badges,
            [RefGroup {
                name: "main".into(),
                refs: vec![
                    BadgeRef::Local {
                        name: "main".into()
                    },
                    on("origin", "main"),
                    on("fork", "main"),
                ],
                current: true,
            }]
        );
    }

    #[test]
    fn only_the_remote_is_cut_from_a_remote_name() {
        let badges = group_refs(&refs(&[(Remote, "origin/feat/x")]), false, None);
        assert_eq!(names(&badges), ["feat/x"]);
    }

    #[test]
    fn labels_sort_current_then_kind_then_size_then_name() {
        let badges = group_refs(
            &refs(&[
                (Tag, "v1.0"),
                (Branch, "zeta"),
                (Branch, "feat"),
                (Branch, "alpha"),
                (Remote, "origin/feat"),
                (Remote, "origin/only-remote"),
            ]),
            false,
            Some("zeta"),
        );
        assert_eq!(
            names(&badges),
            ["zeta", "feat", "alpha", "only-remote", "v1.0"]
        );
        assert!(badges[0].current && !badges[1].current);
    }

    #[test]
    fn a_head_without_refs_is_detached_and_targets_nothing() {
        let badges = group_refs(&[], true, None);
        assert_eq!(badges, [RefGroup::detached()]);
        assert!(badges[0].menu_target().is_none());
        assert!(group_refs(&[], false, None).is_empty());
    }

    #[test]
    fn a_click_prefers_local_then_origin_then_upstream() {
        let remotes = RefGroup {
            name: "feat".into(),
            refs: vec![
                on("fork", "feat"),
                on("upstream", "feat"),
                on("origin", "feat"),
            ],
            current: false,
        };
        assert_eq!(
            remotes.menu_target(),
            Some(Aim::Branch(BranchAim {
                badge: "origin/feat".into(),
                name: "feat".into(),
                remote: Some("origin".into()),
            }))
        );
        let with_local = RefGroup {
            refs: vec![
                on("origin", "feat"),
                BadgeRef::Local {
                    name: "feat".into(),
                },
            ],
            ..remotes
        };
        assert_eq!(
            with_local.menu_target(),
            Some(Aim::Branch(BranchAim {
                badge: "feat".into(),
                name: "feat".into(),
                remote: None,
            }))
        );
    }

    #[test]
    fn names_are_cut_by_what_the_label_carries() {
        assert_eq!(name_room(true, true), 7);
        assert_eq!(name_room(true, false), 11);
        assert_eq!(name_room(false, true), 15);
        assert_eq!(clip_name("feature", 7), "feature");
        assert_eq!(clip_name("feature/login", 7), "feature…");
        assert_eq!(clip_name("ñññññññññ", 7), "ñññññññ…");
    }

    #[test]
    fn the_layer_stays_open_while_cell_or_layer_is_hovered() {
        let mut hover = BadgeHover::default();
        assert!(hover.update("a", ZonePart::Cell, true));
        assert!(!hover.update("a", ZonePart::Layer, true));
        assert!(!hover.update("a", ZonePart::Cell, false));
        assert!(hover.is_open("a"));
        assert!(hover.update("a", ZonePart::Layer, false));
        // Leaving a row that is not the hovered one changes nothing.
        assert!(hover.update("b", ZonePart::Cell, true));
        assert!(!hover.update("a", ZonePart::Cell, false));
        assert!(hover.is_open("b"));
    }
}
