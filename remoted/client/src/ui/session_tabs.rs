use crate::{
    controller::{Model, PaneKind},
    preferences::tr,
    ui::{
        controls::{
            control_size,
            icons::{self, Icon},
        },
        herdr::Agent,
        navigation::Section,
        resources,
        state::Ui,
    },
};
use nsh_base::{button, theme};
use nsh_core::prelude::*;

#[derive(Clone, PartialEq, Eq)]
enum Target {
    Stream(String),
    Herdr(String),
}
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    key: String,
    title: String,
    target: Target,
    active: bool,
    agent: Option<Agent>,
}

fn entries(model: &Model, section: Section, workspace: &str, active: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    if section == Section::Herdr && !model.herdr_grant.is_empty() {
        for tab in model
            .herdr_workspace
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace)
        {
            let panes = model
                .herdr_workspace
                .panes
                .iter()
                .filter(|pane| pane.tab_id == tab.tab_id)
                .collect::<Vec<_>>();
            let current = panes.iter().find(|pane| {
                model.panes.iter().any(|open| {
                    open.stream == active
                        && open.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
                        && open.herdr_pane_id.as_deref() == Some(pane.pane_id.as_str())
                })
            });
            if let Some(pane) = current.copied().or_else(|| {
                model
                    .herdr_workspace
                    .preferred_pane(workspace, Some(&tab.tab_id))
            }) {
                entries.push(Entry {
                    key: format!("tab:{}:{}", model.herdr_grant, tab.tab_id),
                    title: tab.label.clone(),
                    target: Target::Herdr(pane.pane_id.clone()),
                    active: current.is_some(),
                    agent: model
                        .herdr_workspace
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == pane.pane_id)
                        .cloned(),
                });
            }
        }
        for pane in model.herdr_workspace.panes.iter().filter(|pane| {
            (workspace.is_empty() || pane.workspace_id == workspace)
                && !model
                    .herdr_workspace
                    .tabs
                    .iter()
                    .any(|tab| tab.tab_id == pane.tab_id)
        }) {
            entries.push(Entry {
                key: format!("pane:{}:{}", model.herdr_grant, pane.pane_id),
                title: pane.title.clone(),
                target: Target::Herdr(pane.pane_id.clone()),
                active: model.panes.iter().any(|open| {
                    open.stream == active
                        && open.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
                        && open.herdr_pane_id.as_deref() == Some(pane.pane_id.as_str())
                }),
                agent: model
                    .herdr_workspace
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane.pane_id)
                    .cloned(),
            });
        }
    }
    for pane in model.panes.iter().filter(|pane| match section {
        Section::Terminal => pane.kind == PaneKind::Terminal,
        Section::Herdr => pane.kind == PaneKind::Herdr,
    }) {
        if section == Section::Herdr && entries.iter().any(|entry| matches!(&entry.target, Target::Herdr(id)
            if pane.herdr_grant.as_deref() == Some(model.herdr_grant.as_str()) && pane.herdr_pane_id.as_deref() == Some(id.as_str()))) { continue; }
        // An opened pane in the selected space is represented by its tab, even when it isn't the lead pane.
        if section == Section::Herdr
            && pane.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
            && pane.herdr_pane_id.as_ref().is_some_and(|id| {
                model
                    .herdr_workspace
                    .panes
                    .iter()
                    .any(|source| source.pane_id == *id)
            })
        {
            continue;
        }
        entries.push(Entry {
            key: format!("stream:{}", pane.stream),
            title: pane.name.clone(),
            target: Target::Stream(pane.stream.clone()),
            active: pane.stream == active,
            agent: None,
        });
    }
    entries
}

pub fn render(ui: Ui) -> impl IntoElement {
    view()
        .min_w(px(0.0))
        .child(
            scroll()
                .horizontal()
                .test_id("session-tabs")
                .min_w(px(0.0))
                .child(view().flex_row().items_center().gap_1().child(each(
                    move || {
                        ui.model.with(|model| {
                            entries(
                                model,
                                ui.navigation.section.get(),
                                &resources::current_workspace(ui),
                                &ui.active.get(),
                            )
                        })
                    },
                    |entry| entry.key.clone(),
                    move |entry, _| {
                        button()
                            .selected(move || entry.with(|entry| entry.active))
                            .on_press(move || match entry.get_untracked().target {
                                Target::Herdr(id) => resources::open_pane(ui, &id),
                                Target::Stream(stream) => {
                                    ui.navigation.section.set(ui.model.with_untracked(|model| {
                                        model
                                            .panes
                                            .iter()
                                            .find(|pane| pane.stream == stream)
                                            .map(|pane| {
                                                if pane.kind == PaneKind::Herdr {
                                                    Section::Herdr
                                                } else {
                                                    Section::Terminal
                                                }
                                            })
                                            .unwrap_or(Section::Terminal)
                                    }));
                                    ui.active.set(stream);
                                    ui.navigation.show_session();
                                }
                            })
                            .min_h(control_size)
                            .flex_shrink(0.0)
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .rounded(move || theme().radius)
                            .bg(move || theme().background.opacity(0.0))
                            .selected_style(|style| style.bg(move || theme().accent))
                            .hover(|style| style.bg(move || theme().secondary))
                            .focus(|style| {
                                style.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .active(|style| style.opacity(0.75))
                            .child(
                                when(
                                    move || entry.with(|entry| entry.agent.is_some()),
                                    move || {
                                        view().child(each(
                                            move || {
                                                entry.with(|entry| {
                                                    entry
                                                        .agent
                                                        .clone()
                                                        .into_iter()
                                                        .collect::<Vec<_>>()
                                                })
                                            },
                                            |agent| (agent.kind.clone(), agent.status.clone()),
                                            move |agent, _| {
                                                let agent = agent.get_untracked();
                                                view()
                                                    .flex_row()
                                                    .gap_1()
                                                    .child(icons::agent_with_status(
                                                        &agent.kind,
                                                        agent.status.as_deref(),
                                                    ))
                                                    .child(icons::activity(agent.status.as_deref()))
                                            },
                                        ))
                                    },
                                )
                                .otherwise(move || {
                                    icons::render(
                                        if ui.navigation.section.get_untracked() == Section::Herdr {
                                            Icon::Herdr
                                        } else {
                                            Icon::Terminal
                                        },
                                    )
                                }),
                            )
                            .child(
                                text(move || entry.with(|entry| entry.title.clone()))
                                    .text_sm()
                                    .number_of_lines(1),
                            )
                    },
                ))),
        )
        .child(when(
            move || {
                ui.model.with(|model| {
                    entries(
                        model,
                        ui.navigation.section.get(),
                        &resources::current_workspace(ui),
                        &ui.active.get(),
                    )
                    .is_empty()
                })
            },
            move || {
                text(move || {
                    tr(if ui.navigation.section.get() == Section::Herdr {
                        "Herdr 工作区"
                    } else {
                        "终端会话"
                    })
                })
                .text_sm()
            },
        ))
}

pub fn badges(ui: Ui, pane: ReadSignal<crate::controller::Pane>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(each(
            move || {
                pane.with(|pane| {
                    if pane.kind != PaneKind::Terminal
                        || !pane.live()
                        || !ui.model.with(|model| model.ready())
                    {
                        return Vec::new();
                    }
                    icons::primary_program(&pane.programs)
                        .cloned()
                        .into_iter()
                        .collect::<Vec<_>>()
                })
            },
            |record| {
                (
                    record.id.clone(),
                    record.app.clone(),
                    format!("{:?}", record.state),
                )
            },
            move |record, _| icons::program(&record.get_untracked()),
        ))
        .child(each(
            move || {
                pane.with(|pane| {
                    ui.model.with(|model| {
                        model
                            .herdr_workspace
                            .agents
                            .iter()
                            .filter(|agent| {
                                pane.herdr_grant.as_deref() == Some(model.herdr_grant.as_str())
                                    && pane.herdr_pane_id.as_deref() == Some(agent.pane_id.as_str())
                            })
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                })
            },
            |agent| {
                (
                    agent.pane_id.clone(),
                    agent.kind.clone(),
                    agent.status.clone(),
                )
            },
            move |agent, _| {
                let agent = agent.get_untracked();
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(icons::agent_with_status(
                        &agent.kind,
                        agent.status.as_deref(),
                    ))
                    .child(icons::activity(agent.status.as_deref()))
            },
        ))
}
