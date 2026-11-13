//! Authorized targets and Herdr sessions, presented as a navigable list.
use super::{
    controls::{hint, icon_action, icons::Icon, session_row},
    navigation::Section,
    state::Ui,
};
use crate::controller::Command;
use crate::preferences::{tr, tr_format};
use neko_remoted_protocol::messages::Operation;
use nsh_core::prelude::*;

#[cfg(not(target_os = "android"))]
#[path = "local_herdr.rs"]
mod local_herdr;

#[cfg(not(target_os = "android"))]
#[path = "local_hc.rs"]
mod local_hc;

#[derive(Clone, PartialEq)]
struct Target {
    id: String,
    name: String,
    herdr: bool,
    terminal: bool,
}

pub fn render(ui: Ui) -> impl IntoElement {
    let content = view().gap_2();
    #[cfg(not(target_os = "android"))]
    let content = content.child(local_hc::render(ui));
    content
        .child(local_herdr_status(ui))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(move || {
                        if ui.navigation.section.get() == Section::Herdr {
                            tr("Herdr 工作区")
                        } else {
                            tr("终端会话")
                        }
                    })
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .flex_1(),
                )
                .child(local_actions(ui))
                .child(icon_action(
                    Icon::Refresh,
                    move || {
                        #[cfg(not(target_os = "android"))]
                        if ui.model.with(|model| model.local)
                            && ui.navigation.section.get() == Section::Herdr
                        {
                            return "刷新 Herdr 状态";
                        }
                        "刷新会话"
                    },
                    || false,
                    move || ui.model.with(|m| m.opening),
                    move || {
                        #[cfg(not(target_os = "android"))]
                        if ui.navigation.section.get_untracked() == Section::Herdr
                            && let Some(revision) = ui.model.with_untracked(|model| {
                                model
                                    .host
                                    .as_ref()
                                    .filter(|_| model.local)
                                    .map(|host| host.revision)
                            })
                        {
                            ui.send(Command::Host(neoko_terminal_host::Action::RefreshHerdr {
                                revision,
                            }));
                        }
                        ui.send(Command::Request(Operation::Resources {}));
                        let grant = ui.model.with_untracked(|m| m.herdr_grant.clone());
                        if ui.navigation.section.get_untracked() == Section::Herdr
                            && !grant.is_empty()
                        {
                            ui.send(Command::Request(Operation::Herdr {
                                grant,
                                method: "session.snapshot".into(),
                                params: serde_json::json!({}),
                            }));
                        }
                    },
                )),
        )
        .child(when(
            move || {
                ui.model.with(|m| {
                    #[cfg(not(target_os = "android"))]
                    if m.local && ui.navigation.section.get() == Section::Herdr {
                        return false;
                    }
                    !m.resources
                        .iter()
                        .any(|r| match ui.navigation.section.get() {
                            Section::Herdr => r.herdr,
                            Section::Terminal => r.terminal && !r.herdr,
                        })
                })
            },
            move || {
                hint(move || {
                    #[cfg(not(target_os = "android"))]
                    if ui.model.with(|model| model.local) {
                        return tr("还没有本机会话，点击新建会话开始。");
                    }
                    if ui.navigation.section.get() == Section::Herdr {
                        tr("没有获授权的 Herdr 工作区。请在主机端授权已有 Herdr 会话。")
                    } else {
                        tr("没有获授权的普通终端。请在主机端授权已有终端会话。")
                    }
                })
            },
        ))
        .child(when(
            move || ui.model.with(|m| m.opening),
            || hint(move || tr("正在打开会话…")),
        ))
        .child(each(
            move || {
                ui.model.with(|m| {
                    m.resources
                        .iter()
                        .filter(|r| match ui.navigation.section.get() {
                            Section::Herdr => r.herdr,
                            Section::Terminal => r.terminal && !r.herdr,
                        })
                        .map(|r| Target {
                            id: r.id.clone(),
                            name: r.name.clone(),
                            herdr: r.herdr,
                            terminal: r.terminal,
                        })
                        .collect::<Vec<_>>()
                })
            },
            |r| r.id.clone(),
            move |r, _| {
                view()
                    .gap_1()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .style_when(
                                move || {
                                    r.with(|target| {
                                        target.herdr
                                            && target.id
                                                == ui.model.with(|model| model.herdr_grant.clone())
                                            && direct_spaces(ui)
                                    })
                                },
                                |style| style.hidden(),
                            )
                            .child(view().flex_1().min_w(px(0.0)).child(session_row(
                                if r.get_untracked().herdr {
                                    Icon::Herdr
                                } else {
                                    Icon::Terminal
                                },
                                move || r.with(|r| r.name.clone()),
                                move || r.with(|r| ui.model.with(|m| m.herdr_grant == r.id)),
                                move || ui.model.with(|m| m.opening),
                                move || {
                                    let r = r.get_untracked();
                                    if r.herdr {
                                        ui.send(Command::Request(Operation::Herdr {
                                            grant: r.id,
                                            method: "session.snapshot".into(),
                                            params: serde_json::json!({}),
                                        }));
                                    } else if r.terminal {
                                        ui.send(Command::Request(Operation::TerminalAttach {
                                            grant: r.id,
                                            pane: None,
                                        }));
                                        ui.navigation.show_session();
                                    }
                                },
                            )))
                            .child(local_close(
                                ui,
                                move || r.with(|r| r.id.clone()),
                                move || r.with(|r| r.name.clone()),
                                move || r.with(|r| r.terminal && !r.herdr),
                            )),
                    )
                    .child(when(
                        move || {
                            r.with(|r| r.herdr && r.id == ui.model.with(|m| m.herdr_grant.clone()))
                        },
                        move || hierarchy(ui),
                    ))
            },
        ))
}

fn local_herdr_status(ui: Ui) -> impl IntoElement {
    #[cfg(not(target_os = "android"))]
    {
        when(
            move || {
                ui.model.with(|model| {
                    model.host.as_ref().is_none_or(|host| {
                        host.herdr.status != neoko_terminal_host::HerdrStatus::Ready
                    })
                })
            },
            move || local_herdr::render(ui),
        )
        .into_element()
    }
    #[cfg(target_os = "android")]
    {
        let _ = ui;
        view().into_element()
    }
}

/// With one selected source, the workspaces already identify the destination.
fn direct_spaces(ui: Ui) -> bool {
    ui.model.with(|model| {
        model
            .resources
            .iter()
            .filter(|resource| resource.herdr)
            .count()
            == 1
            && !model.herdr_workspace.workspaces.is_empty()
    })
}

/// Spaces are the sidebar's leaves; tabs and individual panes live in the main bar.
fn hierarchy(ui: Ui) -> impl IntoElement {
    view()
        .gap_1()
        .style_when(move || !direct_spaces(ui), |style| style.pl_2())
        .child(each(
            move || {
                ui.model
                    .with(|model| model.herdr_workspace.workspaces.clone())
            },
            |workspace| workspace.workspace_id.clone(),
            move |workspace, _| {
                session_row(
                    Icon::Herdr,
                    move || workspace.with(|workspace| workspace.label.clone()),
                    move || {
                        workspace.with(|workspace| current_workspace(ui) == workspace.workspace_id)
                    },
                    move || ui.model.with(|model| model.opening),
                    move || select_workspace(ui, &workspace.get_untracked().workspace_id),
                )
            },
        ))
        .child(when(
            move || {
                ui.model.with(|model| {
                    model.herdr_workspace.workspaces.is_empty()
                        && model.herdr_workspace.panes.is_empty()
                        && !model.opening
                })
            },
            || hint(move || tr("此工作区当前没有可打开的窗格。")),
        ))
}

pub(super) fn current_workspace(ui: Ui) -> String {
    let selected = ui.navigation.workspace.get();
    let active = ui.active.get();
    ui.model.with(|model| {
        let snapshot = &model.herdr_workspace;
        if selected.0 == model.herdr_grant
            && snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.workspace_id == selected.1)
        {
            return selected.1.clone();
        }
        if let Some(pane) = model
            .panes
            .iter()
            .find(|pane| {
                pane.stream == active
                    && pane.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
            })
            .and_then(|open| {
                snapshot
                    .panes
                    .iter()
                    .find(|pane| Some(pane.pane_id.as_str()) == open.herdr_pane_id.as_deref())
            })
        {
            return pane.workspace_id.clone();
        }
        snapshot
            .workspaces
            .iter()
            .reduce(|selected, candidate| {
                if candidate.focused && !selected.focused {
                    candidate
                } else {
                    selected
                }
            })
            .map(|workspace| workspace.workspace_id.clone())
            .unwrap_or_default()
    })
}

pub(super) fn select_workspace(ui: Ui, workspace: &str) {
    ui.navigation.section.set(Section::Herdr);
    ui.navigation.workspace.set((
        ui.model.with_untracked(|model| model.herdr_grant.clone()),
        workspace.into(),
    ));
    let id = ui.model.with_untracked(|model| {
        let tab = model
            .herdr_workspace
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace)
            .reduce(|selected, candidate| {
                if candidate.focused && !selected.focused {
                    candidate
                } else {
                    selected
                }
            });
        model
            .herdr_workspace
            .preferred_pane(workspace, tab.map(|tab| tab.tab_id.as_str()))
            .or_else(|| model.herdr_workspace.preferred_pane(workspace, None))
            .map(|pane| pane.pane_id.clone())
    });
    if let Some(id) = id {
        open_pane(ui, &id);
    } else {
        ui.active.set(String::new());
        ui.navigation.show_session();
    }
}

pub(super) fn open_pane(ui: Ui, id: &str) {
    let target = ui.model.with_untracked(|model| {
        if !model.ready()
            || model.opening
            || model.herdr_grant.is_empty()
            || !model
                .herdr_workspace
                .panes
                .iter()
                .any(|pane| pane.pane_id == id)
        {
            return None;
        }
        let stream = model
            .panes
            .iter()
            .find(|pane| {
                pane.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
                    && pane.herdr_pane_id.as_deref() == Some(id)
            })
            .map(|pane| pane.stream.clone());
        Some((model.herdr_grant.clone(), stream))
    });
    let Some((grant, stream)) = target else {
        return;
    };
    ui.navigation.section.set(Section::Herdr);
    let workspace = ui.model.with_untracked(|model| {
        model
            .herdr_workspace
            .panes
            .iter()
            .find(|pane| pane.pane_id == id)
            .map(|pane| pane.workspace_id.clone())
            .unwrap_or_default()
    });
    ui.navigation.workspace.set((grant.clone(), workspace));
    if let Some(stream) = stream {
        ui.active.set(stream);
    } else {
        ui.send(Command::Request(Operation::TerminalAttach {
            grant,
            pane: Some(id.to_owned()),
        }));
    }
    ui.navigation.show_session();
}

/// Creation follows the current explicit section, never a startup default.
#[cfg(not(target_os = "android"))]
pub(super) fn creation_backend(ui: Ui) -> neko_remoted_protocol::messages::SessionBackend {
    use neko_remoted_protocol::messages::SessionBackend;
    match ui.navigation.section.get() {
        Section::Herdr => SessionBackend::Herdr,
        Section::Terminal => match ui.navigation.ordinary_backend.get() {
            SessionBackend::Hc => SessionBackend::Hc,
            SessionBackend::Native | SessionBackend::Herdr => SessionBackend::Native,
        },
    }
}

pub(super) fn local_actions(ui: Ui) -> impl IntoElement {
    #[cfg(not(target_os = "android"))]
    {
        let preferences = use_context::<crate::preferences::Ui>();
        let enabled = move || {
            preferences
                .and_then(|settings| settings.startup_backend())
                .is_some()
                && ui.model.with(|model| {
                    use neko_remoted_protocol::messages::SessionBackend;
                    model.local
                        && model.writable()
                        && !model.opening
                        && !model.host_busy
                        && match creation_backend(ui) {
                            SessionBackend::Native => true,
                            SessionBackend::Hc => model.host.as_ref().is_some_and(|host| {
                                host.hc.status == neoko_terminal_host::HcStatus::Ready
                            }),
                            SessionBackend::Herdr => model.host.as_ref().is_some_and(|host| {
                                host.herdr.status == neoko_terminal_host::HerdrStatus::Ready
                            }),
                        }
                })
        };
        when(
            move || ui.model.with(|model| model.local && model.ready()),
            move || {
                icon_action(
                    Icon::Add,
                    move || match creation_backend(ui) {
                        neko_remoted_protocol::messages::SessionBackend::Hc => "新建 hc 终端",
                        neko_remoted_protocol::messages::SessionBackend::Herdr => "新建 Herdr 会话",
                        neko_remoted_protocol::messages::SessionBackend::Native => "新建终端",
                    },
                    || false,
                    move || !enabled(),
                    move || {
                        if enabled() {
                            ui.send(Command::CreateLocalSession {
                                backend: creation_backend(ui),
                            });
                        }
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

fn local_close(
    ui: Ui,
    id: impl Fn() -> String + 'static,
    name: impl Fn() -> String + 'static,
    terminal: impl Fn() -> bool + 'static,
) -> impl IntoElement {
    #[cfg(not(target_os = "android"))]
    {
        let id = stored(id);
        let name = stored(name);
        when(move || ui.model.with(|m| m.local) && terminal(), move || {
            icon_action(Icon::Close, "结束会话…", || false, || false, move || {
                let target = id.with(|id| id());
                let label = name.with(|name| name());
                let generation = ui.model.with_untracked(|m| m.generation);
                spawn(async move {
                    let answer = nsh_base::alert_dialog().title(tr_format("结束“{}”？", &[&label]))
                        .description(tr("会关闭此电脑上的终端进程，已连接设备也将看到会话结束。"))
                        .cancel("cancel", tr("取消")).destructive("close", tr("结束会话")).show().await;
                    if matches!(answer, Ok(result) if result.action.as_deref() == Some("close"))
                        && generation == ui.model.with_untracked(|m| m.generation)
                    {
                        ui.send(Command::CloseLocalTerminal(target));
                    }
                });
            })
        }).into_element()
    }
    #[cfg(target_os = "android")]
    {
        let _ = (ui, id, name, terminal);
        view().into_element()
    }
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "resources_tests.rs"]
mod tests;
