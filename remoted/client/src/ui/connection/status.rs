//! A compact connection indicator; transport state comes from the controller.
use super::{Ui, host_names};
use crate::{
    controller::{Model, Phase},
    preferences::tr,
};
use neko_remoted_protocol::messages::ConnectionKind;
use nsh_base::{PopoverEvent, button, popover, theme};
use nsh_core::{element::ImageSrc, prelude::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Offline,
    Local,
    Relay,
    Connecting,
    Direct,
}
impl Status {
    fn of(model: &Model) -> Self {
        if model.ready() {
            if model.local {
                return Self::Local;
            }
            match model.connection_kind {
                ConnectionKind::Local => Self::Local,
                ConnectionKind::Relay => Self::Relay,
                ConnectionKind::Connecting => Self::Connecting,
                ConnectionKind::Direct => Self::Direct,
            }
        } else if matches!(
            model.phase,
            Phase::Checking
                | Phase::Connecting
                | Phase::AwaitingApproval
                | Phase::Persisting
                | Phase::AwaitingReady
        ) {
            Self::Connecting
        } else {
            Self::Offline
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Offline => "未连接",
            Self::Local => "本机连接",
            Self::Relay => "Relay 中转",
            Self::Connecting => "连接中",
            Self::Direct => "NAT 直连",
        }
    }
    fn markup(self) -> &'static str {
        match self {
            Self::Local => include_str!("../icons/Monitor.svg"),
            Self::Offline => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 3 18 18M9 9a3 3 0 0 0 0 4l2 2a3 3 0 0 0 4 0M15 7l2-2a3 3 0 0 1 4 4l-2 2M5 13l-2 2a3 3 0 0 0 4 4l2-2"/></svg>"#
            }
            Self::Relay => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="8" y="3" width="8" height="6" rx="1"/><rect x="2" y="17" width="6" height="4" rx="1"/><rect x="16" y="17" width="6" height="4" rx="1"/><path d="M12 9v4M5 17v-4h14v4"/></svg>"#
            }
            Self::Connecting => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-3-6.7M21 3v6h-6"/></svg>"#
            }
            Self::Direct => {
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 7 2-2a5 5 0 0 1 7 7l-2 2M15 17l-2 2a5 5 0 0 1-7-7l2-2M8 16l8-8"/></svg>"#
            }
        }
    }
}

fn latency_label(model: &Model) -> SharedString {
    if model.ready()
        && !model.local
        && let Some(millis) = model.latency_ms
    {
        tr("端到端延迟：{} 毫秒")
            .replace("{}", &millis.to_string())
            .into()
    } else {
        tr("端到端延迟：不可用")
    }
}

pub(crate) fn render(ui: Ui) -> impl IntoElement {
    let status = computed(move || ui.model.with(Status::of));
    let label = computed(move || tr(status.get().label()));
    let open = signal(false);
    view()
        .test_id("connection-kind")
        .flex_shrink(0.0)
        .tooltip(crate::ui::controls::tooltip(move || {
            format!(
                "{} · {}",
                ui.model.with(host_names::active_name),
                label.get()
            )
        }))
        .child(
            popover()
                .open(move || Some(open.get()))
                .on_open_change(move |event: &PopoverEvent| {
                    if let PopoverEvent::OpenChange(value, _) = event {
                        open.set(*value);
                    }
                })
                .trigger(
                    button()
                        .accessibility_label(move || label.get())
                        .w(crate::ui::controls::control_size)
                        .h(crate::ui::controls::control_size)
                        .flex_shrink(0.0)
                        .items_center()
                        .justify_center()
                        .rounded(move || theme().radius)
                        .bg(move || theme().background.opacity(0.0))
                        .text_color(move || theme().muted_foreground)
                        .hover(|style| style.bg(move || theme().secondary))
                        .focus(|style| style.outline(px(2.0)).outline_color(move || theme().ring))
                        .active(|style| style.opacity(0.75))
                        .child(
                            svg(move || ImageSrc::svg_markup(status.get().markup()))
                                .w(px(18.0))
                                .h(px(18.0)),
                        ),
                )
                .child(
                    view()
                        .p_3()
                        .gap_2()
                        .bg(move || theme().popover)
                        .text_color(move || theme().popover_foreground)
                        .border_1()
                        .border_color(move || theme().border)
                        .rounded(move || theme().radius)
                        .child(
                            text(move || tr("连接详情"))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            text(move || ui.model.with(host_names::active_name))
                                .text_sm()
                                .select_text(),
                        )
                        .child(text(move || label.get()).text_sm())
                        .child(
                            text(move || ui.model.with(latency_label))
                                .test_id("connection-latency")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(move || ui.model.with(|model| tr(model.status())))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(when(
                            move || ui.model.with(|model| !model.local && model.relay.is_some()),
                            move || {
                                text(move || {
                                    ui.model.with(|model| {
                                        model
                                            .relay
                                            .as_ref()
                                            .map(|relay| relay.origin().to_owned())
                                            .unwrap_or_default()
                                    })
                                })
                                .text_xs()
                                .text_color(move || theme().muted_foreground)
                                .select_text()
                            },
                        )),
                ),
        )
}

#[cfg(test)]
mod tests;
