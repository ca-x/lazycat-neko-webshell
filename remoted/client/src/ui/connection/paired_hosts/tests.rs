use super::*;
use crate::{
    controller::{Client, Model},
    storage::{Binding, HostAliases},
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
        device_id: "paired-device".into(),
    }
}

fn aliases(binding: &Binding, name: &str) -> HostAliases {
    if name.trim().is_empty() {
        return HostAliases::default();
    }
    serde_json::from_value(serde_json::json!([{
        "identity": { "relay": binding.relay, "route": binding.route, "host_key": binding.host_key },
        "name": name.trim(),
    }]))
    .unwrap()
}

fn fixture(phase: Phase, sidebar: bool) -> impl IntoElement {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let ui = Ui::new(
        Model {
            phase,
            bindings: vec![binding()],
            ..Model::default()
        },
        Client::fixture(sender),
    );
    let pending = signal(None::<(Binding, String)>);
    let status = signal(String::new());
    spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                Command::RenameBinding {
                    binding: target,
                    alias,
                } => {
                    assert_eq!(target, binding());
                    ui.model
                        .update(|model| model.alias_saving = Some(target.clone()));
                    pending.set(Some((target, alias)));
                    status.set("已收到改名".into());
                }
                _ => status.set("意外连接操作".into()),
            }
        }
    });
    view()
        .flex_1()
        .child(if sidebar {
            crate::ui::sidebar::render(ui).into_element()
        } else {
            super::super::render(ui).into_element()
        })
        .child(text(move || status.get()))
        .child(when(
            move || sidebar,
            move || {
                action(
                    "测试侧栏改名",
                    false,
                    || false,
                    move || {
                        ui.send(Command::RenameBinding {
                            binding: binding(),
                            alias: "开发主机".into(),
                        });
                    },
                )
            },
        ))
        .child(action(
            "测试确认写入",
            false,
            move || pending.get().is_none(),
            move || {
                let (target, name) = pending.get_untracked().unwrap();
                ui.model.update(|model| {
                    model.aliases = aliases(&target, &name);
                    model.error.clear();
                    model.alias_saving = None;
                });
                pending.set(None);
                status.set("已确认写入".into());
            },
        ))
        .child(action(
            "测试写入失败",
            false,
            move || pending.get().is_none(),
            move || {
                ui.error("主机别名未能确认保存，请重试；仍显示上次确认的名称。".into());
                ui.model.update(|model| model.alias_saving = None);
                pending.set(None);
                status.set("写入未确认".into());
            },
        ))
}

#[component]
fn OfflineFixture() -> impl IntoElement {
    fixture(Phase::Offline, false)
}

#[component]
fn ReadyFixture() -> impl IntoElement {
    fixture(Phase::Ready, false)
}

#[component]
fn SidebarFixture() -> impl IntoElement {
    fixture(Phase::Offline, true)
}

fn start(root: Root, locale: &str) -> Headless {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os: Os::Android,
            locale: Locale::parse(locale).unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(root)
    })
    .unwrap();
    app.first_render().unwrap();
    app
}

fn fallback() -> String {
    let key = neko_remoted_protocol::decode32(&binding().host_key).unwrap();
    format!(
        "远程主机 · {}",
        neko_remoted_protocol::fingerprint(&key)
            .chars()
            .take(12)
            .collect::<String>()
    )
}

#[test]
fn rename_offline_and_ready_wait_for_confirmed_save_without_connecting() -> Result<(), String> {
    for root in [Root::of::<OfflineFixture>(), Root::of::<ReadyFixture>()] {
        let mut app = start(root, "zh-CN");
        app.wait_for("主机别名")?;
        assert!(app.shows(&fallback()));
        let visible = app.screen().texts().join("\n");
        assert!(!visible.contains(&binding().host_key));
        assert!(!visible.contains(&binding().route));
        assert!(!visible.contains("主机 1"));
        app.type_text("主机别名", "开发主机 🐈")?;
        assert!(app.shows(&fallback()));
        app.press("保存别名")?;
        app.wait_for("已收到改名")?;
        assert!(app.shows("正在保存别名…"));
        assert!(app.shows(&fallback()));
        assert!(!app.shows("意外连接操作"));
        app.press("测试确认写入")?;
        app.wait_for("已确认写入")?;
        assert!(!app.shows(&fallback()));
        assert!(app.shows("开发主机 🐈"));
        assert_eq!(app.screen().input_text("主机别名")?, Some("开发主机 🐈"));
        assert!(!app.shows("意外连接操作"));
    }
    Ok(())
}

#[test]
fn rename_save_failure_keeps_draft_and_last_confirmed_name() -> Result<(), String> {
    let mut app = start(Root::of::<OfflineFixture>(), "zh-CN");
    app.type_text("主机别名", "开发主机")?;
    app.press("保存别名")?;
    app.wait_for("已收到改名")?;
    app.press("测试确认写入")?;
    app.wait_for("已确认写入")?;
    app.clear("主机别名")?;
    app.type_text("主机别名", "尚未保存")?;
    app.press("保存别名")?;
    app.wait_for("已收到改名")?;
    app.press("测试写入失败")?;
    app.wait_for("写入未确认")?;
    assert!(app.shows("开发主机"));
    assert!(app.shows("主机别名未能确认保存，请重试；仍显示上次确认的名称。"));
    assert_eq!(app.screen().input_text("主机别名")?, Some("尚未保存"));
    assert!(!app.shows("意外连接操作"));
    Ok(())
}

#[test]
fn rename_clear_restores_fingerprint_only_after_save_confirmation() -> Result<(), String> {
    let mut app = start(Root::of::<OfflineFixture>(), "zh-CN");
    app.type_text("主机别名", "开发主机")?;
    app.press("保存别名")?;
    app.wait_for("已收到改名")?;
    app.press("测试确认写入")?;
    app.wait_for("已确认写入")?;
    app.clear("主机别名")?;
    app.press("保存别名")?;
    app.wait_for("已收到改名")?;
    assert!(app.shows("开发主机"));
    assert!(!app.shows(&fallback()));
    app.press("测试确认写入")?;
    app.wait_for("已确认写入")?;
    assert!(app.shows(&fallback()));
    assert_eq!(app.screen().input_text("主机别名")?, Some(""));
    Ok(())
}

#[test]
fn rename_english_ui_keeps_chinese_alias_as_user_text() -> Result<(), String> {
    let mut app = start(Root::of::<OfflineFixture>(), "en-US");
    app.wait_for("Host alias")?;
    app.type_text("Host alias", "设置")?;
    app.press("Save alias")?;
    app.wait_for("已收到改名")?;
    app.press("测试确认写入")?;
    app.wait_for("已确认写入")?;
    assert!(app.shows("设置"));
    assert_eq!(app.screen().input_text("Host alias")?, Some("设置"));
    Ok(())
}

#[test]
fn rename_sidebar_updates_only_after_confirmation_and_never_shows_raw_identity()
-> Result<(), String> {
    let mut app = start(Root::of::<SidebarFixture>(), "zh-CN");
    app.wait_for(&fallback())?;
    assert!(!app.shows("主机 1"));
    app.press("测试侧栏改名")?;
    app.wait_for("已收到改名")?;
    assert!(app.shows(&fallback()));
    assert!(!app.shows("开发主机"));
    app.press("测试确认写入")?;
    app.wait_for("开发主机")?;
    assert!(!app.shows(&fallback()));
    let visible = app.screen().texts().join("\n");
    assert!(!visible.contains(&binding().host_key));
    assert!(!visible.contains(&binding().route));
    assert!(!app.shows("意外连接操作"));
    Ok(())
}
