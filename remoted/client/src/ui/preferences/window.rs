//! Desktop close-policy settings; lifecycle actions stay with the host.

use super::choice;
use crate::preferences::{CloseBehavior, Ui};
use nsh_base::SelectItem;
use nsh_core::prelude::*;

pub(super) fn render(ui: Ui) -> impl IntoElement {
    choice(
        ui,
        "preferences.close_behavior",
        move || {
            [CloseBehavior::Ask, CloseBehavior::Quit, CloseBehavior::Tray]
                .into_iter()
                .map(|behavior| {
                    SelectItem::new(behavior.as_str(), ui.tr(behavior.translation_key()))
                })
                .collect()
        },
        move || ui.preferences().close_behavior.as_str().into(),
        move || {
            ui.tr(ui.preferences().close_behavior.translation_key())
                .into()
        },
        move |value| {
            let behavior = match value.as_str() {
                "ask" => CloseBehavior::Ask,
                "quit" => CloseBehavior::Quit,
                "tray" => CloseBehavior::Tray,
                _ => return,
            };
            ui.update(|preferences| preferences.close_behavior = behavior);
        },
    )
}
