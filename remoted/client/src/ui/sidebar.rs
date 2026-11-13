use super::{
    connection::host_names,
    controls::{hint, icon_action, icons::Icon, session_row, tooltip},
    navigation::Section,
    resources,
    state::Ui,
};
use crate::{controller::Command, preferences::tr};
use nsh_base::theme;
use nsh_core::prelude::*;
#[path = "agent_sidebar.rs"]
mod agent_sidebar;
#[path = "host_rename.rs"]
mod host_rename;

pub fn render(ui: Ui) -> impl IntoElement {
    view()
        .test_id("workspace-sidebar")
        .flex_1()
        .min_h(px(0.0))
        .bg(move || theme().card)
        .child(
            view()
                .px_2()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    view().flex_1().child(
                        text(move || tr("设备与工作区"))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
                )
                .child(icon_action(
                    Icon::Add,
                    "添加主机",
                    || false,
                    || false,
                    move || {
                        if ui.disconnect() {
                            ui.navigation.settings.set(true);
                            ui.navigation.drawer.set(false);
                        }
                    },
                )),
        )
        .child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .px_2()
                    .gap_2()
                    .child(local_device(ui))
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
                            view()
                                .tooltip(tooltip(move || {
                                    binding.with(|binding| {
                                        format!(
                                            "{} · {}",
                                            binding.relay.origin(),
                                            host_names::fingerprint(binding)
                                        )
                                    })
                                }))
                                .child(host_rename::device(
                                    ui,
                                    binding,
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .child(view().flex_1().min_w(px(0.0)).child(session_row(
                                            Icon::Monitor,
                                            move || {
                                                binding.with(|binding| {
                                                    ui.model.with(|model| {
                                                        host_names::display_name(model, binding)
                                                    })
                                                })
                                            },
                                            move || {
                                                binding.with(|binding| {
                                                    ui.model.with(|model| {
                                                        !model.local
                                                            && model.active_binding.as_ref()
                                                                == Some(binding)
                                                    })
                                                })
                                            },
                                            || false,
                                            move || {
                                                ui.navigation.settings.set(false);
                                                ui.navigation.files.set(false);
                                                let binding = binding.get_untracked();
                                                if !ui.model.with_untracked(|model| {
                                                    model.ready()
                                                        && !model.local
                                                        && model.active_binding.as_ref()
                                                            == Some(&binding)
                                                }) {
                                                    ui.send(Command::Reconnect(binding));
                                                }
                                                ui.navigation.drawer.set(false);
                                            },
                                        )))
                                        .child(when(
                                            move || {
                                                binding.with(|binding| {
                                                    ui.model.with(|model| {
                                                        !model.local
                                                            && model.active_binding.as_ref()
                                                                == Some(binding)
                                                    })
                                                })
                                            },
                                            move || super::connection::status::render(ui),
                                        )),
                                ))
                        },
                    ))
                    .child(when(
                        move || {
                            ui.model
                                .with(|model| model.bindings.is_empty() && !model.local)
                        },
                        || hint(move || tr("添加一台主机，扫码或输入设备码配对。")),
                    ))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(icon_action(
                                Icon::Herdr,
                                "Herdr",
                                move || ui.navigation.section.get() == Section::Herdr,
                                || false,
                                move || {
                                    ui.navigation.section.set(Section::Herdr);
                                    ui.active.set(String::new());
                                    ui.navigation.settings.set(false);
                                },
                            ))
                            .child(icon_action(
                                Icon::Terminal,
                                "终端",
                                move || ui.navigation.section.get() == Section::Terminal,
                                || false,
                                move || {
                                    ui.navigation.section.set(Section::Terminal);
                                    ui.active.set(String::new());
                                    ui.navigation.settings.set(false);
                                },
                            )),
                    )
                    .child(when(
                        move || ui.model.with(|model| model.ready()),
                        move || resources::render(ui),
                    )),
            ),
        )
        .child(agent_sidebar::render(ui))
        .child(
            view()
                .px_2()
                .flex_row()
                .items_center()
                .gap_1()
                .border_t_1()
                .border_color(move || theme().border)
                .child(
                    text(move || ui.model.with(host_names::active_name))
                        .text_xs()
                        .number_of_lines(1)
                        .flex_1()
                        .min_w(px(0.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(icon_action(
                    Icon::Settings,
                    "连接设置",
                    move || ui.navigation.settings.get(),
                    || false,
                    move || {
                        ui.navigation.settings.set(true);
                        ui.navigation.drawer.set(false);
                    },
                )),
        )
}
fn local_device(ui: Ui) -> impl IntoElement {
    #[cfg(not(target_os = "android"))]
    {
        when(
            move || ui.model.with(|model| model.host.is_some()),
            move || {
                session_row(
                    Icon::Monitor,
                    move || tr("本机"),
                    move || ui.model.with(|model| model.local),
                    || false,
                    move || {
                        ui.navigation.settings.set(false);
                        ui.navigation.files.set(false);
                        if !ui
                            .model
                            .with_untracked(|model| model.local && model.ready())
                        {
                            ui.send(Command::SelectLocal);
                        }
                        ui.navigation.drawer.set(false);
                    },
                )
            },
        )
        .into_element()
    }
    #[cfg(target_os = "android")]
    {
        let _ = ui;
        view().into_element()
    }
}
