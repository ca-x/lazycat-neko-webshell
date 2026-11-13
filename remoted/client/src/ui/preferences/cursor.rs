//! Saved terminal preferences composed from the shared settings controls.

use super::choice;
use crate::preferences::{CursorStyle, Ui};
use nsh_base::SelectItem;
use nsh_core::prelude::*;

pub(super) fn render(ui: Ui) -> impl IntoElement {
    view()
        .gap_4()
        .child(choice(
            ui,
            "preferences.cursor_style",
            move || {
                [
                    CursorStyle::Block,
                    CursorStyle::Beam,
                    CursorStyle::Underline,
                ]
                .into_iter()
                .map(|style| SelectItem::new(style.as_str(), ui.tr(style.translation_key())))
                .collect()
            },
            move || ui.preferences().cursor_style.as_str().into(),
            move || {
                ui.tr(ui.preferences().cursor_style.translation_key())
                    .into()
            },
            move |value| {
                let style = match value.as_str() {
                    "block" => CursorStyle::Block,
                    "beam" => CursorStyle::Beam,
                    "underline" => CursorStyle::Underline,
                    _ => return,
                };
                ui.update(|preferences| preferences.cursor_style = style);
            },
        ))
        .child(color_setting(ui))
        .child(choice(
            ui,
            "preferences.cursor_blink",
            move || {
                vec![
                    SelectItem::new("off", ui.tr("preferences.cursor_steady")),
                    SelectItem::new("on", ui.tr("preferences.cursor_blinking")),
                ]
            },
            move || {
                if ui.preferences().cursor_blink {
                    "on"
                } else {
                    "off"
                }
                .into()
            },
            move || {
                ui.tr(if ui.preferences().cursor_blink {
                    "preferences.cursor_blinking"
                } else {
                    "preferences.cursor_steady"
                })
                .into()
            },
            move |value| ui.update(|preferences| preferences.cursor_blink = value == "on"),
        ))
}

fn color_setting(ui: Ui) -> impl IntoElement {
    let saved = computed(move || ui.preferences().cursor_color.unwrap_or_default());
    let draft = signal(saved.get());
    effect(move || draft.set(saved.get()));
    let valid = move || {
        let value = draft.get();
        value.is_empty()
            || (value.len() == 7
                && value.starts_with('#')
                && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
    };
    view()
        .gap_2()
        .child(text(move || ui.tr("preferences.cursor_color")).text_sm())
        .child(
            nsh_base::input()
                .label(move || ui.tr("preferences.cursor_color"))
                .placeholder("#RRGGBB")
                .value(move || draft.get())
                .on_change(move |value: &str| draft.set(value.to_owned()))
                .auto_correct(false)
                .auto_capitalize(AutoCapitalize::None)
                .max_length(7)
                .disabled(move || !ui.editable())
                .min_h(super::super::controls::control_size)
                .px_3()
                .py_2()
                .border_1()
                .border_color(move || nsh_base::theme().input)
                .rounded(move || nsh_base::theme().radius)
                .bg(move || nsh_base::theme().card)
                .text_color(move || nsh_base::theme().foreground)
                .focus(|s| {
                    s.outline(px(2.0))
                        .outline_color(move || nsh_base::theme().ring)
                }),
        )
        .child(super::super::controls::hint(move || {
            ui.tr("preferences.cursor_color_hint")
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(super::super::controls::action(
                    move || ui.tr("preferences.cursor_color_save"),
                    true,
                    move || !ui.editable() || !valid() || draft.get() == saved.get(),
                    move || {
                        if valid() {
                            let value = draft.get_untracked();
                            ui.update(|next| {
                                next.cursor_color = (!value.is_empty()).then_some(value)
                            });
                        }
                    },
                ))
                .child(super::super::controls::action(
                    move || ui.tr("preferences.cursor_color_theme"),
                    false,
                    move || !ui.editable() || ui.preferences().cursor_color.is_none(),
                    move || ui.update(|next| next.cursor_color = None),
                )),
        )
}
