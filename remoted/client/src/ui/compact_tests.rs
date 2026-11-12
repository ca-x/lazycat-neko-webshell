use crate::{
    controller::{Client, Command, Model, Pane, PaneKind, Phase},
    ui::{herdr, shell, state::Ui},
};
use neko_remoted_protocol::messages::{Permission, Resource};
use nsh_core::{
    Size,
    element::Root,
    platform::{Environment, Locale, Os},
    prelude::*,
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

#[component]
fn CompactFixture() -> impl IntoElement {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(128);
    let snapshot = herdr::snapshot(&serde_json::json!({"result":{"snapshot":{
        "workspaces":[{"workspace_id":"w1","label":"Space One","focused":true},{"workspace_id":"w2","label":"Space Two"}],
        "tabs":[{"tab_id":"t1","workspace_id":"w1","label":"Tab One","focused":true},{"tab_id":"t2","workspace_id":"w1","label":"Tab Two"},{"tab_id":"t3","workspace_id":"w2","label":"Tab Three"}],
        "panes":[{"pane_id":"p1","tab_id":"t1","title":"Pane One","focused":true},{"pane_id":"p2","tab_id":"t2","title":"Pane Two"},{"pane_id":"p3","tab_id":"t3","title":"Pane Three"}],
        "agents":[{"pane_id":"p1","agent":"codex","display_agent":"Codex","status":"working"},{"pane_id":"p2","agent":"claude","display_agent":"Claude","status":"blocked"}]
    }}}));
    let ui = Ui::new(
        Model {
            phase: Phase::Ready,
            permission: Some(Permission::Operate),
            herdr_grant: "grant".into(),
            herdr_workspace: snapshot,
            resources: vec![Resource {
                id: "grant".into(),
                name: "Herdr Host".into(),
                backend: "herdr".into(),
                terminal: false,
                herdr: true,
                files: false,
            }],
            panes: vec![Pane {
                kind: PaneKind::Herdr,
                stream: "open-one".into(),
                name: "Pane One".into(),
                herdr_pane_id: Some("p1".into()),
                herdr_grant: Some("grant".into()),
                programs: Vec::new(),
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
    let command = signal(String::new());
    spawn(async move {
        while let Some(value) = receiver.recv().await {
            if !matches!(
                &value,
                Command::Request(neko_remoted_protocol::messages::Operation::TerminalResize { .. })
            ) {
                command.set(format!("{value:?}"));
            }
        }
    });
    provide_context(ui);
    nsh_base::nav_stack().flex_1().child(
        nsh_base::nav_page().header_visible(false).child(
            view()
                .flex_1()
                .child(text(move || command.get()))
                .child(shell::body()),
        ),
    )
}
fn start(os: Os, width: f32) -> Result<Headless, String> {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(width), px(800.0)),
        environment: Environment {
            os,
            locale: Locale::parse("zh-CN").unwrap(),
            ..Environment::default()
        },
        ..EngineConfig::new(
            Root::of::<CompactFixture>()
                .tokens(nsh_base::tokens_of(Some(include_str!("theme.json"))).unwrap()),
        )
    })
    .map_err(|error| error.to_string())?;
    app.first_render()?;
    Ok(app)
}
#[test]
fn desktop_has_one_compact_tab_bar_and_agents_jump_without_a_duplicate_open_list()
-> Result<(), String> {
    let mut app = start(Os::Linux, 1200.0)?;
    app.wait_for("Space One")?;
    app.wait_for("Tab One")?;
    app.wait_for("Tab Two")?;
    assert!(!app.shows("Tab Three"));
    assert!(!app.shows("已打开"));
    assert!(
        !app.shows("Herdr Host"),
        "single-source workspace navigation must not repeat the source row"
    );
    assert!(!app.shows("Pane One"));
    assert_eq!(app.screen().input_text("会话输入")?, None);
    let layout = app.screen().layout_nodes();
    let bar = layout
        .iter()
        .find(|node| node.test_id.as_deref() == Some("workspace-tab-bar") && !node.hidden)
        .unwrap();
    assert!(
        bar.bounds.size.height.0 <= 40.0,
        "extra toolbar rows: {:?}",
        bar.bounds
    );
    let sidebar = layout
        .iter()
        .find(|node| node.test_id.as_deref() == Some("workspace-sidebar") && !node.hidden)
        .unwrap();
    assert!((223.0..=224.0).contains(&sidebar.bounds.size.width.0));
    let output = layout
        .iter()
        .find(|node| node.test_id.as_deref() == Some("terminal-output-card") && !node.hidden)
        .unwrap();
    assert!(
        output.bounds.size.height.0 >= 700.0,
        "terminal lost usable height: {:?}",
        output.bounds
    );

    app.press("Space One · Tab Two · Claude")?;
    app.wait_for("Request(TerminalAttach { grant: \"grant\", pane: Some(\"p2\") })")?;
    app.press("Space Two")?;
    app.wait_for("Tab Three")?;
    assert!(!app.shows("Tab One"));
    app.press("Space One · Tab One · Codex")?;
    app.wait_for("Tab One")?;
    assert!(!app.shows("Tab Three"));
    assert!(!app.shows("pane.focus"));
    Ok(())
}
#[test]
fn android_drawer_keeps_agents_and_workspace_selection_and_closes_after_jump() -> Result<(), String>
{
    let mut app = start(Os::Android, 390.0)?;
    app.wait_for("terminal-observer")?;
    assert!(!app.shows("Space One"));
    app.press("设备侧栏")?;
    app.wait_for("Space One · Tab Two · Claude")?;
    app.press("Space One · Tab One · Codex")?;
    app.wait_for("terminal-observer")?;
    app.wait_until(
        "drawer closed",
        std::time::Duration::from_secs(3),
        |screen| !screen.texts().contains(&"Space One"),
    )?;
    assert!(app.screen().input_text("会话输入")?.is_some());
    let bar = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|node| node.test_id.as_deref() == Some("workspace-tab-bar") && !node.hidden)
        .unwrap();
    assert!(bar.bounds.size.height.0 <= 52.0);
    Ok(())
}
