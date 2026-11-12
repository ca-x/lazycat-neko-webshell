//! Jump targets come only from authoritative pane/agent joins and program records.
use crate::{
    controller::{Model, PaneKind},
    preferences::tr,
    ui::{
        controls::{control_size, icons},
        navigation::Section,
        resources,
        state::Ui,
    },
};
use nsh_base::{button, theme};
use nsh_core::prelude::*;

#[derive(Clone, PartialEq, Eq)]
enum Target {
    Herdr { grant: String, pane: String },
    Terminal(String),
}
#[derive(Clone, PartialEq, Eq)]
struct AgentRow {
    key: String,
    target: Target,
    label: String,
    kind: String,
    status: Option<String>,
    selected: bool,
}
fn rows(model: &Model, active: &str) -> Vec<AgentRow> {
    if !model.ready() {
        return Vec::new();
    }
    let mut rows = Vec::new();
    if !model.herdr_grant.is_empty() {
        for agent in &model.herdr_workspace.agents {
            let Some(pane) = model
                .herdr_workspace
                .panes
                .iter()
                .find(|pane| pane.pane_id == agent.pane_id)
            else {
                continue;
            };
            let workspace = model
                .herdr_workspace
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == pane.workspace_id);
            let tab = model
                .herdr_workspace
                .tabs
                .iter()
                .find(|tab| tab.tab_id == pane.tab_id);
            let context = [
                workspace.map(|workspace| workspace.label.as_str()),
                tab.map(|tab| tab.label.as_str()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let identity = if agent.display_name.is_empty() {
                &agent.kind
            } else {
                &agent.display_name
            };
            rows.push(AgentRow {
                key: format!("{}:{}", model.herdr_grant, pane.pane_id),
                target: Target::Herdr {
                    grant: model.herdr_grant.clone(),
                    pane: pane.pane_id.clone(),
                },
                label: if context.is_empty() {
                    identity.clone()
                } else {
                    format!("{context} · {identity}")
                },
                kind: agent.kind.clone(),
                status: agent.status.clone(),
                selected: model.panes.iter().any(|open| {
                    open.stream == active
                        && open.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
                        && open.herdr_pane_id.as_deref() == Some(pane.pane_id.as_str())
                }),
            });
        }
    }
    for pane in model
        .panes
        .iter()
        .filter(|pane| pane.kind == PaneKind::Terminal && pane.live())
    {
        for record in &pane.programs {
            let Some(app) = record.app.as_deref().filter(|app| !app.trim().is_empty()) else {
                continue;
            };
            let status = icons::program_status(record.state);
            rows.push(AgentRow {
                key: format!("{}:{}", pane.stream, record.id),
                target: Target::Terminal(pane.stream.clone()),
                label: format!("{} · {app}", pane.name),
                kind: app.into(),
                status: Some(status.into()),
                selected: pane.stream == active,
            });
        }
    }
    rows
}
pub fn render(ui: Ui) -> impl IntoElement {
    view()
        .test_id("sidebar-agents")
        .border_t_1()
        .border_color(move || theme().border)
        .px_2()
        .py_2()
        .gap_1()
        .child(
            text(move || tr("Agents"))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            scroll()
                .max_h(move || px((window().height.0 * 0.3).clamp(96.0, 280.0)))
                .child(view().gap_1().child(each(
                    move || ui.model.with(|model| rows(model, &ui.active.get())),
                    |row| (row.key.clone(), row.kind.clone(), row.status.clone()),
                    move |row, _| {
                        let initial = row.get_untracked();
                        button()
                            .selected(move || row.with(|row| row.selected))
                            .on_press(move || match row.get_untracked().target {
                                Target::Herdr { grant, pane } => {
                                    if ui.model.with_untracked(|model| model.herdr_grant == grant) {
                                        resources::open_pane(ui, &pane);
                                    }
                                }
                                Target::Terminal(stream) => {
                                    ui.navigation.section.set(Section::Terminal);
                                    ui.active.set(stream);
                                    ui.navigation.show_session();
                                }
                            })
                            .min_h(control_size)
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .min_w(px(0.0))
                            .rounded(move || theme().radius)
                            .bg(move || theme().background.opacity(0.0))
                            .selected_style(|style| style.bg(move || theme().accent))
                            .hover(|style| style.bg(move || theme().secondary))
                            .focus(|style| {
                                style.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .active(|style| style.opacity(0.75))
                            .child(icons::agent_with_status(
                                &initial.kind,
                                initial.status.as_deref(),
                            ))
                            .child(
                                text(move || row.with(|row| row.label.clone()))
                                    .text_xs()
                                    .flex_1()
                                    .min_w(px(0.0)),
                            )
                            .child(icons::activity(initial.status.as_deref()))
                    },
                ))),
        )
        .child(when(
            move || {
                ui.model
                    .with(|model| rows(model, &ui.active.get()).is_empty())
            },
            || {
                text(move || tr("暂无 Agent 状态"))
                    .text_xs()
                    .text_color(move || theme().muted_foreground)
            },
        ))
}
