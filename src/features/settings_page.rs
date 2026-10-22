//! The settings screen: a rail of sections beside the one being read.
//!
//! Every control here writes straight through to [`crate::settings`], which
//! applies the change in memory and schedules the save. There is no "apply"
//! button and nothing to confirm: a preference is the user saying what they
//! want, and it takes effect as they say it.

use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::{Sizable as _, link::Link};
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, Hsla, IntoElement, Render,
    SharedString, Subscription, Window, div, img, prelude::*, px,
};

use crate::recents;
use crate::settings::{self, Accent, Mode, SavePolicy, Settings};
use crate::ui::chrome::EDGE_INSET;
use crate::ui::lane_palette::LanePalette;
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme::{AccentColor, Theme, wash};
use crate::ui::typography::ui_rems;
use crate::ui::{icons, motion};

/// Width of the section rail.
const RAIL_WIDTH: f32 = 176.0;
/// Width the page's content is held to, so a line of prose stays readable on
/// a wide window.
const PAGE_WIDTH: f32 = 560.0;
/// Side of the app icon above the About page's title.
const LOGO_SIZE: f32 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Appearance,
    History,
    Diff,
    About,
}

impl Section {
    const ALL: [Self; 4] = [Self::Appearance, Self::History, Self::Diff, Self::About];

    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::History => "History",
            Self::Diff => "Diff",
            Self::About => "About",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Appearance => icons::EYE,
            Self::History => icons::GIT_BRANCH,
            Self::Diff => icons::SPLIT_COLUMNS,
            Self::About => icons::INFO_CIRCLE,
        }
    }
}

pub struct SettingsPage {
    section: Section,
    focus_handle: FocusHandle,
    /// The custom accent as text: `#RRGGBB` (or `#RGB`). It and the picker
    /// below are two views of one colour; editing either moves the other.
    custom_accent: Entity<TextInput>,
    /// The custom accent as a colour picker: palette and HSL sliders.
    accent_picker: Entity<ColorPickerState>,
    /// The update row About shows, when the shell provides one.
    update_panel: Option<gpui_kit::AnyView>,
    _subscriptions: Vec<Subscription>,
}

/// What the settings page asks the rest of the app to do.
pub enum SettingsPageEvent {
    /// Import the tabs GitKraken had open: `(paths, the one on screen)`.
    ImportGitKraken {
        paths: Vec<String>,
        active: Option<String>,
    },
}

impl gpui_kit::EventEmitter<SettingsPageEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Both controls open on the custom accent in force, if there is one.
        let custom = match settings::current(cx).look.accent {
            Accent::Custom { red, green, blue } => Some((red, green, blue)),
            _ => None,
        };
        let custom_accent = cx.new(|cx| {
            let mut input = TextInput::new("#1f6feb", window, cx).with_text_size(12.0);
            if let Some((red, green, blue)) = custom {
                input.set_text(hex_color(red, green, blue), window, cx);
            }
            input
        });
        let accent_picker = cx.new(|cx| {
            let picker = ColorPickerState::new(window, cx);
            match custom {
                Some((red, green, blue)) => picker.default_value(rgb_color(red, green, blue)),
                None => picker,
            }
        });
        let subscriptions = vec![
            cx.subscribe_in(&accent_picker, window, Self::on_picker_event),
            cx.subscribe_in(&custom_accent, window, Self::on_custom_text_event),
        ];
        Self {
            section: Section::Appearance,
            focus_handle: cx.focus_handle(),
            custom_accent,
            accent_picker,
            update_panel: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn with_update_panel(mut self, panel: Option<gpui_kit::AnyView>) -> Self {
        self.update_panel = panel;
        self
    }

    /// A colour picked from the palette or dragged on a slider is the accent
    /// at once, and the text follows it.
    fn on_picker_event(
        &mut self,
        _: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(Some(color)) = event else {
            return;
        };
        // The picker's alpha slider has no meaning for an accent: the
        // channels are what is kept.
        let (red, green, blue) = rgb_channels(*color);
        self.custom_accent.update(cx, |input, cx| {
            input.set_text(hex_color(red, green, blue), window, cx)
        });
        apply_custom_accent(red, green, blue, cx);
        cx.notify();
    }

    /// Typed text becomes the accent as soon as it is a whole `#RRGGBB`;
    /// Enter or leaving the field also takes the short `#RGB`, and spells
    /// the field out in full.
    fn on_custom_text_event(
        &mut self,
        input: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = input.read(cx).text().clone();
        let finished = match event {
            TextInputEvent::Edited => text.trim().trim_start_matches('#').len() == 6,
            TextInputEvent::Submitted | TextInputEvent::Blurred => true,
        };
        if !finished {
            return;
        }
        let Some((red, green, blue)) = parse_hex_color(&text) else {
            return;
        };
        if !matches!(event, TextInputEvent::Edited) {
            input.update(cx, |input, cx| {
                input.set_text(hex_color(red, green, blue), window, cx)
            });
        }
        // The picker writing the text lands here too; it already applied.
        if settings::current(cx).look.accent == (Accent::Custom { red, green, blue }) {
            return;
        }
        self.accent_picker.update(cx, |picker, cx| {
            picker.set_value(rgb_color(red, green, blue), window, cx)
        });
        apply_custom_accent(red, green, blue, cx);
        cx.notify();
    }

    fn render_rail(&mut self, theme: &Theme, corner: f32, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(RAIL_WIDTH))
            .rounded_bl(px(corner))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px_2()
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div()
                    .px_2()
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .text_size(ui_rems(11.0))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(theme.text_muted.opacity(0.6))
                    .child("Settings"),
            )
            .children(Section::ALL.into_iter().map(|item| {
                let selected = item == self.section;
                let id = SharedString::from(format!("settings-nav-{}", item.label()));
                div()
                    .id(id.clone())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(8.0))
                    .px_2()
                    .py(px(6.0))
                    .cursor_pointer()
                    .text_size(ui_rems(13.0))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .bg(if selected {
                        wash(0.10)
                    } else {
                        motion::fade_color(&id, wash(0.0), wash(0.06))
                    })
                    .on_hover(motion::on_hover_fade(&id))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.section = item;
                        cx.notify();
                    }))
                    .child(
                        icons::icon(item.icon())
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child(SharedString::from(item.label()))
            }))
            .into_any_element()
    }

    /// One setting: what it is and what it does, with its control on the right.
    fn row(
        &self,
        title: &'static str,
        description: &'static str,
        control: AnyElement,
        theme: &Theme,
    ) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .py(px(10.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_px()
                    .child(
                        div()
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(description),
                    ),
            )
            .child(div().flex_none().child(control))
            .into_any_element()
    }

    /// A switch: the control for a setting that is on or off.
    fn toggle(
        &self,
        id: &'static str,
        on: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        apply: fn(&mut Settings),
    ) -> AnyElement {
        div()
            .id(id)
            .w(px(36.0))
            .h(px(20.0))
            .flex_none()
            .rounded(px(10.0))
            .cursor_pointer()
            .bg(if on { theme.accent } else { wash(0.14) })
            .flex()
            .items_center()
            .px(px(2.0))
            .on_click(cx.listener(move |_, _, _, cx| {
                settings::update(SavePolicy::Immediate, cx, apply);
                cx.notify();
            }))
            .child(
                div()
                    .size(px(16.0))
                    .rounded_full()
                    .bg(if on {
                        theme.on_accent
                    } else {
                        theme.text_muted
                    })
                    // The knob is placed, not animated: a settings switch is
                    // read at a glance and never watched.
                    .ml(px(if on { 16.0 } else { 0.0 })),
            )
            .into_any_element()
    }

    /// A row of choices, one of them current.
    fn choices<T: PartialEq + Copy + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        current: T,
        theme: &Theme,
        cx: &mut Context<Self>,
        apply: fn(&mut Settings, T),
    ) -> AnyElement {
        let mut row = div().flex().flex_row().gap(px(4.0));
        for (value, label) in options.iter().copied() {
            let selected = value == current;
            let key = SharedString::from(format!("{id}-{label}"));
            row = row.child(
                div()
                    .id(key.clone())
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded_md()
                    .cursor_pointer()
                    .text_size(ui_rems(11.5))
                    .text_color(if selected {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .bg(if selected {
                        wash(0.12)
                    } else {
                        motion::fade_color(&key, wash(0.0), wash(0.07))
                    })
                    .on_hover(motion::on_hover_fade(&key))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        settings::update(SavePolicy::Immediate, cx, move |settings| {
                            apply(settings, value)
                        });
                        // A choice may be an appearance: applying it is what
                        // makes the window follow rather than the file alone.
                        crate::ui::theme::apply_settings(cx);
                        cx.notify();
                    }))
                    .child(label),
            );
        }
        row.into_any_element()
    }

    fn render_appearance(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = settings::current(cx);
        let appearance = theme.appearance;
        let accent_row = div()
            .flex()
            .flex_row()
            .gap(px(6.0))
            .children(AccentColor::ALL.into_iter().map(|accent| {
                let choice = accent_choice(accent);
                let selected = choice == settings.look.accent;
                div()
                    .id(SharedString::from(format!(
                        "settings-accent-{}",
                        accent.label()
                    )))
                    .size(px(20.0))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(accent.swatch(appearance))
                    // The current one is ringed rather than filled differently:
                    // the swatch has to keep showing its own colour.
                    .when(selected, |swatch| {
                        swatch.border_2().border_color(theme.text)
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        settings::update(SavePolicy::Immediate, cx, move |settings| {
                            settings.look.accent = choice;
                        });
                        crate::ui::theme::set_accent(accent, cx);
                        cx.notify();
                    }))
            }))
            .into_any_element();

        // The picker and its text, side by side: pick a colour, or type
        // one. The ring says the custom colour is the accent in force.
        let custom_selected = matches!(settings.look.accent, Accent::Custom { .. });
        let featured = AccentColor::ALL
            .into_iter()
            .map(|accent| accent.swatch(appearance))
            .collect();
        let custom_row = self.row(
            "Custom accent",
            "Any colour you like: pick it, or type it as #RRGGBB (or #RGB).",
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .p(px(1.0))
                        .rounded(px(6.0))
                        .border_2()
                        .border_color(if custom_selected {
                            theme.text
                        } else {
                            theme.text.opacity(0.0)
                        })
                        .child(
                            ColorPicker::new(&self.accent_picker)
                                .small()
                                .featured_colors(featured)
                                .accessibility_label("Custom accent"),
                        ),
                )
                // The field fills its parent, and a row's control has no
                // width of its own to fill: give it room for `#rrggbb`.
                .child(div().w(px(84.0)).child(self.custom_accent.clone()))
                .into_any_element(),
            theme,
        );

        self.page(
            "Appearance",
            vec![
                self.row(
                    "Theme",
                    "Light or dark.",
                    self.choices(
                        "settings-appearance",
                        &[(Mode::Dark, "Dark"), (Mode::Light, "Light")],
                        settings.look.mode,
                        theme,
                        cx,
                        |settings, value| settings.look.mode = value,
                    ),
                    theme,
                ),
                self.row(
                    "Accent",
                    "The interactive colour: selection, links, the working indicator.",
                    accent_row,
                    theme,
                ),
                custom_row,
                self.render_graph_palettes(theme, cx),
            ],
            theme,
        )
    }

    /// Settings → Appearance → Graph colors: every lane palette, each shown
    /// as its own colours in the current appearance, the one in force ringed.
    fn render_graph_palettes(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = LanePalette::from(settings::current(cx).look.palette);
        let appearance = theme.appearance;
        let options = LanePalette::ALL.into_iter().map(|palette| {
            let selected = palette == current;
            let id = SharedString::from(format!("settings-graph-palette-{}", palette.label()));
            let swatches = palette
                .colors(appearance)
                .iter()
                .take(8)
                .map(|&color| div().h(px(14.0)).flex_1().bg(gpui_kit::rgb(color)));
            div()
                .id(id.clone())
                .flex()
                .flex_col()
                .gap(px(6.0))
                .p(px(8.0))
                .rounded_md()
                .border_1()
                .border_color(if selected {
                    theme.text
                } else {
                    theme.border.opacity(0.7)
                })
                .bg(if selected {
                    wash(0.06)
                } else {
                    motion::fade_color(&id, wash(0.0), wash(0.05))
                })
                .on_hover(motion::on_hover_fade(&id))
                .cursor_pointer()
                .on_click(cx.listener(move |_, _, _, cx| {
                    settings::update(SavePolicy::Immediate, cx, move |settings| {
                        settings.look.palette = palette.into();
                    });
                    // The graph reads the palette as it paints: redraw the
                    // window, not only this page.
                    cx.refresh_windows();
                    cx.notify();
                }))
                .when_some(palette.inspired_by(), |option, hint| {
                    option.tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(hint).build(window, cx)
                    })
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .text_size(ui_rems(12.0))
                        .text_color(if selected {
                            theme.text
                        } else {
                            theme.text_muted
                        })
                        .child(palette.label())
                        .when(selected, |label| {
                            label.child(
                                icons::icon(icons::CHECK)
                                    .size(px(11.0))
                                    .text_color(theme.text),
                            )
                        }),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .rounded(px(3.0))
                        .overflow_hidden()
                        .children(swatches),
                )
        });
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .py(px(10.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_px()
                    .child(
                        div()
                            .text_size(ui_rems(13.0))
                            .text_color(theme.text)
                            .child("Graph colors"),
                    )
                    .child(
                        div()
                            .text_size(ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child("The colours the history graph paints its branches with."),
                    ),
            )
            .child(div().w_full().grid().grid_cols(2).gap_2().children(options))
            .into_any_element()
    }

    fn render_history(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = settings::current(cx);
        let kraken = recents::gitkraken_repos();
        let mut rows = vec![
            self.row(
                "Branch column",
                "The column of branch and tag badges beside the graph.",
                self.toggle(
                    "settings-branch-column",
                    settings.history.badges,
                    theme,
                    cx,
                    |settings| settings.history.badges = !settings.history.badges,
                ),
                theme,
            ),
            self.row(
                "Author column",
                "Who wrote each commit.",
                self.toggle(
                    "settings-column-author",
                    settings.history.columns.author.shown,
                    theme,
                    cx,
                    |settings| {
                        settings.history.columns.author.shown =
                            !settings.history.columns.author.shown
                    },
                ),
                theme,
            ),
            self.row(
                "Date column",
                "When it was written.",
                self.toggle(
                    "settings-column-date",
                    settings.history.columns.date.shown,
                    theme,
                    cx,
                    |settings| {
                        settings.history.columns.date.shown = !settings.history.columns.date.shown
                    },
                ),
                theme,
            ),
            self.row(
                "Sha column",
                "The commit's short hash.",
                self.toggle(
                    "settings-column-sha",
                    settings.history.columns.sha.shown,
                    theme,
                    cx,
                    |settings| {
                        settings.history.columns.sha.shown = !settings.history.columns.sha.shown
                    },
                ),
                theme,
            ),
        ];
        if !kraken.is_empty() {
            // The tabs GitKraken had open, one click in — the same import
            // the welcome screen offers.
            let count = kraken.len();
            let active = kraken
                .iter()
                .find_map(|(path, on_screen)| on_screen.then(|| path.clone()));
            let paths: Vec<String> = kraken.into_iter().map(|(path, _)| path).collect();
            rows.push(
                self.row(
                    "Import from GitKraken",
                    "Open the repositories GitKraken keeps in its tabs.",
                    div()
                        .id("settings-import-gitkraken")
                        .px(px(9.0))
                        .py(px(3.0))
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(theme.border.opacity(0.85))
                        .text_size(px(11.0))
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .hover(|style| style.bg(crate::ui::theme::ink(0.05)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsPageEvent::ImportGitKraken {
                                paths: paths.clone(),
                                active: active.clone(),
                            });
                        }))
                        .child(SharedString::from(format!("Import {count}")))
                        .into_any_element(),
                    theme,
                ),
            );
        }
        self.page("History", rows, theme)
    }

    fn render_diff(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = settings::current(cx);
        self.page(
            "Diff",
            vec![
                self.row(
                    "Wrap long lines",
                    "Wrap instead of scrolling the code plane sideways.",
                    self.toggle(
                        "settings-diff-wrap",
                        settings.diff.wrap,
                        theme,
                        cx,
                        |settings| settings.diff.wrap = !settings.diff.wrap,
                    ),
                    theme,
                ),
                self.row(
                    "Ignore whitespace",
                    "Hide changes that only move spaces and tabs around.",
                    self.toggle(
                        "settings-diff-whitespace",
                        settings.diff.ignore_whitespace,
                        theme,
                        cx,
                        |settings| {
                            settings.diff.ignore_whitespace = !settings.diff.ignore_whitespace
                        },
                    ),
                    theme,
                ),
            ],
            theme,
        )
    }

    fn render_about(&mut self, theme: &Theme) -> AnyElement {
        let platform = format!("{} · {}", platform_name(), std::env::consts::ARCH);
        let repository = env!("CARGO_PKG_REPOSITORY");
        let identity = div()
            .pb(px(12.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .child(
                div()
                    .text_size(ui_rems(13.0))
                    .text_color(theme.text)
                    .child(env!("CARGO_PKG_DESCRIPTION")),
            )
            .into_any_element();
        let logo = img(crate::assets::APP_ICON)
            .size(px(LOGO_SIZE))
            .mb(px(12.0))
            .into_any_element();
        self.page_under(
            Some(logo),
            crate::APP_NAME,
            vec![
                identity,
                self.fact("Version", env!("CARGO_PKG_VERSION").into(), theme),
            ]
            .into_iter()
            .chain(
                self.update_panel
                    .clone()
                    .map(|panel| self.fact_element("Updates", panel.into_any_element(), theme)),
            )
            .chain([
                self.fact("Platform", platform.into(), theme),
                self.fact("License", env!("CARGO_PKG_LICENSE").into(), theme),
                self.fact_element(
                    "Source code",
                    Link::new("settings-about-repository")
                        .href(repository)
                        .child(repository.trim_start_matches("https://"))
                        .into_any_element(),
                    theme,
                ),
            ])
            .collect(),
            theme,
        )
    }

    /// One line of the About page: what it is, and its value.
    fn fact(&self, label: &'static str, value: SharedString, theme: &Theme) -> AnyElement {
        self.fact_element(
            label,
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.text_muted)
                .child(value)
                .into_any_element(),
            theme,
        )
    }

    fn fact_element(&self, label: &'static str, value: AnyElement, theme: &Theme) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .py(px(10.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .text_size(ui_rems(12.5))
            .child(
                div()
                    .w(px(120.0))
                    .flex_none()
                    .text_color(theme.text)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().flex().justify_end().child(value))
            .into_any_element()
    }

    fn page(&self, title: &'static str, rows: Vec<AnyElement>, theme: &Theme) -> AnyElement {
        self.page_under(None, title, rows, theme)
    }

    /// A page with something drawn above its title, such as About's logo.
    fn page_under(
        &self,
        above_title: Option<AnyElement>,
        title: &'static str,
        rows: Vec<AnyElement>,
        theme: &Theme,
    ) -> AnyElement {
        div()
            .id("settings-page")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w(px(PAGE_WIDTH))
                    .max_w_full()
                    .px(px(EDGE_INSET * 3.0))
                    .py(px(20.0))
                    .flex()
                    .flex_col()
                    .children(above_title)
                    .child(
                        div()
                            .pb(px(8.0))
                            .text_size(ui_rems(16.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(title),
                    )
                    .children(rows),
            )
            .into_any_element()
    }
}

/// The palette's accent as the settings file spells it, and back.
fn accent_choice(accent: AccentColor) -> Accent {
    match accent {
        AccentColor::Indigo => Accent::Indigo,
        AccentColor::Orange => Accent::Orange,
        AccentColor::Amber => Accent::Amber,
        AccentColor::Green => Accent::Green,
        AccentColor::Cyan => Accent::Cyan,
        AccentColor::Blue => Accent::Blue,
        AccentColor::Pink => Accent::Pink,
        AccentColor::Custom { red, green, blue } => Accent::Custom { red, green, blue },
    }
}

/// `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` → its channels. Anything else is
/// not a colour the picker accepts.
pub fn parse_hex_color(text: &str) -> Option<(u8, u8, u8)> {
    let text = text.trim().trim_start_matches('#');
    if !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |slice: &str| u8::from_str_radix(slice, 16).ok();
    match text.len() {
        6 => Some((
            channel(&text[0..2])?,
            channel(&text[2..4])?,
            channel(&text[4..6])?,
        )),
        3 => {
            let wide = |c: &str| format!("{c}{c}");
            Some((
                channel(&wide(&text[0..1]))?,
                channel(&wide(&text[1..2]))?,
                channel(&wide(&text[2..3]))?,
            ))
        }
        _ => None,
    }
}

/// Make a picked colour the accent, and keep it.
fn apply_custom_accent(red: u8, green: u8, blue: u8, cx: &mut App) {
    settings::update(SavePolicy::Immediate, cx, |settings| {
        settings.look.accent = Accent::Custom { red, green, blue };
    });
    crate::ui::theme::set_accent(AccentColor::Custom { red, green, blue }, cx);
}

/// Channels as the text field spells them: `#rrggbb`.
fn hex_color(red: u8, green: u8, blue: u8) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

/// Channels as the picker holds them.
fn rgb_color(red: u8, green: u8, blue: u8) -> Hsla {
    let (h, s, l) = crate::ui::theme::rgb_to_hsl(
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
    );
    gpui_kit::hsla(h, s, l, 1.0)
}

/// What the picker holds, back to channels.
fn rgb_channels(color: Hsla) -> (u8, u8, u8) {
    let rgb = color.to_rgb();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (channel(rgb.r), channel(rgb.g), channel(rgb.b))
}

/// The operating system, as people call it.
fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

pub fn accent_color(choice: Accent) -> AccentColor {
    match choice {
        Accent::Indigo => AccentColor::Indigo,
        Accent::Orange => AccentColor::Orange,
        Accent::Amber => AccentColor::Amber,
        Accent::Green => AccentColor::Green,
        Accent::Cyan => AccentColor::Cyan,
        Accent::Blue => AccentColor::Blue,
        Accent::Pink => AccentColor::Pink,
        Accent::Custom { red, green, blue } => AccentColor::Custom { red, green, blue },
    }
}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        // The rail holds the window's bottom-left corner, the page the
        // bottom-right one.
        let corner = crate::ui::window_frame::corner_radius(window);
        let rail = self.render_rail(&theme, corner, cx);
        let page = match self.section {
            Section::Appearance => self.render_appearance(&theme, cx),
            Section::History => self.render_history(&theme, cx),
            Section::Diff => self.render_diff(&theme, cx),
            Section::About => self.render_about(&theme),
        };
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_row()
            .rounded_b(px(corner))
            .bg(theme.bg)
            .child(rail)
            .child(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accent_survives_the_round_trip_through_the_settings_file() {
        for accent in AccentColor::ALL {
            assert_eq!(accent_color(accent_choice(accent)), accent);
        }
    }

    #[test]
    fn a_custom_accent_survives_the_round_trip_through_the_picker() {
        for (red, green, blue) in [
            (0x1f, 0x6f, 0xeb),
            (0, 0, 0),
            (255, 255, 255),
            (200, 30, 90),
        ] {
            assert_eq!(
                rgb_channels(rgb_color(red, green, blue)),
                (red, green, blue)
            );
        }
    }

    #[test]
    fn the_text_field_spells_what_it_parses() {
        assert_eq!(hex_color(0x1f, 0x6f, 0xeb), "#1f6feb");
        assert_eq!(
            parse_hex_color(&hex_color(0x1f, 0x6f, 0xeb)),
            Some((0x1f, 0x6f, 0xeb))
        );
        assert_eq!(parse_hex_color("#abc"), Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex_color("#abcd"), None);
    }

    #[test]
    fn every_section_names_itself() {
        for section in Section::ALL {
            assert!(!section.label().is_empty());
            assert!(!section.icon().is_empty());
        }
    }
}
