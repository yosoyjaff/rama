//! The toolbar above the graph: undo and redo, pull (with fetch and the pull
//! mode in its menu), push, stash and pop.
//!
//! It is a strip across the top rather than something floating over the
//! rows, which would cover the newest commits, the very ones these buttons
//! act on.

use gpui_kit::{
    AnyElement, Context, Div, ElementId, Hsla, MouseButton, MouseDownEvent, SharedString, Stateful,
    div, prelude::*, px,
};

use super::HistoryView;
use crate::git::actions::GitAction;
use crate::settings::{self, Pull, SavePolicy};
use crate::ui::icons;
use crate::ui::popover;
use crate::ui::theme::{self, Theme};
use crate::ui::tip::tip;

const BUTTON_HEIGHT: f32 = 28.0;
const LABEL_SIZE: f32 = 12.5;

/// A toolbar command. One runs at a time: they all move the same refs, and
/// two at once would leave the graph showing neither result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Undo,
    Redo,
    Fetch,
    Pull,
    Push,
    Stash,
    Pop,
}

/// The bar, left to right, a divider between groups. Fetch has no button
/// of its own: it is in the pull menu.
const GROUPS: [&[Tool]; 3] = [
    &[Tool::Undo, Tool::Redo],
    &[Tool::Pull, Tool::Push],
    &[Tool::Stash, Tool::Pop],
];

/// How a tool presents itself.
struct Face {
    icon: &'static str,
    label: &'static str,
    hint: &'static str,
}

impl Tool {
    fn face(self) -> Face {
        let (icon, label, hint) = match self {
            Self::Undo => (
                icons::UNDO,
                "Undo",
                "Take back the last commit; its changes stay in the working tree",
            ),
            Self::Redo => (
                icons::REDO,
                "Redo",
                "Put back the commit that was just taken back",
            ),
            Self::Fetch => (
                icons::REFRESH,
                "Fetch all",
                "Get every remote's new commits and refs, merging nothing",
            ),
            Self::Pull => (
                icons::ARROW_DOWN,
                "Pull",
                "Bring this branch up to date with its remote",
            ),
            Self::Push => (
                icons::ARROW_UP,
                "Push",
                "Publish this branch's new commits on its remote",
            ),
            Self::Stash => (
                icons::ARCHIVE_MINIMALISTIC,
                "Stash",
                "Set the working tree's changes aside, leaving it clean",
            ),
            Self::Pop => (
                icons::ARCHIVE_UP_MINIMALISTIC,
                "Pop",
                "Take back the changes set aside last",
            ),
        };
        Face { icon, label, hint }
    }

    fn id(self) -> ElementId {
        ElementId::Name(format!("toolbar-{self:?}").to_lowercase().into())
    }

    /// The git action it runs and what to say once it worked. Fetch is not
    /// one: it has its own path, which also refreshes the remote refs.
    fn action(self, pull: Pull) -> Option<(GitAction, &'static str)> {
        let action = match self {
            Self::Fetch => return None,
            Self::Undo => GitAction::Undo,
            Self::Redo => GitAction::Redo,
            Self::Pull => GitAction::Pull { mode: pull.into() },
            Self::Push => GitAction::Push,
            // As plain `git stash`: tracked changes only. Untracked files are
            // the ones no reflog could give back.
            Self::Stash => GitAction::StashPush {
                message: None,
                include_untracked: false,
            },
            // The newest entry, as plain `git stash pop`.
            Self::Pop => GitAction::StashPop { index: 0 },
        };
        let said = match (self, pull) {
            (Self::Undo, _) => "Last commit taken back",
            (Self::Redo, _) => "Commit put back",
            (Self::Pull, Pull::Merge) => "Pulled",
            (Self::Pull, Pull::FastForwardOnly) => "Pulled, fast-forward",
            (Self::Pull, Pull::Rebase) => "Pulled, rebased",
            (Self::Push, _) => "Pushed",
            (Self::Stash, _) => "Changes set aside",
            _ => "Stash applied",
        };
        Some((action, said))
    }
}

/// Lit while it runs, faint while something else does, muted otherwise.
fn ink(running_any: bool, running_this: bool, theme: &Theme) -> Hsla {
    match (running_this, running_any) {
        (true, _) => theme.text,
        (false, true) => theme.text_faint,
        (false, false) => theme.text_muted,
    }
}

fn glyph(name: &'static str, size: f32, color: Hsla) -> impl IntoElement {
    icons::icon(name).size(px(size)).text_color(color)
}

/// The rounded shape every toolbar button shares; it only reacts to the
/// pointer while nothing runs.
fn shape(id: impl Into<ElementId>, idle: bool, color: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(BUTTON_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(LABEL_SIZE))
        .text_color(color)
        .when(idle, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(theme::ink(0.06)))
        })
}

impl HistoryView {
    fn busy(&self) -> bool {
        self.git_running || self.fetch.running
    }

    fn use_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let pull = settings::current(cx).history.pull;
        match tool.action(pull) {
            None => {
                self.fetch_remotes(cx);
                cx.notify();
            }
            // Its button stays lit until the action is over.
            Some((action, said)) => {
                self.lit_button = Some(tool);
                self.run_git(action, Some(said.to_owned()), cx);
            }
        }
    }

    pub(super) fn hide_pull_options(&mut self, cx: &mut Context<Self>) {
        self.popups
            .pull
            .dismiss(cx, |history| &mut history.popups.pull);
    }

    pub(super) fn render_toolbar(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div()
            .flex_none()
            .w_full()
            .h(px(42.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .border_b_1()
            .border_color(theme::hairline(0.06))
            // Equal room at both ends keeps the buttons centred.
            .child(div().flex_1().min_w_0());
        for (at, group) in GROUPS.iter().enumerate() {
            if at > 0 {
                bar = bar.child(
                    div()
                        .flex_none()
                        .w(px(1.0))
                        .h(px(18.0))
                        .mx(px(2.0))
                        .bg(theme.border.opacity(0.6)),
                );
            }
            for &tool in *group {
                bar = bar.child(match tool {
                    Tool::Pull => self.pull_button(theme, cx),
                    tool => self.tool_button(tool, theme, cx),
                });
            }
        }
        bar.child(div().flex_1().min_w_0()).into_any_element()
    }

    fn tool_button(&self, tool: Tool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let face = tool.face();
        let lit = self.lit_button == Some(tool);
        let color = ink(self.busy(), lit, theme);
        shape(tool.id(), !self.busy(), color)
            .px(px(11.0))
            .rounded_full()
            .when(lit, |button| button.bg(theme::ink(0.07)))
            .child(glyph(face.icon, 14.0, color))
            .child(face.label)
            .tooltip(tip(face.hint))
            .on_click(cx.listener(move |history, _, _, cx| {
                cx.stop_propagation();
                history.use_tool(tool, cx);
            }))
            .into_any_element()
    }

    /// Pull in two halves: the label pulls in the saved mode, the chevron
    /// opens Fetch all and the modes. A fetch lights it too, having no
    /// button of its own.
    fn pull_button(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let idle = !self.busy();
        let lit = matches!(self.lit_button, Some(Tool::Pull | Tool::Fetch)) || self.fetch.running;
        let color = ink(self.busy(), lit, theme);
        let face = Tool::Pull.face();
        let mode = settings::current(cx).history.pull;
        let pull = shape(Tool::Pull.id(), idle, color)
            .pl(px(11.0))
            .pr(px(5.0))
            .rounded_l(px(BUTTON_HEIGHT / 2.0))
            .child(glyph(face.icon, 14.0, color))
            .child(SharedString::from(mode.button_label()))
            .tooltip(tip(face.hint))
            .when(idle, |pull| {
                pull.on_click(cx.listener(|history, _, _, cx| {
                    cx.stop_propagation();
                    history.use_tool(Tool::Pull, cx);
                }))
            });
        let options = shape("toolbar-pull-options", idle, color)
            .pl(px(2.0))
            .pr(px(9.0))
            .rounded_r(px(BUTTON_HEIGHT / 2.0))
            .child(glyph(icons::ALT_ARROW_DOWN, 10.0, theme.text_faint))
            .when(idle, |options| {
                options.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|history, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                        history.close_context_menu(cx);
                        history.popups.pull.open(event.position);
                        cx.notify();
                    }),
                )
            });
        div()
            .flex_none()
            .flex()
            .items_center()
            .child(pull)
            .child(options)
            .into_any_element()
    }

    /// Fetch all, a rule, then each pull mode, the saved one ticked.
    pub(super) fn render_pull_options(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let saved = settings::current(cx).history.pull;
        let row = |key: SharedString| {
            popover::menu_row(theme, false, key.clone())
                .id(ElementId::Name(key))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .text_size(px(LABEL_SIZE))
        };
        let fetch = Tool::Fetch.face();
        let fetch = row("toolbar-fetch-all".into())
            .gap(px(6.0))
            .child(glyph(fetch.icon, 13.0, theme.text_muted))
            .child(div().flex_1().child(fetch.label))
            .tooltip(tip(fetch.hint))
            .on_click(cx.listener(|history, _, _, cx| {
                cx.stop_propagation();
                history.hide_pull_options(cx);
                history.use_tool(Tool::Fetch, cx);
            }));
        let modes = Pull::EVERY.into_iter().map(|mode| {
            let tick = (mode == saved).then(|| glyph(icons::CHECK, 11.0, theme.text_muted));
            row(format!("toolbar-pull-{mode:?}").into())
                .child(div().flex_1().child(mode.menu_label()))
                .child(
                    div()
                        .flex_none()
                        .w(px(12.0))
                        .flex()
                        .justify_end()
                        .children(tick),
                )
                .on_click(cx.listener(move |history, _, _, cx| {
                    cx.stop_propagation();
                    settings::update(SavePolicy::Immediate, cx, |s| s.history.pull = mode);
                    history.hide_pull_options(cx);
                }))
        });
        popover::popover_card(theme)
            .w(px(264.0))
            .p(px(4.0))
            .rounded(px(9.0))
            .on_mouse_down_out(cx.listener(|history, _, _, cx| history.hide_pull_options(cx)))
            .child(fetch)
            .child(popover::menu_separator())
            .children(modes)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::PullMode;

    #[test]
    fn the_bar_shows_every_tool_but_fetch_once() {
        let shown: Vec<Tool> = GROUPS
            .iter()
            .flat_map(|group| group.iter().copied())
            .collect();
        assert_eq!(shown.len(), 6);
        assert!(!shown.contains(&Tool::Fetch));
        let mut ids: Vec<String> = shown
            .iter()
            .map(|tool| format!("{:?}", tool.id()))
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), shown.len(), "ids are unique");
    }

    #[test]
    fn every_tool_but_fetch_runs_git_and_says_so() {
        for tool in [
            Tool::Undo,
            Tool::Redo,
            Tool::Pull,
            Tool::Push,
            Tool::Stash,
            Tool::Pop,
        ] {
            let (_, said) = tool.action(Pull::Merge).expect("a git action");
            assert!(!said.is_empty(), "{tool:?}");
        }
        assert!(Tool::Fetch.action(Pull::Merge).is_none());
        assert!(matches!(
            Tool::Pull.action(Pull::Rebase),
            Some((
                GitAction::Pull {
                    mode: PullMode::Rebase
                },
                _
            ))
        ));
    }

    #[test]
    fn pull_modes_reach_git_and_fit_the_button() {
        let modes: Vec<PullMode> = Pull::EVERY.into_iter().map(PullMode::from).collect();
        assert_eq!(
            modes,
            [PullMode::FfPossible, PullMode::FfOnly, PullMode::Rebase]
        );
        assert!(
            Pull::EVERY
                .iter()
                .all(|mode| mode.button_label().len() <= 10)
        );
    }
}
