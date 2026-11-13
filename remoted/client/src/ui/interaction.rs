//! Pane-scoped controls and reusable command items for terminal context menus.
use super::super::{controls, state::Ui};
use crate::{
    controller::{Command, Model, Pane},
    preferences::tr,
};
use neko_remoted_protocol::messages::Operation;
use nsh_base::{button, menu, menu_item, menu_separator, theme};
use nsh_core::{
    element::{ImageSrc, StyleRefinement},
    prelude::*,
};

/// Local host authority is implicit; a remote attachment still needs explicit control.
pub fn input_allowed(model: &Model, pane: &Pane) -> bool {
    model.writable() && pane.live() && !pane.read_only && (model.local || pane.controller)
}

/// Consistent command surface shared by pane and tab menus.
pub fn menu_surface(style: StyleRefinement) -> StyleRefinement {
    style
        .min_w(px(192.0))
        .p_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
}

/// A styled native menu item; its command revalidates the current target on activation.
pub fn menu_action(
    label: impl IntoReactive<SharedString>,
    disabled: impl Fn() -> bool + 'static,
    run: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into_reactive();
    menu_item()
        .disabled(disabled)
        .on_select(run)
        .min_h(controls::control_size)
        .px_3()
        .py_1()
        .rounded(move || theme().radius)
        .hover(|style| style.bg(move || theme().accent))
        .focus(|style| style.bg(move || theme().accent))
        .active(|style| style.bg(move || theme().accent))
        .disabled_style(|style| style.opacity(0.45))
        .child(
            text(move || tr(label.get().as_str()))
                .text_sm()
                .text_color(move || theme().popover_foreground),
        )
}

fn can_control(ui: Ui, pane: ReadSignal<Pane>) -> bool {
    ui.model.with(|model| !model.local && model.writable())
        && pane.with(|pane| pane.live() && !pane.read_only)
}

fn control_label(pane: ReadSignal<Pane>) -> &'static str {
    pane.with(|pane| {
        if pane.controller {
            "释放控制权"
        } else {
            "取得控制权"
        }
    })
}

fn toggle_control(ui: Ui, pane: ReadSignal<Pane>) {
    if can_control(ui, pane) {
        let pane = pane.get_untracked();
        ui.send(Command::Request(Operation::TerminalControl {
            stream: pane.stream,
            take: !pane.controller,
        }));
    }
}

/// These items can also be composed after the renderer's copy/select/paste commands.
pub fn pane_menu_items(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    view()
        .child(when(
            move || ui.model.with(|model| !model.local),
            move || {
                menu_action(
                    move || control_label(pane),
                    move || !can_control(ui, pane),
                    move || toggle_control(ui, pane),
                )
            },
        ))
        .child(menu_action(
            "会话输入",
            || false,
            move || pane.with_untracked(|pane| ui.navigation.toggle_composer(&pane.stream)),
        ))
        .child(when(
            move || {
                ui.model.with(|model| {
                    model.ready() && model.resources.iter().any(|resource| resource.files)
                })
            },
            move || {
                menu_action(
                    "文件",
                    || false,
                    move || {
                        ui.navigation.files.update(|open| *open = !*open);
                    },
                )
            },
        ))
        .child(menu_action(
            "所有会话",
            || false,
            move || {
                ui.active.set(String::new());
                ui.navigation.show_session();
            },
        ))
        .child(when(
            move || pane.with(|pane| pane.kind == crate::controller::PaneKind::Herdr),
            move || {
                view().child(each(
                    move || pane.with(|open| ui.model.with(|model| related_panes(model, open))),
                    |pane| pane.pane_id.clone(),
                    move |target, _| {
                        menu_action(
                            move || target.with(|pane| pane.title.clone()),
                            move || ui.model.with(|model| !model.ready() || model.opening),
                            move || {
                                crate::ui::resources::open_pane(ui, &target.get_untracked().pane_id)
                            },
                        )
                    },
                ))
            },
        ))
        .child(
            menu_separator()
                .h(px(1.0))
                .my_1()
                .bg(move || theme().border),
        )
        .child(menu_action(
            "关闭此窗格连接",
            || false,
            move || {
                ui.send(Command::Request(Operation::TerminalDetach {
                    stream: pane.get_untracked().stream,
                }));
            },
        ))
}

/// A persistent, touch-accessible floating toolbar inside its own pane's reserved inset.
pub fn toolbar(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    view()
        .test_id("terminal-pane-toolbar")
        .flex_row()
        .items_center()
        .gap_1()
        .p_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .child(when(
            move || ui.model.with(|model| !model.local),
            move || {
                controls::icon_action(
                    controls::icons::Icon::Control,
                    move || control_label(pane),
                    move || {
                        ui.model
                            .with(|model| pane.with(|pane| input_allowed(model, pane)))
                    },
                    move || !can_control(ui, pane),
                    move || toggle_control(ui, pane),
                )
            },
        ))
        .child(
            view().tooltip(controls::tooltip("会话输入")).child(
                button()
                    .accessibility_label(move || tr("会话输入"))
                    .selected(move || {
                        pane.with(|pane| ui.navigation.composer_visible(&pane.stream))
                    })
                    .on_press(move || {
                        pane.with_untracked(|pane| ui.navigation.toggle_composer(&pane.stream))
                    })
                    .w(controls::control_size)
                    .h(controls::control_size)
                    .flex_shrink(0.0)
                    .items_center()
                    .justify_center()
                    .rounded(move || theme().radius)
                    .bg(move || theme().background.opacity(0.0))
                    .selected_style(|style| style.bg(move || theme().accent))
                    .hover(|style| style.bg(move || theme().secondary))
                    .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
                    .active(|style| style.opacity(0.75))
                    .child(
                        svg(ImageSrc::svg_markup(include_str!("icons/Keyboard.svg")))
                            .w(px(18.0))
                            .h(px(18.0))
                            .text_color(move || theme().foreground),
                    ),
            ),
        )
        .child(
            menu()
                .popup_style(menu_surface)
                .trigger(controls::icon_trigger(controls::icons::Icon::More, "更多"))
                .child(pane_menu_items(ui, pane)),
        )
}

fn related_panes(
    model: &crate::controller::Model,
    open: &Pane,
) -> Vec<crate::ui::herdr::HerdrPane> {
    if open.herdr_grant.as_deref() != Some(model.herdr_grant.as_str()) {
        return Vec::new();
    }
    let tab = model
        .herdr_workspace
        .panes
        .iter()
        .find(|pane| Some(pane.pane_id.as_str()) == open.herdr_pane_id.as_deref())
        .map(|pane| pane.tab_id.as_str());
    model
        .herdr_workspace
        .panes
        .iter()
        .filter(|pane| Some(pane.tab_id.as_str()) == tab)
        .cloned()
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::{
        controller::{Client, Command, Model, PaneKind, Phase},
        ui::terminal_view,
    };
    use neko_remoted_protocol::{
        self as protocol,
        messages::{Operation, Permission},
    };
    use nsh_core::{
        Size,
        element::Root,
        platform::{Environment, Idiom, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;
    use std::time::Duration;

    fn fixture(readonly: bool) -> impl IntoElement {
        authority_fixture(readonly, false)
    }

    fn authority_fixture(readonly: bool, local: bool) -> impl IntoElement {
        let pane = Pane {
            stream: "interaction-pane".into(),
            name: "PTY".into(),
            kind: PaneKind::Terminal,
            herdr_grant: None,
            herdr_pane_id: None,
            programs: Vec::new(),
            controller: !readonly && !local,
            read_only: readonly,
            exit: None,
            unavailable: None,
            gap: false,
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel(128);
        let ui = Ui::new(
            Model {
                local,
                phase: Phase::Ready,
                permission: Some(if readonly {
                    Permission::ReadOnly
                } else {
                    Permission::Operate
                }),
                panes: vec![pane.clone()],
                ..Model::default()
            },
            Client::fixture(sender),
        );
        let pane = signal(pane);
        let requests = signal(0);
        let last = signal(String::new());
        let snapshot = signal(String::new());
        spawn(async move {
            while let Some(command) = receiver.recv().await {
                match command {
                    Command::SubmitInput { stream, text } => {
                        assert_eq!(stream, "interaction-pane");
                        requests.update(|count| *count += 1);
                        last.set(format!("submitted {text}"));
                    }
                    Command::Request(Operation::TerminalInput { data, .. }) => {
                        requests.update(|count| *count += 1);
                        last.set(format!(
                            "keyboard {:?}",
                            protocol::decode(&data, 16 * 1024).unwrap()
                        ));
                    }
                    _ => {}
                }
            }
        });
        view()
            .flex_1()
            .min_h(px(0.0))
            .child(text(move || format!("REQUESTS={}", requests.get())))
            .child(text(move || last.get()))
            .child(text(move || snapshot.get()))
            .child(
                nsh_base::button()
                    .on_press(move || {
                        let surface = ui
                            .refs
                            .with(|refs| refs.get("interaction-pane").cloned())
                            .unwrap();
                        surface.write("\x1b[31mREAL_PTY_OUTPUT\x1b[0m\x1b[?2004h\x1b[6n".into());
                    })
                    .child(text("fixture output")),
            )
            .child(
                nsh_base::button()
                    .on_press(move || {
                        ui.refs.with(|refs| {
                            let surface = refs.get("interaction-pane").unwrap();
                            let colored = (0..surface.buffer.line_count()).any(|line| {
                                surface
                                    .buffer
                                    .line(line)
                                    .runs
                                    .iter()
                                    .any(|run| run.style.fg.is_some())
                            });
                            snapshot.set(format!(
                                "VT colored={colored} bracketed={} occurrences={} controlling={}",
                                surface.buffer.modes().bracketed_paste,
                                surface.buffer.text().matches("REAL_PTY_OUTPUT").count(),
                                pane.with(|pane| pane.controller)
                            ));
                        });
                    })
                    .child(text("fixture inspect")),
            )
            .child(
                nsh_base::button()
                    .on_press(move || {
                        ui.model.update(|model| model.phase = Phase::Offline);
                    })
                    .child(text("fixture disconnect")),
            )
            .child(crate::ui::workspace_header::render(ui))
            .child(terminal_view::render(ui, pane.read_only()))
    }

    #[component]
    fn OperateFixture() -> impl IntoElement {
        fixture(false)
    }
    #[component]
    fn ReadonlyFixture() -> impl IntoElement {
        fixture(true)
    }

    #[component]
    fn LocalFixture() -> impl IntoElement {
        authority_fixture(false, true)
    }

    fn start<C: Component<Props = ()>>(os: Os) -> Result<Headless, String> {
        Headless::start(EngineConfig {
            viewport: Size::new(px(390.0), px(844.0)),
            environment: Environment {
                os,
                idiom: if os == Os::Android {
                    Idiom::Phone
                } else {
                    Idiom::Desktop
                },
                locale: Locale::parse("zh-CN").unwrap(),
                ..Environment::default()
            },
            ..EngineConfig::new(
                Root::of::<C>()
                    .tokens(nsh_base::tokens_of(Some(include_str!("theme.json"))).unwrap()),
            )
        })
        .map_err(|error| error.to_string())
    }

    #[test]
    fn composer_toggle_keeps_draft_control_and_shared_vt_without_replaying_output()
    -> Result<(), String> {
        for os in [Os::Linux, Os::Android] {
            let mut app = start::<OperateFixture>(os)?;
            app.wait_for("PTY")?;
            if os != Os::Android {
                app.press("会话输入")?;
            }
            app.wait_for("输入内容，发送到当前会话…")?;
            assert!(app.screen().press_target("对话模式")?.is_none());
            app.type_text("会话输入", "草稿猫")?;
            app.press("fixture output")?;
            app.press("fixture inspect")?;
            app.wait_for("VT colored=true bracketed=true occurrences=1 controlling=true")?;
            app.press("会话输入")?;
            app.press("会话输入")?;
            assert_eq!(app.screen().input_text("会话输入")?, Some("草稿猫"));
            app.press("fixture inspect")?;
            app.wait_for("VT colored=true bracketed=true occurrences=1 controlling=true")?;
            assert!(app.shows("REQUESTS=0"));
            app.press("发送")?;
            app.wait_for("submitted 草稿猫")?;
            app.wait_for("REQUESTS=1")?;
            assert_eq!(app.screen().input_text("会话输入")?, Some(""));
            app.press("会话输入")?;
            app.press("会话输入")?;
            assert!(app.shows("REQUESTS=1"));
        }
        Ok(())
    }

    #[test]
    fn terminal_keeps_native_text_and_control_keyboard_input() -> Result<(), String> {
        let mut app = start::<OperateFixture>(Os::Linux)?;
        app.wait_until("native terminal input", Duration::from_secs(3), |screen| {
            screen
                .input_text("远程终端；取得控制权后可输入")
                .ok()
                .flatten()
                .is_some()
        })?;
        app.type_text("远程终端；取得控制权后可输入", "x")?;
        app.wait_for("keyboard [120]")?;
        app.press_key("ctrl+c")?;
        app.wait_for("keyboard [3]")?;
        app.press_key("up")?;
        app.wait_for("keyboard [27, 91, 65]")?;
        app.press("会话输入")?;
        app.press("会话输入")?;
        app.wait_until(
            "native terminal input restored",
            Duration::from_secs(3),
            |screen| {
                screen
                    .input_text("远程终端；取得控制权后可输入")
                    .ok()
                    .flatten()
                    .is_some()
            },
        )?;
        app.type_text("远程终端；取得控制权后可输入", "z")?;
        app.wait_for("keyboard [122]")?;
        Ok(())
    }

    #[test]
    fn readonly_and_disconnected_panes_never_send_composer_or_control_bytes() -> Result<(), String>
    {
        let mut app = start::<ReadonlyFixture>(Os::Android)?;
        app.wait_for("此会话为只读授权。")?;
        assert!(app.type_text("会话输入", "blocked").is_err());
        for _ in 0..2 {
            app.press("会话输入")?;
            assert!(app.type_text("会话输入", "blocked").is_err());
            assert!(!app.shows("中断 Ctrl+C"));
            assert!(app.shows("REQUESTS=0"));
        }
        let mut app = start::<OperateFixture>(Os::Android)?;
        app.wait_for("会话输入")?;
        app.type_text("会话输入", "keep offline draft")?;
        app.press("fixture disconnect")?;
        app.wait_for("连接已断开，输入不会发送。")?;
        assert_eq!(
            app.screen().input_text("会话输入")?,
            Some("keep offline draft")
        );
        assert!(app.type_text("会话输入", "blocked").is_err());
        assert!(app.shows("REQUESTS=0"));
        Ok(())
    }

    #[test]
    fn local_input_has_implicit_control_and_toolbar_cannot_cover_terminal_rows()
    -> Result<(), String> {
        let mut app = start::<LocalFixture>(Os::Linux)?;
        app.first_render()?;
        assert!(app.screen().press_target("取得控制权")?.is_none());
        assert!(app.screen().press_target("释放控制权")?.is_none());
        let input = app
            .screen()
            .layout_nodes()
            .iter()
            .find(|node| node.test_id.as_deref() == Some("terminal-native-input"))
            .unwrap()
            .id;
        assert!(
            app.screen().commands().iter().any(
                |(node, command)| *node == input && matches!(command, nsh_core::Command::Focus)
            )
        );
        app.type_text("id:terminal-native-input", "x")?;
        app.wait_for("keyboard [120]")?;
        let nodes = app.screen().layout_nodes();
        let toolbar = nodes
            .iter()
            .find(|node| node.test_id.as_deref() == Some("terminal-pane-toolbar") && !node.hidden)
            .unwrap();
        let viewport = nodes
            .iter()
            .find(|node| node.test_id.as_deref() == Some("terminal-pane-viewport") && !node.hidden)
            .unwrap();
        assert!(
            toolbar.bounds.bottom().0 <= viewport.bounds.top().0,
            "floating controls must stay in the reserved inset"
        );
        assert_eq!(
            app.screen()
                .count("id:workspace-tab-bar >> id:terminal-pane-toolbar")?,
            0
        );
        app.press("更多")?;
        assert!(app.shows("关闭此窗格连接"));
        assert!(!app.shows("取得控制权"));
        Ok(())
    }

    #[test]
    fn implicit_local_authority_still_requires_live_writable_non_readonly_attachment() {
        let mut model = Model {
            local: true,
            phase: Phase::Ready,
            permission: Some(Permission::Operate),
            ..Model::default()
        };
        let mut pane = Pane {
            stream: "local".into(),
            name: "local".into(),
            kind: PaneKind::Terminal,
            herdr_grant: None,
            herdr_pane_id: None,
            programs: Vec::new(),
            controller: false,
            read_only: false,
            exit: None,
            unavailable: None,
            gap: false,
        };
        assert!(input_allowed(&model, &pane));
        model.local = false;
        assert!(!input_allowed(&model, &pane));
        model.local = true;
        pane.read_only = true;
        assert!(!input_allowed(&model, &pane));
        pane.read_only = false;
        pane.unavailable = Some("detached".into());
        assert!(!input_allowed(&model, &pane));
        pane.unavailable = None;
        pane.exit = Some(0);
        assert!(!input_allowed(&model, &pane));
        pane.exit = None;
        model.phase = Phase::Offline;
        assert!(!input_allowed(&model, &pane));
    }

    #[test]
    fn terminal_context_paste_uses_native_newlines_bracketed_mode_and_input_frames()
    -> Result<(), String> {
        for local in [false, true] {
            let mut app = if local {
                start::<LocalFixture>(Os::Linux)?
            } else {
                start::<OperateFixture>(Os::Linux)?
            };
            app.wait_for("PTY")?;
            app.press("fixture output")?;
            app.set_clipboard(Some("one\r\ntwo\n猫".into()));
            app.right_click("id:terminal-observer")?;
            app.press("粘贴")?;
            app.wait_for(&format!(
                "keyboard {:?}",
                "\x1b[200~one\rtwo\r猫\x1b[201~".as_bytes()
            ))?;
        }
        let mut app = start::<ReadonlyFixture>(Os::Linux)?;
        app.wait_for("PTY")?;
        app.set_clipboard(Some("blocked".into()));
        app.right_click("id:terminal-observer")?;
        assert!(!app.screen().enabled("粘贴")?);
        assert!(app.shows("REQUESTS=0"));
        Ok(())
    }
}
