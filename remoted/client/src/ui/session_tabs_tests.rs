use super::*;
use crate::{
    controller::{Client, Pane, Phase},
    preferences::{Language, Preferences, catalog::Catalog},
    ui::{controls, herdr},
};
use neko_remoted_protocol::messages::{Permission, Resource, SessionBackend};
use neoko_terminal_host::{HcSnapshot, HcStatus, HerdrSnapshot, HerdrStatus, HostSnapshot};
use nsh_core::{
    element::Root,
    platform::{Environment, Idiom, Locale, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn pane(id: &str) -> Pane {
    Pane {
        stream: id.into(),
        name: id.into(),
        kind: PaneKind::Terminal,
        herdr_grant: None,
        herdr_pane_id: None,
        programs: Vec::new(),
        controller: false,
        read_only: false,
        exit: None,
        unavailable: None,
        gap: false,
    }
}

fn fixture(local: bool, backend: SessionBackend) -> impl IntoElement {
    let mut settings = Preferences::defaults(Catalog::shared().unwrap());
    settings.language = Language::ZhCn;
    settings.default_backend = backend;
    crate::preferences::Ui::from_loaded(settings).unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(16);
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
            resources: if local {
                Vec::new()
            } else {
                [
                    ("approved-herdr", "Granted Herdr", "herdr", true),
                    ("hc", "Unsupported HC", "hc", false),
                    ("microvm", "Unsupported MicroVM", "microvm", false),
                    ("ticket", "Unsupported client", "client-ticket", false),
                ]
                .into_iter()
                .map(|(id, name, backend, herdr)| Resource {
                    id: id.into(),
                    name: name.into(),
                    backend: backend.into(),
                    terminal: !herdr,
                    herdr,
                    files: false,
                })
                .collect()
            },
            panes: vec![pane("first"), pane("second")],
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
    ui.navigation.sidebar.set(false);
    ui.navigation
        .section
        .set(if backend == SessionBackend::Herdr {
            Section::Herdr
        } else {
            Section::Terminal
        });
    ui.navigation
        .ordinary_backend
        .set(if backend == SessionBackend::Hc {
            SessionBackend::Hc
        } else {
            SessionBackend::Native
        });
    let log = signal(String::new());
    spawn(async move {
        while let Some(command) = receiver.recv().await {
            log.set(format!("{command:?}"));
        }
    });
    view()
        .flex_1()
        .child(render(ui))
        .child(text(move || log.get()))
        .child(text(move || {
            format!(
                "active={} sidebar={} drawer={}",
                ui.active.get(),
                ui.navigation.sidebar.get(),
                ui.navigation.drawer.get()
            )
        }))
        .child(controls::action(
            "fixture-backend-unavailable",
            false,
            || false,
            move || {
                ui.model
                    .update(|model| model.host.as_mut().unwrap().hc.status = HcStatus::Stopped);
            },
        ))
        .child(controls::action(
            "fixture-ordinary",
            false,
            || false,
            move || ui.navigation.section.set(Section::Terminal),
        ))
        .child(controls::action(
            "fixture-readonly",
            false,
            || false,
            move || {
                ui.model
                    .update(|model| model.permission = Some(Permission::ReadOnly))
            },
        ))
        .child(controls::action(
            "fixture-no-parents",
            false,
            || false,
            move || ui.model.update(|model| model.resources.clear()),
        ))
        .child(controls::action(
            "fixture-webshell-only",
            false,
            || false,
            move || {
                ui.model.update(|model| {
                    model.resources = vec![Resource {
                        id: "approved-provider".into(),
                        name: "Granted provider".into(),
                        backend: "webshell".into(),
                        terminal: true,
                        herdr: false,
                        files: false,
                    }]
                });
            },
        ))
}

#[component]
fn NativeFixture() -> impl IntoElement {
    fixture(true, SessionBackend::Native)
}
#[component]
fn HcFixture() -> impl IntoElement {
    fixture(true, SessionBackend::Hc)
}
#[component]
fn HerdrFixture() -> impl IntoElement {
    fixture(true, SessionBackend::Herdr)
}
#[component]
fn RemoteFixture() -> impl IntoElement {
    fixture(false, SessionBackend::Native)
}

fn start(root: Root, width: f32) -> Result<Headless, String> {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(width), px(700.0)),
        environment: Environment {
            os: Os::Linux,
            idiom: Idiom::Desktop,
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(root)
    })
    .map_err(|error| error.to_string())?;
    app.first_render()?;
    Ok(app)
}

#[test]
fn tab_close_detaches_the_named_stream_without_selecting_or_ending_it() -> Result<(), String> {
    let mut app = start(Root::of::<NativeFixture>(), 900.0)?;
    app.press("关闭此窗格连接 · first")?;
    app.wait_for("Request(TerminalDetach { stream: \"first\" })")?;
    assert!(app.shows("active=second sidebar=false drawer=false"));
    app.right_click("second")?;
    app.press("关闭此窗格连接")?;
    app.wait_until(
        "both attachments detached",
        std::time::Duration::from_secs(2),
        |screen| {
            screen
                .texts()
                .join("\n")
                .contains("TerminalDetach { stream: \"second\" }")
        },
    )?;
    assert!(
        !app.screen()
            .texts()
            .join("\n")
            .contains("CloseLocalTerminal")
    );
    Ok(())
}

#[test]
fn plus_uses_current_section_and_respects_ordinary_backend_readiness() -> Result<(), String> {
    for (root, label, backend) in [
        (Root::of::<NativeFixture>(), "新建终端", "Native"),
        (Root::of::<HcFixture>(), "新建 hc 终端", "Hc"),
        (Root::of::<HerdrFixture>(), "新建 Herdr 会话", "Herdr"),
    ] {
        let mut app = start(root, 900.0)?;
        app.press(label)?;
        app.wait_for(&format!("CreateLocalSession {{ backend: {backend} }}"))?;
    }
    let mut app = start(Root::of::<HcFixture>(), 900.0)?;
    app.press("fixture-backend-unavailable")?;
    assert!(!app.screen().enabled("新建 hc 终端")?);
    let mut app = start(Root::of::<HerdrFixture>(), 900.0)?;
    app.press("fixture-ordinary")?;
    app.press("新建终端")?;
    app.wait_for("CreateLocalSession { backend: Native }")?;
    Ok(())
}

#[test]
fn remote_plus_opens_granted_resource_navigation_without_creating() -> Result<(), String> {
    for (width, expected) in [
        (900.0, "active=second sidebar=true drawer=false"),
        (390.0, "active=second sidebar=false drawer=true"),
    ] {
        let mut app = start(Root::of::<RemoteFixture>(), width)?;
        app.press("新建终端")?;
        app.press("所有会话")?;
        app.wait_for(expected)?;
        assert!(
            !app.screen()
                .texts()
                .join("\n")
                .contains("CreateLocalSession")
        );
    }
    Ok(())
}

#[test]
fn remote_plus_uses_approved_herdr_parent_and_excludes_unsupported_namespaces() -> Result<(), String>
{
    let mut app = start(Root::of::<RemoteFixture>(), 900.0)?;
    app.press("新建终端")?;
    assert!(!app.shows("新建终端 · Unsupported HC"));
    assert!(!app.shows("新建终端 · Unsupported MicroVM"));
    assert!(!app.shows("新建终端 · Unsupported client"));
    app.press("新建终端 · Granted Herdr")?;
    app.wait_for("Request(TerminalCreate { grant: \"approved-herdr\" })")?;
    for command in ["fixture-readonly", "fixture-no-parents"] {
        let mut app = start(Root::of::<RemoteFixture>(), 900.0)?;
        app.press(command)?;
        assert!(!app.screen().enabled("新建终端")?);
    }
    Ok(())
}

#[test]
fn herdr_tab_close_covers_its_attached_panes_without_crossing_grants() {
    let mut model = Model {
        herdr_grant: "grant".into(),
        ..Model::default()
    };
    model.herdr_workspace.tabs.push(herdr::Tab {
        tab_id: "tab".into(),
        workspace_id: "space".into(),
        ..Default::default()
    });
    for id in ["a", "b"] {
        model.herdr_workspace.panes.push(herdr::HerdrPane {
            pane_id: id.into(),
            tab_id: "tab".into(),
            workspace_id: "space".into(),
            ..Default::default()
        });
        let mut open = pane(id);
        open.kind = PaneKind::Herdr;
        open.herdr_grant = Some("grant".into());
        open.herdr_pane_id = Some(id.into());
        model.panes.push(open);
    }
    let mut other = pane("other-grant");
    other.kind = PaneKind::Herdr;
    other.herdr_grant = Some("other".into());
    other.herdr_pane_id = Some("a".into());
    model.panes.push(other);
    let tabs = entries(&model, Section::Herdr, "space", "b");
    assert_eq!(tabs[0].streams, ["a", "b"]);
    assert!(tabs[0].active);
}

#[test]
fn remote_plus_can_create_on_a_provider_with_only_an_ordinary_webshell_grant() -> Result<(), String>
{
    let mut app = start(Root::of::<RemoteFixture>(), 900.0)?;
    app.press("fixture-webshell-only")?;
    app.press("新建终端")?;
    app.press("新建终端 · Granted provider")?;
    app.wait_for("Request(TerminalCreate { grant: \"approved-provider\" })")?;
    Ok(())
}
