use super::*;
use crate::{
    controller::{Client, Model, Phase},
    preferences::{Preferences, catalog::Catalog},
};
use neko_remoted_protocol::messages::{Permission, SessionBackend};
use neoko_terminal_host::{HcSnapshot, HcStatus, HerdrSnapshot, HerdrStatus, HostSnapshot};
use nsh_core::{
    element::Root,
    platform::{Environment, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn fixture(backend: SessionBackend, local: bool, section: Section) -> impl IntoElement {
    let mut values = Preferences::defaults(Catalog::shared().unwrap());
    values.default_backend = backend;
    values.language = crate::preferences::Language::ZhCn;
    crate::preferences::Ui::from_loaded(values).unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let hc = HcSnapshot {
        status: HcStatus::Ready,
        ..HcSnapshot::default()
    };
    let herdr = HerdrSnapshot {
        status: HerdrStatus::Ready,
        ..HerdrSnapshot::default()
    };
    let ui = Ui::new(
        Model {
            local,
            phase: Phase::Ready,
            permission: Some(Permission::Operate),
            host: Some(HostSnapshot {
                revision: 1,
                hc,
                herdr,
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
    ui.navigation.section.set(section);
    ui.navigation
        .ordinary_backend
        .set(if backend == SessionBackend::Hc {
            SessionBackend::Hc
        } else {
            SessionBackend::Native
        });
    let command = signal(String::new());
    spawn(async move {
        if let Some(value) = receiver.recv().await {
            command.set(format!("{value:?}"));
        }
    });
    view()
        .flex_1()
        .w_full()
        .child(local_actions(ui))
        .child(text(move || command.get()))
}

#[component]
fn NativeFixture() -> impl IntoElement {
    fixture(SessionBackend::Native, true, Section::Terminal)
}

#[component]
fn HcFixture() -> impl IntoElement {
    fixture(SessionBackend::Hc, true, Section::Terminal)
}

#[component]
fn HerdrFixture() -> impl IntoElement {
    fixture(SessionBackend::Herdr, true, Section::Herdr)
}

#[component]
fn RemoteFixture() -> impl IntoElement {
    fixture(SessionBackend::Hc, false, Section::Terminal)
}

#[component]
fn DefaultHerdrOrdinaryFixture() -> impl IntoElement {
    fixture(SessionBackend::Herdr, true, Section::Terminal)
}

fn configuration(root: Root) -> EngineConfig {
    EngineConfig {
        environment: Environment {
            os: Os::Linux,
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(root)
    }
}

#[test]
fn new_session_action_uses_current_section_and_keeps_explicit_hc() -> Result<(), String> {
    for (root, label, expected) in [
        (Root::of::<NativeFixture>(), "新建终端", "Native"),
        (Root::of::<HcFixture>(), "新建 hc 终端", "Hc"),
        (Root::of::<HerdrFixture>(), "新建 Herdr 会话", "Herdr"),
        (
            Root::of::<DefaultHerdrOrdinaryFixture>(),
            "新建终端",
            "Native",
        ),
    ] {
        let mut app = Headless::start(configuration(root)).map_err(|error| error.to_string())?;
        app.wait_for(label)?;
        app.press(label)?;
        app.wait_for(&format!("CreateLocalSession {{ backend: {expected} }}"))?;
    }
    Ok(())
}

#[test]
fn remote_workspace_does_not_create_sessions_using_the_local_default() -> Result<(), String> {
    let app = Headless::start(configuration(Root::of::<RemoteFixture>()))
        .map_err(|error| error.to_string())?;
    assert!(app.screen().press_target("新建 hc 终端")?.is_none());
    Ok(())
}
