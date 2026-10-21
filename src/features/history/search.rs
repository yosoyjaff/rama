//! Finding commits: the search field above the graph, which grows out of a
//! magnifier button, marks the matching rows where they are and steps
//! between them; and the button that switches to the branch tips.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::{
    AnyElement, Context, Entity, MouseButton, Render, SharedString, Stateful, Subscription, Task,
    Window, div, prelude::*, px,
};

use super::{HistoryView, Scope, WORKTREE_ROW};
use crate::git::Commit;
use crate::ui::chrome;
use crate::ui::icons;
use crate::ui::motion::{self, AnimationExt as _};
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme::{self, Theme};
use crate::ui::tip::tip as tooltip;

/// The open field's width.
const FIELD_WIDTH: f32 = 280.0;
/// An empty field closes itself after this long.
const CLOSE_WHEN_IDLE: Duration = Duration::from_millis(1_500);
const TOOLTIP_DELAY: Duration = Duration::from_millis(350);

/// The search over the loaded rows.
#[derive(Default)]
pub(super) struct Search {
    pub query: String,
    /// The rows that match, top to bottom. They are marked rather than
    /// filtered, so the graph keeps its shape around them.
    pub hits: Vec<usize>,
    /// The hit the arrows are on.
    pub at: Option<usize>,
}

impl Search {
    pub fn active(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// Find the query in `rows` again. The arrows stay on the commit they
    /// were on if it still matches; otherwise they go to the first hit.
    pub fn rematch(&mut self, rows: &[Commit]) {
        let was_on = self
            .at
            .and_then(|at| self.hits.get(at))
            .and_then(|&row| rows.get(row))
            .map(|commit| commit.sha.clone());
        self.hits = if self.active() {
            rows.iter()
                .enumerate()
                .filter(|(_, c)| c.sha != WORKTREE_ROW && history_search_hit(&self.query, c))
                .map(|(row, _)| row)
                .collect()
        } else {
            Vec::new()
        };
        let still = was_on.and_then(|sha| self.hits.iter().position(|&row| rows[row].sha == sha));
        self.at = still.or_else(|| (!self.hits.is_empty()).then_some(0));
    }

    /// Move to the next hit, or the previous, wrapping round. The row it
    /// lands on, if there are hits.
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        let count = self.hits.len();
        if count == 0 {
            return None;
        }
        let at = match self.at {
            None => 0,
            Some(at) if forward => (at + 1) % count,
            Some(at) => (at + count - 1) % count,
        };
        self.at = Some(at);
        Some(self.hits[at])
    }
}

impl HistoryView {
    /// Type into the search. The first hit scrolls into view but is not
    /// selected until an arrow is used, so typing leaves the side panel be.
    pub(super) fn search_for(&mut self, query: String, cx: &mut Context<Self>) {
        let query = query.trim_start().to_owned();
        if query == self.search.query {
            return;
        }
        self.search.query = query;
        self.search.at = None;
        self.search.rematch(&self.shown_rows);
        if let Some(&first) = self.search.hits.first() {
            self.list.scroll_to_reveal_item(first);
        }
        cx.notify();
    }

    /// Go to the next hit (or previous) and select it, as a click would.
    pub(super) fn next_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(row) = self.search.step(forward) else {
            return;
        };
        self.list.scroll_to_reveal_item(row);
        if let Some(commit) = self.shown_rows.get(row).cloned() {
            self.select_commit(commit, cx);
        }
        cx.notify();
    }
}

/// The ranges of `text` where `query` appears, ignoring case. Ranges are in
/// bytes of `text`, even where lowercasing changes a character's length.
pub(super) fn history_search_ranges(query: &str, text: &str) -> Vec<Range<usize>> {
    let wanted: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    // Each lowercase character, with the bytes of `text` it came from.
    let folded: Vec<(char, Range<usize>)> = text
        .char_indices()
        .flat_map(|(at, c)| {
            c.to_lowercase()
                .map(move |lower| (lower, at..at + c.len_utf8()))
        })
        .collect();
    let mut found: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while let Some(window) = folded.get(i..i + wanted.len()) {
        if window.iter().map(|(c, _)| *c).ne(wanted.iter().copied()) {
            i += 1;
            continue;
        }
        let range = window[0].1.start..window[window.len() - 1].1.end;
        match found.last_mut() {
            Some(previous) if previous.end >= range.start => {
                previous.end = range.end.max(previous.end)
            }
            _ => found.push(range),
        }
        i += wanted.len();
    }
    found
}

/// Whether a commit matches: its sha starts with the query, or its subject
/// or author contains it. Only text that can be highlighted counts.
pub(super) fn history_search_hit(query: &str, commit: &Commit) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    commit.sha.to_lowercase().starts_with(&query.to_lowercase())
        || [&commit.subject, &commit.author.name]
            .iter()
            .any(|text| !history_search_ranges(query, text).is_empty())
}

/// A square toolbar button that washes in on hover. Pressing it does not
/// take focus away from where it was.
fn square_button(id: &'static str) -> Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(chrome::CONTROL_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(chrome::CONTROL_RADIUS))
        .cursor_pointer()
        .on_hover(motion::on_hover_fade(id))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
}

/// The toolbar button that toggles between every commit and the branch tips.
pub struct ScopeToggle {
    history: Entity<HistoryView>,
    _observe: Subscription,
}

impl ScopeToggle {
    pub fn new(history: Entity<HistoryView>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&history, |_, _, cx| cx.notify()),
            history,
        }
    }
}

impl Render for ScopeToggle {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        let tips = self.history.read(cx).scope == Scope::Tips;
        let history = self.history.clone();
        let id = "history-view-trigger";
        square_button(id)
            .occlude()
            .bg(match tips {
                true => theme.accent.opacity(0.12),
                false => motion::fade_color(id, theme::wash(0.0), theme::wash(0.14)),
            })
            .child(
                icons::icon(icons::FOLD_VERTICAL)
                    .size(px(chrome::ICON_SIZE))
                    .text_color(if tips { theme.accent } else { theme.text_muted }),
            )
            .tooltip(tooltip(if tips {
                "Show all commits"
            } else {
                "Show branch tips"
            }))
            .tooltip_show_delay(TOOLTIP_DELAY)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                history.update(cx, |history, cx| {
                    let next = match history.scope {
                        Scope::Tips => Scope::Everything,
                        Scope::Everything => Scope::Tips,
                    };
                    history.show_mode(next, cx);
                });
            })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    /// Only the magnifier is shown.
    Closed,
    Open,
    /// Shrinking back into the magnifier.
    Closing,
}

/// The search field above the graph.
pub struct SearchField {
    history: Entity<HistoryView>,
    input: Entity<TextInput>,
    field: Field,
    /// Bumped to call off a timer that is still pending.
    generation: usize,
    idle_timer: Option<Task<()>>,
    close_timer: Option<Task<()>>,
    on_blur: Option<Subscription>,
    _observe: Subscription,
    _input: Subscription,
}

impl SearchField {
    pub fn new(history: Entity<HistoryView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search", window, cx).with_text_size(11.0));
        // The query can change from the history's side too; keep the field
        // showing it.
        let observe = cx.observe_in(&history, window, |this, history, window, cx| {
            let query = history.read(cx).search.query.clone();
            if this.input.read(cx).text().as_ref() != query {
                this.input
                    .update(cx, |input, cx| input.set_text(query, window, cx));
            }
            cx.notify();
        });
        let typing = cx.subscribe(&input, |this: &mut Self, input, event, cx| match event {
            // Enter goes to the next match, as in a find bar.
            TextInputEvent::Submitted => {
                this.history
                    .update(cx, |history, cx| history.next_hit(true, cx));
            }
            TextInputEvent::Edited => {
                let query = input.read(cx).text().to_string();
                let empty = query.is_empty();
                this.history
                    .update(cx, |history, cx| history.search_for(query, cx));
                match empty {
                    true => this.close_when_idle(cx),
                    false => this.stop_timers(),
                }
            }
            _ => {}
        });
        Self {
            history,
            input,
            field: Field::Closed,
            generation: 0,
            idle_timer: None,
            close_timer: None,
            on_blur: None,
            _observe: observe,
            _input: typing,
        }
    }

    fn stop_timers(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.idle_timer = None;
        self.close_timer = None;
    }

    fn is_empty(&self, cx: &Context<Self>) -> bool {
        self.input.read(cx).text().is_empty()
    }

    /// Close an empty open field if it stays empty a moment.
    fn close_when_idle(&mut self, cx: &mut Context<Self>) {
        self.stop_timers();
        if self.field != Field::Open || !self.is_empty(cx) {
            return;
        }
        let generation = self.generation;
        self.idle_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_WHEN_IDLE).await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.close(cx);
                }
            })
            .ok();
        }));
    }

    /// Shrink back into the magnifier, if the field is open and empty.
    fn close(&mut self, cx: &mut Context<Self>) {
        if self.field != Field::Open || !self.is_empty(cx) {
            return;
        }
        self.stop_timers();
        self.field = Field::Closing;
        let generation = self.generation;
        let shrink = motion::RESIZE.length();
        self.close_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shrink).await;
            this.update(cx, |this, cx| {
                if this.generation == generation && this.field == Field::Closing {
                    // Unmounting the input is what stops its caret.
                    this.field = Field::Closed;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_timers();
        self.field = Field::Open;
        // The input is not mounted yet: focus it once the open field has
        // been drawn, or the focus would land nowhere.
        let this = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            this.update(cx, |this, cx| {
                if this.field == Field::Open {
                    window.focus(&this.input.read(cx).focus_handle(cx), cx);
                    this.close_when_idle(cx);
                    cx.notify();
                }
            })
            .ok();
        });
        cx.notify();
    }

    /// Empty the field and the search.
    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_empty(cx) {
            self.input
                .update(cx, |input, cx| input.set_text("", window, cx));
        }
        // Setting the text from code raises no edit, so tell the history.
        self.history
            .update(cx, |history, cx| history.search_for(String::new(), cx));
        self.close_when_idle(cx);
        cx.notify();
    }

    fn render_closed(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = "history-search-trigger";
        let this = cx.entity().downgrade();
        square_button(id)
            .bg(motion::fade_color(id, theme::wash(0.0), theme::wash(0.14)))
            .child(
                icons::icon(icons::MAGNIFER)
                    .size(px(chrome::ICON_SIZE))
                    .text_color(theme.text_muted),
            )
            .tooltip(tooltip("Search commits"))
            .tooltip_show_delay(TOOLTIP_DELAY)
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                this.update(cx, |this, cx| this.open(window, cx)).ok();
            })
            .into_any_element()
    }
}

/// A tiny icon button inside the field. It keeps the field focused, since
/// losing focus closes the search.
fn field_button(
    id: &'static str,
    glyph: &'static str,
    size: f32,
    color: gpui_kit::Hsla,
    live: bool,
) -> Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.5))
        .when(live, |button| {
            button.cursor_pointer().hover(|s| s.bg(theme::ink(0.08)))
        })
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .child(icons::icon(glyph).size(px(size)).text_color(color))
}

impl Render for SearchField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if self.field == Field::Closed {
            return self.render_closed(&theme, cx);
        }
        if self.on_blur.is_none() {
            let focus = self.input.read(cx).focus_handle(cx);
            self.on_blur =
                Some(cx.on_blur(&focus, window, |this, window, cx| this.clear(window, cx)));
        }

        let (searching, hits, current) = {
            let history = self.history.read(cx);
            (
                history.search.active(),
                history.search.hits.len(),
                history.search.at,
            )
        };
        // Counted over the rows loaded, which are the ones the arrows reach.
        let tally = searching.then(|| {
            let text = match (hits, current) {
                (0, _) => "No results".to_string(),
                (n, Some(at)) => format!("{} of {n}", at + 1),
                (n, None) => format!("{n} results"),
            };
            div()
                .flex_none()
                .text_size(px(10.5))
                .text_color(if hits == 0 {
                    theme.danger_muted
                } else {
                    theme.text_faint
                })
                .child(SharedString::from(text))
        });
        let step = |id, glyph, forward: bool| {
            let history = self.history.downgrade();
            let color = match hits {
                0 => theme.text_faint.opacity(0.5),
                _ => theme.text_muted,
            };
            field_button(id, glyph, 10.0, color, hits > 0).on_click(move |_, _, cx| {
                cx.stop_propagation();
                history
                    .update(cx, |history, cx| history.next_hit(forward, cx))
                    .ok();
            })
        };
        let this = cx.entity().downgrade();
        let close = field_button(
            "history-search-close",
            icons::CLOSE,
            9.0,
            theme.text_faint,
            true,
        )
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            this.update(cx, |this, cx| this.clear(window, cx)).ok();
        });
        let closing = self.field == Field::Closing;
        let animation = SharedString::from(format!(
            "history-search-{}-{}",
            self.generation,
            if closing { "close" } else { "open" }
        ));
        div()
            .id("history-search-expanded")
            .h(px(chrome::CONTROL_SIZE))
            .w(px(FIELD_WIDTH))
            .min_w(px(80.0))
            .flex_shrink(1.0)
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(6.0))
            .pl(px(chrome::EDGE_INSET))
            .pr(px(2.0))
            .rounded(px(chrome::CONTROL_RADIUS))
            .bg(theme::ink(0.035))
            .child(
                div()
                    .flex_none()
                    .size(px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        icons::icon(icons::MAGNIFER)
                            .size(px(11.0))
                            .text_color(theme.text_faint),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(self.input.clone()),
            )
            .children(tally)
            .when(searching, |field| {
                field
                    .child(step("history-search-previous", icons::ALT_ARROW_UP, false))
                    .child(step("history-search-next", icons::ALT_ARROW_DOWN, true))
            })
            .child(close)
            .with_animation(animation, motion::RESIZE.animation(), move |field, t| {
                let open = if closing { 1.0 - t } else { t };
                field
                    .w(px(24.0 + (FIELD_WIDTH - 24.0) * open))
                    .opacity(0.45 + 0.55 * open)
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_marked_by_byte_ignoring_case() {
        // `É` is two bytes long.
        assert_eq!(
            history_search_ranges("rePAR", "RÉPARER la reparation"),
            vec![12..17]
        );
        assert_eq!(
            history_search_ranges("répar", "RÉPARER la réparation"),
            [0..6, 12..18]
        );
        assert_eq!(
            history_search_ranges("aa", "aaaa"),
            vec![0..4],
            "touching matches merge"
        );
        assert!(history_search_ranges("xyz", "nothing here").is_empty());
        assert!(history_search_ranges("   ", "anything").is_empty());
    }

    #[test]
    fn a_hit_is_a_sha_prefix_or_text_in_the_subject_or_author() {
        let commit = Commit {
            sha: "a1b2c3d4".into(),
            subject: "Estructura inicial".into(),
            author: crate::git::Signature {
                name: "Marta Ruiz".into(),
                ..Default::default()
            },
            ..Commit::default()
        };
        for hit in ["A1B2", "ESTRUC", "ruiz"] {
            assert!(history_search_hit(hit, &commit), "{hit}");
        }
        // Scattered letters are not a hit: there would be nothing to mark.
        for miss in ["estrcz", "b2c3", ""] {
            assert!(!history_search_hit(miss, &commit), "{miss:?}");
        }
    }
}
