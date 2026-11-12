//! Native input owns composition; only committed appends reach the PTY.
//!
//! NSH f4d68f7: core element/host.rs Input::on_composition_change documents
//! on_change(final value) before Composition { composing: false, text: "" }.
//! Roll over only committed values. App-owned native generations identify a
//! reset even if it races with another edit; no per-keystroke history is kept.
use super::super::terminal_surface::Buffer;
use crate::preferences::{current_context, tr};
use nsh_base::input;
use nsh_core::prelude::*;
use std::{cell::RefCell, rc::Rc};

#[path = "terminal_input_adapter/keys.rs"]
mod keys;

#[path = "terminal_input_adapter/focus.rs"]
mod focus;
pub(super) use focus::focus_on_tap;

#[path = "terminal_input_adapter/editing.rs"]
mod editing;
use editing::Editing;

// Native selection clamps this UTF-16 offset to the current text's end. Use
// Java's positive int maximum: u32::MAX would become -1 on Android. A stale
// InputValue may be rejected by its edit count; its selection must still use
// the native current end, never the old requested value's length.
const NATIVE_END: u32 = i32::MAX as u32;

#[derive(Clone)]
struct Adapter {
    editing: Rc<RefCell<Editing>>,
    reference: ElementRef,
    value: Signal<String>,
    focused: Signal<bool>,
    composing: Signal<bool>,
    enabled: Memo<bool>,
    send: Rc<dyn Fn(&str)>,
}
impl Adapter {
    fn send(&self, text: &str) {
        if self.enabled.get_untracked() && !text.is_empty() {
            (self.send)(text);
        }
    }
    fn change(&self, text: &str) {
        if !self.enabled.get_untracked() {
            return;
        }
        let (added, value) = {
            let mut editing = self.editing.borrow_mut();
            let added = editing.change(text);
            (added, editing.take_value())
        };
        self.publish(added, value);
    }
    fn composition(&self, composing: bool) {
        if !self.enabled.get_untracked() {
            return;
        }
        let (added, value) = {
            let mut editing = self.editing.borrow_mut();
            let added = editing.composition(composing);
            (added, editing.take_value())
        };
        self.publish(added, value);
        self.composing.set(composing);
    }
    fn reset(&self) {
        let value = self.editing.borrow_mut().reset();
        self.composing.set(false);
        self.set_value(value);
    }
    fn set_value(&self, value: String) {
        self.value.set(value);
        // Core emits commands after this batch's InputValue. An imperative
        // request also repeats when successive resets have equal lengths,
        // which a selected_range prop alone would deduplicate.
        self.reference.set_selection(NATIVE_END, NATIVE_END);
    }
    fn publish(&self, added: String, value: Option<String>) {
        if let Some(value) = value {
            self.set_value(value);
        }
        self.send(&added);
    }
    fn submit(&self, text: &str) {
        if self.composing.get_untracked() {
            return;
        }
        if !self.enabled.get_untracked() {
            return;
        }
        let (added, value) = {
            let mut editing = self.editing.borrow_mut();
            let added = editing.submit(text);
            (added, editing.take_value())
        };
        self.publish(added, value);
        self.send("\r");
    }
}

pub(super) fn render(
    reference: ElementRef,
    enabled: Memo<bool>,
    buffer: Buffer,
    send: impl Fn(&str) + 'static,
) -> impl IntoElement {
    let appearance = current_context();
    let adapter = Adapter {
        editing: Rc::default(),
        reference,
        value: signal(Editing::initial_value()),
        focused: signal(false),
        composing: signal(false),
        enabled,
        send: Rc::new(send),
    };
    let key = adapter.clone();
    let modes = buffer.clone();
    effect(move || {
        if !key.focused.get() || !key.enabled.get() || key.composing.get() {
            return;
        }
        for (stroke, bytes) in keys::bindings(apple_keys()) {
            let key = key.clone();
            let modes = modes.clone();
            on_key_with(
                stroke,
                KeyOptions {
                    when: nsh_core::keymap::When::Always,
                    repeat: true,
                    ..Default::default()
                },
                move |_| {
                    if !key.composing.get_untracked() {
                        if stroke == "enter" {
                            key.reset();
                        }
                        key.send(&bytes.encode(modes.modes()));
                    }
                },
            );
        }
    });
    let disabled = adapter.clone();
    effect(move || {
        if !enabled.get() {
            disabled.reset();
        }
    });
    let (change, composition, submit, paste, focus, blur, key) = (
        adapter.clone(),
        adapter.clone(),
        adapter.clone(),
        adapter.clone(),
        adapter.clone(),
        adapter.clone(),
        adapter.clone(),
    );
    input()
        .element_ref(&reference)
        .test_id("terminal-native-input")
        .absolute()
        .left_0()
        .bottom_0()
        .w(px(1.0))
        .h(move || px(appearance.line_height()))
        .pointer_events(PointerEvents::None)
        .p_0()
        .border(px(0.0))
        .bg(AdaptiveColor::TRANSPARENT)
        .text_color(AdaptiveColor::TRANSPARENT)
        .opacity(0.02)
        .caret(false)
        .auto_correct(false)
        .auto_capitalize(AutoCapitalize::None)
        .readonly(move || !enabled.get())
        .submit_behavior(SubmitBehavior::Submit)
        .paste_behavior(PasteBehavior::Report)
        .value(move || adapter.value.get())
        .selected_range((NATIVE_END, NATIVE_END))
        .label(move || tr("远程终端；取得控制权后可输入"))
        .on_change(move |text: &str| change.change(text))
        .on_composition_change(move |event: &Composition| composition.composition(event.composing))
        .on_submit(move |text: &str| submit.submit(text))
        .on_key_press(move |stroke: &str| {
            if !key.composing.get_untracked() && stroke == "Backspace" {
                // Software keyboards edit through InputConnection, without a
                // hardware keymap command. Hardware bindings consume the key.
                key.send("\x7f");
            }
        })
        .on_paste(move |event: &PasteEvent| {
            let text = event.text.replace("\r\n", "\r").replace('\n', "\r");
            if buffer.modes().bracketed_paste {
                paste.send(&format!("\x1b[200~{text}\x1b[201~"));
            } else {
                paste.send(&text);
            }
        })
        .on_focus(move || {
            focus.focused.set(true);
            // Native initial props can place selection before InputValue.
            // Re-arm the mounted editor when the user actually focuses it.
            if focus.enabled.get_untracked() && !focus.composing.get_untracked() {
                focus.reference.set_selection(NATIVE_END, NATIVE_END);
            }
        })
        .on_blur(move || {
            blur.focused.set(false);
            blur.reset();
        })
}

#[cfg(test)]
#[path = "terminal_input_adapter/tests.rs"]
mod tests;
