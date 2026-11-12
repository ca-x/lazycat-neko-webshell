use super::*;
use crate::{
    controller::{Client, Model, Phase},
    preferences::{Language, Preferences, catalog::Catalog},
};
use neoko_terminal_host::{HcSnapshot, HerdrSnapshot, HostSnapshot};
use nsh_core::{
    element::Root,
    platform::{Environment, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[component]
fn Fixture() -> impl IntoElement {
    let mut values = Preferences::defaults(Catalog::shared().unwrap());
    values.default_backend = SessionBackend::Hc;
    values.language = Language::ZhCn;
    crate::preferences::Ui::from_loaded(values).unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let ui = Ui::new(
        Model {
            local: true,
            phase: Phase::Ready,
            host: Some(HostSnapshot {
                revision: 1,
                hc: HcSnapshot::default(),
                herdr: HerdrSnapshot::default(),
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
    // The same guidance must accompany the default-HC creation action here too.
    ui.navigation
        .section
        .set(crate::ui::navigation::Section::Herdr);
    let count = signal(0);
    spawn(async move {
        while receiver.recv().await.is_some() {
            count.update(|count| *count += 1);
        }
    });
    view()
        .flex_1()
        .child(render(ui))
        .child(text(move || format!("commands {}", count.get())))
        .child(crate::ui::controls::action(
            "fixture reply",
            false,
            || false,
            move || {
                ui.model.update(|model| {
                    let host = model.host.as_mut().unwrap();
                    host.revision += 1;
                    host.hc.status = HcStatus::Ready;
                });
            },
        ))
}

#[test]
fn hc_guidance_follows_the_default_and_waits_for_action_completion() -> Result<(), String> {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os: Os::Linux,
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<Fixture>())
    })
    .map_err(|error| error.to_string())?;
    app.wait_for("hc 尚未连接。请检测安装状态后连接。")?;
    app.press("连接 hc")?;
    app.wait_for("commands 1")?;
    app.wait_for("正在连接 hc…")?;
    assert!(app.screen().press_target("连接 hc")?.is_none());
    assert!(app.screen().press_target("刷新 hc 状态")?.is_none());
    app.press("fixture reply")?;
    app.wait_for("hc 已连接，可创建持久化终端。")?;
    app.press("刷新 hc 状态")?;
    app.wait_for("commands 2")?;
    app.wait_for("正在检测 hc…")?;
    Ok(())
}
