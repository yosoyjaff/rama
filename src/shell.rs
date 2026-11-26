//! The shell: the one view a window's `Root` wraps.
//!
//! It owns the window's chrome — the title bar and the overlay layers — and
//! would own the handlers for any application-wide action. It owns no product
//! logic: that lives in [`crate::features`], and the shell only decides what is
//! mounted where.
//!
//! It mounts the workspace — every open project, one tab each — and gives its
//! tab strip the title bar's wide slot. Anything else of that kind is composed
//! here, in this `render`.

use crate::features::history::FindCommits;
use crate::features::workspace::Workspace;
use crate::keymap;
use crate::settings::TabSide;
use crate::ui::AppTitleBar;
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// The tabs' sidebar, when they run down a side instead of across the band.
const TAB_COLUMN_WIDTH: f32 = 200.;
const TAB_COLUMN_PAD: f32 = 8.;
/// The + on the column's Projects heading.
const SECTION_BUTTON: f32 = 20.;

pub struct Shell {
    focus_handle: FocusHandle,
    /// The mounted screen. An entity, so every open project keeps its state
    /// across every frame the shell redraws.
    workspace: Entity<Workspace>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        // Settings → About shows the updater; the shell is what knows both.
        let update_panel = crate::updater::panel(cx);
        workspace.update(cx, |workspace, _| workspace.set_update_panel(update_panel));

        // The keyboard starts in the mounted screen, not on the chrome.
        workspace.read(cx).focus_handle(cx).focus(window, cx);

        Self {
            focus_handle: cx.focus_handle(),
            workspace,
        }
    }

    /// Open a repository as a tab, or bring its tab to the front.
    pub fn open_repository(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.update(cx, |workspace, cx| {
            workspace.open_repository(path, window, cx)
        });
    }

    /// The top band: the open projects, and the platform's own window
    /// controls.
    ///
    /// The strip lives here rather than below because that is what the band is
    /// for — it is the width the window is not using for anything else, and a
    /// second bar under it would cost a row of graph for nothing.
    fn render_title_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // The settings are a place of their own: no tabs while they are open,
        // and their rail's Back to app is the way out.
        let settings_open = self.workspace.read(cx).settings_active();
        let across = crate::settings::current(cx).look.tabs == TabSide::Top && !settings_open;
        let strip = across.then(|| self.render_tabs(false, cx));
        // A dragged tab may be dropped anywhere along the band.
        let dragging = across && cx.has_active_drag();
        let (home_active, settings_active) = self.workspace.update(cx, |workspace, _| {
            (workspace.home_active(), workspace.settings_active())
        });
        let content = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .when(dragging, |content| content.flex_1())
            .child(self.render_band_button(
                "shell-home",
                crate::ui::icons::HOME_01,
                "Home",
                home_active,
                false,
                |workspace, window, cx| workspace.go_home(window, cx),
            ))
            .child(self.render_band_button(
                "shell-settings",
                crate::ui::icons::SETTINGS_01,
                "Settings",
                settings_active,
                false,
                |workspace, window, cx| workspace.open_settings(window, cx),
            ))
            // A rule between the two places that
            // are always there, apart from the projects that were opened.
            .when_some(strip, |band, strip| {
                band.child(
                    div()
                        .ml(px(4.))
                        // The strip opens with room for a drop bar; this side
                        // gives it back, so the first tab stays where it was.
                        .mr(px(4. - crate::ui::tab_strip::DROP_ROOM))
                        .w(px(1.))
                        .h(px(18.))
                        .flex_none()
                        .bg(crate::ui::theme::Theme::of(cx).border),
                )
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .h_full()
                        .when(dragging, |slot| slot.flex_1())
                        .child(strip),
                )
            });
        // With the tabs down a side the band holds only Home and Settings,
        // and a rule before the window buttons would part nothing.
        let side_tabs = crate::settings::current(cx).look.tabs != TabSide::Top;
        AppTitleBar::new()
            .content(content)
            .content_grows(dragging)
            .when(side_tabs, |bar| bar.without_rule())
    }

    /// The open projects' tabs, across the band or down a `column`, with
    /// the button that opens another right after the last one, as close to
    /// it as the tabs are to each other.
    fn render_tabs(&mut self, column: bool, cx: &mut Context<Self>) -> AnyElement {
        // Across the band the button follows the last tab; a column has it
        // on its Projects heading instead.
        let open_project = match column {
            true => self.render_open_project_button(cx),
            false => self
                .render_band_button(
                    "shell-open-project",
                    crate::ui::icons::PLUS,
                    "Open another project",
                    false,
                    true,
                    |workspace, _, cx| workspace.pick_project(cx),
                )
                .into_any_element(),
        };
        self.workspace.update(cx, |workspace, cx| {
            workspace.render_tab_strip(Some(open_project), column, cx)
        })
    }

    /// The + at the end of the column's heading: a quiet glyph that lights
    /// up under the pointer.
    fn render_open_project_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = crate::ui::theme::Theme::of(cx);
        let (rest, lit) = (theme.text_muted, theme.text);
        let workspace = self.workspace.clone();
        let id = "shell-open-project";
        div()
            .id(id)
            .group(id)
            .size(px(SECTION_BUTTON))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|style| style.bg(crate::ui::theme::wash(0.10)))
            .tooltip(crate::ui::tip::tip("Open project"))
            .on_click(move |_, _, cx| {
                workspace.update(cx, |workspace, cx| workspace.pick_project(cx));
            })
            .child(
                crate::ui::icons::icon(crate::ui::icons::PLUS)
                    .flex_none()
                    .size(px(14.))
                    .text_color(rest)
                    .group_hover(id, move |style| style.text_color(lit)),
            )
            .into_any_element()
    }

    /// The tabs as a sidebar down one edge of the window, under the band.
    /// It meets the window's bottom corner on its side, so it rounds it.
    fn render_tab_column(
        &mut self,
        side: TabSide,
        corner: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let strip = self.render_tabs(true, cx);
        let theme = crate::ui::theme::Theme::of(cx);
        div()
            .flex_none()
            .w(px(TAB_COLUMN_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .px(px(TAB_COLUMN_PAD))
            .pt(px(4.))
            .pb(px(TAB_COLUMN_PAD))
            .bg(theme.surface)
            .border_color(theme.border.opacity(0.6))
            .map(|column| match side {
                TabSide::Right => column.border_l_1().rounded_br(px(corner)),
                _ => column.border_r_1().rounded_bl(px(corner)),
            })
            // Its section labels — Groups, Projects — scroll with the rows.
            .child(strip)
            .into_any_element()
    }

    /// A control on the title band.
    ///
    /// It has to `occlude()` and stop the press itself: anything on the drag
    /// strip that does not is a button that moves the window instead, and a
    /// double click on it zooms.
    fn render_band_button(
        &self,
        id: &'static str,
        icon: &'static str,
        tooltip: &'static str,
        active: bool,
        plate: bool,
        action: fn(&mut Workspace, &mut Window, &mut Context<Workspace>),
    ) -> impl IntoElement + use<> {
        let workspace = self.workspace.clone();
        div()
            .id(id)
            .size(px(if plate {
                crate::ui::tab_strip::PLATE_HEIGHT
            } else {
                24.
            }))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            // The place the window is showing stays lit, the way the active
            // tab does.
            .bg(if active {
                crate::ui::theme::wash(0.10)
            } else {
                // A plate, like the tabs it follows; a bare glyph otherwise.
                crate::ui::motion::fade_color(
                    id,
                    crate::ui::theme::wash(if plate { 0.04 } else { 0.0 }),
                    crate::ui::theme::wash(if plate { 0.07 } else { 0.11 }),
                )
            })
            .on_hover(crate::ui::motion::on_hover_fade(id))
            .tooltip(crate::ui::tip::tip(tooltip))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                workspace.update(cx, |workspace, cx| action(workspace, window, cx));
            })
            .child(BandGlyph { icon, active })
    }
}

/// A band control's glyph, tinted at render time from the theme.
#[derive(IntoElement)]
struct BandGlyph {
    icon: &'static str,
    active: bool,
}

impl RenderOnce for BandGlyph {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = crate::ui::theme::Theme::of(cx);
        crate::ui::icons::icon(self.icon)
            .size(px(15.))
            .text_color(if self.active {
                theme.text
            } else {
                theme.text_muted
            })
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let corner = crate::ui::window_frame::corner_radius(window);
        v_flex()
            .id("shell")
            .relative()
            .rounded(px(corner))
            // The scope an application-wide binding is bound in. Its handlers
            // go on this element, which is the one that declares the context.
            .key_context(keymap::CONTEXT)
            .on_action(cx.listener(|shell, _: &FindCommits, window, cx| {
                shell
                    .workspace
                    .update(cx, |workspace, cx| workspace.find(window, cx));
            }))
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // A card tone and a bottom rule, so the band reads as chrome and the content starts under a line.
            .child({
                let theme = crate::ui::theme::Theme::of(cx);
                div()
                    .w_full()
                    .flex_none()
                    .rounded_t(px(corner))
                    .bg(theme.surface)
                    .border_b_1()
                    .border_color(theme.border.opacity(0.6))
                    .child(self.render_title_bar(cx))
            })
            // `min_h_0` is what lets this row shrink around a tall child: a
            // flex item keeps its content-based minimum height until told
            // otherwise, and a view that scrolls needs the row to resolve
            // against the window instead of against its own content.
            .child({
                let side = crate::settings::current(cx).look.tabs;
                let settings_open = self.workspace.read(cx).settings_active();
                let column = (side != TabSide::Top && !settings_open)
                    .then(|| self.render_tab_column(side, corner, cx));
                let workspace = div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.workspace.clone());
                let row = div().flex_1().min_h_0().w_full().flex().flex_row();
                match side {
                    TabSide::Right => row.child(workspace).children(column),
                    _ => row.children(column).child(workspace),
                }
            })
    }
}
