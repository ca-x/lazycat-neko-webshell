use super::{controls::hint, state::Ui, terminal_observer};
use crate::controller::{Command, Pane, PaneKind};
use crate::preferences::{current_context, tr};
use neko_remoted_protocol::messages::Operation;
use nsh_core::prelude::*;

#[path = "interaction.rs"]
pub(super) mod interaction;

#[path = "terminal_input_adapter.rs"]
mod input_adapter;

pub fn render(ui: Ui, pane: ReadSignal<Pane>) -> impl IntoElement {
    let appearance = current_context();
    let surface = ui.surface(
        &pane.get_untracked().stream,
        pane.get_untracked().kind == PaneKind::Herdr,
    );
    let reference = element_ref();
    let size = signal(None::<(u16, u16)>);
    let writable = computed(move || ui.model.with(|model| model.writable()));
    let controlling = computed(move || {
        ui.model
            .with(|model| pane.with(|pane| interaction::input_allowed(model, pane)))
    });
    effect(move || {
        let visible = pane.with(|p| p.stream == ui.active.get())
            && !ui.navigation.inline_settings()
            && !ui.navigation.files.get();
        // Herdr owns pane geometry through its layout. Only an active ordinary
        // terminal may change the remote PTY size; hidden layouts can be 1x1.
        if controlling.get()
            && writable.get()
            && visible
            && pane.with(|p| p.kind == PaneKind::Terminal)
            && let Some((cols, rows)) = size.get()
        {
            ui.send(Command::Request(Operation::TerminalResize {
                stream: pane.get_untracked().stream,
                cols,
                rows,
            }));
        }
    });
    let stream = pane.get_untracked().stream;
    ui.refs.with(|refs| {
        refs.insert(stream.clone(), surface.clone());
    });
    ui.flush(&stream, &surface);
    let can_input = computed(move || controlling.get() && writable.get());
    let input_buffer = surface.buffer.clone();
    let input_enabled = computed(move || {
        can_input.get()
            && pane.with(|p| p.stream == ui.active.get())
            && !ui.navigation.inline_settings()
            && !ui.navigation.files.get()
    });
    let local_focus = computed(move || {
        input_enabled.get()
            && ui.model.with(|model| model.local)
            && !ui.navigation.drawer.get()
            && !pane.with(|pane| ui.navigation.composer_visible(&pane.stream))
    });
    let input_mounted = signal(false);
    effect(move || {
        if local_focus.get() && input_mounted.get() {
            reference.focus();
        }
    });
    let send_input = move |data: &str| {
        if input_enabled.get_untracked() {
            let herdr = pane.with_untracked(|pane| pane.kind == PaneKind::Herdr);
            for data in super::terminal_input::frames(data, herdr) {
                if !ui.submit(Command::Request(Operation::TerminalInput {
                    stream: pane.get_untracked().stream,
                    data,
                })) {
                    break;
                }
            }
        }
    };
    let paste_buffer = surface.buffer.clone();
    let observer = surface.buffer.clone();
    let clipped = observer.clipped;
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
            move || clipped.get(),
            || {
                view()
                    .px_4()
                    .child(hint(move || tr("此画面超出显示上限，部分内容已截断。")))
            },
        ))
        .child(
            view()
                .test_id("terminal-output-card")
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .bg(move || appearance.background())
                .child(
                    view()
                        .test_id("terminal-pane-viewport")
                        .flex_1()
                        .min_h(px(0.0))
                        .on_layout(move |_: &Bounds<Points>| input_mounted.set(true))
                        // Leave room for the floating controls before measuring terminal rows.
                        .mt(controls_inset)
                        .on_press(move |_: &PressEvent| {
                            if input_enabled.get_untracked() {
                                reference.focus();
                            }
                        })
                        // Android selectable children consume native presses. The root
                        // still reports their touches to this observer; do not claim
                        // them, so long-press selection and scrolling remain native.
                        .on_touch(input_adapter::focus_on_tap(reference, input_enabled))
                        .child(input_adapter::render(
                            reference,
                            input_enabled,
                            input_buffer,
                            send_input,
                        ))
                        .child(terminal_observer::render_interactive(
                            observer.clone(),
                            size,
                            move || {
                                can_input.get()
                                    && pane.with(|p| {
                                        p.kind == PaneKind::Terminal && p.stream == ui.active.get()
                                    })
                                    && !ui.navigation.inline_settings()
                                    && !ui.navigation.files.get()
                            },
                            input_enabled,
                            move |text| {
                                if input_enabled.get_untracked() {
                                    let text = text.replace("\r\n", "\r").replace('\n', "\r");
                                    if paste_buffer.modes().bracketed_paste {
                                        send_input(&format!("\x1b[200~{text}\x1b[201~"));
                                    } else {
                                        send_input(&text);
                                    }
                                }
                            },
                        )),
                )
                .child(
                    view()
                        .absolute()
                        .top_1()
                        .right_1()
                        .child(interaction::toolbar(ui, pane)),
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

fn controls_inset() -> Points {
    // Control height, popup padding, border and the top/bottom clearance.
    super::controls::control_size() + px(16.0)
}
