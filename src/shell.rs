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

use crate::features::workspace::Workspace;
use crate::keymap;
use crate::ui::AppTitleBar;
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;

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

    /// The top band: the open projects, and the platform's own window
    /// controls.
    ///
    /// The strip lives here rather than below because that is what the band is
    /// for — it is the width the window is not using for anything else, and a
    /// second bar under it would cost a row of graph for nothing.
    fn render_title_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (strip, home_active, settings_active) = self.workspace.update(cx, |workspace, cx| {
            (
                workspace.render_tab_strip(cx),
                workspace.home_active(),
                workspace.settings_active(),
            )
        });
        let content = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .child(self.render_band_button(
                "shell-home",
                crate::ui::icons::HOME_01,
                "Home",
                home_active,
                cx,
                |workspace, window, cx| workspace.go_home(window, cx),
            ))
            .child(self.render_band_button(
                "shell-settings",
                crate::ui::icons::SETTINGS_01,
                "Settings",
                settings_active,
                cx,
                |workspace, window, cx| workspace.open_settings(window, cx),
            ))
            // A rule between the two places that
            // are always there, apart from the projects that were opened.
            .child(
                div()
                    .mx(px(4.))
                    .w(px(1.))
                    .h(px(18.))
                    .flex_none()
                    .bg(crate::ui::theme::Theme::of(cx).border),
            )
            .child(div().flex().min_w_0().child(strip))
            .child(self.render_band_button(
                "shell-open-project",
                crate::ui::icons::PLUS,
                "Open another project",
                false,
                cx,
                |workspace, window, cx| workspace.go_home(window, cx),
            ));
        AppTitleBar::new().content(content)
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
        _cx: &mut Context<Self>,
        action: fn(&mut Workspace, &mut Window, &mut Context<Workspace>),
    ) -> impl IntoElement + use<> {
        let workspace = self.workspace.clone();
        div()
            .id(id)
            .size(px(24.))
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
                crate::ui::motion::fade_color(
                    id,
                    crate::ui::theme::wash(0.0),
                    crate::ui::theme::wash(0.11),
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
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.workspace.clone()),
            )
    }
}
