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

use std::rc::Rc;

use gpui_kit::component::Icon;
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable, Hsla,
    IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, Rgba, SharedString,
    Subscription, Window, div, prelude::*, px,
};

use super::history::pane::HistoryPane;
use super::settings_page::{SettingsPage, SettingsPageEvent};
use super::welcome::{Welcome, WelcomeEvent};
use crate::recents;
use crate::settings::{self, GroupColor, Rgb, SavePolicy, TabGroup};
use crate::ui::icons;
use crate::ui::popover::{self, Layer};
use crate::ui::tab_strip::{
    GroupLabel, Handler, Section as SectionLabel, StripEvents, StripItem, Tab, TruncatedLabel,
    reveal, tab_strip,
};
use crate::ui::text_input::{TextInput, TextInputEvent};
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
        /// The tab group it is in. A group's tabs sit next to each other.
        group: Option<u64>,
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
    /// The tab that was open when the settings were, for Back to app.
    before_settings: Option<usize>,
    welcome: Entity<Welcome>,
    focus_handle: FocusHandle,
    /// The tab strip's scroll offset, kept here so activating a tab can bring
    /// its chip into view.
    tab_scroll: gpui_kit::ScrollHandle,
    /// The update row the shell hands down for Settings → About.
    update_panel: Option<gpui_kit::AnyView>,
    /// The tab or group whose name is being edited in the strip.
    renaming: Option<Renaming>,
    /// The tab groups, in no particular order: the strip's order is the
    /// tabs'.
    groups: Vec<TabGroup>,
    /// A right click's menu on a tab or a group label.
    menu: Layer<TabMenu>,
    /// The group menu's custom colour, past its named shades.
    color_picker: Entity<ColorPickerState>,
    /// A dropdown row was clicked once: the dropdown closes unless a second
    /// click makes it a double one. Dropped, it stays.
    dropdown_close: Option<gpui_kit::Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Something in the strip: a tab, by its index in `tabs`, a group, by its
/// id, or a column's section label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Tab(usize),
    Group(u64),
    Section(Section),
}

/// A column's sections: the groups, then the tabs in none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Groups,
    Projects,
}

/// A name being edited in place.
struct Renaming {
    target: Target,
    input: Entity<TextInput>,
    _events: Subscription,
}

struct TabMenu {
    at: Point<Pixels>,
    target: Target,
    /// A group's dropdown across the band, which lists its tabs first.
    listing: bool,
}

/// `color` as the opaque `#rrggbb` a custom group colour is saved as.
fn rgb_of(color: Hsla) -> Rgb {
    let Rgba { r, g, b, .. } = color.into();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    Rgb(channel(r) << 16 | channel(g) << 8 | channel(b))
}

/// The tab menu's width.
const MENU_WIDTH: f32 = 220.0;
/// A colour choice in a group's menu.
const SWATCH: f32 = 16.0;
/// The new-group button on the Groups heading.
const NEW_GROUP_BUTTON: f32 = 20.0;
/// Between a group's plate and its dropdown.
const DROPDOWN_GAP: f32 = 4.0;
/// How long a dropdown waits after a click for the second of a double one:
/// a little over the platforms' own windows (400 ms on Linux, 500 ms by
/// default on macOS and Windows).
const DOUBLE_CLICK_WAIT: std::time::Duration = std::time::Duration::from_millis(520);

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let welcome = cx.new(|cx| Welcome::new(window, cx));
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let mut subscriptions = vec![cx.subscribe_in(
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
        subscriptions.push(cx.subscribe(
            &color_picker,
            |workspace: &mut Self, _, event: &ColorPickerEvent, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                if let Some(Target::Group(id)) = workspace.menu.shown().map(|menu| menu.target) {
                    workspace.set_group_color(id, GroupColor::Custom(rgb_of(*color)), cx);
                }
            },
        ));
        let mut workspace = Self {
            tabs: vec![Surface::Home],
            active: 0,
            before_settings: None,
            welcome,
            focus_handle: cx.focus_handle(),
            tab_scroll: gpui_kit::ScrollHandle::new(),
            update_panel: None,
            renaming: None,
            groups: Vec::new(),
            menu: Layer::default(),
            color_picker,
            dropdown_close: None,
            _subscriptions: subscriptions,
        };
        // The tabs the last session left open come back — but UNLOADED: only
        // the one that was on screen reads its checkout, and the rest load
        // the moment they are visited. Nothing was saved? The process's own
        // checkout, then the demo folder, open as before.
        let saved = settings::current(cx);
        let saved_repos = saved.tabs.open.clone();
        let saved_active = saved.tabs.active.clone();
        let saved_groups = saved.tabs.groups.clone();
        let saved_grouped = saved.tabs.grouped.clone();
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
            workspace.groups = saved_groups;
            for path in saved_repos {
                let group = saved_grouped
                    .get(&path)
                    .copied()
                    .filter(|&id| workspace.group(id).is_some());
                workspace.push_repo(path, false, window, cx);
                workspace.set_group_of(workspace.tabs.len() - 1, group);
            }
            workspace.gather_groups();
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
        let given = settings::current(cx).tabs.names.get(&path).cloned();
        self.tabs.push(Surface::Repo {
            name: SharedString::from(given.unwrap_or_else(|| recents::repository_name(&path))),
            path,
            pane,
            loaded,
            group: None,
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
        let grouped = self
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Surface::Repo {
                    path,
                    group: Some(id),
                    ..
                } => Some((path.clone(), *id)),
                _ => None,
            })
            .collect();
        let groups = self.groups.clone();
        settings::update(SavePolicy::Debounced, cx, |settings| {
            settings.tabs.open = repos;
            settings.tabs.active = active;
            settings.tabs.groups = groups;
            settings.tabs.grouped = grouped;
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
        if !self.settings_active() {
            self.before_settings = Some(self.active);
        }
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
                SettingsPageEvent::Back => workspace.leave_settings(window, cx),
            },
        );
        self.tabs.push(Surface::Settings {
            view,
            _events: events,
        });
        self.activate(self.tabs.len() - 1, window, cx);
    }

    /// Back from the settings to the tab that was open before them, or home.
    /// The settings stay a surface, so their page is where it was next time.
    pub fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self
            .before_settings
            .take()
            .filter(|&index| !matches!(self.tabs.get(index), Some(Surface::Settings { .. }) | None))
            .unwrap_or(0);
        self.activate(back, window, cx);
    }

    /// The view Settings → About shows for updates. The shell owns it.
    pub fn set_update_panel(&mut self, panel: gpui_kit::AnyView) {
        self.update_panel = Some(panel);
    }

    /// Ask for a folder and open it as a tab: the + by the tabs. Home's
    /// picker does the asking, so a folder that is not a repository is
    /// reported there.
    pub fn pick_project(&mut self, cx: &mut Context<Self>) {
        self.welcome
            .update(cx, |welcome, cx| welcome.pick_repository(cx));
    }

    /// Open the active project's find card. Home and the settings have
    /// nothing to find in.
    pub fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Surface::Repo { pane, .. }) = self.tabs.get(self.active) {
            pane.update(cx, |pane, cx| pane.find(window, cx));
        }
    }

    /// Show the home screen without closing anything.
    pub fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(0, window, cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
        // A tab can be opened from Home while its group is folded: the
        // group opens to show it.
        if let Some(id) = self.group_of(self.active)
            && let Some(group) = self.group_mut(id)
        {
            group.collapsed = false;
        }
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
        // A tab inside a group across the band is its group's plate.
        let group = self.group_of(self.active).map(Target::Group);
        let entries = self.strip_entries(self.column(cx));
        if let Some(position) = entries
            .iter()
            .position(|&entry| entry == Target::Tab(self.active))
            .or_else(|| entries.iter().position(|&entry| Some(entry) == group))
        {
            reveal(&self.tab_scroll, position, window);
        }
        // The keyboard lands on the Home with it, so Tab walks its buttons
        // and recent rows straight away.
        if matches!(self.tabs.get(self.active), Some(Surface::Home)) {
            let home = self.welcome.focus_handle(cx);
            window.focus(&home, cx);
        }
        self.sync_open_paths(cx);
        self.persist_tabs(cx);
        self.sync_window_title(window);
        cx.notify();
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Home is the way back; it has no close button and cannot be shut.
        if index == 0 || index >= self.tabs.len() {
            return;
        }
        // Indices shift under an edit in progress: it ends unsaved.
        self.renaming = None;
        self.close_menu(cx);
        self.tabs.remove(index);
        if let Some(before) = self.before_settings {
            self.before_settings = Some(shifted_by_removal(before, index));
        }
        let next = if self.active >= index {
            self.active.saturating_sub(1)
        } else {
            self.active
        };
        self.activate(next, window, cx);
    }

    /// Edit the name of a tab in its chip, or of a group in its label.
    fn start_rename(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let (name, placeholder) = match target {
            Target::Tab(index) => match self.tabs.get(index) {
                Some(Surface::Repo { name, .. }) => (name.clone(), "Tab name"),
                _ => return,
            },
            Target::Group(id) => match self.group(id) {
                Some(group) => (SharedString::from(group.name.clone()), "Group name"),
                None => return,
            },
            Target::Section(_) => return,
        };
        let input = cx.new(|cx| {
            let mut input = TextInput::new(placeholder, window, cx)
                .with_text_size(11.5)
                .compact();
            input.set_text(name, window, cx);
            input
        });
        let events = cx.subscribe_in(
            &input,
            window,
            |workspace: &mut Self, _, event, window, cx| {
                match event {
                    TextInputEvent::Submitted => workspace.finish_rename(true, true, window, cx),
                    // Focus went somewhere on purpose: leave it there.
                    TextInputEvent::Blurred => workspace.finish_rename(true, false, window, cx),
                    TextInputEvent::Edited => {}
                }
            },
        );
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.renaming = Some(Renaming {
            target,
            input,
            _events: events,
        });
        cx.notify();
    }

    /// Rename tab `index` from its menu. A grouped tab is brought on screen
    /// first: across the band it has no plate of its own, and is edited in
    /// its group's, which shows the tab on screen.
    fn rename_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.group_of(index).is_some() && index != self.active {
            self.activate(index, window, cx);
        }
        self.start_rename(Target::Tab(index), window, cx);
    }

    /// End the edit, keeping the typed name when `save`. A blank name gives
    /// the tab its folder's name back. `refocus` hands the keyboard back to
    /// the window when the edit ended from the keyboard.
    fn finish_rename(
        &mut self,
        save: bool,
        refocus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        let typed = renaming.input.read(cx).text().trim().to_string();
        if save && let Target::Group(id) = renaming.target {
            // A group always has a name: a blank one keeps the last.
            if !typed.is_empty()
                && let Some(group) = self.group_mut(id)
            {
                group.name = typed;
                self.persist_tabs(cx);
            }
        } else if save && let Target::Tab(index) = renaming.target {
            if let Some(Surface::Repo { path, name, .. }) = self.tabs.get_mut(index) {
                let path = path.clone();
                let given = (!typed.is_empty()).then_some(typed);
                *name = SharedString::from(
                    given
                        .clone()
                        .unwrap_or_else(|| recents::repository_name(&path)),
                );
                settings::update(SavePolicy::Debounced, cx, |settings| match given {
                    Some(given) => {
                        settings.tabs.names.insert(path, given);
                    }
                    None => {
                        settings.tabs.names.remove(&path);
                    }
                });
            }
            if index == self.active {
                self.sync_window_title(window);
            }
        }
        // A row edited in a dropdown: the edit was what it was open for.
        if self.menu.shown().is_some_and(|menu| menu.listing) {
            self.close_menu(cx);
        }
        if refocus {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    /// The window says what is being read, the way every editor does.
    fn sync_window_title(&self, window: &mut Window) {
        let title = match self.tabs.get(self.active) {
            Some(Surface::Home) | None => crate::APP_NAME.to_string(),
            Some(surface) => format!("{} — {}", surface.title(), crate::APP_NAME),
        };
        window.set_window_title(&title);
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

    /// Whether the tabs run down a column rather than across the band.
    fn column(&self, cx: &App) -> bool {
        settings::current(cx).look.tabs != settings::TabSide::Top
    }

    /// What the strip shows, in order. Down a column: the groups, each a
    /// heading over its tabs unless folded, then the tabs in none. Across
    /// the band: a plate per group, then the tabs in none.
    fn strip_entries(&self, column: bool) -> Vec<Target> {
        let loose = (0..self.tabs.len())
            .filter(|&index| matches!(self.tabs[index], Surface::Repo { group: None, .. }));
        let mut entries = Vec::new();
        if column && !self.groups.is_empty() {
            entries.push(Target::Section(Section::Groups));
        }
        for group in &self.groups {
            entries.push(Target::Group(group.id));
            if column && !group.collapsed {
                entries.extend(self.member_indices(group.id).map(Target::Tab));
            }
        }
        if column {
            entries.push(Target::Section(Section::Projects));
        }
        entries.extend(loose.map(Target::Tab));
        entries
    }

    // ── Groups ──────────────────────────────────────────────────────────────

    fn group(&self, id: u64) -> Option<&TabGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    fn group_mut(&mut self, id: u64) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|group| group.id == id)
    }

    /// The group tab `index` is in.
    fn group_of(&self, index: usize) -> Option<u64> {
        match self.tabs.get(index) {
            Some(Surface::Repo { group, .. }) => *group,
            _ => None,
        }
    }

    fn set_group_of(&mut self, index: usize, to: Option<u64>) {
        if let Some(Surface::Repo { group, .. }) = self.tabs.get_mut(index) {
            *group = to;
        }
    }

    /// The first and last index of a group's tabs.
    fn group_span(&self, id: u64) -> Option<(usize, usize)> {
        let members = || (0..self.tabs.len()).filter(|&index| self.group_of(index) == Some(id));
        Some((members().next()?, members().next_back()?))
    }

    /// The indices of a group's tabs, in order.
    fn member_indices(&self, id: u64) -> impl Iterator<Item = usize> + '_ {
        (0..self.tabs.len()).filter(move |&index| self.group_of(index) == Some(id))
    }

    /// Bring each group's tabs together after its first, the rest keeping
    /// their order. Only for tabs read back from a file, which could have
    /// been edited by hand: every change made here keeps groups together.
    fn gather_groups(&mut self) {
        let mut order: Vec<usize> = Vec::with_capacity(self.tabs.len());
        for index in 0..self.tabs.len() {
            if order.contains(&index) {
                continue;
            }
            order.push(index);
            if let Some(id) = self.group_of(index) {
                order.extend(
                    (index + 1..self.tabs.len()).filter(|&other| self.group_of(other) == Some(id)),
                );
            }
        }
        let mut slots: Vec<Option<Surface>> = self.tabs.drain(..).map(Some).collect();
        self.tabs = order
            .into_iter()
            .filter_map(|index| slots[index].take())
            .collect();
        self.active = 0;
    }

    /// Move tab `from` so it ends up at index `to`, with the active tab and
    /// Back to app following the tabs they point at.
    fn move_tab(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        self.renaming = None;
        let surface = self.tabs.remove(from);
        self.tabs.insert(to, surface);
        self.active = shifted_by_move(self.active, from, to);
        self.before_settings = self
            .before_settings
            .map(|index| shifted_by_move(index, from, to));
    }

    /// Take tab `index` out of its group, to just after the group's other
    /// tabs. Returns where the tab is now.
    fn leave_group(&mut self, index: usize) -> usize {
        let Some(id) = self.group_of(index) else {
            return index;
        };
        let (_, last) = self.group_span(id).unwrap_or((index, index));
        self.move_tab(index, last);
        self.set_group_of(last, None);
        last
    }

    /// Add an empty group, last, and edit its name. A group stays until it
    /// is ungrouped, empty or not.
    fn create_group(&mut self, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let id = self.groups.iter().map(|group| group.id).max().unwrap_or(0) + 1;
        // A colour no other group has, while there is one.
        let color = GroupColor::OFFERED
            .into_iter()
            .find(|&color| !self.groups.iter().any(|group| group.color == color))
            .unwrap_or(GroupColor::OFFERED[self.groups.len() % GroupColor::OFFERED.len()]);
        let taken = |name: &str| self.groups.iter().any(|group| group.name == name);
        let name = std::iter::once("New group".to_string())
            .chain((2..).map(|n| format!("New group {n}")))
            .find(|name| !taken(name))
            .unwrap_or_default();
        self.groups.push(TabGroup {
            id,
            name,
            color,
            collapsed: false,
        });
        self.persist_tabs(cx);
        self.start_rename(Target::Group(id), window, cx);
        id
    }

    /// Put tab `index` in a new group of its own.
    fn new_group(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.tabs.get(index), Some(Surface::Repo { .. })) {
            return;
        }
        let index = self.leave_group(index);
        let id = self.create_group(window, cx);
        self.set_group_of(index, Some(id));
        self.persist_tabs(cx);
    }

    /// Move tab `index` to the end of group `id`.
    fn move_to_group(&mut self, index: usize, id: u64, cx: &mut Context<Self>) {
        if self.group_of(index) == Some(id) || self.group(id).is_none() {
            return;
        }
        let index = self.leave_group(index);
        // Into an empty group it stays where it is.
        let to = match self.group_span(id) {
            Some((_, last)) if index > last => last + 1,
            Some((_, last)) => last,
            None => index,
        };
        self.move_tab(index, to);
        self.set_group_of(to, Some(id));
        if let Some(group) = self.group_mut(id) {
            group.collapsed = false;
        }
        self.persist_tabs(cx);
        cx.notify();
    }

    fn remove_from_group(&mut self, index: usize, cx: &mut Context<Self>) {
        self.leave_group(index);
        self.persist_tabs(cx);
        cx.notify();
    }

    /// Dissolve a group; its tabs stay where they are.
    fn ungroup(&mut self, id: u64, cx: &mut Context<Self>) {
        for index in 0..self.tabs.len() {
            if self.group_of(index) == Some(id) {
                self.set_group_of(index, None);
            }
        }
        self.groups.retain(|group| group.id != id);
        self.persist_tabs(cx);
        cx.notify();
    }

    /// The project tabs the menu's bulk closes act on: every one but Home
    /// and the settings, in strip order.
    fn project_tabs(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.tabs.len()).filter(|&index| matches!(self.tabs[index], Surface::Repo { .. }))
    }

    /// Close every project tab `doomed` names, then show `keep` — by where it
    /// was before the closes moved it.
    fn close_tabs(
        &mut self,
        doomed: Vec<usize>,
        keep: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // From the end, so the indices still to close stay where they were.
        for &index in doomed.iter().rev() {
            self.close(index, window, cx);
        }
        if let Some(keep) = keep {
            let shifted = keep - doomed.iter().filter(|&&index| index < keep).count();
            self.activate(shifted, window, cx);
        }
    }

    fn close_others(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let doomed = self
            .project_tabs()
            .filter(|&other| other != index)
            .collect();
        self.close_tabs(doomed, Some(index), window, cx);
    }

    fn close_to_the_right(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let doomed: Vec<usize> = self.project_tabs().filter(|&other| other > index).collect();
        // Only when the tab on screen goes: the settings stay where they are.
        let keep = doomed.contains(&self.active).then_some(index);
        self.close_tabs(doomed, keep, window, cx);
    }

    fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doomed = self.project_tabs().collect();
        self.close_tabs(doomed, None, window, cx);
    }

    fn close_group(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let members: Vec<usize> = (0..self.tabs.len())
            .filter(|&index| self.group_of(index) == Some(id))
            .collect();
        for index in members.into_iter().rev() {
            self.close(index, window, cx);
        }
        self.groups.retain(|group| group.id != id);
        self.persist_tabs(cx);
        cx.notify();
    }

    fn set_group_color(&mut self, id: u64, color: GroupColor, cx: &mut Context<Self>) {
        if let Some(group) = self.group_mut(id) {
            group.color = color;
            self.persist_tabs(cx);
            cx.notify();
        }
    }

    /// Fold a group down to its heading, or open it again.
    fn toggle_group(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(group) = self.group_mut(id) {
            group.collapsed = !group.collapsed;
            self.persist_tabs(cx);
            cx.notify();
        }
    }

    // ── Menu ────────────────────────────────────────────────────────────────

    fn open_menu(
        &mut self,
        target: Target,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(target, Target::Section(_)) {
            return;
        }
        if let Target::Group(id) = target {
            self.show_group_color(id, window, cx);
        }
        self.menu.open(TabMenu {
            at,
            target,
            listing: false,
        });
        cx.notify();
    }

    /// A group's dropdown, hanging under its plate at strip position
    /// `position`. A second click on the plate closes it.
    fn toggle_dropdown(
        &mut self,
        id: u64,
        position: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The press on the plate has already dismissed an open dropdown (a
        // press outside a menu closes it), and the click lands while it
        // fades: that click is the one closing it, not opening it again.
        let shown = self
            .menu
            .shown()
            .is_some_and(|menu| menu.listing && menu.target == Target::Group(id));
        if shown {
            self.close_menu(cx);
            return;
        }
        // Plates are laid out unscrolled; the strip's offset moves them.
        let Some(plate) = self.tab_scroll.bounds_for_item(position) else {
            return;
        };
        let offset = self.tab_scroll.offset();
        let at = gpui_kit::point(plate.left() + offset.x, plate.bottom() + px(DROPDOWN_GAP));
        self.show_group_color(id, window, cx);
        self.menu.open(TabMenu {
            at,
            target: Target::Group(id),
            listing: true,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.menu.dismiss(cx, |workspace| &mut workspace.menu);
    }

    /// Close the dropdown once a click can no longer turn into a double one,
    /// which edits the row instead.
    fn close_dropdown_unless_clicked_again(&mut self, cx: &mut Context<Self>) {
        self.dropdown_close = Some(cx.spawn(async move |workspace, cx| {
            cx.background_executor().timer(DOUBLE_CLICK_WAIT).await;
            workspace
                .update(cx, |workspace, cx| {
                    workspace.dropdown_close = None;
                    workspace.close_menu(cx);
                })
                .ok();
        }));
    }

    /// Whether group `id`'s dropdown is on screen across the band, fading
    /// out included.
    fn dropdown_open(&self, id: u64) -> bool {
        self.menu
            .shown()
            .is_some_and(|menu| menu.listing && menu.target == Target::Group(id))
    }

    /// Start the colour picker from group `id`'s colour, whichever it is.
    fn show_group_color(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(color) = self.group(id).map(|group| group.color) else {
            return;
        };
        let tone = Theme::of(cx).tab_group(color);
        self.color_picker.update(cx, |picker, cx| {
            picker.set_open(false, cx);
            picker.set_value(tone, window, cx);
        });
    }

    /// Whether the home screen is what the window shows.
    pub fn home_active(&self) -> bool {
        matches!(self.tabs.get(self.active), Some(Surface::Home) | None)
    }

    /// Whether the settings page is what the window shows.
    pub fn settings_active(&self) -> bool {
        matches!(self.tabs.get(self.active), Some(Surface::Settings { .. }))
    }

    /// The strip of open projects, for the window's title band or, as a
    /// `column`, a sidebar. Home and the settings are reached from their own
    /// buttons, so the strip is only what was opened.
    pub fn render_tab_strip(
        &mut self,
        trailing: Option<AnyElement>,
        column: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.active;
        let theme = Theme::of(cx).clone();
        let entries = self.strip_entries(column);
        let editing = |target: Target| {
            self.renaming
                .as_ref()
                .filter(|renaming| renaming.target == target)
                .map(|renaming| renaming.input.clone())
        };
        // Down a column the + heads the Projects section, not the end.
        let (mut section_button, trailing) = match column {
            true => (trailing, None),
            false => (None, trailing),
        };
        let mut items: Vec<StripItem> = Vec::with_capacity(entries.len());
        for &entry in &entries {
            items.push(match entry {
                Target::Tab(index) => {
                    let surface = &self.tabs[index];
                    let editor = editing(entry).map(|input| self.render_rename(&input, cx));
                    StripItem::Tab(Tab {
                        id: ("workspace-tab", index).into(),
                        group: SharedString::from(format!("workspace-tab-{index}")),
                        // A tab names the REPOSITORY: no branch, no badge.
                        title: surface.title(),
                        icon: Some(surface.icon()),
                        active: index == active,
                        closable: index != 0,
                        editor,
                        nested: self.group_of(index).is_some(),
                    })
                }
                Target::Group(id) => {
                    let group = self.group(id).cloned().unwrap_or_default();
                    let on_screen = self.group_of(active) == Some(id);
                    // Across the band a group's tabs have no plate of their
                    // own: the one on screen is renamed in its group's.
                    // Unless its dropdown is open: there the row is edited.
                    let editor = editing(entry)
                        .or_else(|| {
                            (!column && on_screen && !self.dropdown_open(id))
                                .then(|| editing(Target::Tab(active)))
                                .flatten()
                        })
                        .map(|input| self.render_rename(&input, cx));
                    StripItem::Group(GroupLabel {
                        id: ElementId::Name(format!("workspace-group-{id}").into()),
                        name: group.name.into(),
                        color: theme.tab_group(group.color),
                        collapsed: group.collapsed,
                        active: on_screen,
                        current: on_screen.then(|| self.tabs[active].title()),
                        editor,
                    })
                }
                Target::Section(section) => StripItem::Section(match section {
                    Section::Groups => SectionLabel {
                        label: "Groups".into(),
                        action: Some(self.render_new_group_button(cx)),
                    },
                    Section::Projects => SectionLabel {
                        label: "Projects".into(),
                        action: section_button.take(),
                    },
                }),
            });
        }

        let entries = Rc::new(entries);
        let handle = cx.entity().downgrade();
        let handler = |act: fn(&mut Self, Target, &mut Window, &mut Context<Self>)| -> Handler {
            let (handle, entries) = (handle.clone(), entries.clone());
            Rc::new(move |position, window, cx: &mut App| {
                if let Some(&entry) = entries.get(position) {
                    handle
                        .update(cx, |workspace, cx| act(workspace, entry, window, cx))
                        .ok();
                }
            })
        };
        let select: Handler = {
            let (handle, entries) = (handle.clone(), entries.clone());
            Rc::new(move |position, window, cx: &mut App| {
                let Some(&entry) = entries.get(position) else {
                    return;
                };
                handle
                    .update(cx, |workspace, cx| match entry {
                        Target::Tab(index) => workspace.activate(index, window, cx),
                        Target::Group(id) if column => workspace.toggle_group(id, cx),
                        Target::Group(id) => workspace.toggle_dropdown(id, position, window, cx),
                        Target::Section(_) => {}
                    })
                    .ok();
            })
        };
        let events = StripEvents {
            select,
            rename: {
                let (handle, entries) = (handle.clone(), entries.clone());
                Rc::new(move |position, window, cx: &mut App| {
                    let Some(&entry) = entries.get(position) else {
                        return;
                    };
                    handle
                        .update(cx, |workspace, cx| {
                            let target = match entry {
                                // Across the band a group's plate shows its
                                // tab on screen: that is what it renames.
                                Target::Group(id)
                                    if !column
                                        && workspace.group_of(workspace.active) == Some(id) =>
                                {
                                    Target::Tab(workspace.active)
                                }
                                other => other,
                            };
                            workspace.close_menu(cx);
                            workspace.start_rename(target, window, cx);
                        })
                        .ok();
                })
            },
            close: handler(|workspace, entry, window, cx| {
                if let Target::Tab(index) = entry {
                    workspace.close(index, window, cx);
                }
            }),
            menu: {
                let (handle, entries) = (handle.clone(), entries.clone());
                Rc::new(move |position, at, window, cx: &mut App| {
                    if let Some(&entry) = entries.get(position) {
                        handle
                            .update(cx, |workspace, cx| {
                                workspace.open_menu(entry, at, window, cx)
                            })
                            .ok();
                    }
                })
            },
        };
        tab_strip(items, &self.tab_scroll, trailing, column, events)
    }

    /// The button on the Groups heading: a new, empty group.
    fn render_new_group_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx);
        let (rest, lit) = (theme.text_muted, theme.text);
        let id = "workspace-new-group";
        div()
            .id(id)
            .group(id)
            .size(px(NEW_GROUP_BUTTON))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|style| style.bg(crate::ui::theme::wash(0.10)))
            .tooltip(crate::ui::tip::tip("New group"))
            .on_click(cx.listener(|workspace, _, window, cx| {
                cx.stop_propagation();
                workspace.create_group(window, cx);
            }))
            .child(
                icons::icon(icons::FOLDER_01)
                    .flex_none()
                    .size(px(14.0))
                    .text_color(rest)
                    .group_hover(id, move |style| style.text_color(lit)),
            )
            .into_any_element()
    }
}

impl Workspace {
    /// The name field in place of a chip's title. It sits on the window's
    /// drag strip, so a press in it selects text instead of moving the window.
    fn render_rename(&self, input: &Entity<TextInput>, cx: &mut Context<Self>) -> AnyElement {
        div()
            .occlude()
            .w_full()
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .on_key_down(
                cx.listener(|workspace, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        workspace.finish_rename(false, true, window, cx);
                    }
                }),
            )
            .child(input.clone())
            .into_any_element()
    }

    /// The right click's menu on a tab or a group label.
    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.shown()?;
        let (at, target, listing) = (menu.at, menu.target, menu.listing);
        let closing = self.menu.leaving_since();
        let theme = Theme::of(cx).clone();
        let row = |id: String, label: String, danger: bool| {
            let id = ElementId::Name(id.into());
            popover::menu_row(&theme, false, SharedString::from(id.to_string()))
                .id(id.clone())
                .when(danger, |row| row.text_color(theme.danger_muted))
                .child(TruncatedLabel::new(id, label))
        };
        let mut card = popover::popover_card(&theme)
            .w(px(MENU_WIDTH))
            .on_mouse_down_out(cx.listener(|workspace, _, window, cx| {
                // The picker's own popover lies outside the menu: a press in
                // it is not a press away from the menu.
                if workspace.color_picker.read(cx).is_open() {
                    return;
                }
                // A row being edited keeps what was typed, as a tab does.
                let editing_row = workspace.menu.shown().is_some_and(|menu| menu.listing)
                    && workspace
                        .renaming
                        .as_ref()
                        .is_some_and(|renaming| matches!(renaming.target, Target::Tab(_)));
                if editing_row {
                    workspace.finish_rename(true, false, window, cx);
                }
                workspace.close_menu(cx);
            }));
        match target {
            Target::Tab(index) => {
                let current = self.group_of(index);
                card = card
                    .child(
                        row("tab-menu-rename".into(), "Rename tab".into(), false).on_click(
                            cx.listener(move |workspace, _, window, cx| {
                                workspace.close_menu(cx);
                                workspace.rename_tab(index, window, cx);
                            }),
                        ),
                    )
                    .child(popover::menu_separator());
                card = card.child(
                    row(
                        "tab-menu-new-group".into(),
                        "Add to new group".into(),
                        false,
                    )
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.close_menu(cx);
                        workspace.new_group(index, window, cx);
                    })),
                );
                for group in self.groups.iter().filter(|group| Some(group.id) != current) {
                    let id = group.id;
                    let name = match group.name.trim() {
                        "" => format!("{} group", group.color.label()),
                        name => name.to_string(),
                    };
                    card = card.child(
                        row(
                            format!("tab-menu-move-{id}"),
                            format!("Move to {name}"),
                            false,
                        )
                        .child(
                            div()
                                .flex_none()
                                .size(px(8.0))
                                .rounded_full()
                                .bg(theme.tab_group(group.color)),
                        )
                        .on_click(cx.listener(
                            move |workspace, _, _, cx| {
                                workspace.close_menu(cx);
                                workspace.move_to_group(index, id, cx);
                            },
                        )),
                    );
                }
                if current.is_some() {
                    card = card.child(
                        row("tab-menu-ungroup".into(), "Remove from group".into(), false).on_click(
                            cx.listener(move |workspace, _, _, cx| {
                                workspace.close_menu(cx);
                                workspace.remove_from_group(index, cx);
                            }),
                        ),
                    );
                }
                card = card.child(popover::menu_separator()).child(
                    row("tab-menu-close".into(), "Close tab".into(), false).on_click(cx.listener(
                        move |workspace, _, window, cx| {
                            workspace.close_menu(cx);
                            workspace.close(index, window, cx);
                        },
                    )),
                );
                // Only what would close something.
                if self.project_tabs().any(|other| other != index) {
                    card = card.child(
                        row(
                            "tab-menu-close-others".into(),
                            "Close other tabs".into(),
                            false,
                        )
                        .on_click(cx.listener(
                            move |workspace, _, window, cx| {
                                workspace.close_menu(cx);
                                workspace.close_others(index, window, cx);
                            },
                        )),
                    );
                }
                if self.project_tabs().any(|other| other > index) {
                    card = card.child(
                        row(
                            "tab-menu-close-right".into(),
                            "Close tabs to the right".into(),
                            false,
                        )
                        .on_click(cx.listener(
                            move |workspace, _, window, cx| {
                                workspace.close_menu(cx);
                                workspace.close_to_the_right(index, window, cx);
                            },
                        )),
                    );
                }
                card = card.child(
                    row("tab-menu-close-all".into(), "Close all tabs".into(), false).on_click(
                        cx.listener(move |workspace, _, window, cx| {
                            workspace.close_menu(cx);
                            workspace.close_all(window, cx);
                        }),
                    ),
                );
            }
            Target::Section(_) => return None,
            Target::Group(id) => {
                // The dropdown across the band: the group's tabs, the one on
                // screen marked, then what the group itself can do.
                if listing {
                    let members: Vec<usize> = self.member_indices(id).collect();
                    if members.is_empty() {
                        card = card.child(
                            div()
                                .px(px(8.0))
                                .py(px(6.0))
                                .text_color(theme.text_faint)
                                .child("No projects in this group"),
                        );
                    }
                    for index in members {
                        let surface = &self.tabs[index];
                        let key = format!("tab-menu-tab-{index}");
                        if let Some(input) = self
                            .renaming
                            .as_ref()
                            .filter(|renaming| renaming.target == Target::Tab(index))
                            .map(|renaming| renaming.input.clone())
                        {
                            card = card.child(
                                popover::menu_row(&theme, true, key)
                                    .child(self.render_rename(&input, cx)),
                            );
                            continue;
                        }
                        card = card.child(
                            popover::menu_row(&theme, index == self.active, key.clone())
                                .id(ElementId::Name(key.clone().into()))
                                .child(
                                    icons::icon(surface.icon())
                                        .flex_none()
                                        .size(px(14.0))
                                        .text_color(theme.text_muted),
                                )
                                .child(TruncatedLabel::new(
                                    ElementId::Name(format!("{key}-title").into()),
                                    surface.title(),
                                ))
                                // The tab's own menu, in place of the group's.
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(
                                        move |workspace, event: &MouseDownEvent, window, cx| {
                                            cx.stop_propagation();
                                            workspace.open_menu(
                                                Target::Tab(index),
                                                event.position,
                                                window,
                                                cx,
                                            );
                                        },
                                    ),
                                )
                                .on_click(cx.listener(
                                    move |workspace, event: &gpui_kit::ClickEvent, window, cx| {
                                        if event.click_count() == 2 {
                                            // Edited in its row, the dropdown open.
                                            workspace.dropdown_close = None;
                                            workspace.start_rename(Target::Tab(index), window, cx);
                                        } else {
                                            workspace.activate(index, window, cx);
                                            workspace.close_dropdown_unless_clicked_again(cx);
                                        }
                                    },
                                )),
                        );
                    }
                    card = card.child(popover::menu_separator());
                }
                let chosen = self.group(id).map(|group| group.color);
                let mut swatches = div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .py(px(6.0));
                for color in GroupColor::OFFERED {
                    let tone = theme.tab_group(color);
                    swatches = swatches.child(
                        div()
                            .id(ElementId::Name(
                                format!("tab-menu-color-{}", color.label()).into(),
                            ))
                            .size(px(SWATCH))
                            .rounded_full()
                            .bg(tone)
                            .cursor_pointer()
                            .border_2()
                            .border_color(if chosen == Some(color) {
                                theme.text
                            } else {
                                tone
                            })
                            .tooltip(crate::ui::tip::tip(color.label()))
                            .on_click(cx.listener(move |workspace, _, _, cx| {
                                workspace.set_group_color(id, color, cx);
                            })),
                    );
                }
                // Last, past a rule and drawn as a palette, so it reads as
                // "pick another", not as one more colour. A custom colour
                // tints the palette.
                let custom = match chosen {
                    Some(color @ GroupColor::Custom(_)) => Some(theme.tab_group(color)),
                    _ => None,
                };
                let named = GroupColor::OFFERED
                    .map(|color| theme.tab_group(color))
                    .to_vec();
                swatches = swatches
                    .child(div().w(px(1.0)).h(px(SWATCH)).bg(theme.border))
                    .child(
                        ColorPicker::new(&self.color_picker)
                            .icon(
                                Icon::default()
                                    .path(icons::PALLETE_2)
                                    .size(px(SWATCH))
                                    .text_color(custom.unwrap_or(theme.text_muted)),
                            )
                            .featured_colors(named)
                            .accessibility_label("Custom color")
                            .size(px(SWATCH))
                            .cursor_pointer(),
                    );
                card = card
                    .child(
                        row("tab-menu-rename-group".into(), "Rename group".into(), false).on_click(
                            cx.listener(move |workspace, _, window, cx| {
                                workspace.close_menu(cx);
                                workspace.start_rename(Target::Group(id), window, cx);
                            }),
                        ),
                    )
                    .child(swatches)
                    .child(popover::menu_separator())
                    .child(
                        row("tab-menu-ungroup-all".into(), "Ungroup".into(), false).on_click(
                            cx.listener(move |workspace, _, _, cx| {
                                workspace.close_menu(cx);
                                workspace.ungroup(id, cx);
                            }),
                        ),
                    )
                    .child(
                        row("tab-menu-close-group".into(), "Close group".into(), true).on_click(
                            cx.listener(move |workspace, _, window, cx| {
                                workspace.close_menu(cx);
                                workspace.close_group(id, window, cx);
                            }),
                        ),
                    );
            }
        }
        Some(popover::menu_at(
            "workspace-tab-menu",
            at,
            card.into_any_element(),
            closing,
        ))
    }
}

/// Where index `index` ends up once the tab at `from` moves to `to`.
fn shifted_by_move(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

/// Where index `index` ends up once the tab at `removed` closes.
fn shifted_by_removal(index: usize, removed: usize) -> usize {
    if index > removed { index - 1 } else { index }
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
        let menu = self.render_menu(cx);
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .rounded_b(px(corner))
            .bg(Theme::of(cx).bg)
            .child(content)
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    struct StripHost(Entity<Workspace>);

    impl Render for StripHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |workspace, cx| {
                workspace.render_tab_strip(None, false, cx)
            })
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
    fn a_renamed_tab_keeps_its_name_and_a_blank_one_gives_it_back(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                let rename = |workspace: &mut Workspace,
                              text: &str,
                              save,
                              window: &mut Window,
                              cx: &mut Context<Workspace>| {
                    workspace.start_rename(Target::Tab(1), window, cx);
                    let input = workspace.renaming.as_ref().unwrap().input.clone();
                    input.update(cx, |input, cx| input.set_text(text.to_string(), window, cx));
                    workspace.finish_rename(save, true, window, cx);
                };

                rename(workspace, "  Work  ", true, window, cx);
                assert_eq!(titles(workspace), vec!["Home", "Work"]);
                let saved = settings::current(cx).tabs.names.get("/repos/one").cloned();
                assert_eq!(saved.as_deref(), Some("Work"));

                rename(workspace, "Ignored", false, window, cx);
                assert_eq!(titles(workspace), vec!["Home", "Work"]);

                rename(workspace, "   ", true, window, cx);
                assert_eq!(titles(workspace), vec!["Home", "one"]);
                assert!(settings::current(cx).tabs.names.is_empty());
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn renaming_a_grouped_tab_brings_it_on_screen_first(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.open_repository("/repos/two".into(), window, cx);
                workspace.new_group(1, window, cx);
                workspace.finish_rename(true, true, window, cx);
                assert_eq!(workspace.active, 2);

                workspace.rename_tab(1, window, cx);
                assert_eq!(
                    workspace.active, 1,
                    "its group's plate shows the tab on screen"
                );
                assert_eq!(
                    workspace.renaming.as_ref().map(|renaming| renaming.target),
                    Some(Target::Tab(1))
                );
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn a_row_renamed_in_its_dropdown_saves_and_closes_it(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.new_group(1, window, cx);
                workspace.finish_rename(true, true, window, cx);
                let id = workspace.groups[0].id;
                workspace.menu.open(TabMenu {
                    at: gpui_kit::point(px(0.0), px(0.0)),
                    target: Target::Group(id),
                    listing: true,
                });
                assert!(workspace.dropdown_open(id));

                workspace.start_rename(Target::Tab(1), window, cx);
                let input = workspace.renaming.as_ref().unwrap().input.clone();
                input.update(cx, |input, cx| {
                    input.set_text("Work".to_string(), window, cx)
                });
                workspace.finish_rename(true, true, window, cx);

                assert_eq!(titles(workspace), vec!["Home", "Work"]);
                assert!(
                    !workspace.menu.is_open(),
                    "the edit was what it was open for"
                );
            })
            .expect("window update");
    }

    fn groups_of(workspace: &Workspace) -> Vec<Option<u64>> {
        (0..workspace.tabs.len())
            .map(|index| workspace.group_of(index))
            .collect()
    }

    #[gpui_kit::test]
    fn a_tab_moved_into_a_group_lands_beside_its_tabs(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                for name in ["one", "two", "three"] {
                    workspace.open_repository(format!("/repos/{name}"), window, cx);
                }
                workspace.new_group(1, window, cx);
                workspace.finish_rename(true, true, window, cx);
                let id = workspace.groups[0].id;
                assert_eq!(workspace.groups[0].name, "New group");
                workspace.move_to_group(3, id, cx);

                assert_eq!(titles(workspace), vec!["Home", "one", "three", "two"]);
                assert_eq!(groups_of(workspace), vec![None, Some(id), Some(id), None]);
                assert_eq!(workspace.active, 2, "the active tab follows its move");
                assert_eq!(
                    workspace.strip_entries(true),
                    vec![
                        Target::Section(Section::Groups),
                        Target::Group(id),
                        Target::Tab(1),
                        Target::Tab(2),
                        Target::Section(Section::Projects),
                        Target::Tab(3),
                    ]
                );
                // Across the band the group is one plate.
                assert_eq!(
                    workspace.strip_entries(false),
                    vec![Target::Group(id), Target::Tab(3)]
                );

                workspace.remove_from_group(1, cx);
                assert_eq!(titles(workspace), vec!["Home", "three", "one", "two"]);
                assert_eq!(groups_of(workspace), vec![None, Some(id), None, None]);

                workspace.remove_from_group(1, cx);
                assert_eq!(workspace.groups.len(), 1, "an emptied group stays");
                workspace.ungroup(id, cx);
                assert!(workspace.groups.is_empty());
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn a_folded_group_hides_its_tabs_down_the_column(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.open_repository("/repos/two".into(), window, cx);
                workspace.new_group(1, window, cx);
                workspace.finish_rename(true, true, window, cx);
                let id = workspace.groups[0].id;

                workspace.toggle_group(id, cx);
                assert_eq!(
                    workspace.strip_entries(true),
                    vec![
                        Target::Section(Section::Groups),
                        Target::Group(id),
                        Target::Section(Section::Projects),
                        Target::Tab(2),
                    ]
                );

                // Opening a folded tab from Home unfolds its group.
                workspace.open_repository("/repos/one".into(), window, cx);
                assert!(!workspace.groups[0].collapsed);
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn closing_a_group_closes_its_tabs_and_forgets_it(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.open_repository("/repos/one".into(), window, cx);
                workspace.open_repository("/repos/two".into(), window, cx);
                workspace.new_group(2, window, cx);
                workspace.finish_rename(true, true, window, cx);
                let id = workspace.groups[0].id;
                workspace.move_to_group(1, id, cx);

                workspace.close_group(id, window, cx);
                assert_eq!(titles(workspace), vec!["Home"]);
                assert!(workspace.groups.is_empty());
                assert!(settings::current(cx).tabs.groups.is_empty());
            })
            .expect("window update");
    }

    #[gpui_kit::test]
    fn a_new_group_starts_empty_with_a_name_of_its_own(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                workspace.create_group(window, cx);
                workspace.finish_rename(true, true, window, cx);
                workspace.create_group(window, cx);
                // A blank name keeps the one it had.
                let input = workspace.renaming.as_ref().unwrap().input.clone();
                input.update(cx, |input, cx| input.set_text("  ".to_string(), window, cx));
                workspace.finish_rename(true, true, window, cx);

                let names: Vec<&str> = workspace.groups.iter().map(|g| g.name.as_str()).collect();
                assert_eq!(names, vec!["New group", "New group 2"]);
                assert_ne!(workspace.groups[0].color, workspace.groups[1].color);
            })
            .expect("window update");
    }

    #[test]
    fn a_move_shifts_the_tabs_between_its_ends() {
        assert_eq!(shifted_by_move(1, 1, 3), 3);
        assert_eq!(shifted_by_move(2, 1, 3), 1);
        assert_eq!(shifted_by_move(3, 1, 3), 2);
        assert_eq!(shifted_by_move(1, 3, 1), 2);
        assert_eq!(shifted_by_move(4, 1, 3), 4);
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
    fn the_bulk_closes_close_only_projects_and_keep_the_one_asked_for(cx: &mut TestAppContext) {
        let window = workspace(cx);
        window
            .update(cx, |workspace, window, cx| {
                let open = |workspace: &mut Workspace,
                            window: &mut Window,
                            cx: &mut Context<Workspace>| {
                    for name in ["one", "two", "three", "four"] {
                        workspace.open_repository(format!("/repos/{name}"), window, cx);
                    }
                };

                open(workspace, window, cx);
                workspace.activate(4, window, cx);
                workspace.close_to_the_right(2, window, cx);
                assert_eq!(titles(workspace), vec!["Home", "one", "two"]);
                assert_eq!(
                    workspace.active, 2,
                    "the active tab closed: its menu's tab shows"
                );

                workspace.activate(1, window, cx);
                workspace.close_to_the_right(1, window, cx);
                assert_eq!(titles(workspace), vec!["Home", "one"]);
                assert_eq!(workspace.active, 1, "the active tab stays");

                workspace.close_all(window, cx);
                open(workspace, window, cx);
                workspace.close_others(3, window, cx);
                assert_eq!(titles(workspace), vec!["Home", "three"]);
                assert_eq!(workspace.active, 1);

                workspace.open_settings(window, cx);
                workspace.close_all(window, cx);
                assert_eq!(titles(workspace), vec!["Home", "Settings"]);
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
        //
        // Tabs open one at a time in the app, so the strip has reached its
        // full width before one falls off its end. GPUI scrolls an item into
        // view against the viewport of the frame before; fifty at once would
        // measure them against the one-chip strip that frame drew.
        draw(&mut visual);
        let open = |range: std::ops::Range<usize>, visual: &mut gpui_kit::VisualTestContext| {
            visual.update(|window, cx| {
                workspace.update(cx, |workspace, cx| {
                    for index in range {
                        workspace.open_repository(format!("/repos/{index:02}"), window, cx);
                    }
                })
            });
        };
        open(0..49, &mut visual);
        draw(&mut visual);
        open(49..50, &mut visual);
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
                let _ = workspace.render_tab_strip(None, false, cx);
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
