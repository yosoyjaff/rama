//! The context menu on screen, and the life of a pick.
//!
//! A pick with an open gate runs straight away; one behind a question or a
//! form waits in a dialog first. [`super::actions`] decides what the menu
//! holds; this module shows it, holds the dialog and runs the result.

use std::path::Path;

use gpui_kit::{
    AnyElement, ClipboardItem, Context, ElementId, Entity, KeyDownEvent, Pixels, Point,
    SharedString, Size, Subscription, Window, div, prelude::*, px,
};

use super::actions::{Aim, Answers, Entry, Gate, Group, Item, Key, Outcome, Scene};
use super::{HistoryEvent, HistoryView};
use crate::git::actions::GitAction;
use crate::ui::icons;
use crate::ui::popover;
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme::{Theme, hairline};
use crate::ui::toast::{self, ToastKind};

/// Wide enough for the longest label on one line.
const PANEL_WIDTH: f32 = 272.0;
const FOLDER_WIDTH: f32 = 248.0;

/// The menu while it is up.
pub(super) struct ContextMenu {
    pub(super) at: Point<Pixels>,
    groups: Vec<Group>,
    /// The folder showing its items, as (group, row). Folders hold only
    /// items, so one is open at most.
    unfolded: Option<(usize, usize)>,
}

/// A pick waiting on its question or form.
pub(super) struct PendingPick {
    item: Item,
    /// One text box per form field, in form order.
    boxes: Vec<(Key, Entity<TextInput>)>,
    focus_first: bool,
    problem: Option<SharedString>,
    _submits: Vec<Subscription>,
}

/// The words a dialog shows, from whichever gate it is.
#[derive(Debug, PartialEq)]
struct Wording {
    heading: String,
    body: Option<String>,
    button: String,
    danger: bool,
}

impl Wording {
    fn for_gate(gate: &Gate) -> Option<Self> {
        match gate {
            Gate::Open => None,
            Gate::Confirm(q) => Some(Self {
                heading: q.heading.clone(),
                body: Some(q.body.clone()),
                button: q.button.to_owned(),
                danger: q.danger,
            }),
            Gate::Form(f) => Some(Self {
                heading: f.heading.clone(),
                body: f.note.clone(),
                button: f.button.to_owned(),
                danger: false,
            }),
        }
    }
}

impl HistoryView {
    /// Put the menu up at `at` for a right-click on commit `sha`, aimed at
    /// `aim`.
    pub(super) fn open_context_menu(
        &mut self,
        at: Point<Pixels>,
        sha: String,
        aim: Aim,
        cx: &mut Context<Self>,
    ) {
        self.close_column_menu(cx);
        self.close_author_menu(cx);
        let Some(root) = self.root() else {
            return;
        };
        let repo = self.repo.clone().unwrap_or_default();
        let message = self.commits.iter().find(|c| c.sha == sha).map(|c| {
            if c.body.trim().is_empty() {
                c.subject.clone()
            } else {
                format!("{}\n\n{}", c.subject, c.body)
            }
        });
        let scene = Scene {
            root,
            is_head: self.head_sha.as_deref() == Some(sha.as_str()),
            run: self.linear_run_to(&sha),
            sha,
            branch: repo.branch.clone().filter(|b| !b.is_empty()),
            aim,
            forge: repo
                .tracked
                .as_ref()
                .and_then(|remote| crate::git::forge::hosted(&remote.url)),
            remote: repo.tracked.as_ref().map(|remote| remote.name.clone()),
            message,
        };
        self.popups.context.open(ContextMenu {
            at,
            groups: super::actions::menu_for(&scene),
            unfolded: None,
        });
        cx.notify();
    }

    pub(super) fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        self.popups
            .context
            .dismiss(cx, |history| &mut history.popups.context);
    }

    /// A double-click on a branch badge switches to it with no menu. Tags
    /// and commits would detach HEAD, which is the menu's job to ask about.
    pub(super) fn switch_to_badge(&mut self, aim: Aim, cx: &mut Context<Self>) {
        let Aim::Branch(branch) = aim else {
            return;
        };
        let receipt = format!("On {}", branch.name);
        let action = match branch.remote {
            Some(_) => GitAction::CheckoutTracking {
                local: branch.name,
                reference: branch.badge,
            },
            None => GitAction::Checkout { name: branch.badge },
        };
        self.run_git(action, Some(receipt), cx);
    }

    fn choose(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.close_context_menu(cx);
        match item.gate {
            Gate::Open => self.carry_out(item, &Answers::new(), cx),
            _ => self.hold(item, window, cx),
        }
    }

    /// Show `item`'s dialog, with a text box per form field.
    fn hold(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        let fields = match &item.gate {
            Gate::Form(form) => form.fields.clone(),
            _ => Vec::new(),
        };
        let mut boxes = Vec::new();
        let mut submits = Vec::new();
        for field in fields {
            let hint: SharedString = match field.hint {
                "" => field.caption.into(),
                hint => hint.into(),
            };
            let text_box = cx.new(|cx| {
                if field.multiline {
                    TextInput::multi_line(hint, 6, window, cx)
                } else {
                    TextInput::new(hint, window, cx)
                }
            });
            if let Some(initial) = field.initial {
                text_box.update(cx, |text_box, cx| text_box.set_text(initial, window, cx));
            }
            // In a message, Enter starts a new line instead of submitting.
            if !field.multiline {
                submits.push(cx.subscribe(&text_box, |this: &mut Self, _, event, cx| {
                    if matches!(event, TextInputEvent::Submitted) {
                        this.submit_pending(cx);
                    }
                }));
            }
            boxes.push((field.key, text_box));
        }
        self.popups.pending = Some(PendingPick {
            item,
            boxes,
            focus_first: true,
            problem: None,
            _submits: submits,
        });
        cx.notify();
    }

    pub(super) fn drop_pending(&mut self, cx: &mut Context<Self>) {
        self.popups.pending = None;
        cx.notify();
    }

    pub(super) fn submit_pending(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = &mut self.popups.pending else {
            return;
        };
        let answers: Answers = pending
            .boxes
            .iter()
            .map(|(key, text_box)| (*key, text_box.read(cx).text().to_string()))
            .collect();
        // A form that does not add up stays open and says why.
        if let Err(problem) = pending.item.outcome(&answers) {
            pending.problem = Some(problem.into());
            cx.notify();
            return;
        }
        if let Some(pending) = self.popups.pending.take() {
            self.carry_out(pending.item, &answers, cx);
        }
    }

    fn carry_out(&mut self, item: Item, answers: &Answers, cx: &mut Context<Self>) {
        let outcome = match item.outcome(answers) {
            Ok(outcome) => outcome,
            Err(problem) => return self.announce(problem, true, cx),
        };
        match outcome {
            Outcome::Git(action) => {
                let receipt = item
                    .receipt
                    .unwrap_or_else(|| format!("{}: done", item.label));
                self.run_git(*action, Some(receipt), cx);
            }
            Outcome::Clipboard(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                if let Some(receipt) = item.receipt {
                    self.announce(receipt, false, cx);
                }
            }
            Outcome::Browse(url) => cx.open_url(&url),
        }
    }

    /// Run `action` in the background, then reload whatever it can have
    /// moved. `receipt` is the toast on success; without it, the action's
    /// own report is shown, if it made one.
    pub(super) fn run_git(
        &mut self,
        action: GitAction,
        receipt: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root() else {
            return;
        };
        self.git_running = true;
        cx.notify();
        let job = cx
            .background_executor()
            .spawn(async move { crate::git::actions::run(Path::new(&root), action) });
        self.git_task = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |history, cx| {
                history.git_running = false;
                history.lit_button = None;
                match result {
                    Err(error) => history.announce(error.to_string(), true, cx),
                    Ok(report) => {
                        if let Some(text) = receipt.or(report.detail) {
                            history.announce(text, false, cx);
                        }
                        // Refs, HEAD and files may all have changed. A page
                        // still loading would show the old state, and the
                        // status decides the work-in-progress row.
                        history.refresh(cx);
                        history.reread_status(cx);
                        cx.emit(HistoryEvent::RepositoryChanged);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// A toast with how it went. Failures stay until dismissed.
    pub(super) fn announce(
        &mut self,
        text: impl Into<SharedString>,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        let kind = if failed {
            ToastKind::Error
        } else {
            ToastKind::Success
        };
        toast::show(kind, text.into().to_string(), cx);
    }

    // ── Drawing ─────────────────────────────────────────────────────────────

    pub(super) fn render_context_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(menu) = self.popups.context.shown() else {
            return gpui_kit::Empty.into_any_element();
        };
        let (groups, unfolded) = (menu.groups.clone(), menu.unfolded);
        // Unclipped: an open folder's panel hangs past the right edge.
        let mut panel = popover::popover_surface(theme)
            .w(px(PANEL_WIDTH))
            .p(px(3.0))
            .rounded(px(10.0))
            .on_mouse_down_out(cx.listener(|history, _, _, cx| history.close_context_menu(cx)));
        for (g, group) in groups.into_iter().enumerate() {
            if g > 0 {
                panel = panel.child(div().h(px(1.0)).mx(px(5.0)).my(px(3.0)).bg(hairline(0.08)));
            }
            for (r, entry) in group.into_iter().enumerate() {
                let open = unfolded == Some((g, r));
                panel = panel.child(match entry {
                    Entry::Item(item) => self.render_item_row(theme, (g, r), item, cx),
                    Entry::Folder { label, items } => {
                        self.render_folder_row(theme, (g, r), label, items, open, cx)
                    }
                });
            }
        }
        panel.into_any_element()
    }

    /// Fold away any open folder once the pointer is over another row.
    fn hovering(slot: Option<(usize, usize)>) -> impl Fn(&mut Self, &mut Context<Self>) {
        move |history, cx| {
            if let Some(menu) = history.popups.context.live_mut()
                && menu.unfolded != slot
            {
                menu.unfolded = slot;
                cx.notify();
            }
        }
    }

    fn render_item_row(
        &self,
        theme: &Theme,
        slot: (usize, usize),
        item: Item,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = format!("context-menu-{}-{}", slot.0, slot.1);
        let fold = Self::hovering(None);
        row(theme, &id, &item.label, item.danger(), false)
            // `row` already spends the one `on_hover` GPUI allows on its
            // wash, so folders follow the pointer through mouse moves.
            .on_mouse_move(cx.listener(move |history, _, _, cx| fold(history, cx)))
            .on_click(cx.listener(move |history, _, window, cx| {
                cx.stop_propagation();
                history.choose(item.clone(), window, cx);
            }))
            .into_any_element()
    }

    fn render_folder_row(
        &self,
        theme: &Theme,
        slot: (usize, usize),
        label: String,
        items: Vec<Item>,
        open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = format!("context-menu-{}-{}", slot.0, slot.1);
        let unfold = Self::hovering(Some(slot));
        let header = row(theme, &id, &label, false, open)
            .child(
                icons::icon(icons::ALT_ARROW_RIGHT)
                    .size(px(10.0))
                    .flex_none()
                    .text_color(theme.text_faint),
            )
            .on_mouse_move(cx.listener(move |history, _, _, cx| unfold(history, cx)))
            // A click toggles it as well, for when nothing hovers.
            .on_click(cx.listener(move |history, _, _, cx| {
                cx.stop_propagation();
                if let Some(menu) = history.popups.context.live_mut() {
                    menu.unfolded = (menu.unfolded != Some(slot)).then_some(slot);
                    cx.notify();
                }
            }));
        if !open {
            return header.into_any_element();
        }

        // The panel sits over the rows below it. Occluding keeps their mouse
        // moves from firing through and folding it away under the pointer.
        let mut side = popover::popover_card(theme)
            .id(ElementId::Name(format!("{id}-folder").into()))
            .occlude()
            .w(px(FOLDER_WIDTH))
            .ml(px(4.0))
            .p(px(3.0))
            .rounded(px(10.0));
        for (n, item) in items.into_iter().enumerate() {
            let line = row(
                theme,
                &format!("{id}-folder-{n}"),
                &item.label,
                item.danger(),
                false,
            );
            side = side.child(line.on_click(cx.listener(move |history, _, window, cx| {
                cx.stop_propagation();
                history.choose(item.clone(), window, cx);
            })));
        }
        div()
            .relative()
            .child(header)
            .child(div().absolute().left_full().top(px(-4.0)).child(side))
            .into_any_element()
    }

    pub(super) fn render_pending(
        &mut self,
        viewport: Size<Pixels>,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let busy = self.git_running;
        let pending = self.popups.pending.as_mut()?;
        if std::mem::take(&mut pending.focus_first)
            && let Some((_, first)) = pending.boxes.first()
        {
            window.focus(&first.read(cx).focus_handle(cx), cx);
        }
        let wording = Wording::for_gate(&pending.item.gate)?;
        let captions: Vec<&'static str> = match &pending.item.gate {
            Gate::Form(form) => form.fields.iter().map(|f| f.caption).collect(),
            _ => Vec::new(),
        };
        let inputs = captions
            .into_iter()
            .zip(pending.boxes.iter().map(|(_, b)| b.clone()))
            .map(|(caption, text_box)| {
                div()
                    .mt(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.text_muted)
                            .child(caption),
                    )
                    .child(popover::dialog_field(text_box.into_any_element()))
            });
        let problem = pending.problem.clone();

        let confirm = if wording.danger {
            popover::btn_danger(theme, &wording.button)
        } else {
            popover::btn_primary(theme, &wording.button)
        };
        let buttons = div()
            .mt(px(16.0))
            .flex()
            .justify_end()
            .gap(px(8.0))
            .child(
                popover::btn_ghost(theme, "Cancel", "pending-cancel")
                    .id("pending-cancel")
                    .on_click(cx.listener(|history, _, _, cx| history.drop_pending(cx))),
            )
            .child(
                confirm
                    .id("pending-confirm")
                    .when(busy, |button| button.opacity(0.5))
                    .on_click(cx.listener(|history, _, _, cx| {
                        if !history.git_running {
                            history.submit_pending(cx);
                        }
                    })),
            );
        let card = popover::dialog_card(theme)
            .on_key_down(cx.listener(|history, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    history.drop_pending(cx);
                }
            }))
            .child(popover::dialog_title(theme, &wording.heading))
            .when_some(wording.body, |card, body| {
                card.child(div().mt(px(6.0)).child(popover::dialog_body(theme, body)))
            })
            .children(inputs)
            .when_some(problem, |card, problem| {
                card.child(
                    div()
                        .mt(px(10.0))
                        .text_size(px(11.5))
                        .text_color(theme.danger_muted)
                        .child(problem),
                )
            })
            .child(buttons);
        Some(popover::modal(
            "pending-dialog",
            viewport,
            card.into_any_element(),
        ))
    }
}

/// One line of the menu or a folder; red when it destroys something.
fn row(
    theme: &Theme,
    id: &str,
    label: &str,
    danger: bool,
    highlighted: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    popover::menu_row(theme, highlighted, SharedString::from(id.to_owned()))
        .id(ElementId::Name(id.to_owned().into()))
        .gap(px(0.0))
        .px(px(8.0))
        .py(px(5.0))
        .rounded(px(7.0))
        .text_size(px(12.0))
        .text_color(if danger {
            theme.danger_muted
        } else {
            theme.text
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(label.to_owned())),
        )
}

#[cfg(test)]
mod tests {
    use super::super::actions::{Command, Form, Question};
    use super::*;

    #[test]
    fn a_dialog_takes_its_words_from_the_gate() {
        assert_eq!(Wording::for_gate(&Gate::Open), None);
        assert_eq!(
            Wording::for_gate(&Gate::Confirm(Question {
                heading: "Delete v1?".into(),
                body: "No way back.".into(),
                button: "Delete",
                danger: true,
            })),
            Some(Wording {
                heading: "Delete v1?".into(),
                body: Some("No way back.".into()),
                button: "Delete".into(),
                danger: true,
            })
        );
        assert_eq!(
            Wording::for_gate(&Gate::Form(Form {
                heading: "New tag".into(),
                note: None,
                button: "Create",
                fields: Vec::new(),
            })),
            Some(Wording {
                heading: "New tag".into(),
                body: None,
                button: "Create".into(),
                danger: false,
            })
        );
    }

    #[test]
    fn an_open_item_needs_no_dialog() {
        let item = Item {
            label: "Copy sha".into(),
            command: Command::Copy { text: "abc".into() },
            gate: Gate::Open,
            receipt: None,
        };
        assert!(Wording::for_gate(&item.gate).is_none());
        assert!(!item.danger());
    }
}
