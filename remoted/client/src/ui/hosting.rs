//! Native desktop hosting settings, separate from the selected remote workspace.
use super::{
    controls::{action, field_with_edit, heading, hint},
    state::Ui,
};
use crate::controller::Command;
use neoko_terminal_host::Action;
use nsh_core::prelude::*;

mod devices;

pub fn render(ui: Ui) -> impl IntoElement {
    let expanded = signal(false);
    view()
        .gap_3()
        .child(action(
            move || {
                ui.model.with(|m| match &m.host {
                    Some(host) if host.enabled => "远程控制本机 · 已开启",
                    _ => "远程控制本机 · 已关闭",
                })
            },
            false,
            move || ui.model.with(|m| m.host.is_none()),
            move || expanded.update(|v| *v = !*v),
        ))
        .child(when(move || expanded.get(), move || body(ui)))
}

fn body(ui: Ui) -> impl IntoElement {
    let address = signal(
        ui.model
            .with_untracked(|m| m.host.as_ref().map(|h| h.relay.clone()).unwrap_or_default()),
    );
    let revision = signal(
        ui.model
            .with_untracked(|m| m.host.as_ref().map_or(0, |h| h.revision)),
    );
    let edited = signal(false);
    let submitted = signal(None::<(String, bool)>);
    effect(move || {
        let current = ui.model.with(|m| {
            m.host
                .as_ref()
                .map(|h| (h.revision, h.relay.clone(), h.enabled))
        });
        if let Some((next, relay, enabled)) = current {
            let completed = submitted.with_untracked(|s| {
                s.as_ref().is_some_and(|(wanted, on)| {
                    wanted == &relay && *on == enabled && next > revision.get_untracked()
                })
            });
            if completed
                && submitted.with_untracked(|s| {
                    s.as_ref()
                        .is_some_and(|(wanted, _)| *wanted == address.get_untracked())
                })
            {
                edited.set(false);
                submitted.set(None);
            }
            if !edited.get_untracked() {
                address.set(relay);
                revision.set(next);
            }
        }
    });
    view()
        .gap_3()
        .child(heading("允许其他设备连接这台电脑"))
        .child(hint(
            "本机终端始终可用。开启后，需要设备码、指纹核对和明确授权，其他设备才能访问选定会话。",
        ))
        .child(field_with_edit(
            address,
            "主机 Relay 地址",
            "https://relay.example.com",
            move || edited.set(true),
        ))
        .child(action(
            "使用公共中转",
            false,
            || false,
            move || {
                edited.set(true);
                address.set("https://neko-relay.czyt.eu.org".into());
            },
        ))
        .child(action(
            "保存 Relay 地址",
            false,
            || false,
            move || {
                let relay = address.get_untracked();
                let enabled = ui
                    .model
                    .with_untracked(|m| m.host.as_ref().is_some_and(|h| h.enabled));
                submitted.set(Some((relay.clone(), enabled)));
                ui.send(Command::Host(Action::Configure {
                    revision: revision.get_untracked(),
                    relay,
                    enabled,
                }));
            },
        ))
        .child(action(
            move || {
                ui.model.with(|m| {
                    if m.host.as_ref().is_some_and(|h| h.enabled) {
                        "停用远程控制"
                    } else {
                        "启用远程控制"
                    }
                })
            },
            true,
            move || address.with(|s| s.trim().is_empty()),
            move || {
                let relay = address.get_untracked();
                let enabled = !ui
                    .model
                    .with_untracked(|m| m.host.as_ref().is_some_and(|h| h.enabled));
                submitted.set(Some((relay.clone(), enabled)));
                ui.send(Command::Host(Action::Configure {
                    revision: revision.get_untracked(),
                    relay,
                    enabled,
                }));
            },
        ))
        .child(hint(move || {
            ui.model.with(|m| {
                m.host
                    .as_ref()
                    .map(|h| format!("Relay：{}", h.status))
                    .unwrap_or_else(|| "主机服务尚未就绪".into())
            })
        }))
        .child(when(
            move || {
                ui.model
                    .with(|m| m.host.as_ref().is_some_and(|h| !h.error.is_empty()))
            },
            move || {
                text(move || {
                    ui.model
                        .with(|m| m.host.as_ref().map(|h| h.error.clone()).unwrap_or_default())
                })
                .text_sm()
                .text_color(move || nsh_base::theme().destructive)
            },
        ))
        .child(when(
            move || {
                ui.model
                    .with(|m| m.host.as_ref().is_some_and(|h| h.enabled))
            },
            move || invitation(ui),
        ))
        .child(devices::pending(ui))
        .child(devices::paired(ui))
}

fn invitation(ui: Ui) -> impl IntoElement {
    view().gap_2()
        .child(action("生成配对设备码", false, || false, move || {
            if let Some(revision) = ui.model.with_untracked(|m| m.host.as_ref().map(|h| h.revision)) {
                ui.send(Command::Host(Action::Invite { revision }));
            }
        }))
        .child(when(move || ui.model.with(|m| m.host.as_ref().is_some_and(|h| h.invitation.is_some())), move || {
            view().gap_2().child(hint("两端填写相同 Relay 地址。将设备码输入另一台 neoko-terminal，再回到这里核对指纹并批准。"))
                .child(action("复制设备码", false, || false, move || {
                    if let Some(code) = ui.model.with_untracked(|m| m.host.as_ref().and_then(|h| h.invitation.clone())) {
                        nsh_core::element::set_clipboard(code);
                    }
                }))
        }))
}
