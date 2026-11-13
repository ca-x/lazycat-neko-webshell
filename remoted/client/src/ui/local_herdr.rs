use crate::{
    controller::Command,
    preferences::tr,
    ui::{
        controls::{action, hint, icon_action, icons::Icon},
        state::Ui,
    },
};
use neoko_terminal_host::{Action, HerdrStatus};
use nsh_base::{link, theme};
use nsh_core::prelude::*;

fn status(ui: Ui) -> Option<HerdrStatus> {
    ui.model
        .with(|model| model.host.as_ref().map(|host| host.herdr.status))
}

pub fn render(ui: Ui) -> impl IntoElement {
    when(
        move || {
            ui.model.with(|model| model.local)
                && ui.navigation.section.get() == super::Section::Herdr
        },
        move || {
            view()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(move || tr("本机 Herdr"))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .flex_1(),
                        )
                        .child(icon_action(
                            Icon::Refresh,
                            "刷新 Herdr 状态",
                            || false,
                            move || matches!(status(ui), None | Some(HerdrStatus::Checking)),
                            move || {
                                if let Some(revision) = ui.model.with_untracked(|model| {
                                    model
                                        .host
                                        .as_ref()
                                        .filter(|_| model.local)
                                        .map(|host| host.revision)
                                }) {
                                    ui.send(Command::Host(Action::RefreshHerdr { revision }));
                                }
                            },
                        )),
                )
                .child(hint(move || {
                    tr(match status(ui) {
                        None => "本机 Herdr 状态暂不可用。",
                        Some(HerdrStatus::Checking) => "正在检测本机 Herdr…",
                        Some(HerdrStatus::NotInstalled) => {
                            "未安装 Herdr。请按官方说明安装，然后刷新状态。"
                        }
                        Some(HerdrStatus::Stopped) => "Herdr 已安装，服务尚未启动。",
                        Some(HerdrStatus::Ready) => "Herdr 已就绪，可打开下方工作区。",
                        Some(HerdrStatus::Error) => "Herdr 检测失败，请查看错误后重试。",
                    })
                }))
                .child(when(
                    move || status(ui) == Some(HerdrStatus::Error),
                    move || {
                        text(move || {
                            ui.model.with(|model| {
                                model
                                    .host
                                    .as_ref()
                                    .map(|host| host.herdr.error.clone())
                                    .unwrap_or_default()
                            })
                        })
                        .text_sm()
                        .text_color(move || theme().destructive)
                        .role(AccessibilityRole::Alert)
                    },
                ))
                .child(when(
                    move || status(ui) == Some(HerdrStatus::Stopped),
                    move || {
                        action(
                            "启动 Herdr",
                            true,
                            move || status(ui) != Some(HerdrStatus::Stopped),
                            move || {
                                if let Some(revision) = ui.model.with_untracked(|model| {
                                    model
                                        .host
                                        .as_ref()
                                        .filter(|host| {
                                            model.local && host.herdr.status == HerdrStatus::Stopped
                                        })
                                        .map(|host| host.revision)
                                }) {
                                    ui.send(Command::Host(Action::StartHerdr { revision }));
                                }
                            },
                        )
                    },
                ))
                .child(when(
                    move || {
                        matches!(
                            status(ui),
                            Some(HerdrStatus::NotInstalled | HerdrStatus::Error)
                        )
                    },
                    move || {
                        view()
                            .gap_2()
                            .child(
                                link()
                                    .href(move || {
                                        ui.model.with(|model| {
                                            model.host.as_ref().map(|host| {
                                                SharedString::from(host.herdr.install_url.clone())
                                            })
                                        })
                                    })
                                    .min_h(super::super::controls::control_size)
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
                                        text(move || tr("Herdr 官方安装指南"))
                                            .text_sm()
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(hint(move || tr("安装命令仅供参考，请自行在本机终端执行。")))
                            .child(
                                text(move || {
                                    ui.model.with(|model| {
                                        model
                                            .host
                                            .as_ref()
                                            .map(|host| host.herdr.install_command.clone())
                                            .unwrap_or_default()
                                    })
                                })
                                .text_xs(),
                            )
                    },
                ))
        },
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::controller::{Client, Model};
    use neoko_terminal_host::{HerdrSnapshot, HostSnapshot};
    use nsh_core::{
        element::Root,
        platform::{Environment, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    #[component]
    fn Fixture() -> impl IntoElement {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
        let ui = Ui::new(
            Model {
                local: true,
                host: Some(HostSnapshot {
                    revision: 1,
                    hc: neoko_terminal_host::HcSnapshot::default(),
                    herdr: HerdrSnapshot {
                        status: HerdrStatus::NotInstalled,
                        ..HerdrSnapshot::default()
                    },
                    enabled: false,
                    relay: String::new(),
                    status: String::new(),
                    error: String::new(),
                    invitation: None,
                    pending: Vec::new(),
                    devices: Vec::new(),
                    resources: Vec::new(),
                }),
                ..Model::default()
            },
            Client::fixture(sender),
        );
        ui.navigation.section.set(super::super::Section::Herdr);
        let last = signal(String::new());
        spawn(async move {
            while let Some(command) = receiver.recv().await {
                last.set(format!("{command:?}"));
            }
        });
        let mut content = view()
            .gap_2()
            .child(render(ui))
            .child(text(move || last.get()))
            .child(
                nsh_base::button()
                    .on_press(|| set_locale(Some(Locale::EN_US)))
                    .child(text("fixture english")),
            )
            .child(
                nsh_base::button()
                    .on_press(|| set_locale(Locale::parse("zh-CN")))
                    .child(text("fixture chinese")),
            );
        for (label, next) in [
            ("fixture stopped", HerdrStatus::Stopped),
            ("fixture ready", HerdrStatus::Ready),
            ("fixture error", HerdrStatus::Error),
            ("fixture checking", HerdrStatus::Checking),
        ] {
            content = content.child(
                nsh_base::button()
                    .on_press(move || {
                        ui.model.update(|model| {
                            let host = model.host.as_mut().unwrap();
                            host.revision += 1;
                            host.herdr.status = next;
                            host.herdr.error = "fixture discovery failed".into();
                        });
                    })
                    .child(text(label)),
            );
        }
        content.child(
            nsh_base::button()
                .on_press(move || {
                    ui.model.update(|model| model.local = false);
                })
                .child(text("fixture remote")),
        )
    }

    #[test]
    fn local_states_offer_explicit_actions_with_current_revision() -> Result<(), String> {
        let mut app = Headless::start(EngineConfig {
            environment: Environment {
                os: Os::Linux,
                locale: Locale::parse("zh-CN").unwrap(),
                ..Environment::default()
            },
            ..EngineConfig::new(Root::of::<Fixture>())
        })
        .map_err(|error| error.to_string())?;
        app.wait_for("未安装 Herdr。请按官方说明安装，然后刷新状态。")?;
        app.wait_for("Herdr 官方安装指南")?;
        app.wait_for("curl -fsSL https://herdr.dev/install.sh | sh")?;
        assert!(!app.shows("启动 Herdr"));
        app.press("fixture english")?;
        app.wait_for("Local Herdr")?;
        app.wait_for("Official Herdr installation guide")?;
        assert!(app.screen().press_target("Refresh Herdr status")?.is_some());
        assert!(!app.shows("Herdr 官方安装指南"));
        app.press("fixture chinese")?;
        app.wait_for("Herdr 官方安装指南")?;
        app.press("刷新 Herdr 状态")?;
        app.wait_for("Host(RefreshHerdr { revision: 1 })")?;
        app.press("fixture stopped")?;
        app.wait_for("Herdr 已安装，服务尚未启动。")?;
        assert!(!app.shows("Herdr 官方安装指南"));
        app.press("启动 Herdr")?;
        app.wait_for("Host(StartHerdr { revision: 2 })")?;
        app.press("fixture ready")?;
        app.wait_for("Herdr 已就绪，可打开下方工作区。")?;
        assert!(!app.shows("启动 Herdr"));
        app.press("fixture error")?;
        app.wait_for("Herdr 检测失败，请查看错误后重试。")?;
        app.wait_for("fixture discovery failed")?;
        app.wait_for("Herdr 官方安装指南")?;
        app.press("刷新 Herdr 状态")?;
        app.wait_for("Host(RefreshHerdr { revision: 4 })")?;
        app.press("fixture checking")?;
        app.wait_for("正在检测本机 Herdr…")?;
        assert!(!app.shows("Herdr 官方安装指南"));
        app.press("fixture remote")?;
        assert!(!app.shows("本机 Herdr"));
        Ok(())
    }
}
