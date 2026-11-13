//! Isolated native Linux previews, using real UI components and deterministic in-memory state.
//!
//! Run the ignored `linux_session_preview` test on the task-owned DISPLAY=:199.
//! `NEKO_PREVIEW_NARROW=1` selects 390×844; the default is 1100×760.
//! `NEKO_PREVIEW_DARK=1` selects dark appearance; the default is light.
//! `NEKO_PREVIEW_SECTION=herdr` selects Herdr; the default selects the ordinary terminal.
//! Narrow width changes layout only; this remains a native Linux desktop window.
//! The preview uses the same fixed terminal grid and input path as the app.
//! `NEKO_PREVIEW_LIST=1` shows the section's session hierarchy instead of an open pane.
//! `NEKO_PREVIEW_SETTINGS=1` opens settings, including the actual preference controls.
//! `NEKO_PREVIEW_LANGUAGE=zh-CN|en-US`, `NEKO_PREVIEW_THEME=<catalog id>` and
//! `NEKO_PREVIEW_FONT_SIZE=8..40` configure display preferences without touching storage.
//! `NEKO_PREVIEW_HERDR_TEXT=/tmp/...` uses a captured, real plain-text Herdr screen.
//! Diagnostics and optional late output retain `NEKO_PREVIEW_DIAGNOSTICS=1` and
//! `NEKO_PREVIEW_LATE_OUTPUT=1`. Close the window to finish the graphical test.
use super::{
    herdr,
    navigation::Section,
    shell,
    state::{Chunk, Ui},
};
use crate::{
    controller::{Client, Command, Model, Pane, PaneKind, Phase},
    preferences::{Appearance, Language, Preferences, Ui as DisplayPreferences, catalog::Catalog},
};
use neko_remoted_protocol::{
    RelayAddress, encode,
    messages::{FileEntry, Permission, Resource},
};
use nsh_core::{element::Root, prelude::*};

fn enabled(name: &str) -> bool {
    std::env::var(name).as_deref() == Ok("1")
}

fn preview_preferences() -> Preferences {
    let mut preferences = Preferences::defaults(Catalog::shared().expect("preview theme catalog"));
    preferences.appearance = if enabled("NEKO_PREVIEW_DARK") {
        Appearance::Dark
    } else {
        Appearance::Light
    };
    preferences.language = match std::env::var("NEKO_PREVIEW_LANGUAGE").as_deref() {
        Ok("en-US") => Language::EnUs,
        Ok("zh-CN") | Err(_) => Language::ZhCn,
        Ok(value) => panic!("unsupported preview language: {value}"),
    };
    if let Ok(theme) = std::env::var("NEKO_PREVIEW_THEME") {
        preferences.theme_id = theme;
    } else if preferences.appearance == Appearance::Light {
        preferences.theme_id = "light".into();
    }
    if let Ok(size) = std::env::var("NEKO_PREVIEW_FONT_SIZE") {
        preferences.font_size = size.parse().expect("preview font size must be an integer");
    }
    preferences
        .validate(Catalog::shared().expect("preview theme catalog"))
        .expect("valid preview preferences");
    preferences
}

fn preview_snapshot() -> herdr::Snapshot {
    herdr::snapshot(&serde_json::json!({"result": {"snapshot": {
        "workspaces": [{"workspace_id": "w1", "label": "项目工作区", "focused": true}],
        "tabs": [{"tab_id": "t1", "workspace_id": "w1", "label": "开发", "focused": true}],
        "panes": [
            {"pane_id": "w1:p1", "tab_id": "t1", "title": "Shell", "focused": true},
            {"pane_id": "w1:p2", "tab_id": "t1", "title": "Build"}
        ],
        "agents": [
            {"pane_id": "w1:p1", "agent": "codex", "display_agent": "Codex", "agent_status": "working"},
            {"pane_id": "w1:p2", "agent": "claude", "display_agent": "Claude", "agent_status": "idle"}
        ]
    }}}))
}

#[component]
fn SessionPreview() -> impl IntoElement {
    // Install display context before composing terminals and controls, without reading credentials.
    DisplayPreferences::from_loaded(preview_preferences()).expect("preview display context");
    provide_context(crate::terminal::factory());
    let relay = RelayAddress::parse("https://relay.example.com").expect("preview origin");
    let binding = crate::storage::Binding {
        relay: relay.clone(),
        route: encode(&[1; 32]),
        host_key: encode(&[2; 32]),
        device_id: "preview-device".into(),
    };
    let (sender, mut receiver) = tokio::sync::mpsc::channel(32);
    let ui = Ui::new(
        Model {
            phase: Phase::Ready,
            permission: Some(Permission::Operate),
            relay: Some(relay),
            active_binding: Some(binding.clone()),
            bindings: vec![binding],
            resources: vec![
                Resource {
                    id: "preview-terminal".into(),
                    name: "开发主机终端".into(),
                    backend: "pty".into(),
                    terminal: true,
                    herdr: false,
                    files: true,
                },
                Resource {
                    id: "preview-herdr".into(),
                    name: "项目工作区".into(),
                    backend: "herdr".into(),
                    terminal: true,
                    herdr: true,
                    files: false,
                },
            ],
            panes: vec![
                Pane {
                    kind: PaneKind::Herdr,
                    herdr_pane_id: Some("w1:p1".into()),
                    herdr_grant: Some("preview-herdr".into()),
                    programs: Vec::new(),
                    stream: "preview-herdr".into(),
                    name: "Herdr · 项目工作区".into(),
                    controller: false,
                    read_only: false,
                    exit: None,
                    unavailable: None,
                    gap: false,
                },
                Pane {
                    kind: PaneKind::Terminal,
                    herdr_pane_id: None,
                    herdr_grant: None,
                    programs: Vec::new(),
                    stream: "preview-terminal".into(),
                    name: "终端 · 开发主机".into(),
                    controller: true,
                    read_only: false,
                    exit: None,
                    unavailable: None,
                    gap: false,
                },
            ],
            herdr_grant: "preview-herdr".into(),
            herdr_workspace: preview_snapshot(),
            file_grant: "preview-terminal".into(),
            path: "project".into(),
            entries: vec![FileEntry {
                name: "README.md".into(),
                directory: false,
                size: 128,
            }],
            ..Model::default()
        },
        Client::fixture(sender),
    );
    let herdr = std::env::var("NEKO_PREVIEW_SECTION").as_deref() == Ok("herdr");
    ui.navigation.section.set(if herdr {
        Section::Herdr
    } else {
        Section::Terminal
    });
    if !enabled("NEKO_PREVIEW_LIST") {
        ui.active.set(
            if herdr {
                "preview-herdr"
            } else {
                "preview-terminal"
            }
            .into(),
        );
    }
    ui.navigation.settings.set(enabled("NEKO_PREVIEW_SETTINGS"));
    ui.pending.with(|pending| {
        for (stream, text) in [
            ("preview-terminal", "\x1b[32mdeveloper@neko\x1b[0m:~/project$ git status\r\nOn branch main\r\nWorking tree clean\r\n\r\n\x1b[31mRED\x1b[0m \x1b[7mINVERSE\x1b[0m \x1b[38;2;12;34;56mTRUECOLOR\x1b[0m\r\n\x1b[32mdeveloper@neko\x1b[0m:~/project$ "),
            ("preview-herdr", "Herdr · 项目工作区\n\n● Codex 正在处理你的请求\n  检查终端布局与消息视图\n  中文、English 和长链接保持原始行列\nhttps://example.com/projects/neoko-terminal/issues/native-terminal-layout-and-conversation-mode\n\n› 请检查这个项目的测试结果\n\n测试完成：28 passed, 0 failed\n等待下一条指令…\n"),
        ] {
            pending.entry(stream.into()).or_default().push_back(Chunk { text: text.into(), reset: false });
        }
    });
    if let Some(path) = std::env::var_os("NEKO_PREVIEW_HERDR_TEXT") {
        let path = std::path::PathBuf::from(path);
        assert!(path.is_absolute() && path.starts_with(std::env::temp_dir()));
        let text = std::fs::read_to_string(path).expect("captured Herdr screen");
        assert!(text.len() <= 256 * 1024, "bounded real screen");
        ui.pending.with(|pending| {
            pending.insert("preview-herdr".into(), [Chunk { text, reset: true }].into());
        });
    }
    // A bounded task-owned VT fixture, through the ordinary Surface output path.
    if let Some(path) = std::env::var_os("NEKO_PREVIEW_OUTPUT") {
        let path = std::path::PathBuf::from(path);
        assert!(path.is_absolute() && path.starts_with(std::env::temp_dir()));
        let text = std::fs::read_to_string(path).expect("native VT preview output");
        assert!(text.len() <= 256 * 1024, "bounded VT fixture");
        ui.pending.with(|pending| {
            pending
                .entry("preview-terminal".into())
                .or_default()
                .push_back(Chunk { text, reset: false });
        });
    }
    spawn(async move {
        while let Some(command) = receiver.recv().await {
            if enabled("NEKO_PREVIEW_DIAGNOSTICS") {
                match &command {
                    Command::Request(
                        neko_remoted_protocol::messages::Operation::TerminalInput { stream, data },
                    ) => eprintln!("preview TerminalInput stream={stream}; data={data}"),
                    Command::SubmitInput { stream, text } => {
                        eprintln!(
                            "preview SubmitInput stream={stream}; text={text:?}; no host acknowledgment"
                        );
                    }
                    _ => {}
                }
            }
            ui.model.update(|m| {
                m.notice = if locale().as_str().starts_with("zh") {
                    "图形预览：操作未发送到任何主机。"
                } else {
                    "Preview only: no action was sent to a host."
                }
                .into()
            });
        }
    });
    provide_context(ui);
    if enabled("NEKO_PREVIEW_DIAGNOSTICS") {
        spawn(async move {
            timer(std::time::Duration::from_secs(1)).await;
            ui.refs.with(|surfaces| {
                for (stream, surface) in surfaces {
                    if enabled("NEKO_PREVIEW_LATE_OUTPUT") {
                        surface.write(format!(
                            "\r\n\x1b[32mLAYOUT-STABLE {stream}\x1b[0m\r\nNative output rendering check\r\n"
                        ));
                    }
                    eprintln!(
                        "preview {stream}: size={:?}; text={:?}",
                        surface.buffer.size.get_untracked(),
                        surface.buffer.text()
                    );
                }
            });
        });
    }
    nsh_base::nav_stack().flex_1().child(
        nsh_base::nav_page()
            .header_visible(false)
            .child(shell::body()),
    )
}

#[test]
#[ignore = "opens an isolated Linux graphical preview; close the window to finish"]
fn linux_session_preview() {
    assert_eq!(
        std::env::var("DISPLAY").as_deref(),
        Ok(":199"),
        "preview must use the task-owned :199 Xvfb display, never the user desktop"
    );
    let size = if enabled("NEKO_PREVIEW_NARROW") {
        (390, 844)
    } else {
        (1100, 760)
    };
    let appearance = if enabled("NEKO_PREVIEW_DARK") {
        nsh_platform_linux::Appearance::Dark
    } else {
        nsh_platform_linux::Appearance::Light
    };
    let root = crate::fonts::root(Root::of::<SessionPreview>())
        .tokens(nsh_base::tokens_of(Some(include_str!("theme.json"))).expect("preview theme"));
    let exit = nsh_platform_linux::run_app(
        nsh_platform_linux::AppSource::Rust(root),
        nsh_platform_linux::AppOptions {
            title: "neoko-terminal isolated UI preview".into(),
            app_id: "community.lazycat.neko.remoted.preview".into(),
            size,
            appearance,
            data_dir: Some(std::env::temp_dir().join("neko-ui-preview")),
            ..Default::default()
        },
    );
    assert_eq!(
        i32::from(exit),
        0,
        "isolated native preview exited unsuccessfully"
    );
}
