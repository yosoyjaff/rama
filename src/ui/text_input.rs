//! A single text field, with the small surface the product actually uses.
//!
//! The component library's input is a full editor: selections, IME,
//! undo, context menus, multi-line layout modes. What a search box or a commit
//! message box wants from it is four things — read the text, set the text,
//! know when it changed, know when Enter was pressed — and a field that is
//! asked for its text in a render should not need a `Window` to answer.
//!
//! So this holds the editor and mirrors its text: the mirror is what
//! [`TextInput::text`] returns, updated from the change event, which keeps the
//! reading side free of `cx` and the writing side honest.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, Textarea, TextareaState,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, relative,
};

use crate::ui::theme::Theme;
use crate::ui::typography::ui_rems;

/// What a field reports to whoever owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputEvent {
    /// The text changed, by typing, pasting or a programmatic set.
    Edited,
    /// Enter, on a field that takes it as a submission.
    Submitted,
    /// Focus left the field.
    Blurred,
}

/// A field is one of the library's three shapes: a line, a box that grows,
/// or a source-code editor with a gutter. They are separate state types with
/// separate elements, and the difference reaches the render, so it is carried
/// explicitly rather than hidden behind a row count.
enum Field {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
    Code(Entity<EditorState>),
}

pub struct TextInput {
    field: Field,
    /// The text as of the last event. See the module docs: this is what makes
    /// `text()` callable from a render.
    text: SharedString,
    text_size: f32,
    /// Draw a single line at the library's extra-small size: 20px tall, no
    /// vertical padding — for a field set inside a row, not a form.
    compact: bool,
    _subscription: Subscription,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    /// A single-line field.
    pub fn new(
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(placeholder, 1, window, cx)
    }

    /// A field that grows with its content, up to `max_rows`.
    pub fn multi_line(
        placeholder: impl Into<SharedString>,
        max_rows: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(placeholder, max_rows, window, cx)
    }

    /// A source-code editor: line numbers down the gutter, soft tabs, and
    /// the library's search. Code wants numbers — the merge editor's output
    /// box edits a file's text, and reading it unnumbered is guesswork.
    pub fn code_editor(
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .placeholder(placeholder)
                // A resolution box has no structure to fold.
                .folding(false)
        });
        let subscription = cx.subscribe(&state, |this: &mut Self, state, event, cx| {
            this.on_input_event(state.read(cx).value(), event, cx)
        });
        Self {
            field: Field::Code(state),
            text: SharedString::default(),
            text_size: 13.0,
            compact: false,
            _subscription: subscription,
        }
    }

    fn build(
        placeholder: impl Into<SharedString>,
        max_rows: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let (field, subscription) = if max_rows > 1 {
            let state = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(placeholder)
                    .auto_grow(1, max_rows)
            });
            let subscription = cx.subscribe(&state, |this: &mut Self, state, event, cx| {
                this.on_input_event(state.read(cx).value(), event, cx)
            });
            (Field::Area(state), subscription)
        } else {
            let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            let subscription = cx.subscribe(&state, |this: &mut Self, state, event, cx| {
                this.on_input_event(state.read(cx).value(), event, cx)
            });
            (Field::Line(state), subscription)
        };
        Self {
            field,
            text: SharedString::default(),
            text_size: 13.0,
            compact: false,
            _subscription: subscription,
        }
    }

    fn on_input_event(&mut self, value: SharedString, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => {
                self.text = value;
                cx.emit(TextInputEvent::Edited);
                cx.notify();
            }
            // A field that grows takes Enter as a newline; only a single line
            // reports it as a press.
            InputEvent::PressEnter { .. } => cx.emit(TextInputEvent::Submitted),
            InputEvent::Blur => cx.emit(TextInputEvent::Blurred),
            InputEvent::Focus => {}
        }
    }

    /// Draw the text at `size` pixels-at-default rather than the field's own.
    pub fn with_text_size(mut self, size: f32) -> Self {
        self.text_size = size;
        self
    }

    /// Draw a single-line field at the library's extra-small size, so it
    /// fits a 22px slot with its text centred.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// The text, as of the last change.
    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn set_text(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = text.into();
        if self.text == text {
            return;
        }
        self.text = text.clone();
        match &self.field {
            Field::Line(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
            Field::Area(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
            Field::Code(state) => state.update(cx, |state, cx| state.set_value(text, window, cx)),
        }
        cx.notify();
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.field {
            Field::Line(state) => state.read(cx).focus_handle(cx),
            Field::Area(state) => state.read(cx).focus_handle(cx),
            Field::Code(state) => state.read(cx).focus_handle(cx),
        }
    }

    /// The entity id of the editor state actually drawn — gpui-component
    /// names its element after it, so a UI test can observe the field.
    pub fn editor_id(&self) -> u64 {
        match &self.field {
            Field::Line(state) => state.entity_id().as_u64(),
            Field::Area(state) => state.entity_id().as_u64(),
            Field::Code(state) => state.entity_id().as_u64(),
        }
    }

    /// Put the caret in this field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle(cx).focus(window, cx);
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        // No plate of its own: a field here sits inside chrome that already
        // drew one, and a second border around it reads as a box in a box.
        // Both heights exist because the library pins a multi-line field to
        // `h_auto` (its content, capped by `auto_grow`): the root's `h_full`
        // makes this wrapper a definite-height slot, and the field's own
        // `h(relative(1.0))` fills it — the merge editor's edit area. Against
        // an auto-height container a percentage is nothing, so the field
        // falls back to its content size and the dialogs are untouched.
        div()
            .w_full()
            .h_full()
            .text_size(ui_rems(self.text_size))
            .text_color(theme.text)
            .child(match &self.field {
                Field::Line(state) => {
                    let input = Input::new(state).appearance(false).bordered(false);
                    if self.compact {
                        input.xsmall().into_any_element()
                    } else {
                        input.into_any_element()
                    }
                }
                Field::Area(state) => Textarea::new(state)
                    .appearance(false)
                    .bordered(false)
                    .h(relative(1.0))
                    .into_any_element(),
                Field::Code(state) => Editor::new(state)
                    .appearance(false)
                    .bordered(false)
                    .h(relative(1.0))
                    .into_any_element(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        InteractiveElement as _, TestSupportExt as _, prelude::FluentBuilder as _, px,
        test::TestWindowExt as _,
    };

    /// The chain the merge editor builds over the edit box: a sized area
    /// holding a padded slot with the field in it. `definite` chooses
    /// whether the slot has a height of its own — the merge editor's does,
    /// a dialog's fields do not.
    struct Probe {
        input: Entity<TextInput>,
        definite: bool,
    }

    impl Probe {
        fn build(
            kind: FieldKind,
            definite: bool,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Self {
            let input = cx.new(|cx| {
                let input = match kind {
                    FieldKind::MultiLine => {
                        TextInput::multi_line("Edit the resolution", 6, window, cx)
                    }
                    FieldKind::Code => TextInput::code_editor("Edit the resolution", window, cx),
                };
                let mut long = String::new();
                for _ in 0..20 {
                    long.push_str("line of the resolution\n");
                }
                let mut input = input;
                input.set_text(long, window, cx);
                input
            });
            Probe { input, definite }
        }
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().h(px(32.0)).flex_none())
                .child(
                    div()
                        .id("editing-area")
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .test_support()
                        .when(self.definite, |slot| {
                            slot.child(
                                div()
                                    .id("pad")
                                    .size_full()
                                    .p(px(8.0))
                                    .test_support()
                                    .child(self.input.clone()),
                            )
                        })
                        .when(!self.definite, |slot| {
                            slot.child(
                                div()
                                    .id("pad")
                                    .p(px(8.0))
                                    .test_support()
                                    .child(self.input.clone()),
                            )
                        }),
                )
        }
    }

    enum FieldKind {
        MultiLine,
        Code,
    }

    fn probe(kind: FieldKind, definite: bool, cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            crate::ui::theme::init(cx);
        });
        let field_id = std::cell::Cell::new(0u64);
        let handle = cx.add_window(|window, cx| {
            let probe = Probe::build(kind, definite, window, cx);
            field_id.set(probe.input.read(cx).editor_id());
            probe
        });
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let field = window
                .find(gpui_kit::ElementId::NamedInteger(
                    "input".into(),
                    field_id.get(),
                ))
                .bounds();
            let slot = window.find("pad").bounds();
            if definite {
                // Six rows of the field's own cap would be about 136px;
                // filling is the slot's height minus its padding.
                let expected = slot.size.height - px(16.0);
                assert!(
                    (field.size.height - expected).abs() < px(2.0),
                    "field height {:?} should fill the slot's {expected:?}",
                    field.size.height
                );
                assert!(field.size.height > px(200.0));
            } else {
                // A percentage is nothing against an auto-height container,
                // so the field keeps its content size — what dialogs rely on.
                assert!(
                    field.size.height < px(200.0),
                    "field height {:?} should stay content-sized",
                    field.size.height
                );
            }
        });
    }

    #[gpui_kit::test]
    fn a_multi_line_field_fills_a_sized_slot(cx: &mut gpui_kit::TestAppContext) {
        probe(FieldKind::MultiLine, true, cx);
    }

    #[gpui_kit::test]
    fn a_multi_line_field_stays_content_sized_in_an_auto_slot(cx: &mut gpui_kit::TestAppContext) {
        probe(FieldKind::MultiLine, false, cx);
    }

    #[gpui_kit::test]
    fn a_code_editor_fills_a_sized_slot(cx: &mut gpui_kit::TestAppContext) {
        probe(FieldKind::Code, true, cx);
    }
}
