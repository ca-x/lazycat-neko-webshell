//! Availability and explicit connection of the local, isolated hc backend.
use crate::ui::{
    controls::{action, control_size, hint, icon_action, icons::Icon},
    state::Ui,
};
use crate::{controller::Command, preferences::tr};
use neko_remoted_protocol::messages::SessionBackend;
use neoko_terminal_host::{Action, HcStatus};
use nsh_base::{link, theme};
use nsh_core::prelude::*;

fn status(ui: Ui) -> Option<HcStatus> {
    ui.model
        .with(|model| model.host.as_ref().map(|host| host.hc.status))
}

pub fn render(ui: Ui) -> impl IntoElement {
    let preferences = use_context::<crate::preferences::Ui>();
    let pending = signal(None::<(u64, bool)>);
    effect(move || {
        if let Some((revision, _)) = pending.get() {
            let finished = ui.model.with(|model| {
                !model.host_busy
                    && model
                        .host
                        .as_ref()
                        .is_none_or(|host| host.revision != revision)
            });
            if finished {
                pending.set(None);
            }
        }
    });
    let busy = move || pending.get().is_some() || ui.model.with(|model| model.host_busy);
    when(
        move || {
            ui.model.with(|model| model.local)
                && preferences.and_then(|settings| settings.new_session_backend())
                    == Some(SessionBackend::Hc)
        },
        move || {
            view()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .child(
                            text(move || tr("本机 hc"))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .flex_1(),
                        )
                        .child(icon_action(
                            Icon::Refresh,
                            "刷新 hc 状态",
                            || false,
                            move || status(ui).is_none() || busy(),
                            move || run(ui, pending, false),
                        )),
                )
                .child(hint(move || {
                    tr(if let Some((_, connect)) = pending.get() {
                        if connect {
                            "正在连接 hc…"
                        } else {
                            "正在检测 hc…"
                        }
                    } else {
                        match status(ui) {
                            None => "hc 状态暂不可用。",
                            Some(HcStatus::Stopped) => "hc 尚未连接。请检测安装状态后连接。",
                            Some(HcStatus::NotInstalled) => {
                                "未安装 hc。请按官方说明安装，然后刷新状态。"
                            }
                            Some(HcStatus::Ready) => "hc 已连接，可创建持久化终端。",
                            Some(HcStatus::Error) => "hc 连接失败，请查看错误后重试。",
                        }
                    })
                }))
                .child(when(
                    move || status(ui) == Some(HcStatus::Error),
                    move || {
                        text(move || {
                            ui.model.with(|model| {
                                model
                                    .host
                                    .as_ref()
                                    .map(|host| host.hc.error.clone())
                                    .unwrap_or_default()
                            })
                        })
                        .text_sm()
                        .select_text()
                        .text_color(move || theme().destructive)
                        .role(AccessibilityRole::Alert)
                    },
                ))
                .child(when(
                    move || matches!(status(ui), Some(HcStatus::Stopped | HcStatus::Error)),
                    move || {
                        action(
                            "连接 hc",
                            true,
                            move || status(ui).is_none() || busy(),
                            move || run(ui, pending, true),
                        )
                    },
                ))
                .child(when(
                    move || matches!(status(ui), Some(HcStatus::NotInstalled | HcStatus::Error)),
                    move || {
                        link()
                            .href(move || {
                                ui.model.with(|model| {
                                    model
                                        .host
                                        .as_ref()
                                        .map(|host| SharedString::from(host.hc.install_url.clone()))
                                })
                            })
                            .min_h(control_size)
                            .items_center()
                            .justify_center()
                            .px_2()
                            .rounded(move || theme().radius)
                            .bg(move || theme().secondary)
                            .focus(|style| {
                                style.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .hover(|style| style.bg(move || theme().accent))
                            .active(|style| style.opacity(0.75))
                            .child(
                                text(move || tr("hc 官方安装指南"))
                                    .text_sm()
                                    .text_color(move || theme().foreground),
                            )
                    },
                ))
        },
    )
}

fn run(ui: Ui, pending: Signal<Option<(u64, bool)>>, connect: bool) {
    if pending.get_untracked().is_some() || ui.model.with_untracked(|model| model.host_busy) {
        return;
    }
    if let Some(revision) = ui
        .model
        .with_untracked(|model| model.host.as_ref().map(|host| host.revision))
    {
        pending.set(Some((revision, connect)));
        let action = if connect {
            Action::ConnectHc { revision }
        } else {
            Action::RefreshHc { revision }
        };
        if !ui.submit(Command::Host(action)) {
            pending.set(None);
        }
    }
}

#[cfg(test)]
#[path = "local_hc_tests.rs"]
mod tests;
