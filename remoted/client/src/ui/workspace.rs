//! Composition only. Hidden session surfaces remain mounted across secondary routes.
use super::{
    connection, file_view, session_list, state::Ui, terminal_status_bar, terminal_view,
    workspace_header,
};
use nsh_core::prelude::*;

pub fn render(ui: Ui) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(workspace_header::render(ui))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .style_when(move || !ui.navigation.inline_settings(), |s| s.hidden())
                .child(connection::actions(ui))
                .child(connection::render(ui)),
        )
        .child(when(
            move || !ui.navigation.inline_settings() && ui.navigation.files.get(),
            move || file_view::render(ui),
        ))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .style_when(
                    move || ui.navigation.inline_settings() || ui.navigation.files.get(),
                    |s| s.hidden(),
                )
                .child(when(
                    move || ui.active.with(|active| active.is_empty()),
                    move || session_list::render(ui),
                ))
                .child(each(
                    move || ui.model.with(|m| m.panes.clone()),
                    |p| p.stream.clone(),
                    move |p, _| terminal_view::render(ui, p),
                )),
        )
        .child(when(
            move || {
                !ui.navigation.inline_settings()
                    && !ui.navigation.files.get()
                    && ui.model.with(|model| model.ready())
            },
            move || terminal_status_bar::render(ui),
        ))
}
