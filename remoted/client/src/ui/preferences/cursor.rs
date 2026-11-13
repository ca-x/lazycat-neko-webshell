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
