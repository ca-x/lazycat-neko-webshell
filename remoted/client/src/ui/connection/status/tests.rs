use super::*;
use crate::{
    controller::Client,
    storage::{Binding, HostAliases, HostNames},
};
use nsh_core::{
    element::Root,
    platform::{Environment, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn binding() -> Binding {
    Binding {
        relay: neko_remoted_protocol::RelayAddress::parse("https://relay.example.com").unwrap(),
        route: neko_remoted_protocol::encode(&[1; 32]),
        host_key: neko_remoted_protocol::encode(&[2; 32]),
        device_id: "device".into(),
    }
}
fn names() -> serde_json::Value {
    let target = binding();
    serde_json::json!([{"identity": {"relay": target.relay, "route": target.route, "host_key": target.host_key}, "name": "离线主机"}])
}
#[component]
fn Fixture() -> impl IntoElement {
    let target = binding();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let ui = Ui::new(
        Model {
            phase: Phase::Ready,
            relay: Some(target.relay.clone()),
            bindings: vec![target.clone()],
            active_binding: Some(target),
            host_names: serde_json::from_value::<HostNames>(names()).unwrap(),
            remote_host_name: "真实主机.local".into(),
            connection_kind: ConnectionKind::Relay,
            ..Model::default()
        },
        Client::fixture(sender),
    );
    let requests = signal(0usize);
    spawn(async move {
        while receiver.recv().await.is_some() {
            requests.update(|value| *value += 1);
        }
    });
    view()
        .flex_1()
        .child(render(ui))
        .child(
            button()
                .on_press(move || ui.model.update(|model| model.latency_ms = Some(42)))
                .child(text("latency-sample")),
        )
        .child(
            button()
                .on_press(move || ui.model.update(|model| model.latency_ms = None))
                .child(text("latency-unavailable")),
        )
        .child(text(move || {
            format!(
                "requests={};generation={}",
                requests.get(),
                ui.model.with(|model| model.generation)
            )
        }))
        .child(
            button()
                .on_press(move || {
                    ui.model
                        .update(|model| model.connection_kind = ConnectionKind::Connecting)
                })
                .child(text("start-direct")),
        )
        .child(
            button()
                .on_press(move || {
                    ui.model
                        .update(|model| model.connection_kind = ConnectionKind::Direct)
                })
                .child(text("finish-direct")),
        )
        .child(
            button()
                .on_press(move || {
                    ui.model
                        .update(|model| model.connection_kind = ConnectionKind::Relay)
                })
                .child(text("fallback-relay")),
        )
        .child(
            button()
                .on_press(move || {
                    ui.model.update(|model| {
                        model.phase = Phase::Offline;
                        model.remote_host_name.clear();
                    })
                })
                .child(text("offline")),
        )
        .child(
            button()
                .on_press(move || {
                    ui.model.update(|model| {
                        model.local = true;
                        model.phase = Phase::Ready;
                    })
                })
                .child(text("local")),
        )
        .child(
            button()
                .on_press(move || {
                    ui.model.update(|model| {
                        model.aliases = serde_json::from_value::<HostAliases>(names()).unwrap()
                    })
                })
                .child(text("alias")),
        )
}
fn start(os: Os, locale: &str) -> Result<Headless, String> {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os,
            locale: Locale::parse(locale).unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(
            Root::of::<Fixture>()
                .tokens(nsh_base::tokens_of(Some(include_str!("../../theme.json"))).unwrap()),
        )
    })
    .map_err(|error| error.to_string())?;
    app.first_render()?;
    Ok(app)
}
#[test]
fn connection_icon_tracks_real_kind_and_touch_details_without_sending_commands()
-> Result<(), String> {
    for os in [Os::Linux, Os::Android] {
        let mut app = start(os, "zh-CN")?;
        assert!(!app.shows("Relay 中转"));
        app.press("Relay 中转")?;
        app.wait_for("连接详情")?;
        app.wait_for("真实主机.local")?;
        let content = app.screen().texts().join("\n");
        assert!(!content.contains(&binding().route) && !content.contains(&binding().host_key));
        app.press_key("esc")?;
        app.press("start-direct")?;
        app.press("连接中")?;
        app.wait_for("连接详情")?;
        app.press_key("esc")?;
        app.press("finish-direct")?;
        app.press("NAT 直连")?;
        app.wait_for("连接详情")?;
        app.wait_for("NAT 直连")?;
        app.press_key("esc")?;
        app.press("fallback-relay")?;
        assert!(app.screen().press_target("Relay 中转")?.is_some());
        app.press("offline")?;
        app.press("未连接")?;
        app.wait_for("离线主机")?;
        app.press_key("esc")?;
        app.press("local")?;
        app.press("本机连接")?;
        app.wait_for("本机")?;
        assert!(app.shows("requests=0;generation=0"));
    }
    Ok(())
}
#[test]
fn english_connection_details_keep_server_hostname_and_alias_as_user_content() -> Result<(), String>
{
    let mut app = start(Os::Linux, "en-US")?;
    app.press("Relay connection")?;
    app.wait_for("Connection details")?;
    app.wait_for("真实主机.local")?;
    app.press_key("esc")?;
    app.press("alias")?;
    app.press("Relay connection")?;
    app.wait_for("离线主机")?;
    Ok(())
}
#[test]
fn disconnected_status_never_claims_a_previous_direct_path_is_active() {
    let mut model = Model {
        phase: Phase::Offline,
        connection_kind: ConnectionKind::Direct,
        ..Model::default()
    };
    assert_eq!(Status::of(&model), Status::Offline);
    model.phase = Phase::Connecting;
    assert_eq!(Status::of(&model), Status::Connecting);
    model.phase = Phase::Ready;
    assert_eq!(Status::of(&model), Status::Direct);
    model.local = true;
    assert_eq!(Status::of(&model), Status::Local);
}

#[component]
fn WorkspaceFixture() -> impl IntoElement {
    use crate::{
        controller::{Pane, PaneKind},
        ui::state::Chunk,
    };
    let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
    let target = binding();
    let ui = Ui::new(
        Model {
            phase: Phase::Ready,
            permission: Some(neko_remoted_protocol::messages::Permission::Operate),
            bindings: vec![target.clone()],
            active_binding: Some(target),
            panes: vec![Pane {
                kind: PaneKind::Terminal,
                stream: "kept-stream".into(),
                name: "test-tab".into(),
                programs: Vec::new(),
                herdr_pane_id: None,
                herdr_grant: None,
                controller: true,
                read_only: false,
                exit: None,
                unavailable: None,
                gap: false,
            }],
            ..Model::default()
        },
        Client::fixture(sender),
    );
    spawn(async move { while receiver.recv().await.is_some() {} });
    ui.pending.with(|pending| {
        pending
            .entry("kept-stream".into())
            .or_default()
            .push_back(Chunk {
                text: "kept-output猫".into(),
                reset: false,
            })
    });
    view()
        .flex_1()
        .child(
            button()
                .on_press(move || {
                    ui.model
                        .update(|model| model.connection_kind = ConnectionKind::Direct)
                })
                .child(text("path-direct")),
        )
        .child(crate::ui::workspace::render(ui))
}

#[test]
fn path_changes_preserve_terminal_nodes_buffer_and_composer_draft() -> Result<(), String> {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os: Os::Linux,
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(
            Root::of::<WorkspaceFixture>()
                .tokens(nsh_base::tokens_of(Some(include_str!("../../theme.json"))).unwrap()),
        )
    })
    .map_err(|error| error.to_string())?;
    app.first_render()?;
    app.wait_for("kept-output猫")?;
    app.press("会话输入")?;
    app.type_text("会话输入", "保留草稿")?;
    let grid_id = app
        .screen()
        .layout_nodes()
        .iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-grid-row") && !node.hidden)
        .ok_or("no visible terminal row")?
        .id;
    app.press("path-direct")?;
    assert_eq!(app.screen().input_text("会话输入")?, Some("保留草稿"));
    app.wait_for("kept-output猫")?;
    assert_eq!(
        app.screen()
            .layout_nodes()
            .iter()
            .find(|node| node.test_id.as_deref() == Some("terminal-grid-row") && !node.hidden)
            .ok_or("terminal row disappeared after connection-kind update")?
            .id,
        grid_id
    );
    assert!(app.screen().press_target("NAT 直连")?.is_some());
    Ok(())
}

#[test]
fn connection_details_show_end_to_end_latency_in_both_languages_and_hide_stale_samples()
-> Result<(), String> {
    for (locale, trigger, unavailable, measured) in [
        (
            "zh-CN",
            "Relay 中转",
            "端到端延迟：不可用",
            "端到端延迟：42 毫秒",
        ),
        (
            "en-US",
            "Relay connection",
            "End-to-end latency: unavailable",
            "End-to-end latency: 42 ms",
        ),
    ] {
        for os in [Os::Linux, Os::Android] {
            let mut app = start(os, locale)?;
            app.press(trigger)?;
            app.wait_for(unavailable)?;
            app.press_key("esc")?;
            app.press("latency-sample")?;
            app.press(trigger)?;
            app.wait_for(measured)?;
            app.press_key("esc")?;
            app.press("latency-unavailable")?;
            app.press(trigger)?;
            app.wait_for(unavailable)?;
            app.press_key("esc")?;
            app.press("latency-sample")?;
            app.press("offline")?;
            app.press(if locale == "zh-CN" {
                "未连接"
            } else {
                "Not connected"
            })?;
            app.wait_for(unavailable)?;
            app.press_key("esc")?;
            app.press("local")?;
            app.press(if locale == "zh-CN" {
                "本机连接"
            } else {
                "Local connection"
            })?;
            app.wait_for(unavailable)?;
            assert!(app.shows("requests=0;generation=0"));
        }
    }
    Ok(())
}
