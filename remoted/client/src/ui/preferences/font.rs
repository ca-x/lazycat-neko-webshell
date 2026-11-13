//! Font controls share the app's acknowledged preferences and keep custom edits local.

use super::choice;
use crate::preferences::{MAX_FONT_SIZE, MIN_FONT_SIZE, Ui, font};
use crate::ui::controls::{action, hint};
use nsh_base::{SelectItem, input, theme};
use nsh_core::prelude::*;

pub(super) fn render(ui: Ui) -> impl IntoElement {
    view()
        .gap_4()
        .child(choice(
            ui,
            "preferences.font_family",
            move || {
                let mut items = vec![
                    SelectItem::new(font::HACK, font::HACK),
                    SelectItem::new(font::MAPLE, font::MAPLE),
                ];
                if os().is_desktop() {
                    items.push(SelectItem::new(
                        font::SYSTEM,
                        ui.tr("preferences.font_system"),
                    ));
                    let family = ui.preferences().font_family;
                    if !matches!(family.as_str(), font::HACK | font::MAPLE | font::SYSTEM) {
                        items.push(SelectItem::new(family.clone(), family));
                    }
                }
                items
            },
            move || ui.preferences().font_family,
            move || {
                let family = ui.preferences().font_family;
                if family == font::SYSTEM {
                    ui.tr("preferences.font_system").into()
                } else {
                    family
                }
            },
            move |family| ui.update(|preferences| preferences.font_family = family),
        ))
        .child(when(|| os().is_desktop(), move || custom_family(ui)))
        .child(size(ui))
}

fn custom_family(ui: Ui) -> impl IntoElement {
    let saved = computed(move || ui.preferences().font_family);
    let draft = signal(saved.get());
    effect(move || draft.set(saved.get()));
    let save = move || {
        let family = draft.get_untracked();
        if font::validation_error(&family).is_none() {
            ui.update(|preferences| preferences.font_family = family);
        }
    };
    view()
        .gap_2()
        .child(text(move || ui.tr("preferences.font_custom")).text_sm())
        .child(
            input()
                .label(move || ui.tr("preferences.font_custom"))
                .value(move || draft.get())
                .on_change(move |value: &str| draft.set(value.to_owned()))
                .on_submit(move |_: &str| save())
                .max_length(font::MAX_FAMILY_BYTES as u32)
                .auto_correct(false)
                .auto_capitalize(AutoCapitalize::None)
                .disabled(move || !ui.editable())
                .min_h(crate::ui::controls::control_size)
                .px_3()
                .py_2()
                .border_1()
                .border_color(move || theme().input)
                .rounded(move || theme().radius)
                .bg(move || theme().card)
                .text_color(move || theme().foreground)
                .caret_color(move || color(theme().primary))
                .selection_color(move || color(theme().accent))
                .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .disabled_style(|s| s.opacity(0.45))
                .w_full(),
        )
        .child(hint(move || {
            ui.tr(font::validation_error(&draft.get()).unwrap_or("preferences.font_custom_hint"))
                .to_owned()
        }))
        .child(action(
            move || ui.tr("preferences.font_save"),
            true,
            move || {
                !ui.editable()
                    || draft.get() == saved.get()
                    || font::validation_error(&draft.get()).is_some()
            },
            save,
        ))
}

fn size(ui: Ui) -> impl IntoElement {
    view()
        .gap_2()
        .child(text(move || ui.tr("preferences.font_size")).text_sm())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .items_center()
                .child(action(
                    move || ui.tr("preferences.smaller"),
                    false,
                    move || !ui.editable() || ui.preferences().font_size <= MIN_FONT_SIZE,
                    move || {
                        ui.update(|preferences| {
                            preferences.font_size = preferences.font_size.saturating_sub(1)
                        })
                    },
                ))
                .child(
                    text(move || ui.preferences().font_size.to_string())
                        .text_base()
                        .tabular_nums(),
                )
                .child(action(
                    move || ui.tr("preferences.larger"),
                    false,
                    move || !ui.editable() || ui.preferences().font_size >= MAX_FONT_SIZE,
                    move || {
                        ui.update(|preferences| {
                            preferences.font_size = preferences.font_size.saturating_add(1)
                        })
                    },
                )),
        )
}
