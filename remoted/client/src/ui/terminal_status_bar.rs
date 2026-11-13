//! Read-only terminal status derived from the current host and attachment.

use crate::{
    controller::{Model, Pane},
    preferences::tr,
    ui::{
        connection::{host_names, status},
        state::Ui,
        terminal_view::interaction,
    },
};
use neko_remoted_protocol::messages::ConnectionKind;
use nsh_base::theme;
use nsh_core::{element::ImageSrc, prelude::*};

fn pane_status(model: &Model, pane: &Pane) -> SharedString {
    tr(if pane.unavailable.is_some() {
        "会话连接已中断。"
    } else if pane.exit.is_some() {
        "会话已退出。"
    } else if !model.writable() || pane.read_only {
        "只读浏览"
    } else if interaction::input_allowed(model, pane) {
        "可输入"
    } else {
        "取得控制权后即可输入。"
    })
}

/// A terminal-scoped status strip; measured RTT always refers to the selected remote host.
pub(super) fn render(ui: Ui) -> impl IntoElement {
    view()
        .test_id("terminal-status-bar")
        .min_h(px(24.0))
        .px_2()
        .py_1()
        .flex_row()
        .items_center()
        .gap_2()
        .flex_shrink(0.0)
        .border_t_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .text_color(move || theme().muted_foreground)
        .child(
            text(move || ui.model.with(host_names::active_name))
                .text_xs()
                .number_of_lines(1)
                .max_w(px(160.0)),
        )
        .child(text(move || ui.model.with(status::connection_label)).text_xs())
        .child(when(
            move || {
                ui.model.with(|model| {
                    model.ready() && !model.local && model.connection_kind != ConnectionKind::Local
                })
            },
            move || {
                view()
                    .test_id("terminal-status-latency")
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .tooltip(crate::ui::controls::tooltip(move || {
                        ui.model.with(status::latency_label)
                    }))
                    .label(move || ui.model.with(status::latency_label))
                    .child(
                        svg(ImageSrc::svg_markup(include_str!("icons/Signal.svg")))
                            .w(px(16.0))
                            .h(px(16.0)),
                    )
                    .child(
                        text(move || {
                            ui.model.with(|model| {
                                model
                                    .latency_ms
                                    .map_or_else(|| "—".into(), |ms| format!("{ms} ms"))
                            })
                        })
                        .text_xs(),
                    )
            },
        ))
        .child(view().flex_1())
        .child(
            text(move || {
                ui.model.with(|model| {
                    model
                        .panes
                        .iter()
                        .find(|pane| pane.stream == ui.active.get())
                        .map(|pane| pane_status(model, pane))
                        .unwrap_or_default()
                })
            })
            .text_xs()
            .number_of_lines(1)
            .flex_shrink(1.0),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        controller::{Client, Phase},
        storage::Binding,
    };
    use nsh_core::{
        element::Root,
        platform::{Environment, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;

    #[component]
    fn Fixture() -> impl IntoElement {
        let (sender, _receiver) = tokio::sync::mpsc::channel(8);
        let binding = Binding {
            relay: neko_remoted_protocol::RelayAddress::parse("https://relay.example.com").unwrap(),
            route: "route".into(),
            host_key: neko_remoted_protocol::encode(&[2; 32]),
            device_id: "device".into(),
        };
        let ui = Ui::new(
            Model {
                phase: Phase::Ready,
                active_binding: Some(binding),
                remote_host_name: "host.local".into(),
                latency_ms: Some(42),
                connection_kind: ConnectionKind::Direct,
                ..Model::default()
            },
            Client::fixture(sender),
        );
        view()
            .child(render(ui))
            .child(
                nsh_base::button()
                    .on_press(move || ui.model.update(|model| model.phase = Phase::Offline))
                    .child(text("fixture-offline")),
            )
            .child(
                nsh_base::button()
                    .on_press(move || {
                        ui.model.update(|model| {
                            model.local = true;
                            model.phase = Phase::Ready;
                        })
                    })
                    .child(text("fixture-local")),
            )
    }

    #[test]
    fn measured_rtt_is_visible_only_for_a_live_remote_host() -> Result<(), String> {
        let mut app = Headless::start(EngineConfig {
            environment: Environment {
                os: Os::Linux,
                locale: Locale::parse("zh-CN").unwrap(),
                ..Environment::default()
            },
            ..EngineConfig::new(Root::of::<Fixture>())
        })
        .map_err(|error| error.to_string())?;
        app.wait_for("42 ms")?;
        app.wait_for("NAT 直连")?;
        app.press("fixture-offline")?;
        assert!(!app.shows("42 ms"));
        app.wait_for("未连接")?;
        app.press("fixture-local")?;
        app.wait_for("本机连接")?;
        assert!(!app.shows("42 ms"));
        Ok(())
    }
}
