//! The update state the UI observes.
//!
//! An explicit phase rather than a pile of booleans: every screen that shows
//! update status shows the same four answers, and "checking" cannot coexist
//! with "failed".

use std::sync::Arc;

use anyhow::Result;
use gpui_kit::{App, AppContext as _, Context, EventEmitter, SharedString, Task};
use semver::Version;

use super::check::{available_update, current_target};
use super::install::{self, Installed};
use super::manifest::{ReleaseArtifact, ReleaseManifest};
use super::source::ReleaseSource;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum UpdatePhase {
    /// Nothing has been asked yet.
    #[default]
    Idle,
    Checking,
    /// The last check found nothing newer for this build.
    UpToDate,
    /// A newer build exists for this target. `notes` is the release page.
    Available {
        version: SharedString,
        notes: Option<SharedString>,
    },
    /// The new build is downloading and being put in place.
    Installing {
        version: SharedString,
    },
    /// The new build is installed; it runs from the next start.
    ReadyToRestart {
        version: SharedString,
    },
    /// The check or the install failed. Recoverable: the message is for the
    /// user, and the next attempt starts from scratch.
    Failed(SharedString),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    PhaseChanged(UpdatePhase),
}

pub struct UpdateState {
    current: Version,
    source: Option<Arc<dyn ReleaseSource>>,
    phase: UpdatePhase,
    /// What the last check offered, kept for [`Self::install`].
    offered: Option<(String, ReleaseArtifact)>,
    installed: Option<Installed>,
    /// Holds the in-flight check or install. Dropping the state cancels it,
    /// and a second one replaces the first rather than racing it.
    check: Option<Task<()>>,
}

impl EventEmitter<UpdateEvent> for UpdateState {}

impl UpdateState {
    /// `current` is this build's version — pass `env!("CARGO_PKG_VERSION")`.
    ///
    /// A version that does not parse is treated as `0.0.0`, so a development
    /// build still sees updates instead of silently never checking.
    pub fn new(current: &str, source: Option<Arc<dyn ReleaseSource>>) -> Self {
        Self {
            current: Version::parse(current).unwrap_or_else(|_| Version::new(0, 0, 0)),
            source,
            phase: UpdatePhase::default(),
            offered: None,
            installed: None,
            check: None,
        }
    }

    pub fn phase(&self) -> &UpdatePhase {
        &self.phase
    }

    pub fn current_version(&self) -> &Version {
        &self.current
    }

    /// Whether a check can be started right now — false while one is running,
    /// and false with no source configured.
    pub fn is_checkable(&self) -> bool {
        self.source.is_some()
            && !matches!(
                self.phase,
                UpdatePhase::Checking
                    | UpdatePhase::Installing { .. }
                    | UpdatePhase::ReadyToRestart { .. }
            )
    }

    /// Whether [`Self::install`] can put the offered build in place, rather
    /// than the user downloading it from the release page.
    pub fn can_install(&self) -> bool {
        install::supported() && self.offered.is_some()
    }

    /// Download the offered build, verify it and put it in place of this one.
    /// The new version runs from the next start: see [`Self::restart`].
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, UpdatePhase::Available { .. }) || !self.can_install() {
            return;
        }
        let Some((version, artifact)) = self.offered.clone() else {
            return;
        };
        self.set_phase(
            UpdatePhase::Installing {
                version: version.clone().into(),
            },
            cx,
        );
        let work = cx.background_spawn(async move { install::install(&artifact, &version) });
        self.check = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |state, cx| {
                state.check = None;
                let version = state
                    .offered
                    .as_ref()
                    .map(|(v, _)| v.clone())
                    .unwrap_or_default();
                match result {
                    Ok(installed) => {
                        state.installed = Some(installed);
                        state.set_phase(
                            UpdatePhase::ReadyToRestart {
                                version: version.into(),
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        state.set_phase(UpdatePhase::Failed(format!("{error:#}").into()), cx)
                    }
                }
            })
            .ok();
        }));
    }

    /// Start the installed build and quit this one.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let Some(installed) = self.installed.clone() else {
            return;
        };
        match install::relaunch(&installed) {
            Ok(()) => cx.quit(),
            Err(error) => self.set_phase(UpdatePhase::Failed(format!("{error:#}").into()), cx),
        }
    }

    /// Ask the source for the channel's manifest.
    ///
    /// The fetch runs on the background executor; only the decision and the
    /// state change run on the application thread.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if !self.is_checkable() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        self.set_phase(UpdatePhase::Checking, cx);

        let fetch = cx.background_spawn(async move { source.fetch().await });
        self.check = Some(cx.spawn(async move |this, cx| {
            let result = fetch.await;
            this.update(cx, |state, cx| state.finish_check(result, cx))
                .ok();
        }));
    }

    /// Apply a finished check. Public so a host that fetches manifests its own
    /// way can drive this state without implementing [`ReleaseSource`].
    pub fn finish_check(&mut self, result: Result<ReleaseManifest>, cx: &mut Context<Self>) {
        self.check = None;
        let phase = match result {
            Ok(manifest) => {
                match available_update(&self.current, &manifest, current_target()) {
                    Some(artifact) => {
                        self.offered = Some((manifest.version.clone(), artifact.clone()));
                        UpdatePhase::Available {
                            version: manifest.version.clone().into(),
                            notes: manifest.notes.clone().map(Into::into),
                        }
                    }
                    // Current, or no build for this target: the same answer to
                    // the user either way.
                    None => {
                        self.offered = None;
                        UpdatePhase::UpToDate
                    }
                }
            }
            Err(error) => UpdatePhase::Failed(error.to_string().into()),
        };
        self.set_phase(phase, cx);
    }

    fn set_phase(&mut self, phase: UpdatePhase, cx: &mut Context<Self>) {
        if self.phase == phase {
            return;
        }
        self.phase = phase.clone();
        cx.emit(UpdateEvent::PhaseChanged(phase));
        cx.notify();
    }

    /// A short status line for the status bar.
    pub fn summary(&self, _cx: &App) -> SharedString {
        match &self.phase {
            UpdatePhase::Idle => format!("Version {}", self.current).into(),
            UpdatePhase::Checking => "Checking for updates…".into(),
            UpdatePhase::UpToDate => format!("Version {} is the latest", self.current).into(),
            UpdatePhase::Available { version, .. } => {
                format!("Version {version} is available").into()
            }
            UpdatePhase::Installing { version } => format!("Installing version {version}…").into(),
            UpdatePhase::ReadyToRestart { version } => {
                format!("Version {version} is installed. Restart to use it.").into()
            }
            UpdatePhase::Failed(error) => format!("Update failed: {error}").into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::manifest::ReleaseArtifact;
    use crate::update::source::StaticSource;

    fn source(version: &str) -> Arc<dyn ReleaseSource> {
        Arc::new(StaticSource::new(
            ReleaseManifest::new(version).with_artifact(ReleaseArtifact {
                target: current_target().into(),
                url: "https://example.com/rama.tar.gz".into(),
                size: 1,
                sha256: "00".into(),
                signature: None,
            }),
        ))
    }

    #[gpui_kit::test]
    fn a_newer_release_is_offered_and_the_same_one_is_not(cx: &mut gpui_kit::TestAppContext) {
        let newer = cx.new(|_| UpdateState::new("0.1.0", Some(source("0.2.0"))));
        newer.update(cx, |state, cx| state.check(cx));
        cx.run_until_parked();
        newer.read_with(cx, |state, _| {
            assert_eq!(
                state.phase(),
                &UpdatePhase::Available {
                    version: "0.2.0".into(),
                    notes: None
                }
            );
            // A test build is a development build: it is pointed at the
            // release page instead of replacing itself.
            assert_eq!(state.can_install(), install::supported());
        });

        let same = cx.new(|_| UpdateState::new("0.2.0", Some(source("0.2.0"))));
        same.update(cx, |state, cx| state.check(cx));
        cx.run_until_parked();
        same.read_with(cx, |state, _| {
            assert_eq!(state.phase(), &UpdatePhase::UpToDate);
            assert!(!state.can_install());
        });
    }
}
