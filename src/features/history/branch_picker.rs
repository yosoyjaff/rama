//! The branch selector above the graph: the branch that is out, and a
//! dropdown to check out another one.
//!
//! The trigger is a small "branch" caption over the branch
//! name and a caret. It opens a card with a search field and every local
//! branch; the one that is out is highlighted, and picking another checks it
//! out.

use std::path::Path;

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};

use crate::git::actions::GitAction;
use crate::ui::popover::{self, Layer};
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme::{Theme, wash};
use crate::ui::typography::ui_rems;
use crate::ui::{icons, motion};

/// Width of the dropdown card.
const CARD_WIDTH: f32 = 272.0;
/// Past this the list scrolls.
const LIST_MAX_HEIGHT: f32 = 480.0;

pub enum BranchPickerEvent {
    /// A branch was checked out: everything reading this checkout is behind.
    RepositoryChanged,
    /// The checkout failed, with git's own words.
    ActionFailed(SharedString),
}

pub struct BranchPicker {
    repo: Option<String>,
    /// The branch that is out, as the graph read it.
    branch: SharedString,
    menu: Layer<()>,
    input: Entity<TextInput>,
    /// Local branches, read when the menu opens.
    branches: Vec<String>,
    busy: bool,
    load_task: Option<Task<()>>,
    git_task: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<BranchPickerEvent> for BranchPicker {}

impl BranchPicker {
    pub fn new(repo: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search", window, cx));
        let subscription = cx.subscribe(&input, |picker: &mut Self, _, event, cx| match event {
            TextInputEvent::Edited => cx.notify(),
            // Enter takes the first match, the way a filtered list reads.
            TextInputEvent::Submitted => {
                if let Some(name) = picker.matches(cx).first().cloned() {
                    picker.checkout(name, cx);
                }
            }
            TextInputEvent::Blurred => {}
        });
        Self {
            repo,
            branch: SharedString::default(),
            menu: Layer::default(),
            input,
            branches: Vec::new(),
            busy: false,
            load_task: None,
            git_task: None,
            _subscription: subscription,
        }
    }

    /// Point the picker at another checkout.
    pub fn open_repo(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        self.repo = repo;
        self.branches.clear();
        self.close(cx);
    }

    /// The branch that is out, from the graph.
    pub fn set_branch(&mut self, branch: SharedString, cx: &mut Context<Self>) {
        if self.branch != branch {
            self.branch = branch;
            cx.notify();
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu.open(());
        self.input
            .update(cx, |input, cx| input.set_text("", window, cx));
        let focus = self.input.read(cx).focus_handle(cx);
        focus.focus(window, cx);
        self.load(cx);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.menu.dismiss(cx, |picker| &mut picker.menu);
        cx.notify();
    }

    /// Read the local branches. The list is small and changes under the
    /// application's feet, so it is read on every open.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let refs = cx
                .background_executor()
                .spawn(async move { crate::git::repo_refs(Path::new(&repo)) })
                .await;
            this.update(cx, |picker, cx| {
                if let Ok(refs) = refs {
                    picker.branches = refs.local.into_iter().map(|branch| branch.name).collect();
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn matches(&self, cx: &gpui_kit::App) -> Vec<String> {
        let query = self.input.read(cx).text().trim().to_lowercase();
        self.branches
            .iter()
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    fn checkout(&mut self, name: String, cx: &mut Context<Self>) {
        self.close(cx);
        if name.as_str() == self.branch.as_ref() || self.busy {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.busy = true;
        self.git_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::git::actions::run(Path::new(&repo), GitAction::Checkout { name })
                })
                .await;
            this.update(cx, |picker, cx| {
                picker.busy = false;
                match result {
                    Ok(_) => cx.emit(BranchPickerEvent::RepositoryChanged),
                    Err(error) => {
                        cx.emit(BranchPickerEvent::ActionFailed(error.to_string().into()))
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn render_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = self.branch.clone();
        let rows = self.matches(cx).into_iter().map(|name| {
            let active = name.as_str() == current.as_ref();
            let key = SharedString::from(format!("history-branch-picker-{name}"));
            let pick = name.clone();
            popover::menu_row(theme, false, key.clone())
                .id(gpui_kit::ElementId::Name(key))
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(5.0))
                .rounded(px(4.0))
                .text_size(ui_rems(12.5))
                .when(active, |row| row.bg(theme.success.opacity(0.28)))
                .on_click(cx.listener(move |picker, _, _, cx| {
                    cx.stop_propagation();
                    picker.checkout(pick.clone(), cx);
                }))
                .child(
                    icons::icon(icons::GIT_BRANCH)
                        .size(px(13.0))
                        .text_color(theme.text_muted),
                )
                .child(div().min_w_0().truncate().child(SharedString::from(name)))
        });
        popover::popover_card(theme)
            .w(px(CARD_WIDTH))
            .p(px(4.0))
            .on_mouse_down_out(cx.listener(|picker, _, _, cx| picker.close(cx)))
            .child(
                div()
                    .mb(px(4.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.accent.opacity(0.8))
                    .child(popover::palette_search_icon(theme))
                    .child(div().flex_1().min_w_0().child(self.input.clone())),
            )
            .child(
                div()
                    .id("history-branch-picker-list")
                    .max_h(px(LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .into_any_element()
    }
}

impl Render for BranchPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let menu = self
            .menu
            .shown()
            .map(|_| (self.render_menu(&theme, cx), self.menu.leaving_since()));
        let open = self.menu.is_open();
        div()
            .id("history-branch-picker")
            .relative()
            .flex_none()
            .max_w(px(260.0))
            .min_w_0()
            .flex()
            .flex_col()
            .px(px(6.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .when(open, |trigger| trigger.bg(wash(0.10)))
            .when(!open, |trigger| {
                trigger
                    .bg(motion::fade_color(
                        "history-branch-picker",
                        wash(0.0),
                        wash(0.10),
                    ))
                    .on_hover(motion::on_hover_fade("history-branch-picker"))
            })
            .occlude()
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
                window.prevent_default()
            })
            .on_click(cx.listener(|picker, _, window, cx| {
                cx.stop_propagation();
                if picker.menu.is_open() {
                    picker.close(cx);
                } else {
                    picker.open(window, cx);
                }
            }))
            .child(
                div()
                    .text_size(ui_rems(9.5))
                    .line_height(px(11.0))
                    .text_color(theme.text_muted)
                    .child("branch"),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_rems(13.0))
                            .line_height(px(17.0))
                            .text_color(if self.busy {
                                theme.text_muted
                            } else {
                                theme.text
                            })
                            .child(self.branch.clone()),
                    )
                    .child(
                        icons::icon(if open {
                            icons::ALT_ARROW_UP
                        } else {
                            icons::ALT_ARROW_DOWN
                        })
                        .size(px(11.0))
                        .text_color(theme.text_muted),
                    ),
            )
            .when_some(menu, |trigger, (menu, closing)| {
                trigger.child(popover::anchored_menu_below(
                    "history-branch-picker-menu",
                    menu,
                    closing,
                ))
            })
    }
}
