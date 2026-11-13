//! Tab creation and attachment dismissal, without ending host sessions.

use crate::{
    controller::Command,
    ui::{controls, navigation, state::Ui, terminal_view::interaction},
};
use neko_remoted_protocol::messages::{Operation, Resource};
use nsh_base::{button, menu, theme};
use nsh_core::prelude::*;

/// Close attachments only; selecting the next tab belongs to the model update owner.
pub(super) fn detach(ui: Ui, streams: &[String]) {
    for stream in streams {
        if ui
            .model
            .with_untracked(|model| model.panes.iter().any(|pane| pane.stream == *stream))
        {
            ui.send(Command::Request(Operation::TerminalDetach {
                stream: stream.clone(),
            }));
        }
    }
}

fn creation_parent(resource: &Resource) -> bool {
    (matches!(
        resource.backend.as_str(),
        "native" | "webshell" | "zellij" | "ssh"
    ) && resource.terminal)
        || (resource.backend == "herdr" && resource.herdr)
}

fn unavailable_reason(ui: Ui) -> &'static str {
    ui.model.with(|model| {
        if !model.ready() {
            "主机尚未就绪"
        } else if !model.writable() {
            "此会话为只读授权。"
        } else if model.opening {
            "正在打开会话…"
        } else if !model.resources.iter().any(creation_parent) {
            "没有可用于新建终端的已授权目标。"
        } else {
            "新建终端"
        }
    })
}

fn can_create_remote(ui: Ui) -> bool {
    ui.model.with(|model| {
        !model.local
            && model.writable()
            && !model.opening
            && model.resources.iter().any(creation_parent)
    })
}

pub(super) fn add_button(ui: Ui) -> impl IntoElement {
    when(
        move || ui.model.with(|model| model.local),
        move || crate::ui::resources::local_actions(ui),
    )
    .otherwise(move || {
        view()
            .tooltip(controls::tooltip(move || unavailable_reason(ui)))
            .child(
                menu()
                    .popup_style(interaction::menu_surface)
                    .trigger(
                        button()
                            .accessibility_label(move || crate::preferences::tr("新建终端"))
                            .disabled(move || !can_create_remote(ui))
                            .w(controls::control_size)
                            .h(controls::control_size)
                            .items_center()
                            .justify_center()
                            .rounded(move || theme().radius)
                            .bg(move || theme().background.opacity(0.0))
                            .hover(|style| style.bg(move || theme().secondary))
                            .focus(|style| {
                                style.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .active(|style| style.opacity(0.75))
                            .disabled_style(|style| style.opacity(0.45))
                            .child(controls::icons::render(controls::icons::Icon::Add)),
                    )
                    .child(each(
                        move || {
                            ui.model.with(|model| {
                                model
                                    .resources
                                    .iter()
                                    .filter(|resource| creation_parent(resource))
                                    .map(|resource| (resource.id.clone(), resource.name.clone()))
                                    .collect::<Vec<_>>()
                            })
                        },
                        |resource| resource.0.clone(),
                        move |parent, _| {
                            interaction::menu_action(
                                move || {
                                    format!(
                                        "{} · {}",
                                        crate::preferences::tr("新建终端"),
                                        parent.with(|parent| parent.1.clone())
                                    )
                                },
                                move || !can_create_remote(ui),
                                move || {
                                    let grant = parent.get_untracked().0;
                                    if can_create_remote(ui)
                                        && ui.model.with_untracked(|model| {
                                            model.resources.iter().any(|resource| {
                                                resource.id == grant && creation_parent(resource)
                                            })
                                        })
                                    {
                                        ui.send(Command::Request(Operation::TerminalCreate {
                                            grant,
                                        }));
                                    }
                                },
                            )
                        },
                    ))
                    .child(interaction::menu_action(
                        "所有会话",
                        || false,
                        move || {
                            ui.navigation.settings.set(false);
                            ui.navigation.files.set(false);
                            if navigation::wide() {
                                ui.navigation.sidebar.set(true);
                            } else {
                                ui.navigation.drawer.set(true);
                            }
                        },
                    )),
            )
    })
}
