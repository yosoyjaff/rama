//! The side panel: what this repository contains, in four lists.
//!
//! Local branches, remote branches, tags and stashes, each a section that
//! folds, each with its count in the header. A filter across the top narrows
//! every section at once, and clicking a branch checks it out.
//!
//! The panel reads refs, never the history: `for-each-ref` answers all of it
//! without walking a single commit, which is what keeps it instant on a
//! repository the graph takes a moment to page through.

use std::collections::HashSet;
use std::path::Path;

use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Render, SharedString, Subscription, Task, Window, div, prelude::*, px,
};

use crate::git::actions::{GitAction, StashEntry};
use crate::git::{BranchRef, RepoRefs};
use crate::ui::chrome::EDGE_INSET;
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme::{Theme, wash};
use crate::ui::typography::ui_rems;
use crate::ui::{icons, motion};

/// Width of the panel. Wide enough for a branch name of ordinary length at
/// 11px, narrow enough that it never competes with the graph.
pub const NAV_WIDTH: f32 = 224.0;
/// Height of a section header.
const SECTION_HEIGHT: f32 = 28.0;
/// Height of one row inside a section.
const ROW_HEIGHT: f32 = 24.0;
/// How far a row is indented past its section's chevron.
const ROW_INDENT: f32 = 28.0;

/// Which list a section shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Section {
    Local,
    Remote,
    Tags,
    Stashes,
}

impl Section {
    #[cfg(test)]
    const ALL: [Self; 4] = [Self::Local, Self::Remote, Self::Tags, Self::Stashes];

    fn label(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Remote => "Remote",
            Self::Tags => "Tags",
            Self::Stashes => "Stashes",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Local => icons::LAPTOP,
            Self::Remote => icons::CLOUD,
            Self::Tags => icons::TAG,
            Self::Stashes => icons::ARCHIVE_MINIMALISTIC,
        }
    }

    fn empty_hint(self, filtering: bool) -> &'static str {
        if filtering {
            return "No matches";
        }
        match self {
            Self::Local => "No local branches",
            Self::Remote => "No remote branches",
            Self::Tags => "No tags",
            Self::Stashes => "Nothing stashed",
        }
    }
}

/// What the panel asks the rest of the screen to do.
pub enum RepoNavEvent {
    /// A ref was checked out, or a stash applied: the graph and the panels
    /// reading this checkout are now behind.
    RepositoryChanged,
    /// The action failed, with git's own words.
    ActionFailed(SharedString),
}

pub struct RepoNav {
    repo: Option<String>,
    refs: RepoRefs,
    collapsed: HashSet<Section>,
    filter: Entity<TextInput>,
    query: String,
    loading: bool,
    /// The ref an action is running on, so its row can say so.
    busy: Option<String>,
    task: Option<Task<()>>,
    git_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    _filter_events: Subscription,
}

impl EventEmitter<RepoNavEvent> for RepoNav {}

impl RepoNav {
    pub fn new(repo: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new("Filter", window, cx).with_text_size(11.0));
        let filter_events = cx.subscribe(&filter, |this: &mut Self, filter, event, cx| {
            if matches!(event, TextInputEvent::Edited) {
                this.query = filter.read(cx).text().to_string();
                cx.notify();
            }
        });
        let mut panel = Self {
            repo,
            refs: RepoRefs::default(),
            // Branches are what the panel is opened for; the rest starts
            // folded so the two lists that matter are both on screen.
            collapsed: HashSet::from([Section::Tags, Section::Stashes]),
            filter,
            query: String::new(),
            loading: false,
            busy: None,
            task: None,
            git_task: None,
            focus_handle: cx.focus_handle(),
            _filter_events: filter_events,
        };
        panel.reload(cx);
        panel
    }

    /// Point the panel at another checkout.
    pub fn open_repo(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        if self.repo == repo {
            return;
        }
        self.repo = repo;
        self.refs = RepoRefs::default();
        self.reload(cx);
    }

    /// Read the refs again. Called on open and after anything that moves one.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            self.refs = RepoRefs::default();
            cx.notify();
            return;
        };
        self.loading = true;
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::git::repo_refs(Path::new(&repo)) })
                .await;
            this.update(cx, |panel, cx| {
                panel.loading = false;
                if let Ok(refs) = result {
                    panel.refs = refs;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn run(&mut self, name: String, action: GitAction, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(name);
        cx.notify();
        self.git_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::git::actions::run(Path::new(&repo), action) })
                .await;
            this.update(cx, |panel, cx| {
                panel.busy = None;
                match result {
                    Ok(_) => {
                        panel.reload(cx);
                        cx.emit(RepoNavEvent::RepositoryChanged);
                    }
                    Err(error) => cx.emit(RepoNavEvent::ActionFailed(error.to_string().into())),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn matches(&self, name: &str) -> bool {
        let query = self.query.trim().to_lowercase();
        query.is_empty() || name.to_lowercase().contains(&query)
    }

    fn local(&self) -> Vec<&BranchRef> {
        self.refs
            .local
            .iter()
            .filter(|branch| self.matches(&branch.name))
            .collect()
    }

    fn remote(&self) -> Vec<&BranchRef> {
        self.refs
            .remote
            .iter()
            .filter(|branch| self.matches(&branch.name))
            .collect()
    }

    fn tags(&self) -> Vec<&String> {
        self.refs
            .tags
            .iter()
            .filter(|tag| self.matches(tag))
            .collect()
    }

    fn stashes(&self) -> Vec<&StashEntry> {
        self.refs
            .stashes
            .iter()
            .filter(|stash| self.matches(&stash.message))
            .collect()
    }

    /// How many refs the panel is showing, which is what its header counts.
    fn viewing(&self) -> usize {
        self.local().len() + self.remote().len()
    }

    fn toggle(&mut self, section: Section, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&section) {
            self.collapsed.insert(section);
        }
        cx.notify();
    }

    fn render_section(
        &mut self,
        section: Section,
        count: Option<usize>,
        rows: Vec<AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = !self.collapsed.contains(&section);
        let id = SharedString::from(format!("repo-nav-{}", section.label().to_lowercase()));
        let fade = id.clone();
        let filtering = !self.query.trim().is_empty();
        div()
            .w_full()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme.border.opacity(0.7))
            .child(
                div()
                    .id(id)
                    .h(px(SECTION_HEIGHT))
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(EDGE_INSET))
                    .cursor_pointer()
                    .bg(motion::fade_color(&fade, wash(0.0), wash(0.06)))
                    .on_hover(motion::on_hover_fade(&fade))
                    .on_click(cx.listener(move |panel, _, _, cx| panel.toggle(section, cx)))
                    .child(
                        icons::icon(if open {
                            icons::ALT_ARROW_DOWN
                        } else {
                            icons::ALT_ARROW_RIGHT
                        })
                        .size(px(11.0))
                        .text_color(theme.text_faint),
                    )
                    .child(
                        icons::icon(section.icon())
                            .size(px(13.0))
                            .text_color(theme.text_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            // Uppercase with tracking: a section label is a
                            // divider that happens to have words in it.
                            .text_size(ui_rems(10.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(theme.text_muted)
                            .child(SharedString::from(section.label().to_uppercase())),
                    )
                    .children(count.map(|count| {
                        div()
                            .flex_none()
                            .text_size(ui_rems(10.0))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(count.to_string()))
                    })),
            )
            .when(open, |element| {
                element.child(if rows.is_empty() {
                    div()
                        .pl(px(ROW_INDENT))
                        .pr(px(EDGE_INSET))
                        .py(px(5.0))
                        .text_size(ui_rems(10.5))
                        .text_color(theme.text_faint.opacity(0.8))
                        .child(SharedString::from(section.empty_hint(filtering)))
                        .into_any_element()
                } else {
                    div()
                        .pb(px(2.0))
                        .flex()
                        .flex_col()
                        .children(rows)
                        .into_any_element()
                })
            })
            .into_any_element()
    }

    /// One ref row: a glyph, the name, and — for the checked-out branch — the
    /// dot that says "you are here".
    ///
    /// Long in the arguments because a row is: what it is called, what it
    /// looks like, what it says on the right and what it does. Bundling those
    /// into a struct would name the same four things twice.
    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &self,
        id: impl Into<SharedString>,
        icon: &'static str,
        label: String,
        current: bool,
        trailing: Option<String>,
        theme: &Theme,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let id: SharedString = id.into();
        let busy = self.busy.as_deref() == Some(label.as_str());
        let tint = if current {
            theme.success
        } else {
            theme.text_faint
        };
        div()
            .id(id.clone())
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .pl(px(ROW_INDENT))
            .pr(px(EDGE_INSET))
            .cursor_pointer()
            .bg(motion::fade_color(&id, wash(0.0), wash(0.07)))
            .on_hover(motion::on_hover_fade(&id))
            .on_click(cx.listener(move |panel, _, _, cx| on_click(panel, cx)))
            .child(
                icons::icon(icon)
                    .size(px(11.0))
                    .text_color(tint.opacity(if current { 1.0 } else { 0.8 })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_rems(11.0))
                    .when(current, |label| {
                        label.font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    })
                    .text_color(if current {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .child(SharedString::from(label)),
            )
            .children(trailing.map(|text| {
                div()
                    .flex_none()
                    .text_size(ui_rems(10.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(text))
            }))
            .when(busy, |row| row.opacity(0.5))
            .when(current, |row| {
                row.child(
                    div()
                        .flex_none()
                        .size(px(5.0))
                        .rounded_full()
                        .bg(theme.success),
                )
            })
            .into_any_element()
    }
}

impl Focusable for RepoNav {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RepoNav {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        // The panel sits in the window's bottom-left corner.
        let corner = crate::ui::window_frame::corner_radius(window);
        let viewing = self.viewing();

        let local_refs: Vec<BranchRef> = self.local().into_iter().cloned().collect();
        let local: Vec<AnyElement> = local_refs
            .into_iter()
            .map(|branch| {
                let name = branch.name.clone();
                let checkout = branch.name.clone();
                let trailing = match (branch.ahead, branch.behind) {
                    (0, 0) => None,
                    (ahead, 0) => Some(format!("↑{ahead}")),
                    (0, behind) => Some(format!("↓{behind}")),
                    (ahead, behind) => Some(format!("↑{ahead} ↓{behind}")),
                };
                self.render_row(
                    format!("repo-nav-local-{name}"),
                    icons::GIT_BRANCH,
                    name.clone(),
                    branch.current,
                    trailing,
                    &theme,
                    cx,
                    move |panel, cx| {
                        // Already out: a checkout would be a no-op that still
                        // reloads the whole graph.
                        if !branch.current {
                            panel.run(
                                checkout.clone(),
                                GitAction::Checkout {
                                    name: checkout.clone(),
                                },
                                cx,
                            );
                        }
                    },
                )
            })
            .collect();

        let remote_refs: Vec<BranchRef> = self.remote().into_iter().cloned().collect();
        let remote: Vec<AnyElement> = remote_refs
            .into_iter()
            .map(|branch| {
                let name = branch.name.clone();
                // `origin/feature` checks out as `feature`, tracking it.
                let local_name = name
                    .split_once('/')
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_else(|| name.clone());
                let reference = name.clone();
                self.render_row(
                    format!("repo-nav-remote-{name}"),
                    icons::CLOUD,
                    name.clone(),
                    false,
                    None,
                    &theme,
                    cx,
                    move |panel, cx| {
                        panel.run(
                            reference.clone(),
                            GitAction::CheckoutTracking {
                                local: local_name.clone(),
                                reference: reference.clone(),
                            },
                            cx,
                        );
                    },
                )
            })
            .collect();

        let tags_refs: Vec<String> = self.tags().into_iter().cloned().collect();
        let tags: Vec<AnyElement> = tags_refs
            .into_iter()
            .map(|tag| {
                let sha = tag.clone();
                self.render_row(
                    format!("repo-nav-tag-{tag}"),
                    icons::TAG,
                    tag.clone(),
                    false,
                    None,
                    &theme,
                    cx,
                    move |panel, cx| {
                        // A tag is not a branch: checking one out detaches
                        // HEAD, which is what git does and what the row says.
                        panel.run(
                            sha.clone(),
                            GitAction::CheckoutDetached { sha: sha.clone() },
                            cx,
                        );
                    },
                )
            })
            .collect();

        let stashes_refs: Vec<StashEntry> = self.stashes().into_iter().cloned().collect();
        let stashes: Vec<AnyElement> = stashes_refs
            .into_iter()
            .map(|stash| {
                let index = stash.index;
                let label = format!("{}  {}", stash.selector, stash.message);
                self.render_row(
                    format!("repo-nav-stash-{}", stash.index),
                    icons::ARCHIVE_UP_MINIMALISTIC,
                    label,
                    false,
                    None,
                    &theme,
                    cx,
                    move |panel, cx| {
                        panel.run(
                            format!("stash@{{{index}}}"),
                            GitAction::StashPop { index },
                            cx,
                        );
                    },
                )
            })
            .collect();

        let local_section =
            self.render_section(Section::Local, Some(local.len()), local, &theme, cx);
        let remote_section =
            self.render_section(Section::Remote, Some(remote.len()), remote, &theme, cx);
        let tags_section = self.render_section(Section::Tags, Some(tags.len()), tags, &theme, cx);
        let stashes_section =
            self.render_section(Section::Stashes, Some(stashes.len()), stashes, &theme, cx);

        div()
            .track_focus(&self.focus_handle)
            .w(px(NAV_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .rounded_bl(px(corner))
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border)
            .child(
                // What the panel is showing, counted, above the filter.
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(EDGE_INSET + 4.0))
                    .py(px(6.0))
                    .text_size(ui_rems(10.5))
                    .text_color(theme.text_muted)
                    .child("Viewing")
                    .child(
                        div()
                            .text_color(theme.text)
                            .child(SharedString::from(viewing.to_string())),
                    ),
            )
            .child(
                div().flex_none().px(px(EDGE_INSET)).pb(px(6.0)).child(
                    div()
                        .h(px(26.0))
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(7.0))
                        .rounded_md()
                        .bg(theme.input_bg)
                        .border_1()
                        .border_color(theme.border)
                        .child(div().flex_1().min_w_0().child(self.filter.clone()))
                        .child(
                            icons::icon(icons::MAGNIFER)
                                .size(px(12.0))
                                .text_color(theme.text_faint),
                        ),
                ),
            )
            .child(
                div()
                    .id("repo-nav-sections")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .child(local_section)
                    .child(remote_section)
                    .child(tags_section)
                    .child(stashes_section),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_names_itself_and_says_when_it_is_empty() {
        for section in Section::ALL {
            assert!(!section.label().is_empty());
            assert!(!section.icon().is_empty());
            assert_eq!(section.empty_hint(true), "No matches");
            assert_ne!(section.empty_hint(false), "No matches");
        }
    }
}
