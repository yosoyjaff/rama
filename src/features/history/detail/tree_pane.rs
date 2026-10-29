//! The panel for the working tree: the changes on each side of the index,
//! the commit box, and while a merge, rebase, cherry-pick or revert is
//! stopped, its conflicts and the buttons that end it.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Stateful, Window, div, prelude::*, px,
};

use super::rows::{LINE_HEIGHT, List, dimmed_folder, tip};
use super::work::{ConflictMenu, Edit, Job, SUMMARY_GUIDE, Section, way_out};
use super::{DetailPanel, PanelEvent};
use crate::git::DiffFileSummary;
use crate::git::actions::GitAction;
use crate::ui::icons;
use crate::ui::popover;
use crate::ui::theme::{self, Theme};
use crate::ui::typography::ui_rems;

/// A small outlined button in a section's heading. `live` is false while
/// it cannot be pressed.
fn pill(id: SharedString, label: &'static str, tone: Hsla, live: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(6.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(tone.opacity(if live { 0.65 } else { 0.3 }))
        .text_size(px(10.0))
        .text_color(if live { tone } else { tone.opacity(0.5) })
        .when(live, |pill| {
            pill.cursor_pointer()
                .hover(move |style| style.bg(tone.opacity(0.1)))
        })
        .child(label)
}

/// A row of the conflict lists: a mark in `tone` and the whole path.
fn conflict_row(
    id: String,
    icon: &'static str,
    tone: Hsla,
    path: &str,
    theme: &Theme,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .gap(px(6.0))
        .pl(px(18.0))
        .pr(px(8.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme::ink(0.04)))
        .child(
            icons::icon(icon)
                .size(px(12.0))
                .flex_none()
                .text_color(tone),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(ui_rems(13.0))
                .text_color(theme.text)
                .child(dimmed_folder(path, theme)),
        )
}

/// A quiet line of text inside a section.
fn aside(text: &'static str, color: Hsla) -> AnyElement {
    div()
        .px(px(16.0))
        .pb(px(6.0))
        .text_size(px(10.5))
        .text_color(color)
        .child(text)
        .into_any_element()
}

impl DetailPanel {
    /// While an operation has files in conflict or just settled, only those
    /// are listed. Otherwise the unstaged files, then the staged ones at the
    /// bottom. Files a stash left in conflict are listed the same way, with
    /// the staged ones as settled.
    pub(super) fn render_worktree(
        &mut self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tree = &self.worktree;
        let staging = tree.staging();
        let resolved = tree.resolved();
        let conflicted = tree.conflicted();
        let in_operation = tree.in_operation();
        let conflict_view = in_operation || !conflicted.is_empty();

        let mut lists: Vec<AnyElement> = Vec::new();
        if conflict_view {
            lists.push(self.conflicts(&conflicted, theme, cx));
        }
        if conflict_view && (!conflicted.is_empty() || !resolved.is_empty()) {
            lists.push(self.resolved(&resolved, theme, cx));
        } else {
            // During an operation an empty unstaged list tells nothing.
            if !in_operation || !staging.unstaged.is_empty() {
                lists.push(self.side(Section::Unstaged, &staging.unstaged, theme, cx));
            }
            // Pushes the staged files down when there is room.
            lists.push(div().flex_1().into_any_element());
            lists.push(self.side(Section::Staged, &staging.staged, theme, cx));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.tree_header(theme, cx))
            .child(self.listing_bar(theme, cx))
            .child(
                div()
                    .id("panel-tree-lists")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .children(lists),
            )
            .child(self.amend_box(theme, cx))
            .child(self.message_box(theme, cx))
            .children(self.problem(theme))
            .child(self.commit_buttons(theme, cx))
            .children(self.discard_dialog(theme, window, cx))
            .children(self.conflict_menu(theme, cx))
            .into_any_element()
    }

    /// The top band: how many files changed on which branch, and the button
    /// that throws all of it away. During an operation, whether its
    /// conflicts are settled yet.
    fn tree_header(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tree = &self.worktree;
        let top = div()
            .flex_none()
            .flex()
            .items_center()
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme::ink(0.03));
        if tree.in_operation() || !tree.conflicted().is_empty() {
            // A stash that did not apply cleanly leaves conflicts with no
            // operation to name.
            let noun = match tree.in_operation() {
                true => format!("{} conflicts", tree.operation.state.noun()),
                false => "Conflicts".to_owned(),
            };
            let (icon, tone, text) = match tree.conflicted().is_empty() {
                true => (icons::CHECK, theme.success, format!("{noun} resolved")),
                false => (
                    icons::DANGER_TRIANGLE,
                    theme.warning,
                    format!("{noun} detected"),
                ),
            };
            return top
                .justify_center()
                .gap(px(6.0))
                .child(
                    icons::icon(icon)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(tone),
                )
                .child(
                    div()
                        .text_size(ui_rems(12.5))
                        .text_color(theme.text)
                        .child(SharedString::from(text)),
                )
                .into_any_element();
        }
        let changed = tree
            .status
            .as_ref()
            .map_or(0, |status| status.entries.len());
        let branch = tree
            .status
            .as_ref()
            .and_then(|status| status.branch.clone())
            .unwrap_or_else(|| "detached HEAD".into());
        let armed = changed > 0 && !tree.busy();
        let danger = theme.danger;
        let shade = match armed {
            true => theme.text_muted,
            false => theme.text_faint.opacity(0.4),
        };
        let discard = div()
            .id("panel-discard-all")
            .group("panel-discard-all")
            .flex_none()
            .size(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .text_color(shade)
            .child(
                // An svg takes no colour from its parent, so it carries the
                // button's, hover included.
                icons::icon(icons::TRASH_BIN_MINIMALISTIC)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(shade)
                    .when(armed, |glyph| {
                        glyph
                            .group_hover("panel-discard-all", move |style| style.text_color(danger))
                    }),
            )
            .tooltip(tip("Discard all changes"))
            .when(armed, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(danger.opacity(0.1)).text_color(danger))
                    .on_click(cx.listener(|panel, _, _, cx| {
                        panel.worktree.asking_to_discard = true;
                        cx.notify();
                    }))
            });
        let files = if changed == 1 { "file" } else { "files" };
        top.gap(px(8.0))
            .child(discard)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("{changed} changed {files} on"))),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(4.0))
                    .rounded(px(3.0))
                    .bg(theme::ink(0.07))
                    .text_size(px(11.0))
                    .text_color(theme.text)
                    .child(SharedString::from(branch)),
            )
            .into_any_element()
    }

    /// A list under a heading that folds it, with its count and maybe a
    /// button.
    fn section(
        &self,
        section: Section,
        title: String,
        button: Option<AnyElement>,
        body: Vec<AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = !self.worktree.folded.contains(&section);
        let chevron = match open {
            true => icons::ALT_ARROW_DOWN,
            false => icons::ALT_ARROW_RIGHT,
        };
        let heading = div()
            .id(SharedString::from(format!("panel-section-{section:?}")))
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .cursor_pointer()
            .child(
                icons::icon(chevron)
                    .size(px(12.0))
                    .flex_none()
                    .text_color(theme.text_faint),
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(SharedString::from(title)),
            )
            .on_click(cx.listener(move |panel, _, _, cx| {
                let folded = &mut panel.worktree.folded;
                if !folded.remove(&section) {
                    folded.insert(section);
                }
                cx.notify();
            }));
        div()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme.border.opacity(0.4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .child(heading)
                    .children(button),
            )
            .when(open, |list| list.children(body))
            .into_any_element()
    }

    /// The unstaged or the staged files, with the button that moves them
    /// all across.
    fn side(
        &self,
        section: Section,
        files: &[DiffFileSummary],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let staged = section == Section::Staged;
        let (title, verb, tone, list) = match staged {
            true => (
                "Staged Files",
                "Unstage All",
                theme.text_muted,
                List::Staged,
            ),
            false => (
                "Unstaged Files",
                "Stage All Changes",
                theme.success,
                List::Unstaged,
            ),
        };
        let live = !self.worktree.busy();
        let button = (!files.is_empty()).then(|| {
            let paths: Vec<String> = files.iter().map(|file| file.path.clone()).collect();
            pill(
                format!("panel-move-all-{section:?}").into(),
                verb,
                tone,
                live,
            )
            .when(live, |pill| {
                pill.on_click(cx.listener(move |panel, _, _, cx| {
                    let paths = paths.clone();
                    let edit = match staged {
                        true => Edit::Unstage(paths),
                        false => Edit::Stage(paths),
                    };
                    panel.edit(edit, cx);
                }))
            })
            .into_any_element()
        });
        let body = match files.is_empty() {
            true => vec![aside("No files.", theme.text_faint)],
            false => self.file_rows(list, files, theme, cx),
        };
        let title = format!("{title} ({})", files.len());
        self.section(section, title, button, body, theme, cx)
    }

    /// The files git could not merge. A click opens the merge editor; a
    /// right-click, the menu that settles one with a side whole.
    fn conflicts(&self, paths: &[String], theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tone = theme.warning;
        let live = !self.worktree.busy();
        let button =
            (!paths.is_empty()).then(|| {
                let all = paths.to_vec();
                pill("panel-resolve-all".into(), "Mark All Resolved", tone, live)
                    .when(live, |pill| {
                        pill.on_click(cx.listener(move |panel, _, _, cx| {
                            panel.edit(Edit::Stage(all.clone()), cx)
                        }))
                    })
                    .into_any_element()
            });
        let mut body: Vec<AnyElement> = paths
            .iter()
            .map(|path| {
                let (open, menu) = (path.clone(), path.clone());
                conflict_row(
                    format!("panel-conflict-{path}"),
                    icons::DANGER_TRIANGLE,
                    tone,
                    path,
                    theme,
                )
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(PanelEvent::OpenConflict(open.clone()))),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |panel, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        panel.worktree.menu.open(ConflictMenu {
                            at: event.position,
                            path: menu.clone(),
                        });
                        cx.notify();
                    }),
                )
                .into_any_element()
            })
            .collect();
        if self.worktree.operation.autostash_pending {
            body.push(aside(
                "Your uncommitted changes were set aside and come back when this ends.",
                theme.text_muted,
            ));
        }
        let title = format!("Conflicted Files ({})", paths.len());
        self.section(Section::Conflicted, title, button, body, theme, cx)
    }

    /// The files that were in conflict and are settled now; any of them can
    /// go back into conflict, markers and all, to settle it again.
    fn resolved(
        &self,
        files: &[DiffFileSummary],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let live = !self.worktree.busy();
        let body = files
            .iter()
            .map(|file| {
                let (open, again) = (file.path.clone(), file.path.clone());
                let undo = live.then(|| {
                    pill(
                        format!("panel-reconflict-{}", file.path).into(),
                        "Mark Conflicted",
                        theme.text_muted,
                        true,
                    )
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        cx.stop_propagation();
                        panel.edit(Edit::Reconflict(again.clone()), cx);
                    }))
                });
                conflict_row(
                    format!("panel-resolved-{}", file.path),
                    icons::CHECK,
                    theme.success,
                    &file.path,
                    theme,
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(PanelEvent::OpenTreeFile {
                        path: open.clone(),
                        staged: true,
                    })
                }))
                .children(undo)
                .into_any_element()
            })
            .collect();
        let title = format!("Resolved Files ({})", files.len());
        self.section(Section::Resolved, title, None, body, theme, cx)
    }

    fn amend_box(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .px(px(10.0))
            .pt(px(8.0))
            .child(
                Checkbox::new("panel-amend")
                    .label("Amend previous commit")
                    .checked(self.worktree.amend)
                    .xsmall()
                    .text_size(px(11.0))
                    .text_color(theme.text_muted)
                    .on_click(cx.listener(|panel, checked: &bool, _, cx| {
                        panel.worktree.amend = *checked;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// The summary, with how many characters remain before most tools cut
    /// it, and the description under it.
    fn message_box(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let typed = self.summary.read(cx).text().chars().count();
        let left = SUMMARY_GUIDE as i64 - typed as i64;
        let counter = div()
            .id("panel-summary-left")
            .flex_none()
            .text_size(px(12.0))
            .text_color(if left < 0 {
                theme.warning
            } else {
                theme.text_faint
            })
            .child(SharedString::from(left.to_string()))
            .tooltip(tip(format!(
                "Most tools show a summary of up to {SUMMARY_GUIDE} characters without cutting it."
            )));
        div()
            .flex_none()
            .m(px(10.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(4.0))
            .border_1()
            .border_color(theme.border.opacity(0.85))
            .bg(theme.surface_raised.opacity(0.7))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(10.0))
                    .pt(px(10.0))
                    .child(div().flex_1().min_w_0().child(self.summary.clone()))
                    .child(counter),
            )
            .child(
                div()
                    .min_h(px(54.0))
                    .px(px(10.0))
                    .pt(px(5.0))
                    .pb(px(8.0))
                    .child(self.description.clone()),
            )
            .into_any_element()
    }

    /// What went wrong last, or what git had to say about it.
    fn problem(&self, theme: &Theme) -> Option<AnyElement> {
        let problem = self.worktree.problem.clone()?;
        Some(
            div()
                .flex_none()
                .px(px(12.0))
                .text_size(px(11.0))
                .text_color(theme.danger_muted)
                .child(problem)
                .into_any_element(),
        )
    }

    /// The commit button; during an operation, the one that finishes it and
    /// the one that aborts it, side by side.
    fn commit_buttons(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let button = |id: &'static str, label: SharedString| {
            div()
                .id(id)
                .h(px(32.0))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .text_size(ui_rems(12.0))
                .child(label)
        };
        let tree = &self.worktree;
        let row = div().flex_none().flex().gap(px(6.0)).p(px(10.0));
        if tree.in_operation() {
            let state = tree.operation.state;
            let out = way_out(state, !tree.conflicted().is_empty());
            let live = out.ready && !tree.busy();
            let (green, red) = (theme.success, theme.danger);
            let action = out.action;
            let finish = button("panel-finish", out.label.into())
                .border_color(green.opacity(if live { 0.7 } else { 0.35 }))
                .bg(green.opacity(if live { 0.28 } else { 0.12 }))
                .text_color(if live { theme.text } else { theme.text_faint })
                .when(live, |button| {
                    button
                        .cursor_pointer()
                        .hover(move |style| style.bg(green.opacity(0.38)))
                        .on_click(
                            cx.listener(move |panel, _, _, cx| panel.settle(action.clone(), cx)),
                        )
                });
            let abort =
                button("panel-abort", format!("Abort {}", state.noun()).into())
                    .border_color(red.opacity(0.8))
                    .bg(red.opacity(0.35))
                    .text_color(theme.text)
                    .when(!tree.busy(), |button| {
                        button
                            .cursor_pointer()
                            .hover(move |style| style.bg(red.opacity(0.5)))
                            .on_click(cx.listener(|panel, _, _, cx| {
                                panel.settle(GitAction::ConflictAbort, cx)
                            }))
                    });
            return row.child(finish).child(abort).into_any_element();
        }
        let (label, ready) = self.commit_button(cx);
        let ready = ready && tree.job != Job::Changing;
        let accent = theme.accent;
        let commit = button("panel-commit", label.into())
            .border_color(theme.border.opacity(0.85))
            .text_size(px(11.5))
            .map(|button| match ready {
                true => button.bg(accent.opacity(0.18)).text_color(theme.text),
                false => button
                    .bg(theme.surface_raised.opacity(0.5))
                    .text_color(theme.text_faint),
            })
            .when(ready, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent.opacity(0.28)))
                    .on_click(cx.listener(|panel, _, _, cx| panel.commit(cx)))
            });
        row.child(commit).into_any_element()
    }

    /// Asking first: throwing every change away cannot be undone.
    fn discard_dialog(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.worktree.asking_to_discard {
            return None;
        }
        let entries = self
            .worktree
            .status
            .as_ref()
            .map_or(&[][..], |status| &status.entries);
        let (untracked, tracked): (Vec<_>, Vec<_>) =
            entries.iter().partition(|entry| entry.untracked);
        let tracked: Vec<String> = tracked.into_iter().map(|e| e.path.clone()).collect();
        let untracked: Vec<String> = untracked.into_iter().map(|e| e.path.clone()).collect();
        let body = match entries.len() {
            1 => "1 file goes back to its last commit".to_owned(),
            n => format!("{n} files go back to their last commit"),
        } + match untracked.is_empty() {
            true => ". This cannot be undone.",
            false => ", and new files are deleted. This cannot be undone.",
        };
        let close = |panel: &mut Self, cx: &mut Context<Self>| {
            panel.worktree.asking_to_discard = false;
            cx.notify();
        };
        let buttons = div()
            .mt(px(16.0))
            .flex()
            .justify_end()
            .gap(px(8.0))
            .child(
                popover::btn_ghost(theme, "Cancel", "panel-discard-cancel")
                    .id("panel-discard-cancel")
                    .on_click(cx.listener(move |panel, _, _, cx| close(panel, cx))),
            )
            .child(
                popover::btn_danger(theme, "Discard All")
                    .id("panel-discard-confirm")
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        close(panel, cx);
                        let (tracked, untracked) = (tracked.clone(), untracked.clone());
                        panel.edit(Edit::Discard { tracked, untracked }, cx);
                    })),
            );
        let card = popover::dialog_card(theme)
            .on_key_down(cx.listener(move |panel, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    close(panel, cx);
                }
            }))
            .child(popover::dialog_title(theme, "Discard all changes?"))
            .child(div().mt(px(6.0)).child(popover::dialog_body(theme, body)))
            .child(buttons);
        Some(popover::modal(
            "panel-discard",
            window.viewport_size(),
            card.into_any_element(),
        ))
    }

    fn close_conflict_menu(&mut self, cx: &mut Context<Self>) {
        self.worktree
            .menu
            .dismiss(cx, |panel| &mut panel.worktree.menu);
    }

    /// The menu over a conflicted file: the merge editor, either side whole
    /// (named by its branch), or marking it resolved as it stands.
    fn conflict_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.worktree.menu.shown()?;
        let (at, path) = (menu.at, menu.path.clone());
        let operation = &self.worktree.operation;
        let ours = operation.current_branch.as_deref().unwrap_or("current");
        let theirs = operation.incoming.as_deref().unwrap_or("incoming");
        let entry = |id: &'static str, label: String| {
            popover::menu_row(theme, false, id)
                .id(id)
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(7.0))
                .text_size(px(12.0))
                .child(SharedString::from(label))
        };
        let apply = |edit: fn(String) -> Edit, path: String| {
            cx.listener(move |panel: &mut Self, _, _, cx| {
                panel.close_conflict_menu(cx);
                panel.edit(edit(path.clone()), cx);
            })
        };
        let open = path.clone();
        let card = popover::popover_card(theme)
            .id("panel-conflict-menu")
            .w(px(248.0))
            .p(px(3.0))
            .rounded(px(10.0))
            .on_mouse_down_out(cx.listener(|panel, _, _, cx| panel.close_conflict_menu(cx)))
            .child(
                entry("panel-conflict-open", "Open in Merge Editor".into()).on_click(cx.listener(
                    move |panel, _, _, cx| {
                        panel.close_conflict_menu(cx);
                        cx.emit(PanelEvent::OpenConflict(open.clone()));
                    },
                )),
            )
            .child(
                entry("panel-conflict-ours", format!("Keep {ours}")).on_click(apply(
                    |path| Edit::Take {
                        path,
                        theirs: false,
                    },
                    path.clone(),
                )),
            )
            .child(
                entry("panel-conflict-theirs", format!("Keep {theirs}")).on_click(apply(
                    |path| Edit::Take { path, theirs: true },
                    path.clone(),
                )),
            )
            .child(
                entry("panel-conflict-resolved", "Mark Resolved".into())
                    .on_click(apply(|path| Edit::Stage(vec![path]), path)),
            );
        Some(popover::menu_at(
            "panel-conflict-menu-at",
            at,
            card.into_any_element(),
            self.worktree.menu.leaving_since(),
        ))
    }
}
