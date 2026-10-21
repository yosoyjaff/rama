//! The screen the application opens on: open a repository, clone one, or
//! start one — and the list of the ones opened before.
//!
//! It is a screen rather than a dialog because it is where the product begins:
//! with nothing open there is nothing else to show, and with something open it
//! is still how another project is reached.
//!
//! The three actions are three modes of the same card stack. Cloning and
//! creating ask for what they need in place, instead of opening a dialog over
//! a screen that is already doing nothing else.

use std::path::{Path, PathBuf};

use gpui_kit::base::Button;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, PathPromptOptions, Render, SharedString, Subscription, Task, Window,
    div, prelude::*, px,
};

use crate::recents::{self, RecentEntry};
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme::{Theme, wash};
use crate::ui::typography::ui_rems;
use crate::ui::{icons, motion};

/// What the screen asks the window to do.
pub enum WelcomeEvent {
    /// Open this checkout, in a tab of its own.
    OpenRepository(String),
    /// Import the tabs GitKraken had open: `(paths, the one on screen)`.
    ImportGitKraken {
        paths: Vec<String>,
        active: Option<String>,
    },
}

/// Which of the three actions the screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Main,
    Clone,
    Create,
}

pub struct Welcome {
    mode: Mode,
    /// Paths already open, so a row can say so instead of opening twice.
    open_paths: Vec<String>,
    clone_url: Entity<TextInput>,
    clone_target: Entity<TextInput>,
    create_path: Entity<TextInput>,
    error: Option<SharedString>,
    busy: bool,
    /// A folder picker is up. A second click must not stack another one
    /// over it: the platform dialog is not modal to this window everywhere.
    picking: bool,
    task: Option<Task<()>>,
    focus_handle: FocusHandle,
    _inputs: Vec<Subscription>,
}

impl EventEmitter<WelcomeEvent> for Welcome {}

impl Welcome {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clone_url = cx.new(|cx| {
            TextInput::new("https://github.com/user/repo.git", window, cx).with_text_size(12.0)
        });
        let clone_target =
            cx.new(|cx| TextInput::new("Folder to clone into", window, cx).with_text_size(12.0));
        let create_path = cx.new(|cx| {
            TextInput::new("Folder for the new repository", window, cx).with_text_size(12.0)
        });
        // A redraw per keystroke: the buttons below are enabled by what these
        // hold, so the screen has to follow them. Enter in a field is the
        // form's button.
        let inputs = vec![
            cx.subscribe(&clone_url, Self::on_clone_field),
            cx.subscribe(&clone_target, Self::on_clone_field),
            cx.subscribe(&create_path, |welcome, _, event, cx| {
                if *event == TextInputEvent::Submitted {
                    welcome.create_repository(cx);
                }
                cx.notify();
            }),
        ];
        Self {
            mode: Mode::Main,
            open_paths: Vec::new(),
            clone_url,
            clone_target,
            create_path,
            error: None,
            busy: false,
            picking: false,
            task: None,
            focus_handle: cx.focus_handle(),
            _inputs: inputs,
        }
    }

    fn on_clone_field(
        &mut self,
        _: Entity<TextInput>,
        event: &TextInputEvent,
        cx: &mut Context<Self>,
    ) {
        if *event == TextInputEvent::Submitted {
            self.clone_repository(cx);
        }
        cx.notify();
    }

    /// Switch to one of the forms, or back, with the keyboard where the form
    /// starts: its first field.
    fn show(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.error = None;
        match mode {
            Mode::Main => window.focus(&self.focus_handle, cx),
            Mode::Clone => {
                let handle = self.clone_url.read(cx).focus_handle(cx);
                window.focus(&handle, cx)
            }
            Mode::Create => {
                let handle = self.create_path.read(cx).focus_handle(cx);
                window.focus(&handle, cx)
            }
        }
        cx.notify();
    }

    /// Which checkouts are open, so the recent list can mark them.
    pub fn set_open_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if self.open_paths != paths {
            self.open_paths = paths;
            cx.notify();
        }
    }

    /// Ask the platform for a folder, then open whatever it points at.
    fn pick_repository(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.picking {
            return;
        }
        self.picking = true;
        self.error = None;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            // On Linux the picker is a desktop portal, and a machine without
            // one fails here rather than cancelling. Say so: silence reads as
            // a dead button.
            let answer = paths.await;
            this.update(cx, |welcome, _| welcome.picking = false).ok();
            let picked = match answer {
                Ok(Ok(picked)) => picked,
                Ok(Err(error)) => {
                    this.update(cx, |welcome, cx| {
                        welcome.error = Some(format!("no folder picker: {error}").into());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                // The prompt was dropped with the window: nothing to report.
                Err(_) => return,
            };
            let Some(path) = picked.and_then(|paths| paths.into_iter().next()) else {
                return;
            };
            // The folder the user picked may be *inside* a checkout; git's own
            // answer to "which repository is this" is what opens the root
            // rather than a subdirectory that behaves almost like one.
            let root = cx
                .background_executor()
                .spawn(async move { crate::git::open::repository_at(&path).ok_or(path) })
                .await;
            this.update(cx, |welcome, cx| match root {
                Ok(root) => welcome.open(PathBuf::from(root), cx),
                Err(path) => {
                    welcome.error =
                        Some(format!("{} is not a git repository", path.display()).into());
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Ask for a folder and put its path in one of the fields.
    fn pick_folder(&mut self, field: Entity<TextInput>, cx: &mut Context<Self>) {
        if self.picking {
            return;
        }
        self.picking = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose folder".into()),
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let picked = paths.await.ok().and_then(Result::ok).flatten();
            this.update(cx, |welcome, _| welcome.picking = false).ok();
            let Some(path) = picked.and_then(|paths| paths.into_iter().next()) else {
                return;
            };
            this.update_in(cx, |_, window, cx| {
                field.update(cx, |input, cx| {
                    input.set_text(path.to_string_lossy().into_owned(), window, cx)
                });
            })
            .ok();
        }));
    }

    fn clone_repository(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let url = self.clone_url.read(cx).text().trim().to_string();
        let target = self.clone_target.read(cx).text().trim().to_string();
        if url.is_empty() || target.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::git::create::clone(&url, Path::new(&target)) })
                .await;
            this.update(cx, |welcome, cx| {
                welcome.busy = false;
                welcome.finish(result, cx);
            })
            .ok();
        }));
    }

    fn create_repository(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let path = self.create_path.read(cx).text().trim().to_string();
        if path.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::git::create::init(Path::new(&path)) })
                .await;
            this.update(cx, |welcome, cx| {
                welcome.busy = false;
                welcome.finish(result, cx);
            })
            .ok();
        }));
    }

    fn finish(&mut self, result: anyhow::Result<PathBuf>, cx: &mut Context<Self>) {
        match result {
            Ok(path) => {
                self.mode = Mode::Main;
                self.open(path, cx);
            }
            Err(error) => self.error = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = path.to_string_lossy().into_owned();
        recents::add(&path, cx);
        cx.emit(WelcomeEvent::OpenRepository(path));
        cx.notify();
    }

    /// One of the three things that can be done with nothing open: a verb
    /// with an ellipsis, because each asks for more before it acts, and the
    /// object it acts on beneath.
    #[allow(clippy::too_many_arguments)]
    fn action_button(
        &self,
        id: &'static str,
        icon: &'static str,
        label: &'static str,
        object: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        Button::new(id)
            .accessibility_label(label)
            .flex_1()
            .min_w_0()
            .justify_start()
            .gap_2()
            .px_3()
            .py(px(10.0))
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(motion::fade_color(id, wash(0.02), wash(0.08)))
            .on_hover(motion::on_hover_fade(id))
            .focus_visible(|style| style.border_color(theme.accent))
            .on_click(cx.listener(move |welcome, _, window, cx| on_click(welcome, window, cx)))
            .child(
                div()
                    .size(px(28.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(theme.accent.opacity(0.12))
                    .child(icons::icon(icon).size(px(15.0)).text_color(theme.accent)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(ui_rems(12.5))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(label),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(object),
                    ),
            )
            .into_any_element()
    }

    /// One row of the recent list: the checkout, where it lives, and when it
    /// was last opened — or that it is open now, in which case the row goes
    /// to its tab.
    fn recent_row(
        &self,
        entry: RecentEntry,
        now: i64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let already_open = self.open_paths.contains(&entry.path);
        let id = SharedString::from(format!("welcome-recent-{}", entry.path));
        let open_path = entry.path.clone();
        let forget_key_path = entry.path.clone();
        let forget_path = entry.path.clone();
        let location = recents::display_path(&entry.path);
        Button::new(id.clone())
            .group(id.clone())
            .accessibility_label(SharedString::from(format!("{} — {location}", entry.name)))
            .w_full()
            .justify_start()
            .gap_2()
            .px(px(10.0))
            .py(px(6.0))
            .rounded_md()
            .border_1()
            .border_color(theme.border.opacity(0.0))
            .bg(motion::fade_color(&id, wash(0.0), wash(0.07)))
            .on_hover(motion::on_hover_fade(&id))
            .focus_visible(|style| style.border_color(theme.accent))
            .on_click(cx.listener(move |welcome, _, _, cx| {
                welcome.open(PathBuf::from(open_path.clone()), cx)
            }))
            // Delete forgets the focused row: the keyboard's way to the
            // button that only shows itself under the pointer.
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "delete" | "backspace") {
                    cx.stop_propagation();
                    recents::remove(&forget_key_path, cx);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .size(px(16.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icons::icon(icons::GIT_BRANCH).size(px(13.0)).text_color(
                        if already_open {
                            theme.accent
                        } else {
                            theme.text_muted
                        },
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(12.5))
                            .text_color(theme.text)
                            .child(SharedString::from(entry.name.clone())),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_rems(10.5))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(location)),
                    ),
            )
            // A fixed trailing lane, so the times line up down the list.
            .child(
                div()
                    .w(px(72.0))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .text_size(ui_rems(10.5))
                    .child(if already_open {
                        div()
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(theme.accent.opacity(0.14))
                            .text_color(theme.accent)
                            .child("Open")
                    } else {
                        div()
                            .text_color(theme.text_faint)
                            .child(SharedString::from(relative_time(entry.opened_at, now)))
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("welcome-forget-{}", entry.path)))
                    .accessibility_label("Remove from Recent")
                    // The row takes the keyboard (Delete); this is the
                    // pointer's shortcut to the same thing.
                    .focusable(false)
                    .flex_none()
                    .size(px(20.0))
                    .rounded(px(4.0))
                    .opacity(0.0)
                    .group_hover(id, |style| style.opacity(0.8))
                    .hover(|style| style.bg(wash(0.12)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        recents::remove(&forget_path, cx);
                        cx.notify();
                    }))
                    .child(
                        icons::icon(icons::CLOSE)
                            .size(px(10.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .into_any_element()
    }

    /// A labelled field with an optional Browse button beside it.
    fn field(
        &self,
        label: &'static str,
        input: Entity<TextInput>,
        browse: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let browse_target = input.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(ui_rems(11.0))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(label),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(30.0))
                            .flex()
                            .items_center()
                            .px(px(8.0))
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.input_bg)
                            .child(input),
                    )
                    .when(browse, |row| {
                        row.child(
                            Button::new(SharedString::from(format!("welcome-browse-{label}")))
                                .h(px(30.0))
                                .px(px(10.0))
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .hover(|style| style.bg(wash(0.08)))
                                .focus_visible(|style| style.border_color(theme.accent))
                                .text_size(ui_rems(11.5))
                                .text_color(theme.text_muted)
                                .on_click(cx.listener(move |welcome, _, _, cx| {
                                    welcome.pick_folder(browse_target.clone(), cx)
                                }))
                                .child("Browse…"),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The button that commits a form.
    fn primary_button(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        Button::new(id)
            .disabled(!enabled)
            .h(px(30.0))
            .px(px(14.0))
            .rounded_md()
            .border_1()
            .border_color(theme.border.opacity(0.0))
            .text_size(ui_rems(12.0))
            .bg(if enabled {
                theme.accent_strong
            } else {
                wash(0.08)
            })
            .text_color(if enabled {
                theme.on_accent
            } else {
                theme.text_faint
            })
            .focus_visible(|style| style.border_color(theme.text))
            .on_click(cx.listener(move |welcome, _, _, cx| on_click(welcome, cx)))
            .child(label)
            .into_any_element()
    }

    /// A form's way back, and its title: what the form does.
    fn form_header(
        &self,
        title: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .child(
                Button::new("welcome-back")
                    .accessibility_label("Back")
                    .gap(px(4.0))
                    .px(px(6.0))
                    .py(px(3.0))
                    .ml(px(-6.0))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border.opacity(0.0))
                    .text_size(ui_rems(11.0))
                    .text_color(theme.text_muted)
                    .hover(|style| style.text_color(theme.text).bg(wash(0.06)))
                    .focus_visible(|style| style.border_color(theme.accent))
                    .on_click(
                        cx.listener(|welcome, _, window, cx| welcome.show(Mode::Main, window, cx)),
                    )
                    .child("← Back"),
            )
            .child(
                div()
                    .text_size(ui_rems(18.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(title),
            )
            .into_any_element()
    }

    fn render_main(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let actions = div()
            .w_full()
            .flex()
            .flex_row()
            .gap_2()
            .child(self.action_button(
                "welcome-open",
                icons::FOLDER_WITH_FILES,
                "Open…",
                "A local repository",
                theme,
                cx,
                |welcome, _, cx| welcome.pick_repository(cx),
            ))
            .child(self.action_button(
                "welcome-clone",
                icons::CLOUD,
                "Clone…",
                "From a URL",
                theme,
                cx,
                |welcome, window, cx| welcome.show(Mode::Clone, window, cx),
            ))
            .child(self.action_button(
                "welcome-create",
                icons::PLUS,
                "Create…",
                "In a new folder",
                theme,
                cx,
                |welcome, window, cx| welcome.show(Mode::Create, window, cx),
            ));

        let now = chrono::Utc::now().timestamp();
        let rows: Vec<AnyElement> = recents::all(cx)
            .into_iter()
            .map(|entry| self.recent_row(entry, now, theme, cx))
            .collect();
        let recent = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px(px(10.0))
                    .text_size(ui_rems(11.5))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(theme.text_muted)
                    .child("Recent"),
            )
            .child(if rows.is_empty() {
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child("Repositories you open will appear here.")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .children(rows)
                    .into_any_element()
            });

        // GitKraken had checkouts open? One click brings them in — a
        // FIRST-TIME offer: once imported, the entry leaves the welcome (the
        // settings screen keeps its own).
        let imported_before = crate::settings::current(cx).gitkraken_imported;
        let kraken = recents::gitkraken_repos();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(ui_rems(18.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(SharedString::from(format!(
                                "Welcome to {}",
                                crate::APP_NAME
                            ))),
                    )
                    .child(actions),
            )
            .child(recent)
            .when(!imported_before && !kraken.is_empty(), |body| {
                let count = kraken.len();
                let active = kraken
                    .iter()
                    .find_map(|(path, on_screen)| on_screen.then(|| path.clone()));
                let paths: Vec<String> = kraken.into_iter().map(|(path, _)| path).collect();
                body.child(
                    Button::new("welcome-import-gitkraken")
                        .w_full()
                        .justify_start()
                        .gap(px(6.0))
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded_md()
                        .border_1()
                        .border_color(theme.border.opacity(0.7))
                        .hover(|style| style.bg(crate::ui::theme::ink(0.04)))
                        .focus_visible(|style| style.border_color(theme.accent))
                        .on_click(cx.listener(move |welcome, _, _, cx| {
                            welcome.error = None;
                            cx.emit(WelcomeEvent::ImportGitKraken {
                                paths: paths.clone(),
                                active: active.clone(),
                            });
                        }))
                        .child(
                            icons::icon(icons::GIT_BRANCH)
                                .size(px(13.0))
                                .text_color(theme.text_muted),
                        )
                        .child(
                            div()
                                .text_size(ui_rems(11.5))
                                .text_color(theme.text)
                                .child("Import from GitKraken"),
                        )
                        .child(
                            div()
                                .text_size(ui_rems(10.5))
                                .text_color(theme.text_muted)
                                .child(SharedString::from(format!(
                                    "{count} repositories from its open tabs"
                                ))),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Focusable for Welcome {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Welcome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        // The page fills the window's bottom edge.
        let corner = crate::ui::window_frame::corner_radius(window);

        let body: AnyElement = match self.mode {
            Mode::Main => self.render_main(&theme, cx),
            Mode::Clone => {
                let ready = !self.clone_url.read(cx).text().trim().is_empty()
                    && !self.clone_target.read(cx).text().trim().is_empty()
                    && !self.busy;
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.form_header("Clone a repository", &theme, cx))
                    .child(self.field("Repository URL", self.clone_url.clone(), false, &theme, cx))
                    .child(self.field("Clone into", self.clone_target.clone(), true, &theme, cx))
                    .child(div().flex().child(self.primary_button(
                        "welcome-clone-run",
                        if self.busy { "Cloning…" } else { "Clone" },
                        ready,
                        &theme,
                        cx,
                        |welcome, cx| welcome.clone_repository(cx),
                    )))
                    .into_any_element()
            }
            Mode::Create => {
                let ready = !self.create_path.read(cx).text().trim().is_empty() && !self.busy;
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.form_header("Create a repository", &theme, cx))
                    .child(self.field("Folder", self.create_path.clone(), true, &theme, cx))
                    .child(div().flex().child(self.primary_button(
                        "welcome-create-run",
                        if self.busy { "Creating…" } else { "Create" },
                        ready,
                        &theme,
                        cx,
                        |welcome, cx| welcome.create_repository(cx),
                    )))
                    .into_any_element()
            }
        };

        div()
            .id("welcome")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .rounded_b(px(corner))
            .bg(theme.bg)
            // Escape leaves a form, unless a busy clone is still running.
            .on_key_down(cx.listener(|welcome, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && welcome.mode != Mode::Main && !welcome.busy {
                    welcome.show(Mode::Main, window, cx);
                }
            }))
            .child(
                // Centred while it fits; scrolls from the top when it does not.
                div()
                    .min_h_full()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(560.0))
                            .max_w_full()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .p_8()
                            .child(body)
                            .children(self.error.clone().map(|error| {
                                div()
                                    .w_full()
                                    .px(px(10.0))
                                    .py(px(6.0))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme.danger.opacity(0.4))
                                    .bg(theme.danger.opacity(0.08))
                                    .text_size(ui_rems(11.0))
                                    .text_color(theme.danger)
                                    .child(error)
                            })),
                    ),
            )
    }
}

/// How long ago a checkout was opened, as a recent list says it: minutes,
/// hours, days, then the date itself.
fn relative_time(opened_at: i64, now: i64) -> String {
    let seconds = (now - opened_at).max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 1 {
        "Just now".into()
    } else if hours < 1 {
        format!("{minutes} min ago")
    } else if days < 1 {
        format!("{hours} h ago")
    } else if days == 1 {
        "Yesterday".into()
    } else if days < 7 {
        format!("{days} days ago")
    } else {
        chrono::DateTime::from_timestamp(opened_at, 0)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%b %-d")
                    .to_string()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_times_read_from_minutes_to_dates() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(now - 20, now), "Just now");
        assert_eq!(relative_time(now - 5 * 60, now), "5 min ago");
        assert_eq!(relative_time(now - 3 * 3600, now), "3 h ago");
        assert_eq!(relative_time(now - 30 * 3600, now), "Yesterday");
        assert_eq!(relative_time(now - 4 * 86_400, now), "4 days ago");
        assert!(!relative_time(now - 30 * 86_400, now).contains("ago"));
        // A clock that went backwards is not a time in the future.
        assert_eq!(relative_time(now + 600, now), "Just now");
    }
}
