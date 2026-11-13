//! Saved terminal preferences composed from the shared settings controls.

use super::choice;
use crate::preferences::Ui;
use crate::ui::controls::hint;
use nsh_base::SelectItem;
use nsh_core::prelude::*;

pub(super) fn render(ui: Ui) -> impl IntoElement {
    view()
        .gap_2()
        .child(choice(
            ui,
            "preferences.remote_auto_control",
            move || {
                vec![
                    SelectItem::new("manual", ui.tr("preferences.remote_control_manual")),
                    SelectItem::new("auto", ui.tr("preferences.remote_control_auto")),
                ]
            },
            move || {
                if ui.preferences().remote_auto_control {
                    "auto"
                } else {
                    "manual"
                }
                .into()
            },
            move || {
                ui.tr(if ui.preferences().remote_auto_control {
                    "preferences.remote_control_auto"
                } else {
                    "preferences.remote_control_manual"
                })
                .into()
            },
            move |value| ui.update(|preferences| preferences.remote_auto_control = value == "auto"),
        ))
        .child(hint(move || ui.tr("preferences.remote_auto_control_hint")))
}
