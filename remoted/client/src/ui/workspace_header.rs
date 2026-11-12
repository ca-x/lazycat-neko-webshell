use super::{
    controls::{hint, icon_action, icons::Icon},
    navigation,
    state::Ui,
    terminal_view::interaction,
};
use crate::preferences::tr;
use nsh_base::theme;
use nsh_core::prelude::*;
#[path = "session_tabs.rs"]
pub(super) mod session_tabs;

/// One navigation/command row; normal connection status belongs to the sidebar.
pub fn render(ui: Ui) -> impl IntoElement {
    view()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .test_id("workspace-tab-bar")
                .flex_row()
                .items_center()
                .gap_1()
                .px_1()
                .child(icon_action(
                    Icon::Devices,
                    "设备侧栏",
                    move || {
                        if navigation::wide() {
                            ui.navigation.sidebar.get()
                        } else {
                            ui.navigation.drawer.get()
                        }
                    },
                    || false,
                    move || {
                        if navigation::wide() {
                            ui.navigation.sidebar.update(|open| *open = !*open);
                        } else {
                            ui.navigation.drawer.update(|open| *open = !*open);
                        }
                    },
                ))
                .child(
                    view().flex_1().min_w(px(0.0)).child(
                        when(
                            move || ui.navigation.settings.get(),
                            || text(move || tr("连接设置")).text_sm(),
                        )
                        .otherwise(move || session_tabs::render(ui)),
                    ),
                )
                .child(when(
                    move || !ui.navigation.settings.get(),
                    move || {
                        each(
                            move || {
                                ui.model.with(|model| {
                                    model
                                        .panes
                                        .iter()
                                        .filter(|pane| pane.stream == ui.active.get())
                                        .cloned()
                                        .collect::<Vec<_>>()
                                })
                            },
                            |pane| pane.stream.clone(),
                            move |pane, _| interaction::toolbar(ui, pane),
                        )
                    },
                ))
                .child(when(
                    move || {
                        navigation::wide()
                            && ui.model.with(|model| {
                                model.ready()
                                    && model.resources.iter().any(|resource| resource.files)
                            })
                    },
                    move || {
                        icon_action(
                            Icon::Files,
                            "文件",
                            move || ui.navigation.files.get(),
                            || false,
                            move || {
                                ui.navigation.settings.set(false);
                                ui.navigation.files.update(|open| *open = !*open);
                            },
                        )
                    },
                )),
        )
        .child(when(
            move || ui.model.with(|model| !model.error.is_empty()),
            move || {
                view()
                    .px_2()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        text(move || ui.model.with(|model| tr(&model.error)))
                            .flex_1()
                            .text_sm()
                            .text_color(move || theme().destructive)
                            .role(AccessibilityRole::Alert),
                    )
                    .child(icon_action(
                        Icon::Close,
                        "关闭提示",
                        || false,
                        || false,
                        move || ui.model.update(|model| model.error.clear()),
                    ))
            },
        ))
        .child(when(
            move || ui.model.with(|model| !model.notice.is_empty()),
            move || {
                view()
                    .px_2()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        view()
                            .flex_1()
                            .child(hint(move || ui.model.with(|model| model.notice.clone()))),
                    )
                    .child(icon_action(
                        Icon::Close,
                        "关闭提示",
                        || false,
                        || false,
                        move || ui.model.update(|model| model.notice.clear()),
                    ))
            },
        ))
}
