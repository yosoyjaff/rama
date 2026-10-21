//! Everything the window has open, in one strip.
//!
//! One tab is one surface: the home screen, a project, the settings. Only the
//! projects are chips in the strip; home and the settings are always there,
//! so they are the two buttons ahead of it on the title band, lit while they
//! are the surface on screen. One strip, because a strip inside a strip would
//! ask which level a tab belongs to before it could be read.
//!
//! A patch is NOT a tab. It opens inside the project it belongs to, in the
//! place its graph occupies, and closes back to it.
//!
//! A project tab keeps its whole screen: its graph, its selection, its side
//! panel. Switching away and back finds it where it was left, because the tab
//! holds the entity and not a snapshot of it.

use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};

use super::history::pane::HistoryPane;
use super::settings_page::{SettingsPage, SettingsPageEvent};
use super::welcome::{Welcome, WelcomeEvent};
use crate::recents;
use crate::settings::{self, SavePolicy};
use crate::ui::icons;
use crate::ui::tab_strip::{Tab, tab_strip};
use crate::ui::theme::Theme;

/// One open surface.
enum Surface {
    /// Always first, never closable: it is the way to another project, and a
    /// window with no tabs at all would have no way forward.
    Home,
    Repo {
        path: String,
        name: SharedString,
        pane: Entity<HistoryPane>,
        /// Whether this tab's pane is reading its checkout. Restored tabs
        /// start unloaded — only the one on screen loads, and the rest load
        /// the moment they are visited.
        loaded: bool,
    },
    Settings {
        view: Entity<SettingsPage>,
        /// Lives and dies with the tab, so reopening Settings adds none.
        _events: Subscription,
    },
}

impl Surface {
    fn title(&self) -> SharedString {
        match self {
            Self::Home => "Home".into(),
            Self::Repo { name, .. } => name.clone(),
            Self::Settings { .. } => "Settings".into(),
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Self::Home => icons::FOLDER_WITH_FILES,
            Self::Repo { .. } => icons::GIT_BRANCH,
            Self::Settings { .. } => icons::SETTINGS_01,
        }
    }
}

pub struct Workspace {
    tabs: Vec<Surface>,
    active: usize,
    welcome: Entity<Welcome>,
    focus_handle: FocusHandle,
    /// The tab strip's scroll offset, kept here so activating a tab can bring
    /// its chip into view.
    tab_scroll: gpui_kit::ScrollHandle,
    /// The update row the shell hands down for Settings → About.
    update_panel: Option<gpui_kit::AnyView>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let welcome = cx.new(|cx| Welcome::new(window, cx));
        let subscriptions = vec![cx.subscribe_in(
            &welcome,
            window,
            |workspace: &mut Self, _, event, window, cx| match event {
                WelcomeEvent::OpenRepository(path) => {
                    workspace.open_repository(path.clone(), window, cx)
                }
                WelcomeEvent::ImportGitKraken { paths, active } => {
                    workspace.import_gitkraken(paths.clone(), active.clone(), window, cx)
                }
            },
        )];
        let mut workspace = Self {
            tabs: vec![Surface::Home],
            active: 0,
            welcome,
            focus_handle: cx.focus_handle(),
            tab_scroll: gpui_kit::ScrollHandle::new(),
            update_panel: None,
            _subscriptions: subscriptions,
        };
        // The tabs the last session left open come back — but UNLOADED: only
        // the one that was on screen reads its checkout, and the rest load
        // the moment they are visited. Nothing was saved? The process's own
        // checkout, then the demo folder, open as before.
        let saved = settings::current(cx);
        let saved_repos = saved.tabs.open.clone();
        let saved_active = saved.tabs.active.clone();
        drop(saved);
        if saved_repos.is_empty() {
            // The repository the process was started in, when it is one:
            // running `rama` inside a checkout should show that checkout, not
            // a screen asking which one.
            if let Some(path) = crate::git::open::repository_at_cwd() {
                workspace.open_repository(path, window, cx);
            }
            // TEMPORARY (demo of a full tab strip): open every repo under
            // RAMA_DEMO_REPOS, without touching the recents. Remove after
            // review.
            if let Ok(dir) = std::env::var("RAMA_DEMO_REPOS") {
                let mut paths: Vec<String> = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.path().to_string_lossy().into_owned())
                    .collect();
                paths.sort();
                for path in paths {
                    workspace.push_repo(path, true, window, cx);
                }
                workspace.activate(workspace.tabs.len() - 1, window, cx);
            }
        } else {
            for path in saved_repos {
                workspace.push_repo(path, false, window, cx);
            }
            // The tab that was on screen comes back on screen — which is
            // what loads it.
            let target = saved_active.and_then(|active| {
                workspace
                    .tabs
                    .iter()
                    .position(|tab| matches!(tab, Surface::Repo { path, .. } if *path == active))
            });
            workspace.activate(target.unwrap_or(workspace.tabs.len() - 1), window, cx);
        }
        workspace
    }

    /// Append a repo tab. `loaded: false` gives it an idle pane that reads
    /// nothing until the tab is visited — a restored tab costs no reads, no
    /// processes and no network until it is on screen.
    fn push_repo(
        &mut self,
        path: String,
        loaded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = if loaded {
            cx.new(|cx| HistoryPane::new(Some(path.clone()), window, cx))
        } else {
            cx.new(|cx| HistoryPane::new(None, window, cx))
        };
        self.tabs.push(Surface::Repo {
            name: SharedString::from(recents::repository_name(&path)),
            path,
            pane,
            loaded,
        });
    }

    /// Remember the open tabs and which was on screen, so the next launch
    /// comes back exactly here.
    fn persist_tabs(&self, cx: &mut Context<Self>) {
        let repos: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Surface::Repo { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        let active = match self.tabs.get(self.active) {
            Some(Surface::Repo { path, .. }) => Some(path.clone()),
            _ => None,
        };
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.tabs.open = repos;
            settings.tabs.active = active;
        });
    }

    /// Open a project, or bring the tab already holding it to the front.
    pub fn open_repository(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| matches!(tab, Surface::Repo { path: open, .. } if *open == path))
        {
            self.activate(index, window, cx);
            return;
        }
        recents::add(&path, cx);
        self.push_repo(path, true, window, cx);
        self.activate(self.tabs.len() - 1, window, cx);
    }

    /// Import the tabs GitKraken had open. They land unloaded — only the one
    /// that was on screen in GitKraken loads here, the rest when visited.
    pub fn import_gitkraken(
        &mut self,
        paths: Vec<String>,
        active: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            let already = self
                .tabs
                .iter()
                .any(|tab| matches!(tab, Surface::Repo { path: open, .. } if *open == path));
            if !already {
                recents::add(&path, cx);
                self.push_repo(path, false, window, cx);
            }
        }
        let target = active.and_then(|active| {
            self.tabs
                .iter()
                .position(|tab| matches!(tab, Surface::Repo { path, .. } if *path == active))
        });
        self.activate(target.unwrap_or(self.tabs.len() - 1), window, cx);
        // The welcome's entry is a first-time offer: imported once, it goes.
        settings::update(SavePolicy::Immediate, cx, |settings| {
            settings.gitkraken_imported = true;
        });
    }

    /// The settings tab, opened or focused.
    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| matches!(tab, Surface::Settings { .. }))
        {
            self.activate(index, window, cx);
            return;
        }
        let update_panel = self.update_panel.clone();
        let view = cx.new(|cx| SettingsPage::new(window, cx).with_update_panel(update_panel));
        // Its import asks the strip to open what GitKraken had open.
        let events = cx.subscribe_in(
            &view,
            window,
            |workspace: &mut Self, _, event, window, cx| match event {
                SettingsPageEvent::ImportGitKraken { paths, active } => {
                    workspace.import_gitkraken(paths.clone(), active.clone(), window, cx);
                }
            },
        );
        self.tabs.push(Surface::Settings {
            view,
            _events: events,
        });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    /// The view Settings → About shows for updates. The shell owns it.
    pub fn set_update_panel(&mut self, panel: gpui_kit::AnyView) {
        self.update_panel = Some(panel);
    }

    /// Show the home screen without closing anything.
    pub fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(0, window, cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
        // A tab restored but never visited reads nothing until now: this is
        // the visit, and this is where its checkout loads.
        if let Some(Surface::Repo {
            path, pane, loaded, ..
        }) = self.tabs.get_mut(self.active)
            && !*loaded
        {
            let path = path.clone();
            *loaded = true;
            pane.update(cx, |pane, cx| pane.open_repo(Some(path), cx));
        }
        // Only on activation, never per frame: pinned every render, the chip
        // would snap back whenever the wheel moved the strip away from it.
        if let Some(position) = self
            .strip_indices()
            .iter()
            .position(|&tab| tab == self.active)
        {
            self.tab_scroll.scroll_to_item(position);
        }
        // The keyboard lands on the Home with it, so Tab walks its buttons
        // and recent rows straight away.
        if matches!(self.tabs.get(self.active), Some(Surface::Home)) {
            let home = self.welcome.focus_handle(cx);
            window.focus(&home, cx);
        }
        self.sync_open_paths(cx);
        self.persist_tabs(cx);
        // The window says what is being read, the way every editor does.
        let title = match self.tabs.get(self.active) {
            Some(Surface::Home) | None => crate::APP_NAME.to_string(),
            Some(surface) => format!("{} — {}", surface.title(), crate::APP_NAME),
        };
        window.set_window_title(&title);
        cx.notify();
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Home is the way back; it has no close button and cannot be shut.
        if index == 0 || index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        let next = if self.active >= index {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.activate(next, window, cx);
    }

    /// Keep the welcome screen's "already open" marks true.
    fn sync_open_paths(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Surface::Repo { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        self.welcome
            .update(cx, |welcome, cx| welcome.set_open_paths(paths, cx));
    }

    /// Strip position → index into `tabs`: the projects, in order.
    fn strip_indices(&self) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, surface)| matches!(surface, Surface::Repo { .. }))
            .map(|(index, _)| index)
            .collect()
    }

    /// Whether the home screen is what the window shows.
    pub fn home_active(&self) -> bool {
        matches!(self.tabs.get(self.active), Some(Surface::Home) | None)
    }

    /// Whether the settings page is what the window shows.
    pub fn settings_active(&self) -> bool {
        matches!(self.tabs.get(self.active), Some(Surface::Settings { .. }))
    }

    /// The strip of open projects, for the window's title band. Home and the
    /// settings are reached from their own buttons before it, so the strip is
    /// only what was opened.
    pub fn render_tab_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active;
        let shown = self.strip_indices();
        let tabs: Vec<Tab> = shown
            .iter()
            .map(|&index| (index, &self.tabs[index]))
            .map(|(index, surface)| Tab {
                id: ("workspace-tab", index).into(),
                group: SharedString::from(format!("workspace-tab-{index}")),
                // A tab names the REPOSITORY: no branch, no badge.
                title: surface.title(),
                icon: Some(surface.icon()),
                active: index == active,
                closable: index != 0,
            })
            .collect();

        let select_handle = cx.entity().downgrade();
        let close_handle = select_handle.clone();
        let close_shown = shown.clone();
        tab_strip(
            tabs,
            &self.tab_scroll,
            move |position, window, cx: &mut App| {
                let index = shown[position];
                select_handle
                    .update(cx, |workspace, cx| workspace.activate(index, window, cx))
                    .ok();
            },
            move |position, window, cx: &mut App| {
                let index = close_shown[position];
                close_handle
                    .update(cx, |workspace, cx| workspace.close(index, window, cx))
                    .ok();
            },
        )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let corner = crate::ui::window_frame::corner_radius(window);
        let content: AnyElement = match self.tabs.get(self.active) {
            Some(Surface::Repo { pane, .. }) => pane.clone().into_any_element(),
            Some(Surface::Settings { view, .. }) => view.clone().into_any_element(),
            // Including the case where the active index has outrun the tabs:
            // home is always there.
            _ => self.welcome.clone().into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .rounded_b(px(corner))
            .bg(Theme::of(cx).bg)
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    struct StripHost(Entity<Workspace>);

    impl Render for StripHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0
                .update(cx, |workspace, cx| workspace.render_tab_strip(cx))
        }
    }

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::theme::init(cx);
            crate::settings::init(cx);
            crate::recents::init(cx);
        });
    }

    /// A workspace with nothing open. The process may have started inside a
    /// checkout, and every test here asserts on what it opens next.
    fn workspace(cx: &mut TestAppContext) -> gpui_kit::WindowHandle<Workspace> {
        init(cx);
        let window = cx.add_window(Workspace::new);
        window
            .update(cx, |workspace, window, cx| {
                workspace.tabs.truncate(1);
                workspace.activate(0, window, cx);
            })
            .expect("window update");
        window
    }

    fn titles(workspace: &Workspace) -> Vec<String> {
        workspace
            .tabs
            .iter()
            .map(|tab| tab.title().to_string())
            .collect()
    }

    #[gpui_kit::test]
    fn the_strip_holds_projects_and_nothing_a_project_opens(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.open_repository("/repos/two".into(), window, cx);
                workspace.open_settings(window, cx);

                assert_eq!(titles(workspace), vec!["Home", "one", "two", "Settings"]);
                assert_eq!(workspace.active, 3);
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn a_project_opens_once(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.go_home(window, cx);
                workspace.open_repository("/repos/one".into(), window, cx);
                assert_eq!(workspace.tabs.len(), 2, "the project opened twice");
                assert_eq!(workspace.active, 1);
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn the_settings_tab_opens_once_however_often_the_gear_is_pressed(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_settings(window, cx);
                workspace.open_settings(window, cx);
                assert_eq!(workspace.tabs.len(), 2);
                assert!(matches!(workspace.tabs[1], Surface::Settings { .. }));
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn the_band_buttons_light_for_the_surface_on_screen(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.go_home(window, cx);
                assert!(workspace.home_active() && !workspace.settings_active());
                workspace.open_settings(window, cx);
                assert!(workspace.settings_active() && !workspace.home_active());
                workspace.open_repository("/repos/one".into(), window, cx);
                assert!(!workspace.settings_active() && !workspace.home_active());
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn home_never_closes_and_closing_falls_back_to_the_tab_before(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.close(0, window, cx);
                assert_eq!(workspace.tabs.len(), 1, "home closed");

                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.open_settings(window, cx);
                assert_eq!(workspace.active, 2);
                workspace.close(2, window, cx);
                assert_eq!(workspace.active, 1, "back to the project");
                workspace.close(1, window, cx);
                assert_eq!(workspace.active, 0, "back to home");
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn a_full_strip_scrolls_the_active_tab_into_view(cx: &mut TestAppContext) {
        init(cx);
        let window = cx.add_window(Workspace::new);
        let workspace = window.root(cx).expect("the workspace is the root");
        let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        let draw = |visual: &mut gpui_kit::VisualTestContext| {
            let workspace = workspace.clone();
            visual.draw(
                gpui_kit::point(px(0.0), px(0.0)),
                gpui_kit::size(px(600.0), px(40.0)),
                move |_, cx| {
                    // The strip scrolls, and a scroll listener paints under
                    // a view: draw it inside one, as the shell does.
                    let strip = cx.new(|_| StripHost(workspace));
                    div().w(px(600.0)).flex().child(strip)
                },
            );
        };
        let visible = |workspace: &Entity<Workspace>,
                       position: usize,
                       cx: &mut gpui_kit::VisualTestContext| {
            workspace.read_with(cx, |workspace, _| {
                let scroll = &workspace.tab_scroll;
                let chip = scroll
                    .bounds_for_item(position)
                    .expect("the chip was laid out");
                let viewport = scroll.bounds();
                let offset = scroll.offset().x;
                chip.left() + offset >= viewport.left() && chip.right() + offset <= viewport.right()
            })
        };

        // The strip is on screen before anything opens, as in the app: the
        // scroll handle measures its viewport a frame behind.
        draw(&mut visual);
        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                for index in 0..50 {
                    workspace.open_repository(format!("/repos/{index:02}"), window, cx);
                }
            })
        });
        draw(&mut visual);
        assert!(
            visible(&workspace, 49, &mut visual),
            "the tab just opened is on screen"
        );
        assert!(
            !visible(&workspace, 0, &mut visual),
            "fifty tabs do not fit in 600px"
        );

        // Back to the first: the strip scrolls back to it.
        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| workspace.activate(1, window, cx))
        });
        draw(&mut visual);
        assert!(
            visible(&workspace, 0, &mut visual),
            "the first tab is on screen again"
        );
    }

    #[gpui_kit::test]
    fn every_surface_paints(cx: &mut TestAppContext) {
        init(cx);
        let window = cx.add_window(Workspace::new);
        let workspace = window.root(cx).expect("the workspace is the root");
        let mut visual = gpui_kit::VisualTestContext::from_window(window.into(), cx);
        let draw = |visual: &mut gpui_kit::VisualTestContext| {
            visual.draw(
                gpui_kit::point(px(0.0), px(0.0)),
                gpui_kit::size(px(1280.0), px(800.0)),
                |_, _| gpui_kit::Empty,
            );
        };

        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.tabs.truncate(1);
                workspace.activate(0, window, cx);
            })
        });
        draw(&mut visual);

        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.open_repository("/repos/one".into(), window, cx)
            })
        });
        draw(&mut visual);

        visual.update(|window, cx| {
            workspace.update(cx, |workspace, cx| workspace.open_settings(window, cx))
        });
        draw(&mut visual);

        // And the strip that switches between them.
        visual.update(|_, cx| {
            workspace.update(cx, |workspace, cx| {
                let _ = workspace.render_tab_strip(cx);
            })
        });
    }

    #[gpui_kit::test]
    fn closing_a_repository_tab_frees_its_pane(cx: &mut TestAppContext) {
        let repo = crate::git::test_repo::TestRepo::new();
        repo.commit("first");
        let path = repo.path().to_string_lossy().into_owned();
        let window = workspace(cx);
        let pane = window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository(path, window, cx);
                let Some(Surface::Repo { pane, .. }) = workspace.tabs.get(1) else {
                    panic!("the repository did not open a tab");
                };
                let pane = pane.downgrade();
                workspace.close(1, window, cx);
                pane
            })
            .expect("window update");
        cx.run_until_parked();
        assert!(pane.upgrade().is_none(), "the closed tab's pane is alive");
    }

    #[gpui_kit::test]
    fn reopening_settings_leaves_nothing_behind(cx: &mut TestAppContext) {
        let window = workspace(cx);
        let (pages, kept) = window
            .update(cx, |workspace, window, cx| {
                let before = workspace._subscriptions.len();
                let mut pages = Vec::new();
                for _ in 0..3 {
                    workspace.open_settings(window, cx);
                    let Some(Surface::Settings { view, .. }) = workspace.tabs.last() else {
                        panic!("settings did not open");
                    };
                    pages.push(view.downgrade());
                    let last = workspace.tabs.len() - 1;
                    workspace.close(last, window, cx);
                }
                (pages, workspace._subscriptions.len() - before)
            })
            .expect("window update");
        cx.run_until_parked();
        assert_eq!(kept, 0, "closed settings tabs left subscriptions behind");
        assert!(pages.iter().all(|page| page.upgrade().is_none()));
    }
}
