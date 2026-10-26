//! One row of the history list: a commit, the working tree, or the button at
//! the end that loads older commits.

use super::WORKTREE_ROW;
use super::columns::SUBJECT_MIN;
use std::time::Duration;

use gpui_kit::{
    AnyElement, Context, Entity, FontWeight, HighlightStyle, MouseButton, MouseDownEvent,
    SharedString, StyledText, TextRun, Window, div, prelude::*, px,
};

use super::cells::{
    format_date, history_author_initial, history_author_name, history_author_tint, lane_color,
    wip_status_counts,
};
use super::columns::ColumnLayout;
use super::search::history_search_ranges;
use super::transition::HistoryRowTransition;
use super::{HistoryEvent, HistoryView, actions, branch_column};
use crate::git::Commit;
use crate::settings::{Authors, ColumnId};
use crate::ui::icons;
use crate::ui::motion::{self, AnimationExt as _};
use crate::ui::text_input::TextInput;
use crate::ui::theme::{self, Theme};
use crate::ui::tip::tip;

pub(super) const ROW_HEIGHT: f32 = 28.0;

// The working tree's row: file counts beside the node, then the summary box.
const COUNTS_INSET: f32 = 10.0;
const SUMMARY_INSET: f32 = 15.0;
const COUNT_TEXT: f32 = 12.0;
const COUNT_GAP: f32 = 6.0;
const COUNT_ICON: f32 = 12.0;
const COUNT_ICON_GAP: f32 = 3.0;
const SUMMARY_HEIGHT: f32 = 22.0;
const SUMMARY_NUDGE: f32 = 3.0;
const SUMMARY_MIN: f32 = 100.0;
/// The summary box is as wide as its text plus this.
const SUMMARY_SLACK: f32 = 35.0;

/// A table cell of an optional column.
fn column_cell(width: f32, min: f32, opacity: f32) -> gpui_kit::Div {
    div()
        .w(px(width))
        .min_w(px(min))
        .h_full()
        .flex_shrink(1.0)
        .flex()
        .items_center()
        .opacity(opacity)
}

/// The author: a chip with their initials in their colour and their name,
/// or just the name.
fn author_cell(
    sha: &str,
    width: f32,
    display: Authors,
    commit: &Commit,
    opacity: f32,
    theme: &Theme,
) -> AnyElement {
    let name = history_author_name(&commit.author.name);
    let cell = column_cell(width, ColumnId::Author.limits().0, opacity);
    if display == Authors::Name {
        return cell
            .pr(px(8.0))
            .truncate()
            .text_color(theme.text_muted)
            .child(name)
            .into_any_element();
    }
    let badge = div()
        .flex_none()
        .size(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .bg(gpui_kit::rgb(history_author_tint(&commit.author.email)))
        .font_family(theme.font_mono.clone())
        .font_weight(FontWeight::BOLD)
        .text_size(px(8.0))
        .text_color(theme.bg)
        .child(history_author_initial(&name));
    let label = div()
        .min_w_0()
        .truncate()
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(10.5))
        .text_color(theme.text.opacity(0.85))
        .child(name.clone());
    let chip = div()
        .id(SharedString::from(format!("history-author-{sha}")))
        .flex_none()
        .h(px(18.0))
        .min_w_0()
        .max_w_full()
        .flex()
        .items_center()
        .gap(px(6.0))
        .pl(px(4.0))
        .pr(px(6.0))
        .overflow_hidden()
        .rounded(px(6.0))
        .bg(theme::wash(0.06))
        .child(badge)
        .child(label)
        .tooltip(tip(name))
        .tooltip_show_delay(Duration::from_millis(300));
    cell.justify_start().child(chip).into_any_element()
}

fn date_cell(width: f32, when: &str, opacity: f32, theme: &Theme) -> AnyElement {
    column_cell(width, ColumnId::Date.limits().0, opacity)
        .pr(px(8.0))
        .truncate()
        .text_size(px(10.5))
        .text_color(theme.text_muted)
        .child(SharedString::from(format_date(when)))
        .into_any_element()
}

/// The width of `text` set in the window's font at the readout size.
fn text_width(text: SharedString, window: &mut Window, theme: &Theme) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let run = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color: theme.text,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text, px(COUNT_TEXT), &[run], None);
    line.width.as_f32()
}

impl HistoryView {
    fn sha_cell(
        &self,
        sha: &str,
        width: f32,
        opacity: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copied = self.just_copied.as_deref() == Some(sha);
        let text: SharedString = match copied {
            true => "Copied".into(),
            false => sha.chars().take(7).collect::<String>().into(),
        };
        let full = sha.to_string();
        let chip = div()
            .id(SharedString::from(format!("history-sha-{sha}")))
            .w_full()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(|style| style.bg(theme::ink(0.07)))
            .font_family(theme.font_mono.clone())
            .text_size(px(10.5))
            .text_color(if copied {
                theme.accent
            } else {
                theme.text_muted
            })
            .child(text)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.copy_sha(full.clone(), cx);
            }));
        column_cell(width, ColumnId::Sha.limits().0, opacity)
            .pr(px(6.0))
            .child(chip)
            .into_any_element()
    }

    /// The row at `index` of the list.
    pub(super) fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::of(cx).clone();
        if index >= self.shown_rows.len() {
            return self.load_more_row(&theme, cx);
        }
        let (Some(commit), Some(_)) = (
            self.shown_rows.get(index).cloned(),
            self.graph.rows.get(index),
        ) else {
            return gpui_kit::Empty.into_any_element();
        };
        if commit.sha == WORKTREE_ROW {
            return self.wip_row(index, &theme, window, cx);
        }
        let row = self.commit_row(index, commit.clone(), &theme, cx);
        // Rows entering or leaving a fold animate their height.
        let change = self
            .scope_change
            .as_ref()
            .and_then(|t| t.rows.get(index).copied());
        let grows = match change {
            Some(HistoryRowTransition::Entering) => true,
            Some(HistoryRowTransition::Exiting) => false,
            _ => return row,
        };
        let name = format!(
            "history-row-{}-{}-{}",
            self.scope_epoch,
            commit.sha,
            if grows { "in" } else { "out" }
        );
        div()
            .flex_none()
            .w_full()
            .overflow_hidden()
            .child(row)
            .with_animation(
                SharedString::from(name),
                motion::COLLAPSE.animation(),
                move |slot, t| {
                    let shown = if grows { t } else { 1.0 - t };
                    slot.h(px(ROW_HEIGHT * shown)).opacity(0.35 + 0.65 * shown)
                },
            )
            .into_any_element()
    }

    /// The last row while older commits remain: loads them, or retries a
    /// page that failed.
    fn load_more_row(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (busy, failed) = (self.paging.reading, self.paging.failure.is_some());
        let label = match (busy, failed) {
            (true, _) => "Loading…",
            (false, true) => "Retry",
            (false, false) => "Load more",
        };
        let hover = (
            theme.element_hover,
            theme.border_strong.opacity(0.75),
            theme.text,
        );
        let button = div()
            .id("history-load-older")
            .h(px(28.0))
            .px(px(11.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border.opacity(0.85))
            .bg(theme.surface_raised.opacity(0.72))
            .text_size(px(11.0))
            .text_color(if busy {
                theme.text_faint
            } else {
                theme.text_muted
            })
            .when(!busy, |button| {
                button
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover.0).border_color(hover.1).text_color(hover.2))
                    .on_click(cx.listener(|this, _, _, cx| this.read_older(cx)))
                    .child(
                        icons::icon(if failed {
                            icons::REFRESH
                        } else {
                            icons::ALT_ARROW_DOWN
                        })
                        .size(px(11.0))
                        .flex_none()
                        .text_color(theme.text_faint),
                    )
            })
            .child(label);
        div()
            .flex_none()
            .w_full()
            .h(px(48.0))
            .flex()
            .items_center()
            .justify_center()
            .child(button)
            .into_any_element()
    }

    /// Room for the branch badges column, so the graph lines up with the
    /// commit rows below.
    fn badge_gap(&self) -> Option<gpui_kit::Div> {
        self.branch_column_open.then(|| {
            div()
                .flex_none()
                .w(px(branch_column::REFS_COLUMN_WIDTH))
                .h_full()
        })
    }

    /// The working tree's row: its node, the summary field, and how many
    /// files are added, changed, deleted and renamed. In a conflict it is
    /// tinted and says what stopped instead.
    fn wip_row(
        &mut self,
        index: usize,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self
            .tree_status
            .as_ref()
            .map(|s| s.entries.as_slice())
            .unwrap_or(&[]);
        let clashes = entries.iter().filter(|e| e.conflicted).count();
        let counts = wip_status_counts(entries);
        let total = counts.total();
        let headline = self
            .worktree_row()
            .map(|wip| wip.subject)
            .unwrap_or_default();
        let field = self.wip_input.clone().filter(|_| clashes == 0);
        let field = field.map(|field: Entity<TextInput>| {
            // As wide as what is typed in it, plus room.
            let width = text_width(field.read(cx).text().clone(), window, theme) + SUMMARY_SLACK;
            div()
                .ml(px(SUMMARY_NUDGE))
                .h(px(SUMMARY_HEIGHT))
                .w(px(width))
                .min_w(px(SUMMARY_MIN))
                .flex_shrink_1()
                .flex()
                .items_center()
                .rounded(px(2.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.input_bg)
                .text_color(theme.text)
                .child(field)
        });
        let readings = counts.present().map(|(status, n)| {
            div()
                .flex()
                .items_center()
                .gap(px(COUNT_ICON_GAP))
                .child(
                    icons::icon(status.icon())
                        .size(px(COUNT_ICON))
                        .flex_none()
                        .text_color(status.color(theme)),
                )
                .child(SharedString::from(n.to_string()))
        });
        let summary_gap = match field.is_some() {
            true => SUMMARY_INSET,
            false => COUNTS_INSET,
        };
        let summary = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(COUNT_GAP))
            .ml(px(summary_gap))
            .when(clashes > 0, |summary| {
                summary
                    .child(
                        icons::icon(icons::DANGER_TRIANGLE)
                            .size(px(COUNT_ICON))
                            .flex_none()
                            .text_color(theme.warning),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.warning)
                            .child(headline),
                    )
            })
            .when(clashes == 0 && total == 0, |summary| {
                summary.child(div().italic().child("No changes"))
            })
            .children(readings);
        let graph = self.graph.rows[index].clone();
        let node = self.graph_cell(index, graph, theme, cx);
        div()
            .relative()
            .group("history-row")
            .flex_none()
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .hover(|style| style.bg(theme.element_hover.opacity(0.5)))
            // The conflict mark is drawn over the row rather than as a border,
            // which would push the node off the line drawn for it.
            .when(clashes > 0, |row| {
                row.bg(theme.warning.opacity(0.10)).child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.0))
                        .bg(theme.warning.opacity(0.7)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    // Show the working tree's diff, and the side panel's
                    // staging view where the commit is written.
                    cx.emit(HistoryEvent::SelectWorkingTree);
                    this.selected = None;
                    cx.notify();
                }),
            )
            .children(self.badge_gap())
            .child(node)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .pr(px(8.0))
                    .text_size(px(COUNT_TEXT))
                    .text_color(theme.text_muted)
                    .children(field)
                    .child(summary),
            )
            .into_any_element()
    }

    /// A commit's row: its badges, its node, its subject, and the optional
    /// columns. During a search, matches keep full strength with what matched
    /// marked, and everything else fades back.
    fn commit_row(
        &mut self,
        index: usize,
        commit: Commit,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let graph = self.graph.rows[index].clone();
        let sha = commit.sha.clone();
        let selected = self.selected.as_deref() == Some(sha.as_str());
        let searching = self.search.active();
        let hit = searching && self.search.hits.binary_search(&index).is_ok();
        let current = hit && self.search.at.and_then(|at| self.search.hits.get(at)) == Some(&index);
        let strength = if searching && !hit { 0.35 } else { 1.0 };
        let subject: SharedString = match commit.subject.as_str() {
            "" => "(no subject)".into(),
            text => text.to_string().into(),
        };
        let marks = match hit {
            true => history_search_ranges(&self.search.query, &subject),
            false => Vec::new(),
        };
        let mark = HighlightStyle {
            color: Some(theme.on_accent),
            background_color: Some(theme.accent_strong.opacity(0.85)),
            ..HighlightStyle::default()
        };

        let badges = self.branch_column_open.then(|| {
            let badges = self.ref_groups(&commit);
            let node_x = self.sizing.shown.get().x(graph.lane);
            let color = lane_color(graph.lane, theme, cx);
            self.render_refs_cell(&sha, badges, node_x, color, theme, cx)
        });
        let layout = ColumnLayout::of(cx);
        let columns: Vec<AnyElement> = layout
            .visible()
            .into_iter()
            .map(|column| {
                let width = layout.width(column);
                match column {
                    ColumnId::Author => {
                        author_cell(&sha, width, layout.authors, &commit, strength, theme)
                    }
                    ColumnId::Date => date_cell(width, &commit.author.when, strength, theme),
                    ColumnId::Sha => self.sha_cell(&sha, width, strength, theme, cx),
                }
            })
            .collect();
        // A right-click away from the badges aims at the most useful ref on
        // the commit, so the branch items do not depend on the badges column.
        let current_branch = self
            .repo
            .as_ref()
            .and_then(|r| r.branch.clone())
            .unwrap_or_default();
        let target = actions::aim_for_row(&commit.refs, &current_branch);
        let node = self.graph_cell(index, graph, theme, cx);
        let menu_sha = sha.clone();
        div()
            .id(SharedString::from(format!("history-row-{sha}")))
            .flex_none()
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .text_size(px(11.0))
            .cursor_pointer()
            .when(hit && !selected, |row| {
                row.bg(theme.accent.opacity(if current { 0.1 } else { 0.05 }))
            })
            .when(selected, |row| row.bg(theme.accent.opacity(0.1)))
            .hover(|style| style.bg(theme::ink(0.025)))
            // A click selects; opening the full diff is a deliberate second
            // click, from the side panel.
            .on_click(cx.listener(move |this, _, _, cx| this.select_commit(commit.clone(), cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    this.open_context_menu(event.position, menu_sha.clone(), target.clone(), cx);
                }),
            )
            .children(badges)
            .child(node)
            .child(
                div()
                    .flex_1()
                    .min_w(px(SUBJECT_MIN))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .pr(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(theme.text)
                            .opacity(strength)
                            .child(
                                StyledText::new(subject)
                                    .with_highlights(marks.into_iter().map(|r| (r, mark))),
                            ),
                    ),
            )
            .child(div().h_full().flex().flex_shrink(1.0).children(columns))
            .into_any_element()
    }
}
