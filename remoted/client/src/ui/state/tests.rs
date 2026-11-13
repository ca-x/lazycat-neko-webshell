use super::*;
use crate::controller::{Pane, PaneKind, Phase};
use nsh_core::{
    element::Root,
    platform::{Environment, Os},
};
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

fn model(stream: &str, local: bool) -> Model {
    Model {
        phase: Phase::Ready,
        local,
        panes: vec![Pane {
            stream: stream.into(),
            name: stream.into(),
            kind: PaneKind::Terminal,
            herdr_pane_id: None,
            herdr_grant: None,
            programs: Vec::new(),
            controller: true,
            read_only: false,
            exit: None,
            unavailable: None,
            gap: false,
        }],
        ..Model::default()
    }
}

#[component]
fn Fixture() -> impl IntoElement {
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let ui = Ui::new(Model::default(), Client::fixture(sender));
    ui.workspace_model(0, Some(0), model("local", true));
    let local = ui.surface("local", false);
    local.write("\x1b[31mLOCAL".into());
    ui.navigation.toggle_composer("local");
    ui.navigation
        .workspace
        .set(("grant".into(), "workspace".into()));
    ui.workspace_model(1, Some(1), model("remote", false));
    let remote = ui.surface("remote", false);
    remote.write("REMOTE".into());
    // A background model update must not change the displayed host or buffers.
    ui.workspace_model(0, None, model("local", true));
    assert_eq!(ui.active.get_untracked(), "remote");
    local.write(" BACKGROUND".into());
    ui.workspace_model(0, Some(2), model("local", true));
    assert_eq!(ui.active.get_untracked(), "local");
    assert_eq!(ui.model.with_untracked(|model| model.generation), 2);
    assert_eq!(
        ui.surface("local", false).buffer.revision,
        local.buffer.revision
    );
    assert!(local.buffer.line(0).text().contains("LOCAL BACKGROUND"));
    assert!(ui.navigation.composer_visible("local"));
    assert_eq!(
        ui.navigation.workspace.get_untracked(),
        ("grant".into(), "workspace".into())
    );
    ui.workspace_model(1, Some(3), model("remote", false));
    assert_eq!(
        ui.surface("remote", false).buffer.revision,
        remote.buffer.revision
    );
    assert!(remote.buffer.line(0).text().contains("REMOTE"));
    // Closing a stream retires its cache; reconnect output cannot leak into another VT.
    let mut closed = model("remote", false);
    closed.panes.clear();
    ui.workspace_model(1, Some(3), closed);
    assert!(ui.refs.with(|refs| refs.is_empty()));
    assert!(ui.workspaces.with(|cache| cache[&1].refs.is_empty()));
    ui.workspace_model(1, Some(3), model("remote", false));
    ui.surface("remote", false).write("PREVIOUS HOST".into());
    ui.dismiss_tab("tab:grant:reused");
    let mut replacement = model("remote", false);
    replacement.active_binding = Some(crate::storage::Binding {
        relay: neko_remoted_protocol::RelayAddress::parse("https://other.example.com").unwrap(),
        route: neko_remoted_protocol::encode(&[1; 32]),
        host_key: neko_remoted_protocol::encode(&[2; 32]),
        device_id: "another-host".into(),
    });
    ui.workspace_model(1, Some(4), replacement);
    assert!(
        !ui.tab_dismissed("tab:grant:reused"),
        "a recycled worker cannot hide another host's tabs"
    );
    assert_eq!(ui.active.get_untracked(), "remote");
    assert!(
        !ui.surface("remote", false)
            .buffer
            .line(0)
            .text()
            .contains("PREVIOUS HOST")
    );
    text("workspace cache verified")
}

#[test]
fn workspace_switch_preserves_terminal_and_navigation_while_background_output_continues()
-> Result<(), String> {
    let mut app = Headless::start(EngineConfig {
        environment: Environment {
            os: Os::Linux,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<Fixture>())
    })
    .map_err(|error| error.to_string())?;
    app.wait_for("workspace cache verified")
}
