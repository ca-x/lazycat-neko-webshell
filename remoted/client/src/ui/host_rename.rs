//! A device-scoped name editor that waits for the controller's confirmed save.

use crate::{
    controller::Command,
    preferences::tr,
    storage::{self, Binding},
    ui::{connection::host_names, controls, state::Ui, terminal_view::interaction},
};
use nsh_base::{context_menu, dialog, menu, theme};
use nsh_core::prelude::*;

/// Wrap the device row in native command surfaces and keep its edit draft with the row.
pub(super) fn device(
    ui: Ui,
    binding: ReadSignal<Binding>,
    row: impl IntoElement,
) -> impl IntoElement {
    let open = signal(false);
    let draft = signal(String::new());
    let failure = signal(String::new());
    let submitted = signal(None::<String>);
    let initialized = signal(false);
    let edit = move || {
        if !initialized.get_untracked() {
            draft.set(binding.with_untracked(|binding| {
                ui.model.with_untracked(|model| {
                    model.aliases.get(binding).unwrap_or_default().to_owned()
                })
            }));
            initialized.set(true);
        }
        open.set(true);
    };
    effect(move || {
        let Some(expected) = submitted.get() else {
            return;
        };
        let target = binding.get();
        ui.model.with(|model| {
            if model.alias_saving.as_ref() == Some(&target) {
                return;
            }
            if model.aliases.get(&target).unwrap_or_default() == expected {
                submitted.set(None);
                initialized.set(false);
                open.set(false);
            } else if !model.error.is_empty() {
                failure.set(model.error.clone());
                submitted.set(None);
            }
        });
    });
    let busy =
        move || submitted.get().is_some() || ui.model.with(|model| model.alias_saving.is_some());
    let changed = move || {
        binding.with(|binding| {
            ui.model.with(|model| {
                draft.with(|draft| draft.trim() != model.aliases.get(binding).unwrap_or_default())
            })
        })
    };
    let save = move || {
        if busy() || !changed() {
            return;
        }
        let alias = match storage::normalize_host_alias(&draft.get_untracked()) {
            Ok(alias) => alias,
            Err(error) => {
                failure.set(error.to_string());
                return;
            }
        };
        failure.set(String::new());
        ui.model.update(|model| model.error.clear());
        if ui.submit(Command::RenameBinding {
            binding: binding.get_untracked(),
            alias: alias.clone(),
        }) {
            submitted.set(Some(alias));
        } else {
            failure.set(ui.model.with_untracked(|model| model.error.clone()));
        }
    };
    let items = move || interaction::menu_action("重命名", busy, edit);
    view()
        .child(
            context_menu()
                .popup_style(interaction::menu_surface)
                .trigger(
                    view()
                        .flex_row()
                        .items_center()
                        .child(view().flex_1().min_w(px(0.0)).child(row))
                        .child(
                            menu()
                                .popup_style(interaction::menu_surface)
                                .trigger(controls::icon_trigger(
                                    controls::icons::Icon::More,
                                    "主机别名",
                                ))
                                .child(items()),
                        ),
                )
                .child(items()),
        )
        .child(when(open, move || {
            let input_ref = element_ref();
            dialog()
                .label(move || tr("主机别名"))
                .close_on_escape(move || !busy())
                .close_on_backdrop_press(move || !busy())
                .close_on_swipe(move || !busy())
                .on_cancel(move || {
                    if !busy() {
                        open.set(false);
                    }
                })
                .on_open(move || input_ref.focus())
                .child(
                    view().flex_1().p_4().items_center().justify_center().child(
                        view()
                            .w_full()
                            .max_w(px(400.0))
                            .p_4()
                            .gap_3()
                            .rounded(move || theme().radius)
                            .bg(move || theme().popover)
                            .text_color(move || theme().popover_foreground)
                            .border_1()
                            .border_color(move || theme().border)
                            .child(controls::heading("主机别名"))
                            .child(
                                text(move || {
                                    binding.with(|binding| {
                                        ui.model
                                            .with(|model| host_names::display_name(model, binding))
                                    })
                                })
                                .text_sm()
                                .number_of_lines(1),
                            )
                            .child(
                                nsh_base::input()
                                    .element_ref(&input_ref)
                                    .label(move || tr("主机别名"))
                                    .placeholder(move || tr("仅保存在此客户端").to_string())
                                    .value(move || draft.get())
                                    .on_change(move |text: &str| draft.set(text.into()))
                                    .on_submit(move |_: &str| save())
                                    .readonly(busy)
                                    .min_h(controls::control_size)
                                    .px_3()
                                    .py_2()
                                    .rounded(move || theme().radius)
                                    .bg(move || theme().background)
                                    .border_1()
                                    .border_color(move || theme().input)
                                    .text_color(move || theme().foreground)
                                    .focus(|style| {
                                        style.outline(px(2.0)).outline_color(move || theme().ring)
                                    }),
                            )
                            .child(controls::hint("留空恢复默认名称。"))
                            .child(when(
                                move || !failure.get().is_empty(),
                                move || {
                                    text(move || tr(&failure.get()))
                                        .text_sm()
                                        .text_color(move || theme().destructive)
                                        .role(AccessibilityRole::Alert)
                                },
                            ))
                            .child(
                                view()
                                    .flex_row()
                                    .justify_end()
                                    .gap_2()
                                    .child(controls::action("取消", false, busy, move || {
                                        initialized.set(false);
                                        failure.set(String::new());
                                        open.set(false);
                                    }))
                                    .child(controls::action(
                                        move || {
                                            if busy() {
                                                "正在保存别名…"
                                            } else {
                                                "保存别名"
                                            }
                                        },
                                        true,
                                        move || busy() || !changed(),
                                        save,
                                    )),
                            ),
                    ),
                )
        }))
}

#[cfg(test)]
#[path = "host_rename_tests.rs"]
mod tests;
