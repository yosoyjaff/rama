//! Bootstrap: initialize the layers, register the features, open the window.
//!
//! The module tree is the architecture:
//!
//! * [`ui`] — reusable chrome. The title bar and its window controls; product
//!   agnostic, and the layer to lift into its own crate the day a second
//!   application shares it.
//! * [`update`] — the release manifest, the update decision and its state.
//! * [`actions`] / [`keymap`] — the application's commands and their keys.
//! * [`assets`] — the icons this application ships, ahead of the bundle.
//! * [`git`] — reading and writing the repository, through the git CLI.
//! * [`settings`] — what the interface remembers between launches.
//! * [`recents`] — the repositories this machine has opened.
//! * [`features`] — the product itself, one module per capability.
//! * [`shell`] — the one view a window's `Root` wraps, and the only place that
//!   decides what is mounted where.

mod actions;
mod assets;
mod features;
mod git;
mod keymap;
mod recents;
mod settings;
mod shell;
mod ui;
mod update;
mod updater;

use crate::actions::Quit;
use crate::ui::AppTitleBar;
use gpui_kit::component::Root;
use gpui_kit::*;

use crate::shell::Shell;

/// What the application calls itself: the title bar, the window switcher and
/// the macOS Window menu all show this.
pub const APP_NAME: &str = "Rama";

/// The reverse-DNS id Linux desktops match against the `.desktop` file to find
/// the icon and group windows in the taskbar. Change it with the product.
const APP_ID: &str = "app.rama";

/// The window's default size, and the smallest it may become.
///
/// Physical window geometry is one of the few places a raw pixel value is
/// right: it is a platform quantity, not a design token, and it must not
/// follow the application's font-size zoom.
const DEFAULT_WIDTH: f32 = 1200.;
const DEFAULT_HEIGHT: f32 = 800.;
const MIN_WIDTH: f32 = 720.;
const MIN_HEIGHT: f32 = 480.;

fn main() {
    let app = gpui_kit::application().with_assets(crate::assets::Assets);

    app.run(move |cx| {
        // Must come first: it initializes the theme, the component globals and
        // the base layer every component below assumes.
        gpui_kit::init(cx);
        init(cx);

        let options = window_options(cx);
        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let shell = cx.new(|cx| Shell::new(window, cx));
                // Root is required as the window's first level: it mounts the
                // dialog, sheet and notification layers, and owns modal focus
                // restoration and the window's rem size. On Linux it also
                // draws the window's frame (see `ui::window_frame`).
                cx.new(|cx| Root::new(shell, window, cx))
            })
            .expect("failed to open the main window");
            // The window is up: only now does the network get asked whether
            // a newer release exists, off the main thread.
            cx.update(updater::check_after_launch);
        })
        .detach();

        cx.activate(true);
    });
}

/// Install the application's keymap, its menus and its quit handler.
fn init(cx: &mut App) {
    keymap::init(cx);
    crate::ui::theme::init(cx);
    crate::settings::init(cx);
    crate::recents::init(cx);
    // The window opens in the appearance the user last chose, not in the
    // library's default and then a flash into theirs.
    crate::ui::theme::apply_settings(cx);

    cx.on_action(|_: &Quit, cx| cx.quit());
    updater::init(cx);
    cx.set_menus(app_menus());
}

fn window_options(cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(DEFAULT_WIDTH), px(DEFAULT_HEIGHT)), cx);

    // The band draws no title of its own — the tabs are the identity — but the
    // window switcher, the taskbar and the macOS Window menu read this one,
    // and a nameless window is what an unfinished app looks like.
    let mut options = AppTitleBar::window_options();
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(APP_NAME.into());
    }

    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
        app_id: Some(APP_ID.to_owned()),
        // Linux: our own title band and caption buttons, inside gpui-kit's
        // frame with its border and resize edges (`ui::window_frame`). Unset,
        // this asks for SERVER decorations, which stacks a compositor title
        // bar over ours where the compositor has one, and leaves a square,
        // edge-less window on GNOME, which never does.
        window_decorations: cfg!(target_os = "linux").then_some(WindowDecorations::Client),
        // The shadow around gpui-kit's frame has to be see-through.
        window_background: if cfg!(target_os = "linux") {
            WindowBackgroundAppearance::Transparent
        } else {
            WindowBackgroundAppearance::Opaque
        },
        // Carries the platform contract the custom title bar needs: a
        // transparent system title bar with the macOS traffic lights placed
        // for its height, and app-owned dragging.
        ..options
    }
}

/// The macOS menu bar. Ignored on platforms without one.
///
/// One item, because one is what the platform requires. A menu is a product
/// decision: add entries with their Actions as the application grows them.
fn app_menus() -> Vec<Menu> {
    vec![Menu {
        name: APP_NAME.into(),
        disabled: false,
        items: vec![MenuItem::action("Quit", Quit)],
    }]
}
