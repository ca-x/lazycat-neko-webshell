use super::{
    controls::{action, field, field_with_edit, heading, hint},
    state::Ui,
};
use crate::controller::{Command, Phase};
use nsh_core::prelude::*;

#[path = "host_names.rs"]
pub(super) mod host_names;
mod paired_hosts;
mod public_relay;
pub(super) mod status;

/// Icon actions shared by the settings route and its native presentation.
pub(super) fn actions(ui: Ui) -> impl IntoElement {
    use super::controls::{icon_action, icons::Icon};
    view()
        .px_4()
        .py_2()
        .flex_row()
        .gap_2()
        .child(icon_action(
            Icon::Back,
            "返回会话",
            || false,
            || false,
            move || ui.navigation.settings.set(false),
        ))
        .child(when(
            move || ui.model.with(|model| model.ready()),
            move || {
                icon_action(
                    Icon::Close,
                    "断开连接",
                    || false,
                    || false,
                    move || ui.send(Command::Disconnect),
                )
            },
        ))
}

pub fn render(ui: Ui) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .p_4()
            .gap_4()
            .child(appearance_settings())
            .child(remote_access_settings(ui))
            .child(connection_settings(ui)),
    )
}

/// Paired hosts and Relay setup, shared with the native settings category.
pub(super) fn connection_settings(ui: Ui) -> impl IntoElement {
    let address = signal(ui.model.with_untracked(|model| {
        model
            .relay
            .as_ref()
            .map(|relay| relay.origin().to_owned())
            .unwrap_or_default()
    }));
    let edited = signal(false);
    let restored = signal(ui.model.with_untracked(|model| model.relay.is_some()));
    effect(move || {
        if restored.get_untracked() {
            return;
        }
        if let Some(relay) = ui.model.with(|model| model.relay.clone()) {
            if !edited.get_untracked() {
                address.set(relay.origin().to_owned());
            }
            restored.set(true);
        }
    });
    let code = signal(String::new());
    let name = signal("我的远程设备".to_owned());
    let scanning = signal(false);
    let expanded = signal(false);
    view()
        .gap_4()
        .child(paired_hosts::render(ui))
        .child(heading("连接远程主机"))
        .child(when(
            move || ui.model.with(|model| model.phase == Phase::Offline) || expanded.get(),
            move || {
                view()
                    .gap_3()
                    .child(hint("填写与小橘终端相同的 HTTPS/WSS Relay 地址。"))
                    .child(field_with_edit(
                        address,
                        "Relay 地址",
                        "https://relay.example.com",
                        move || edited.set(true),
                    ))
                    .child(action(
                        move || public_relay::label(locale()),
                        false,
                        || false,
                        move || {
                            edited.set(true);
                            address.set(public_relay::ORIGIN.to_owned());
                        },
                    ))
                    .child(hint(move || public_relay::hint(locale())))
                    .child(action(
                        move || {
                            if ui.model.with(|model| model.phase == Phase::Checking) {
                                "正在检查…"
                            } else {
                                "检查并连接 Relay"
                            }
                        },
                        true,
                        move || {
                            address.with(|value| value.trim().is_empty())
                                || ui.model.with(|model| {
                                    !matches!(model.phase, Phase::Offline | Phase::RelayReady)
                                })
                        },
                        move || {
                            expanded.set(false);
                            ui.send(Command::Check(address.get_untracked()));
                        },
                    ))
            },
        ))
        .child(when(
            move || {
                ui.model
                    .with(|model| model.relay.is_some() && model.phase != Phase::Offline)
            },
            move || {
                view()
                    .gap_2()
                    .child(hint(move || {
                        ui.model.with(|model| {
                            model
                                .relay
                                .as_ref()
                                .map(|relay| relay.origin().to_owned())
                                .unwrap_or_default()
                        })
                    }))
                    .child(action(
                        "编辑 Relay",
                        false,
                        move || ui.model.with(|model| model.phase != Phase::RelayReady),
                        move || expanded.set(true),
                    ))
            },
        ))
        .child(when(
            move || ui.model.with(|model| model.phase == Phase::RelayReady),
            move || {
                view()
                    .gap_4()
                    .child(heading("配对设备"))
                    .child(field(name, "本设备名称", "主机批准时显示的名称"))
                    .child(field(code, "设备码", "NR1.…"))
                    .child(hint(if cfg!(target_os = "android") {
                        "扫码和手输使用同一设备码校验；扫码不会更改 Relay。"
                    } else {
                        "从主机复制设备码并粘贴，或手动输入；无需摄像头。"
                    }))
                    .child(action(
                        move || {
                            if scanning.get() {
                                "正在扫码…"
                            } else {
                                if cfg!(target_os = "android") {
                                    "扫描设备码"
                                } else {
                                    "粘贴设备码"
                                }
                            }
                        },
                        false,
                        move || scanning.get(),
                        move || {
                            scanning.set(true);
                            spawn(async move {
                                match read_device_code().await {
                                    Ok(Ok(value)) => code.set(value),
                                    Ok(Err(error)) => {
                                        ui.error(format!("读取设备码未完成，可继续手输：{error:#}"))
                                    }
                                    Err(error) => ui.error(format!("读取设备码不可用：{error}")),
                                }
                                scanning.set(false);
                            });
                        },
                    ))
                    .child(action(
                        "请求主机批准",
                        true,
                        move || code.with(|value| value.trim().is_empty()) || scanning.get(),
                        move || {
                            let value = code.get_untracked();
                            code.set(String::new());
                            ui.send(Command::Pair {
                                code: value,
                                name: name.get_untracked(),
                            });
                        },
                    ))
            },
        ))
        .child(when(
            move || ui.model.with(|model| !model.fingerprint.is_empty()),
            move || {
                hint(move || {
                    ui.model
                        .with(|model| format!("主机指纹：{}", model.fingerprint))
                })
            },
        ))
        .child(when(
            move || {
                ui.model.with(|model| {
                    matches!(
                        model.phase,
                        Phase::Connecting
                            | Phase::AwaitingApproval
                            | Phase::Persisting
                            | Phase::AwaitingReady
                    )
                })
            },
            move || {
                action(
                    "取消连接",
                    false,
                    || false,
                    move || ui.send(Command::Disconnect),
                )
            },
        ))
}

// NSH clipboard services belong to the UI thread, unlike the Android bridge.
async fn read_device_code() -> Result<Result<String, anyhow::Error>, String> {
    #[cfg(target_os = "android")]
    {
        background(crate::platform::scan_code())
            .await
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "android"))]
    {
        let value = nsh_core::element::clipboard_string().await;
        Ok(if value.trim().is_empty() {
            Err(anyhow::anyhow!("剪贴板没有设备码，请手动输入"))
        } else {
            Ok(value)
        })
    }
}

/// Hosting controls only, without connection or appearance settings.
pub(super) fn remote_access_settings(ui: Ui) -> impl IntoElement {
    #[cfg(not(target_os = "android"))]
    {
        super::hosting::render(ui).into_element()
    }
    #[cfg(target_os = "android")]
    {
        let _ = ui;
        view().into_element()
    }
}

fn appearance_settings() -> nsh_core::element::AnyElement {
    match use_context::<crate::preferences::Ui>() {
        Some(preferences) => super::preferences::render(preferences).into_element(),
        None => view().into_element(),
    }
}
