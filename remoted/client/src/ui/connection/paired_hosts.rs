use super::{host_names, *};
use crate::preferences::tr;

pub(super) fn render(ui: Ui) -> impl IntoElement {
    when(
        move || ui.model.with(|model| !model.bindings.is_empty()),
        move || {
            view()
                .gap_3()
                .child(heading("已配对设备"))
                .child(hint(
                    "忘记仅清除本机配对，不撤销主机授权；如需撤销，请在小橘管理此设备。",
                ))
                .child(when(
                    move || ui.model.with(|model| !model.error.is_empty()),
                    move || {
                        text(move || ui.model.with(|model| tr(&model.error)))
                            .text_sm()
                            .text_color(move || nsh_base::theme().destructive)
                    },
                ))
                .child(each(
                    move || ui.model.with(|model| model.bindings.clone()),
                    |binding| {
                        (
                            binding.relay.origin().to_owned(),
                            binding.route.clone(),
                            binding.host_key.clone(),
                            binding.device_id.clone(),
                        )
                    },
                    move |binding, _| {
                        let draft = signal(binding.with_untracked(|binding| {
                            ui.model.with_untracked(|model| {
                                model.aliases.get(binding).unwrap_or_default().to_owned()
                            })
                        }));
                        view()
                            .gap_2()
                            .child(
                                text(move || {
                                    binding.with(|binding| {
                                        ui.model
                                            .with(|model| host_names::display_name(model, binding))
                                    })
                                })
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                text(move || {
                                    binding.with(|binding| {
                                        format!(
                                            "{} · {}",
                                            binding.relay.origin(),
                                            host_names::fingerprint(binding)
                                        )
                                    })
                                })
                                .text_sm(),
                            )
                            .child(field(draft, "主机别名", "仅保存在此客户端"))
                            .child(hint("留空恢复默认名称。"))
                            .child(action(
                                move || {
                                    binding.with(|binding| {
                                        if ui.model.with(|model| {
                                            model.alias_saving.as_ref() == Some(binding)
                                        }) {
                                            "正在保存别名…"
                                        } else {
                                            "保存别名"
                                        }
                                    })
                                },
                                true,
                                move || {
                                    binding.with(|binding| {
                                        ui.model.with(|model| {
                                            model.alias_saving.is_some()
                                                || draft.with(|draft| {
                                                    draft.trim()
                                                        == model
                                                            .aliases
                                                            .get(binding)
                                                            .unwrap_or_default()
                                                })
                                        })
                                    })
                                },
                                move || {
                                    ui.send(Command::RenameBinding {
                                        binding: binding.get_untracked(),
                                        alias: draft.get_untracked(),
                                    });
                                },
                            ))
                            .child(action(
                                "重新连接",
                                false,
                                || false,
                                move || {
                                    ui.send(Command::Reconnect(binding.get_untracked()));
                                },
                            ))
                            .child(action(
                                "忘记此配对（仅本机）",
                                false,
                                || false,
                                move || {
                                    ui.send(Command::Forget(binding.get_untracked()));
                                },
                            ))
                    },
                ))
        },
    )
}

#[cfg(test)]
mod tests;
