use super::{
    controls::{
        hint,
        icons::{self, Icon},
    },
    state::Ui,
    terminal_observer,
    terminal_surface::Surface,
};
use crate::controller::{Command, Pane, PaneKind};
use crate::preferences::{current_context, tr};
use neko_remoted_protocol::messages::Operation;
use nsh_base::TerminalEvent;
use nsh_core::prelude::*;

#[path = "interaction.rs"]
pub(super) mod interaction;

pub fn render(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    let appearance = current_context();
    let mode = computed(move || pane.with(|pane| ui.navigation.mode(&pane.stream)));
    let surface = Surface::new();
    let reference = surface.reference;
    provide_context(surface.factory());
    let size = signal((80_u16, 24_u16));
    let writable = computed(move || ui.model.with(|model| model.writable()));
    let controlling =
        computed(move || pane.with(|pane| pane.controller && !pane.read_only && pane.live()));
    effect(move || {
        let visible = pane.with(|p| p.stream == ui.active.get())
            && !ui.navigation.settings.get()
            && !ui.navigation.files.get();
        // Herdr owns pane geometry through its layout. Only an active ordinary
        // terminal may change the remote PTY size; hidden layouts can be 1x1.
        if controlling.get()
            && writable.get()
            && visible
            && pane.with(|p| p.kind == PaneKind::Terminal)
        {
            let (cols, rows) = size.get();
            if cols > 1 && rows > 1 {
                ui.send(Command::Request(Operation::TerminalResize {
                    stream: pane.get_untracked().stream,
                    cols,
                    rows,
                }));
            }
        }
    });
    let stream = pane.get_untracked().stream;
    ui.refs.with(|refs| {
        refs.insert(stream.clone(), surface.clone());
    });
    ui.flush(&stream, &surface);
    let can_input = computed(move || controlling.get() && writable.get());
    let observer = surface.buffer.clone();
    view()
        .flex_1()
        .min_h(px(0.0))
        .style_when(
            move || pane.with(|pane| pane.stream != ui.active.get()),
            |style| style.hidden(),
        )
        .child(when(
            move || pane.with(|pane| pane.unavailable.is_some()),
            move || {
                view()
                    .px_4()
                    .gap_1()
                    .child(hint(move || {
                        tr("会话连接已中断。关闭此标签后，可从会话列表重新打开。")
                    }))
                    .child(
                        text(move || {
                            pane.with(|pane| pane.unavailable.clone().unwrap_or_default())
                        })
                        .text_sm()
                        .select_text()
                        .role(AccessibilityRole::Alert),
                    )
            },
        ))
        .child(when(
            move || pane.with(|pane| pane.gap),
            || {
                view()
                    .px_4()
                    .child(hint(move || tr("输出存在缺口；当前画面可能不完整。")))
            },
        ))
        .child(when(
            move || mode.get() == interaction::Mode::Conversation,
            move || interaction::submissions(ui, pane),
        ))
        .child(when(
            move || mode.get() == interaction::Mode::Conversation,
            move || {
                view()
                    .px_3()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(icons::render(Icon::Terminal))
                    .child(
                        text(move || tr("真实终端输出"))
                            .text_sm()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(super::workspace_header::session_tabs::badges(ui, pane))
            },
        ))
        .child(
            view()
                .test_id("terminal-output-card")
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .style_when(
                    move || mode.get() == interaction::Mode::Conversation,
                    |style| {
                        style
                            .border_1()
                            .border_color(move || nsh_base::theme().border)
                            .rounded(move || nsh_base::theme().radius)
                    },
                )
                .bg(move || appearance.background())
                .child(
                    when(
                        move || can_input.get() && mode.get() == interaction::Mode::Terminal,
                        move || {
                            appearance
                                .terminal()
                                .component_ref(&reference)
                                .flex_1()
                                .min_h(px(0.0))
                                .accessibility_label(move || tr("远程终端；取得控制权后可输入"))
                                .bg(move || appearance.background())
                                .text_color(move || appearance.foreground())
                                .on_data(move |data: &SharedString| {
                                    if mode.get_untracked() == interaction::Mode::Terminal
                                        && ui.model.with_untracked(|model| model.writable())
                                        && pane.with_untracked(|pane| {
                                            pane.controller && !pane.read_only && pane.live()
                                        })
                                    {
                                        let herdr =
                                            pane.with_untracked(|p| p.kind == PaneKind::Herdr);
                                        for frame in super::terminal_input::frames(data, herdr) {
                                            if !ui.submit(Command::Request(
                                                Operation::TerminalInput {
                                                    stream: pane.get_untracked().stream,
                                                    data: frame,
                                                },
                                            )) {
                                                break;
                                            }
                                        }
                                    }
                                })
                                .on_resize(move |event: &TerminalEvent| {
                                    let TerminalEvent::Resize(cols, rows) = event else {
                                        return;
                                    };
                                    if *cols > 0 && *rows > 0 {
                                        size.set((
                                            (*cols).clamp(1, 1000) as u16,
                                            (*rows).clamp(1, 1000) as u16,
                                        ));
                                    }
                                })
                        },
                    )
                    .otherwise(move || terminal_observer::render(observer.clone(), size)),
                ),
        )
        .child(
            view()
                .style_when(
                    move || !pane.with(|pane| ui.navigation.composer_visible(&pane.stream)),
                    |style| style.hidden(),
                )
                .child(super::composer::render(ui, pane)),
        )
}
