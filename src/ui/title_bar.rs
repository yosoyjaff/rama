//! The band at the top of the window: it moves the window, carries the tab
//! strip, and on Linux and Windows holds the window's own buttons.
//!
//! GPUI Kit's `TitleBar` is not used because it always paints its own
//! minimize/maximize/close, in one fixed order on the right. Here each
//! platform gets what it expects:
//!
//! * **macOS** — nothing is drawn; the native traffic lights are placed for
//!   this band by [`AppTitleBar::window_options`] and the band leaves room.
//! * **Windows** — the system glyph font at the system button width, each
//!   button tagged with its `WindowControlArea` so Windows handles it (Snap
//!   Layouts included).
//! * **Linux** — the desktop's `button-layout` picks the corner and order,
//!   see [`arrange`]. Nothing is drawn under server-side decorations, where
//!   the window manager already painted a frame.

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Decorations, Div, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Render, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled, TitlebarOptions, Window, WindowButton,
    WindowControlArea, WindowOptions, div, point, prelude::FluentBuilder as _, px, rems, rgb, svg,
    white,
};

// Window geometry, not product UI: these must stay put when the font size
// changes, so they are pixels rather than rems.

/// Height of the band.
pub const TITLE_BAR_HEIGHT: Pixels = px(38.);
/// Content sits this far below the band's top so it looks centred against
/// the window's rounded corner.
const TOP_NUDGE: Pixels = px(2.);
/// Inset on both ends of the band.
const EDGE: Pixels = px(10.);
/// Room kept free on the left for the macOS traffic lights.
const MAC_LIGHTS_ROOM: Pixels = px(80.);
/// The traffic lights' distance from the window's left edge.
const MAC_LIGHTS_X: Pixels = px(14.);
/// Diameter of one traffic light, to centre them vertically.
const MAC_LIGHT: Pixels = px(12.);
/// Empty band that always stays free to grab the window by.
const GRAB_ROOM: Pixels = px(48.);

/// One Linux caption button, its glyph, and the space between buttons.
const BUTTON: Pixels = px(24.);
const BUTTON_GLYPH: Pixels = px(16.);
const BUTTON_GAP: Pixels = px(2.);
/// Space between the Linux buttons and the content next to them.
const BUTTONS_GAP: Pixels = px(6.);
/// The rule in front of the trailing buttons.
const RULE_HEIGHT: Pixels = px(20.);
const RULE_LEAD: Pixels = px(4.);

/// Width of one Windows caption button, the system's own metric.
const WIN_BUTTON: Pixels = px(36.);

/// The close button's hover red. Not a theme colour: it is the system's, and
/// a theme must not change it.
const SYSTEM_CLOSE_RED: u32 = 0x00e8_1123;

/// The window's title band. `content` is the stretch that holds the tabs;
/// the rest of the band drags the window.
#[derive(IntoElement, Default)]
pub struct AppTitleBar {
    content: Option<AnyElement>,
}

impl AppTitleBar {
    pub fn new() -> Self {
        Self::default()
    }

    /// The window options the band relies on: a see-through system title
    /// bar, the traffic lights centred in the band, and the app in charge of
    /// dragging — otherwise macOS also handles the double click and holds
    /// every click on the band back while it waits for a second one.
    pub fn window_options() -> WindowOptions {
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: true,
                traffic_light_position: Some(point(MAC_LIGHTS_X, lights_y())),
            }),
            app_owns_titlebar_drag: true,
            ..Default::default()
        }
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }
}

fn lights_y() -> Pixels {
    (TITLE_BAR_HEIGHT - MAC_LIGHT) / 2.
}

/// Which buttons go on each side, given the desktop's layout and what the
/// window can do.
///
/// Buttons the window cannot use are dropped. Ones the layout leaves out are
/// added back: with client-side decorations these are the only buttons the
/// window has, and GNOME's default `appmenu:close` would otherwise leave a
/// window that cannot be minimized. Added buttons join close's side (the
/// right if close is nowhere), with close on the outer edge.
fn arrange(
    left: &[WindowButton],
    right: &[WindowButton],
    can: impl Fn(WindowButton) -> bool,
) -> (Vec<WindowButton>, Vec<WindowButton>) {
    let keep = |side: &[WindowButton]| -> Vec<WindowButton> {
        side.iter().copied().filter(|b| can(*b)).collect()
    };
    let (mut left, mut right) = (keep(left), keep(right));
    let all = [
        WindowButton::Minimize,
        WindowButton::Maximize,
        WindowButton::Close,
    ];
    let missing: Vec<WindowButton> = all
        .into_iter()
        .filter(|b| can(*b) && !left.contains(b) && !right.contains(b))
        .collect();
    if missing.is_empty() {
        return (left, right);
    }
    let close_left = left.contains(&WindowButton::Close);
    let home = if close_left { &mut left } else { &mut right };
    home.extend(missing);
    // Close outermost: first on the left, last on the right.
    let rank = |b: &WindowButton| match b {
        WindowButton::Minimize => 1,
        WindowButton::Maximize => 2,
        WindowButton::Close if close_left => 0,
        WindowButton::Close => 3,
    };
    home.sort_by_key(rank);
    (left, right)
}

/// The Linux buttons for each side, or `None` when this window must draw
/// none.
#[cfg(target_os = "linux")]
fn linux_buttons(window: &Window, cx: &App) -> Option<(Vec<WindowButton>, Vec<WindowButton>)> {
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return None;
    }
    let layout = cx
        .button_layout()
        .unwrap_or_else(gpui_kit::WindowButtonLayout::linux_default);
    let controls = window.window_controls();
    let can = |b: WindowButton| match b {
        WindowButton::Minimize => controls.minimize,
        WindowButton::Maximize => controls.maximize,
        WindowButton::Close => true,
    };
    let side = |s: &[Option<WindowButton>]| s.iter().flatten().copied().collect::<Vec<_>>();
    let sides = arrange(&side(&layout.left), &side(&layout.right), can);
    (!sides.0.is_empty() || !sides.1.is_empty()).then_some(sides)
}

#[cfg(not(target_os = "linux"))]
fn linux_buttons(_: &Window, _: &App) -> Option<(Vec<WindowButton>, Vec<WindowButton>)> {
    None
}

/// A Linux caption button. The platform does not route clicks through
/// `WindowControlArea` here, so the button acts itself; `occlude` keeps a
/// press on it from dragging or zooming the window.
fn linux_button(button: WindowButton, maximized: bool, cx: &App) -> impl IntoElement {
    let (id, icon) = match button {
        WindowButton::Minimize => ("window-minimize", IconName::WindowMinimize),
        WindowButton::Maximize if maximized => ("window-restore", IconName::WindowRestore),
        WindowButton::Maximize => ("window-maximize", IconName::WindowMaximize),
        WindowButton::Close => ("window-close", IconName::Close),
    };
    let theme = cx.theme();
    let (hover_bg, hover_fg): (Hsla, Hsla) = if button == WindowButton::Close {
        (rgb(SYSTEM_CLOSE_RED).into(), white())
    } else {
        (theme.foreground.opacity(0.10), theme.foreground)
    };
    let group = SharedString::from(format!("{id}-group"));

    div()
        .id(id)
        .group(group.clone())
        .flex_none()
        .size(BUTTON)
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            match button {
                WindowButton::Minimize => window.minimize_window(),
                WindowButton::Maximize => window.zoom_window(),
                WindowButton::Close => window.remove_window(),
            }
        })
        // `svg`, not `Icon`: only an interactive element takes `group_hover`,
        // and the glyph must follow the button's hover.
        .child(
            svg()
                .path(icon.path())
                .flex_none()
                .size(BUTTON_GLYPH)
                .text_color(theme.muted_foreground)
                .group_hover(group, move |s| s.text_color(hover_fg)),
        )
}

fn linux_group(buttons: Vec<WindowButton>, maximized: bool, cx: &App) -> Option<Div> {
    if buttons.is_empty() {
        return None;
    }
    let row = h_flex()
        .gap(BUTTON_GAP)
        .children(buttons.into_iter().map(|b| linux_button(b, maximized, cx)));
    Some(grab_proof(row).flex_none())
}

/// The Windows buttons: Segoe Fluent glyphs, behaviour left to Windows.
fn windows_buttons(maximized: bool, cx: &App) -> Option<impl IntoElement> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    let fg = cx.theme().foreground;
    let red: Hsla = rgb(SYSTEM_CLOSE_RED).into();
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea| {
        let close = area == WindowControlArea::Close;
        let (hover, pressed, hover_fg) = if close {
            (red, red.opacity(0.8), white())
        } else {
            (fg.opacity(0.10), fg.opacity(0.07), fg)
        };
        div()
            .id(id)
            .flex_none()
            .w(WIN_BUTTON)
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family("Segoe Fluent Icons")
            .text_size(rems(10. / 16.))
            .text_color(fg)
            .hover(move |s| s.bg(hover).text_color(hover_fg))
            .active(move |s| s.bg(pressed).text_color(hover_fg))
            .occlude()
            .window_control_area(area)
            .child(glyph)
    };
    let (max_id, max_glyph) = if maximized {
        ("window-restore", "\u{e923}")
    } else {
        ("window-maximize", "\u{e922}")
    };
    Some(
        h_flex()
            .flex_none()
            .h_full()
            .child(button(
                "window-minimize",
                "\u{e921}",
                WindowControlArea::Min,
            ))
            .child(button(max_id, max_glyph, WindowControlArea::Max))
            .child(button("window-close", "\u{e8bb}", WindowControlArea::Close)),
    )
}

/// Wrap `el` so a press on it stays out of the band's drag: the band starts
/// a window move on any press that reaches it.
fn grab_proof(el: impl IntoElement) -> Div {
    div()
        .h_full()
        .flex()
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(el)
}

/// Whether a press on the band may still turn into a window move. Lives in
/// window state because the band itself is rebuilt every frame.
struct PressState {
    armed: bool,
}

impl Render for PressState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl RenderOnce for AppTitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // RAMA_FAKE_MACOS previews the macOS layout on another system, with
        // painted stand-ins for the traffic lights.
        let fake_mac = std::env::var_os("RAMA_FAKE_MACOS").is_some();
        let mac = cfg!(target_os = "macos") || fake_mac;
        let windows = cfg!(target_os = "windows");
        let web = cfg!(target_family = "wasm");
        let own_frame = matches!(window.window_decorations(), Decorations::Client { .. });
        let maximized = window.is_maximized();
        let press = window.use_state(cx, |_, _| PressState { armed: false });

        let (left, right) = linux_buttons(window, cx)
            .filter(|_| !fake_mac)
            .unwrap_or_default();
        let has_left = !left.is_empty();
        let trailing_buttons = !right.is_empty() || windows;
        let left = linux_group(left, maximized, cx);
        let right = linux_group(right, maximized, cx).map(|g| g.ml(BUTTONS_GAP));
        let rule = cx.theme().border.opacity(0.6);

        div()
            .id("title-bar")
            .relative()
            .flex_none()
            .h(TITLE_BAR_HEIGHT)
            .pt(TOP_NUDGE)
            .pl(if mac { MAC_LIGHTS_ROOM } else { EDGE })
            .pr(if windows { px(0.) } else { EDGE })
            .flex()
            .items_center()
            .when(!web, |bar| bar.window_control_area(WindowControlArea::Drag))
            // Start the move only once the pointer moves with the button
            // held: handing macOS the drag on the press itself makes it read
            // a quick second click as its own double click.
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&press, |p, _: &MouseDownEvent, _, _| p.armed = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&press, |p, _, _, _| p.armed = false),
            )
            .on_mouse_down_out(window.listener_for(&press, |p, _, _, _| p.armed = false))
            .on_mouse_move(
                window.listener_for(&press, |p, event: &MouseMoveEvent, window, _| {
                    if p.armed && event.pressed_button == Some(MouseButton::Left) {
                        p.armed = false;
                        window.start_window_move();
                    }
                }),
            )
            .on_click(|event, window, _| {
                if event.click_count() != 2 {
                    return;
                }
                if cfg!(target_os = "macos") {
                    // Honours the system's double-click setting.
                    window.titlebar_double_click();
                } else {
                    window.zoom_window();
                }
            })
            // With our own frame there is no system bar to right-click, so
            // the window menu is offered here.
            .when(own_frame && !web, |bar| {
                bar.on_mouse_down(MouseButton::Right, |event: &MouseDownEvent, window, _| {
                    window.show_window_menu(event.position)
                })
            })
            .when(fake_mac, |bar| {
                bar.child(
                    h_flex()
                        .absolute()
                        .left(MAC_LIGHTS_X)
                        .top(lights_y())
                        .gap(px(8.))
                        .children(
                            [0xff5f57, 0xfebc2e, 0x28c840]
                                .map(|c| div().size(MAC_LIGHT).rounded_full().bg(rgb(c))),
                        ),
                )
            })
            .children(left)
            // The tabs give up width first; the strip scrolls what is left.
            .children(self.content.map(|content| {
                grab_proof(content)
                    .min_w_0()
                    .flex_shrink(1.)
                    .when(has_left, |slot| slot.ml(BUTTON_GAP))
            }))
            .child(div().flex_1().min_w(GRAB_ROOM).h_full())
            .when(trailing_buttons, |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .ml(RULE_LEAD)
                        .w(px(1.))
                        .h(RULE_HEIGHT)
                        .bg(rule),
                )
            })
            .children(right)
            .children(windows_buttons(maximized, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WindowButton::{Close, Maximize, Minimize};

    #[test]
    fn a_full_layout_is_kept_as_the_desktop_wrote_it() {
        let (l, r) = arrange(&[Close], &[Maximize, Minimize], |_| true);
        assert_eq!((l, r), (vec![Close], vec![Maximize, Minimize]));
    }

    #[test]
    fn missing_buttons_join_close_with_close_outermost() {
        assert_eq!(
            arrange(&[], &[Close], |_| true),
            (vec![], vec![Minimize, Maximize, Close])
        );
        assert_eq!(
            arrange(&[Close], &[], |_| true),
            (vec![Close, Minimize, Maximize], vec![])
        );
        assert_eq!(
            arrange(&[], &[], |_| true),
            (vec![], vec![Minimize, Maximize, Close])
        );
    }

    #[test]
    fn what_the_window_cannot_do_is_not_drawn() {
        let no_min = |b: WindowButton| b != Minimize;
        assert_eq!(
            arrange(&[Minimize], &[Close], no_min),
            (vec![], vec![Maximize, Close])
        );
    }

    #[test]
    fn a_button_on_the_far_side_stays_there() {
        assert_eq!(
            arrange(&[Minimize], &[Close], |_| true),
            (vec![Minimize], vec![Maximize, Close])
        );
    }
}
