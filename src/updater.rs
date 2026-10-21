//! The shell's wiring of [`crate::update`]: who checks, when, and where the
//! answer shows up.
//!
//! * A quiet check runs a few seconds after launch, once the first screen is
//!   up, on the background executor — the window never waits for the network.
//! * What it finds is a notification in the bottom-right corner: **Update**
//!   while a release is available, **Restart** once it is installed.
//! * Settings → About shows the same state with the same buttons
//!   ([`UpdatePanel`]), and a manual **Check for updates**.
//!
//! The three commands are Actions ([`crate::actions`]): the notification's
//! button and the settings button dispatch the same value, handled once here.
//! A failed *startup* check stays silent — an offline launch is not news — but
//! a check the user asked for, and any failed install, says what happened.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::{
    Anchor, AnyView, App, AppContext as _, Context, Entity, Global, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, Window, div,
};

use crate::actions::{CheckForUpdates, InstallUpdate, RestartToUpdate};
use crate::ui::theme::Theme;
use crate::ui::typography::ui_rems;
use crate::update::github::GithubSource;
use crate::update::{UpdateEvent, UpdatePhase, UpdateState, install};

/// How long after launch the first check waits: long enough for the first
/// screen to have drawn and settled.
const STARTUP_DELAY: Duration = Duration::from_secs(4);

struct Updater {
    state: Entity<UpdateState>,
    /// Whether the user is waiting on the current check or install, which
    /// decides whether a failure or an "up to date" is worth a notification.
    asked: bool,
    _events: Subscription,
}

impl Global for Updater {}

/// Marks the update notification, so each new phase replaces the last one
/// instead of stacking.
struct UpdateNotice;

/// Install the update state and its command handlers. Once, at startup.
pub fn init(cx: &mut App) {
    // The copy the last update moved aside was held open by the process that
    // ran it; this one is free to remove it.
    install::remove_leftovers();
    let source = Arc::new(GithubSource::new(env!("CARGO_PKG_REPOSITORY")));
    let state = cx.new(|_| UpdateState::new(env!("CARGO_PKG_VERSION"), Some(source)));
    let events = cx.subscribe(&state, |_, event: &UpdateEvent, cx| {
        let UpdateEvent::PhaseChanged(phase) = event;
        announce(phase.clone(), cx);
    });
    cx.set_global(Updater {
        state,
        asked: false,
        _events: events,
    });
    cx.on_action(|_: &CheckForUpdates, cx| check(true, cx));
    cx.on_action(|_: &InstallUpdate, cx| {
        cx.global_mut::<Updater>().asked = true;
        update_state(cx).update(cx, |state, cx| state.install(cx));
    });
    cx.on_action(|_: &RestartToUpdate, cx| {
        update_state(cx).update(cx, |state, cx| state.restart(cx));
    });
}

/// The quiet check after launch. Call once the window is open.
pub fn check_after_launch(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(STARTUP_DELAY).await;
        cx.update(|cx| check(false, cx));
    })
    .detach();
}

/// The panel Settings → About mounts.
pub fn panel(cx: &mut App) -> AnyView {
    cx.new(UpdatePanel::new).into()
}

fn update_state(cx: &App) -> Entity<UpdateState> {
    cx.global::<Updater>().state.clone()
}

fn check(asked: bool, cx: &mut App) {
    cx.global_mut::<Updater>().asked = asked;
    update_state(cx).update(cx, |state, cx| state.check(cx));
}

/// Turn a phase into the notification that says it, when it is worth one.
fn announce(phase: UpdatePhase, cx: &mut App) {
    let asked = cx.global::<Updater>().asked;
    let can_install = update_state(cx).read(cx).can_install();
    let note = match phase {
        UpdatePhase::Available { version, notes } => {
            let note = Notification::new()
                .title("Update available")
                .message(format!(
                    "{} {version} is ready to install.",
                    crate::APP_NAME
                ))
                .with_type(NotificationType::Info);
            if can_install {
                note.action(|_, _, cx| {
                    Button::new("update-install")
                        .primary()
                        .small()
                        .label("Update")
                        .on_click(cx.listener(|note, _, window, cx| {
                            note.dismiss(window, cx);
                            cx.dispatch_action(&InstallUpdate);
                        }))
                })
            } else if let Some(notes) = notes {
                // A development build, or one that cannot replace itself:
                // the release page is where the new version is.
                note.action(move |_, _, cx| {
                    let notes = notes.clone();
                    Button::new("update-open-release")
                        .primary()
                        .small()
                        .label("View release")
                        .on_click(cx.listener(move |note, _, window, cx| {
                            note.dismiss(window, cx);
                            cx.open_url(&notes);
                        }))
                })
            } else {
                note
            }
        }
        UpdatePhase::Installing { version } => Notification::new()
            .title(format!("Installing {} {version}…", crate::APP_NAME))
            .message("It downloads in the background; keep working.")
            .with_type(NotificationType::Info)
            .autohide(false),
        UpdatePhase::ReadyToRestart { version } => Notification::new()
            .title("Update installed")
            .message(format!(
                "Restart {} to use version {version}.",
                crate::APP_NAME
            ))
            .with_type(NotificationType::Success)
            .action(|_, _, cx| {
                Button::new("update-restart")
                    .primary()
                    .small()
                    .label("Restart")
                    .on_click(cx.listener(|note, _, window, cx| {
                        note.dismiss(window, cx);
                        cx.dispatch_action(&RestartToUpdate);
                    }))
            }),
        UpdatePhase::UpToDate if asked => Notification::new()
            .title(format!("{} is up to date", crate::APP_NAME))
            .with_type(NotificationType::Success),
        UpdatePhase::Failed(error) if asked => Notification::new()
            .title("Update failed")
            .message(error)
            .with_type(NotificationType::Error)
            .autohide(false),
        UpdatePhase::Idle
        | UpdatePhase::Checking
        | UpdatePhase::UpToDate
        | UpdatePhase::Failed(_) => {
            return;
        }
    }
    .id::<UpdateNotice>()
    .placement(Anchor::BottomRight);
    // After the current update, so the window is free to borrow.
    cx.defer(move |cx| {
        let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
            return;
        };
        window
            .update(cx, |_, window, cx| {
                window.remove_notification::<UpdateNotice>(cx);
                window.push_notification(note, cx);
            })
            .ok();
    });
}

/// Settings → About's update row: where things stand, and the one thing to
/// do about it.
pub struct UpdatePanel {
    _observe: Subscription,
}

impl UpdatePanel {
    fn new(cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&update_state(cx), |_, _, cx| cx.notify());
        Self { _observe: observe }
    }
}

impl Render for UpdatePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let state = update_state(cx);
        let (phase, summary, can_install) = {
            let state = state.read(cx);
            (
                state.phase().clone(),
                state.summary(cx),
                state.can_install(),
            )
        };
        let button = |id: &'static str, label: &'static str| Button::new(id).small().label(label);
        let control = match &phase {
            UpdatePhase::Checking => button("settings-update-busy", "Checking…").disabled(true),
            UpdatePhase::Installing { .. } => {
                button("settings-update-busy", "Installing…").disabled(true)
            }
            UpdatePhase::Available { .. } if can_install => {
                button("settings-update-install", "Update")
                    .primary()
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(InstallUpdate), cx))
            }
            UpdatePhase::Available { notes, .. } => {
                let notes = notes.clone();
                button("settings-update-release", "View release")
                    .primary()
                    .disabled(notes.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(notes) = &notes {
                            cx.open_url(notes);
                        }
                    })
            }
            UpdatePhase::ReadyToRestart { .. } => button("settings-update-restart", "Restart")
                .primary()
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RestartToUpdate), cx)),
            UpdatePhase::Idle | UpdatePhase::UpToDate | UpdatePhase::Failed(_) => {
                button("settings-update-check", "Check for updates")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(CheckForUpdates), cx))
            }
        };
        let tone = match phase {
            UpdatePhase::Failed(_) => theme.danger_muted,
            UpdatePhase::Available { .. } | UpdatePhase::ReadyToRestart { .. } => theme.text,
            _ => theme.text_muted,
        };
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(12.0))
                    .text_color(tone)
                    .child(summary),
            )
            .child(div().flex_none().child(control))
    }
}
