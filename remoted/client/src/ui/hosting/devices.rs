//! Explicit device approval, resource choices and revocation.
use super::*;
use neko_remoted_protocol::messages::Permission;

pub fn pending(ui: Ui) -> impl IntoElement {
    view().gap_2().child(each(
        move || {
            ui.model.with(|m| {
                m.host
                    .as_ref()
                    .map(|h| {
                        h.pending
                            .iter()
                            .map(|p| {
                                (
                                    p.id.clone(),
                                    p.name.clone(),
                                    p.fingerprint.clone(),
                                    h.revision,
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
        },
        |p| p.0.clone(),
        move |pending, _| {
            let selected = signal(Vec::<String>::new());
            let permission = signal(Permission::ReadOnly);
            view()
                .gap_2()
                .child(heading("待批准的设备"))
                .child(
                    text(move || pending.with(|p| p.1.clone()))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(hint(move || pending.with(|p| format!("设备指纹：{}", p.2))))
                .child(hint(
                    "请与连接端显示的指纹核对，再选择此设备可以访问的会话。",
                ))
                .child(resource_choices(ui, selected))
                .child(permission_choices(permission))
                .child(action(
                    "批准此设备",
                    true,
                    move || selected.with(Vec::is_empty),
                    move || {
                        let p = pending.get_untracked();
                        ui.send(Command::Host(Action::Approve {
                            revision: p.3,
                            pending: p.0,
                            permission: permission.get_untracked(),
                            resources: selected.get_untracked(),
                        }));
                    },
                ))
        },
    ))
}

fn resource_choices(ui: Ui, selected: Signal<Vec<String>>) -> impl IntoElement {
    view().gap_1().child(each(
        move || {
            ui.model.with(|m| {
                m.host
                    .as_ref()
                    .map(|h| {
                        h.resources
                            .iter()
                            .map(|r| (r.id.clone(), r.name.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
        },
        |r| r.0.clone(),
        move |resource, _| {
            action(
                move || {
                    resource.with(|r| {
                        format!(
                            "{} · {}",
                            r.1,
                            if selected.with(|ids| ids.contains(&r.0)) {
                                "已选择"
                            } else {
                                "未选择"
                            }
                        )
                    })
                },
                false,
                || false,
                move || {
                    let id = resource.get_untracked().0;
                    selected.update(|ids| {
                        if ids.contains(&id) {
                            ids.retain(|v| v != &id)
                        } else {
                            ids.push(id)
                        }
                    });
                },
            )
        },
    ))
}

fn permission_choices(permission: Signal<Permission>) -> impl IntoElement {
    view()
        .flex_row()
        .gap_2()
        .child(super::super::controls::tab(
            "只读",
            move || permission.get() == Permission::ReadOnly,
            move || permission.set(Permission::ReadOnly),
        ))
        .child(super::super::controls::tab(
            "可操作",
            move || permission.get() == Permission::Operate,
            move || permission.set(Permission::Operate),
        ))
}

pub fn paired(ui: Ui) -> impl IntoElement {
    let expanded = signal(false);
    view().gap_2()
        .child(action(move || ui.model.with(|m| format!("已配对设备（{}）", m.host.as_ref().map_or(0, |h| h.devices.len()))),
            false, || false, move || expanded.update(|v| *v = !*v)))
        .child(when(move || expanded.get(), move || {
            view().gap_2().child(each(
                move || ui.model.with(|m| m.host.as_ref().map(|h| h.devices.iter().map(|d|
                    (d.id.clone(), d.name.clone(), d.fingerprint.clone(), d.permission, d.resources.clone(), d.online, d.rtt_ms, h.revision)
                ).collect::<Vec<_>>()).unwrap_or_default()),
                |d| d.0.clone(), move |device, _| {
                    let editing = signal(false);
                    let resources = signal(device.with_untracked(|d| d.4.clone()));
                    let permission = signal(device.with_untracked(|d| d.3));
                    let revision = signal(device.with_untracked(|d| d.7));
                    view().gap_2()
                        .child(text(move || device.with(|d| d.1.clone())).text_sm().font_weight(FontWeight::SEMIBOLD))
                        .child(hint(move || device.with(|d| format!("{} · {}{}", if d.5 { "已连接" } else { "离线" },
                            if d.3 == Permission::Operate { "可操作" } else { "只读" },
                            d.6.map(|rtt| format!(" · {rtt} ms")).unwrap_or_default()))))
                        .child(action("管理设备", false, || false, move || {
                            if !editing.get_untracked() {
                                let d = device.get_untracked();
                                resources.set(d.4); permission.set(d.3); revision.set(d.7);
                            }
                            editing.update(|v| *v = !*v);
                        }))
                        .child(when(move || editing.get(), move || {
                            view().gap_2()
                                .child(hint(move || device.with(|d| format!("指纹：{}", d.2))))
                                .child(resource_choices(ui, resources))
                                .child(permission_choices(permission))
                                .child(action("保存授权", false, move || resources.with(Vec::is_empty), move || {
                                    ui.send(Command::Host(Action::SetPermission { revision: revision.get_untracked(), device: device.get_untracked().0,
                                        permission: permission.get_untracked(), resources: resources.get_untracked() }));
                                }))
                                .child(action("撤销此设备…", false, || false, move || {
                                    let d = device.get_untracked();
                                    spawn(async move {
                                        let answer = nsh_base::alert_dialog().title(format!("撤销“{}”？", d.1))
                                            .description("该设备会立即断开。再次访问需要重新配对和授权。")
                                            .cancel("cancel", "取消").destructive("revoke", "撤销设备").show().await;
                                        if matches!(answer, Ok(result) if result.action.as_deref() == Some("revoke")) {
                                            ui.send(Command::Host(Action::Revoke { revision: d.7, device: d.0 }));
                                        }
                                    });
                                }))
                        }))
                }))
        }))
}
