//! Presentation-only interaction modes. Both modes observe the same terminal Surface.
use super::super::{controls, state::Ui};
use crate::{
    controller::{Pane, interaction::SubmissionState},
    preferences::tr,
};
use nsh_base::{button, theme};
use nsh_core::{element::ImageSrc, prelude::*};

pub use crate::ui::navigation::InteractionMode as Mode;

/// Shared with the single workspace tab bar, so the terminal needs no second toolbar.
pub fn toolbar(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    let more = signal(false);
    let controlling = move || {
        pane.with(|pane| pane.controller && !pane.read_only && pane.live())
            && ui.model.with(|model| model.writable())
    };
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(controls::icon_action(
            controls::icons::Icon::Control,
            move || {
                pane.with(|pane| {
                    if pane.unavailable.is_some() {
                        "会话连接已中断。"
                    } else if pane.exit.is_some() {
                        "会话已退出。"
                    } else if !ui.model.with(|model| model.writable()) || pane.read_only {
                        "只读浏览"
                    } else if pane.controller {
                        "释放控制权"
                    } else {
                        "取得控制权"
                    }
                })
            },
            controlling,
            move || {
                !ui.model.with(|model| model.writable())
                    || pane.with(|pane| pane.read_only || !pane.live())
            },
            move || {
                let pane = pane.get_untracked();
                ui.send(crate::controller::Command::Request(
                    neko_remoted_protocol::messages::Operation::TerminalControl {
                        stream: pane.stream,
                        take: !pane.controller,
                    },
                ));
            },
        ))
        .child(mode_button(ui, pane))
        .child(when(
            move || {
                os() != nsh_core::platform::Os::Android
                    && pane.with(|pane| ui.navigation.mode(&pane.stream) == Mode::Terminal)
            },
            move || {
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
                )
            },
        ))
        .child(
            view().tooltip(controls::tooltip("更多")).child(
                nsh_base::popover()
                    .open(move || Some(more.get()))
                    .on_open_change(move |event: &nsh_base::PopoverEvent| {
                        if let nsh_base::PopoverEvent::OpenChange(open, _) = event {
                            more.set(*open);
                        }
                    })
                    .trigger(controls::icon_trigger(controls::icons::Icon::More, "更多"))
                    .child(
                        view()
                            .p_2()
                            .gap_1()
                            .bg(move || theme().popover)
                            .border_1()
                            .border_color(move || theme().border)
                            .rounded(move || theme().radius)
                            .child(controls::action(
                                "会话输入",
                                false,
                                || false,
                                move || {
                                    pane.with_untracked(|pane| {
                                        ui.navigation.toggle_composer(&pane.stream)
                                    });
                                    more.set(false);
                                },
                            ))
                            .child(when(
                                move || {
                                    ui.model.with(|model| {
                                        model.ready()
                                            && model.resources.iter().any(|resource| resource.files)
                                    })
                                },
                                move || {
                                    controls::action(
                                        "文件",
                                        false,
                                        || false,
                                        move || {
                                            ui.navigation.files.update(|open| *open = !*open);
                                            more.set(false);
                                        },
                                    )
                                },
                            ))
                            .child(controls::icon_action(
                                controls::icons::Icon::Back,
                                "所有会话",
                                || false,
                                || false,
                                move || {
                                    ui.active.set(String::new());
                                    ui.navigation.show_session();
                                    more.set(false);
                                },
                            ))
                            .child(when(
                                move || {
                                    pane.with(|pane| {
                                        pane.kind == crate::controller::PaneKind::Herdr
                                    })
                                },
                                move || {
                                    view().gap_1().child(each(
                                        move || {
                                            pane.with(|open| {
                                                ui.model.with(|model| related_panes(model, open))
                                            })
                                        },
                                        |pane| pane.pane_id.clone(),
                                        move |target, _| {
                                            controls::session_row(
                                                controls::icons::Icon::Terminal,
                                                move || target.with(|pane| pane.title.clone()),
                                                move || {
                                                    target.with(|target| {
                                                        pane.with(|open| {
                                                            open.herdr_pane_id.as_deref()
                                                                == Some(target.pane_id.as_str())
                                                        })
                                                    })
                                                },
                                                move || ui.model.with(|model| model.opening),
                                                move || {
                                                    crate::ui::resources::open_pane(
                                                        ui,
                                                        &target.get_untracked().pane_id,
                                                    );
                                                    more.set(false);
                                                },
                                            )
                                        },
                                    ))
                                },
                            ))
                            .child(controls::action(
                                "关闭此窗格连接",
                                false,
                                || false,
                                move || {
                                    ui.send(crate::controller::Command::Request(
                                    neko_remoted_protocol::messages::Operation::TerminalDetach {
                                        stream: pane.get_untracked().stream,
                                    },
                                ));
                                    more.set(false);
                                },
                            )),
                    ),
            ),
        )
}

fn mode_button(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    let conversation =
        move || pane.with(|pane| ui.navigation.mode(&pane.stream) == Mode::Conversation);
    let label = move || {
        tr(if conversation() {
            "终端模式"
        } else {
            "对话模式"
        })
    };
    view().tooltip(controls::tooltip(label)).child(
        button()
            .accessibility_label(label)
            .selected(conversation)
            .on_press(move || {
                pane.with_untracked(|pane| {
                    ui.navigation.set_mode(
                        &pane.stream,
                        if conversation() {
                            Mode::Terminal
                        } else {
                            Mode::Conversation
                        },
                    );
                })
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
                when(conversation, || {
                    svg(ImageSrc::svg_markup(include_str!(
                        "icons/TerminalSquare.svg"
                    )))
                    .w(px(18.0))
                    .h(px(18.0))
                    .text_color(move || theme().foreground)
                })
                .otherwise(|| {
                    svg(ImageSrc::svg_markup(include_str!(
                        "icons/MessageSquare.svg"
                    )))
                    .w(px(18.0))
                    .h(px(18.0))
                    .text_color(move || theme().foreground)
                }),
            ),
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

pub fn submissions(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    let reference = element_ref();
    let latest = computed(move || {
        let stream = pane.with(|pane| pane.stream.clone());
        ui.model.with(|model| {
            model
                .interaction
                .entries()
                .filter(|entry| entry.stream == stream)
                .last()
                .map(|entry| entry.id)
        })
    });
    effect(move || {
        latest.get();
        reference.scroll_to_end(false);
    });
    scroll()
        .element_ref(&reference)
        .max_h(px(220.0))
        .child(view().gap_2().px_3().child(each(
            move || {
                let stream = pane.with(|pane| pane.stream.clone());
                ui.model.with(|model| {
                    model
                        .interaction
                        .entries()
                        .filter(|entry| entry.stream == stream)
                        .cloned()
                        .collect::<Vec<_>>()
                })
            },
            |entry| (entry.id, entry.state),
            move |entry, _| {
                view().items_end().child(
                    view()
                        .w_full()
                        .max_w(px(640.0))
                        .gap_1()
                        .px_3()
                        .py_2()
                        .rounded(move || theme().radius)
                        .bg(move || theme().secondary)
                        .child(
                            text(move || tr("你提交的文本"))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            text(move || entry.with(|entry| entry.text.clone()))
                                .select_text()
                                .text_sm(),
                        )
                        .child(
                            text(move || {
                                entry.with(|entry| {
                                    tr(match entry.state {
                                        SubmissionState::Pending => "等待主机确认输入…",
                                        SubmissionState::Accepted => "主机已接受输入",
                                        SubmissionState::Rejected => "输入未被接受",
                                        SubmissionState::Unknown => "输入结果未知；未自动重发",
                                    })
                                })
                            })
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
            },
        )))
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
        platform::{Environment, Locale, Os},
    };
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;
    use std::time::Duration;

    fn fixture(readonly: bool) -> impl IntoElement {
        let pane = Pane {
            stream: "interaction-pane".into(),
            name: "PTY".into(),
            kind: PaneKind::Terminal,
            herdr_grant: None,
            herdr_pane_id: None,
            programs: Vec::new(),
            controller: !readonly,
            read_only: readonly,
            exit: None,
            unavailable: None,
            gap: false,
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel(128);
        let ui = Ui::new(
            Model {
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

    fn start<C: Component<Props = ()>>(os: Os) -> Result<Headless, String> {
        Headless::start(EngineConfig {
            viewport: Size::new(px(390.0), px(844.0)),
            environment: Environment {
                os,
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
    fn switching_modes_keeps_draft_control_and_shared_vt_without_replaying_output()
    -> Result<(), String> {
        assert_eq!(Mode::default_for(Os::Linux), Mode::Terminal);
        assert_eq!(Mode::default_for(Os::Android), Mode::Conversation);
        for os in [Os::Linux, Os::Android] {
            let mut app = start::<OperateFixture>(os)?;
            app.wait_for("PTY")?;
            if os != Os::Android {
                app.press("会话输入")?;
            }
            app.wait_for("输入内容，发送到当前会话…")?;
            assert_eq!(app.shows("真实终端输出"), os == Os::Android);
            app.type_text("会话输入", "草稿猫")?;
            app.press("fixture output")?;
            if os != Os::Android {
                app.press("对话模式")?;
            }
            app.wait_for("真实终端输出")?;
            app.wait_for("REAL_PTY_OUTPUT")?;
            assert!(app.screen().in_selectable_view("REAL_PTY_OUTPUT")?);
            app.press("fixture inspect")?;
            app.wait_for("VT colored=true bracketed=true occurrences=1 controlling=true")?;
            app.press("终端模式")?;
            app.wait_until("terminal mode selected", Duration::from_secs(3), |screen| {
                !screen.texts().contains(&"真实终端输出")
            })?;
            assert_eq!(app.screen().input_text("会话输入")?, Some("草稿猫"));
            app.press("对话模式")?;
            app.press("fixture inspect")?;
            app.wait_for("VT colored=true bracketed=true occurrences=1 controlling=true")?;
            assert!(app.shows("REQUESTS=0"));
            app.press("发送")?;
            app.wait_for("submitted 草稿猫")?;
            app.wait_for("REQUESTS=1")?;
            assert!(!app.shows("主机已接受输入"));
            assert_eq!(app.screen().input_text("会话输入")?, Some(""));
            app.press("终端模式")?;
            app.press("对话模式")?;
            assert!(app.shows("REQUESTS=1"));
        }
        Ok(())
    }

    #[test]
    fn terminal_mode_keeps_native_text_and_control_keyboard_input() -> Result<(), String> {
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
        app.press("对话模式")?;
        app.wait_for("真实终端输出")?;
        app.press("终端模式")?;
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
    fn readonly_and_disconnected_modes_never_send_composer_or_control_bytes() -> Result<(), String>
    {
        let mut app = start::<ReadonlyFixture>(Os::Android)?;
        app.wait_for("此会话为只读授权。")?;
        assert!(app.type_text("会话输入", "blocked").is_err());
        for mode in ["终端模式", "对话模式"] {
            app.press(mode)?;
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
}
