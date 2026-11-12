use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub tabs: Vec<Tab>,
    pub panes: Vec<HerdrPane>,
    pub agents: Vec<Agent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    pub workspace_id: String,
    pub label: String,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    pub label: String,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HerdrPane {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub title: String,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Agent {
    pub pane_id: String,
    pub kind: String,
    pub display_name: String,
    pub status: Option<String>,
}

impl Snapshot {
    /// Keep collection order when no pane is focused, and never cross spaces/tabs.
    pub fn preferred_pane(&self, workspace: &str, tab: Option<&str>) -> Option<&HerdrPane> {
        self.panes
            .iter()
            .filter(|pane| {
                pane.workspace_id == workspace && tab.is_none_or(|tab| pane.tab_id == tab)
            })
            .reduce(|selected, candidate| {
                if candidate.focused && !selected.focused {
                    candidate
                } else {
                    selected
                }
            })
    }
}

pub fn snapshot(value: &Value) -> Snapshot {
    let result = value.get("result").unwrap_or(value);
    let source = result.get("snapshot").unwrap_or(result);
    let mut parsed = Snapshot::default();
    read_collections(source, "", "", &mut parsed, 0);
    for pane in &mut parsed.panes {
        if pane.workspace_id.is_empty()
            && let Some(tab) = parsed.tabs.iter().find(|tab| tab.tab_id == pane.tab_id)
        {
            pane.workspace_id.clone_from(&tab.workspace_id);
        }
    }
    for value in items(source, "agents").take(256) {
        let id = string(value, &["pane_id"]);
        if id.is_empty() || !parsed.panes.iter().any(|pane| pane.pane_id == id) {
            continue;
        }
        if let Some(mut agent) = read_agent(value, &id) {
            if let Some(pane_agent) = parsed.agents.iter().find(|agent| agent.pane_id == id) {
                if agent.kind.is_empty() {
                    agent.kind.clone_from(&pane_agent.kind);
                }
                if agent.display_name.is_empty() {
                    agent.display_name.clone_from(&pane_agent.display_name);
                }
            }
            parsed.agents.retain(|previous| previous.pane_id != id);
            parsed.agents.push(agent);
        }
    }
    parsed
}

fn items<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn string(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default()
        .to_owned()
}

fn read_agent(value: &Value, pane_id: &str) -> Option<Agent> {
    let kind = string(value, &["agent", "kind"]);
    let display_name = string(value, &["display_agent", "name"]);
    let status = value
        .get("agent_status")
        .or_else(|| value.get("status"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if kind.is_empty() && display_name.is_empty() && status.is_none() {
        return None;
    }
    Some(Agent {
        pane_id: pane_id.to_owned(),
        kind,
        display_name,
        status,
    })
}

fn read_collections(
    value: &Value,
    workspace: &str,
    tab: &str,
    parsed: &mut Snapshot,
    depth: usize,
) {
    if depth > 2 {
        return;
    }
    for value in items(value, "panes").take(256) {
        let id = string(value, &["pane_id", "id"]);
        if id.is_empty()
            || parsed.panes.len() >= 256
            || parsed.panes.iter().any(|pane| pane.pane_id == id)
        {
            continue;
        }
        let mut workspace_id = string(value, &["workspace_id"]);
        if workspace_id.is_empty() {
            workspace_id = workspace.to_owned();
        }
        let mut tab_id = string(value, &["tab_id"]);
        if tab_id.is_empty() {
            tab_id = tab.to_owned();
        }
        let title = string(
            value,
            &["title", "terminal_title_stripped", "terminal_title", "name"],
        );
        parsed.panes.push(HerdrPane {
            pane_id: id.clone(),
            workspace_id,
            tab_id,
            title: if title.is_empty() { id.clone() } else { title },
            focused: value
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
        if let Some(agent) = read_agent(value, &id) {
            parsed.agents.push(agent);
        }
    }
    for value in items(value, "tabs").take(256) {
        let id = string(value, &["tab_id", "id"]);
        if id.is_empty()
            || parsed.tabs.len() >= 256
            || parsed.tabs.iter().any(|tab| tab.tab_id == id)
        {
            continue;
        }
        let mut workspace_id = string(value, &["workspace_id"]);
        if workspace_id.is_empty() {
            workspace_id = workspace.to_owned();
        }
        let label = string(value, &["label", "name"]);
        parsed.tabs.push(Tab {
            tab_id: id.clone(),
            workspace_id: workspace_id.clone(),
            label: if label.is_empty() { id.clone() } else { label },
            focused: value
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
        read_collections(value, &workspace_id, &id, parsed, depth + 1);
    }
    for value in items(value, "workspaces").take(256) {
        let id = string(value, &["workspace_id", "id"]);
        if id.is_empty()
            || parsed.workspaces.len() >= 256
            || parsed
                .workspaces
                .iter()
                .any(|workspace| workspace.workspace_id == id)
        {
            continue;
        }
        let label = string(value, &["label", "name"]);
        parsed.workspaces.push(Workspace {
            workspace_id: id.clone(),
            label: if label.is_empty() { id.clone() } else { label },
            focused: value
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
        read_collections(value, &id, "", parsed, depth + 1);
    }
}

pub fn panes(value: &Value) -> Vec<(String, String)> {
    let snapshot = snapshot(value);
    let mut panes = snapshot
        .panes
        .iter()
        .map(|pane| {
            let workspace = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == pane.workspace_id);
            let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == pane.tab_id);
            let mut labels = Vec::new();
            if let Some(workspace) = workspace {
                labels.push(workspace.label.as_str());
            }
            if let Some(tab) = tab {
                labels.push(tab.label.as_str());
            }
            labels.push(&pane.title);
            let mut title = labels.join(" / ");
            if let Some(agent) = snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane.pane_id && !agent.display_name.is_empty())
            {
                title.push_str(" · ");
                title.push_str(&agent.display_name);
            }
            (pane.pane_id.clone(), title)
        })
        .collect::<Vec<_>>();
    panes.sort_by(|left, right| left.0.cmp(&right.0));
    panes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pane_selection_keeps_first_order_and_respects_workspace_and_tab() {
        let mut parsed = snapshot(
            &serde_json::json!({"workspaces":[{"id":"w"},{"id":"other"}],"tabs":[{"id":"t","workspace_id":"w"}],"panes":[
                {"id":"first","workspace_id":"w","tab_id":"t"},{"id":"second","workspace_id":"w","tab_id":"t"},{"id":"foreign","workspace_id":"other","tab_id":"t","focused":true}
            ]}),
        );
        assert_eq!(
            parsed.preferred_pane("w", Some("t")).unwrap().pane_id,
            "first"
        );
        parsed.panes[1].focused = true;
        assert_eq!(
            parsed.preferred_pane("w", Some("t")).unwrap().pane_id,
            "second"
        );
        assert!(parsed.preferred_pane("w", Some("missing")).is_none());
    }

    #[test]
    fn typed_snapshot_joins_authoritative_agents_without_guessing_identity() {
        let parsed = snapshot(&json!({"result":{"snapshot": {
            "workspaces":[{"workspace_id":"w", "label":"Project"}],
            "tabs":[{"tab_id":"t", "workspace_id":"w", "label":"Build"}],
            "panes":[{"pane_id":"p", "tab_id":"t", "title":"claude in title", "agent":"codex", "agent_status":"idle"}],
            "agents":[{"pane_id":"p", "agent":"gemini", "display_agent":"Gemini CLI", "agent_status":"blocked"}, {"pane_id":"outside", "agent":"claude"}]
        }}, "panes":[{"pane_id":"spoof"}]}));
        assert_eq!(parsed.panes.len(), 1);
        assert_eq!(parsed.panes[0].workspace_id, "w");
        assert_eq!(
            parsed.agents,
            vec![Agent {
                pane_id: "p".into(),
                kind: "gemini".into(),
                display_name: "Gemini CLI".into(),
                status: Some("blocked".into())
            }]
        );
    }

    #[test]
    fn typed_snapshot_does_not_infer_agents_from_titles_or_display_labels() {
        let parsed = snapshot(
            &json!({"panes":[{"pane_id":"p", "title":"Codex"}, {"pane_id":"q", "display_agent":"Claude"}]}),
        );
        assert_eq!(parsed.agents.len(), 1);
        assert!(parsed.agents[0].kind.is_empty());
        assert_eq!(parsed.agents[0].status, None);
    }

    #[test]
    fn typed_snapshot_preserves_order_and_nested_ownership() {
        let parsed = snapshot(
            &json!({"workspaces":[{"id":"w", "tabs":[{"id":"t", "panes":[{"id":"z"},{"id":"a"},{"id":"a"}]}]}]}),
        );
        assert_eq!(
            parsed
                .panes
                .iter()
                .map(|pane| pane.pane_id.as_str())
                .collect::<Vec<_>>(),
            ["z", "a"]
        );
        assert!(
            parsed
                .panes
                .iter()
                .all(|pane| pane.workspace_id == "w" && pane.tab_id == "t")
        );
    }

    #[test]
    fn joined_status_keeps_explicit_pane_identity_but_does_not_invent_missing_status() {
        let parsed = snapshot(
            &json!({"panes":[{"pane_id":"p", "agent":"codex", "agent_status":"working"}], "agents":[{"pane_id":"p", "name":"Build"}]}),
        );
        assert_eq!(parsed.agents[0].kind, "codex");
        assert_eq!(parsed.agents[0].display_name, "Build");
        assert_eq!(parsed.agents[0].status, None);
    }

    #[test]
    fn typed_snapshot_limits_untrusted_collections_and_ignores_metadata_panes() {
        let parsed = snapshot(
            &json!({"panes":(0..400).map(|id| json!({"pane_id":format!("p{id}")})).collect::<Vec<_>>(), "metadata":{"panes":[{"pane_id":"fake"}]}}),
        );
        assert_eq!(parsed.panes.len(), 256);
        assert!(!parsed.panes.iter().any(|pane| pane.pane_id == "fake"));
    }

    #[test]
    fn snapshot_preserves_workspace_and_tab_labels_without_treating_them_as_panes() {
        let snapshot = json!({"workspaces":[{"id":"workspace","label":"工作区","tabs":[{"id":"tab","label":"服务","panes":[{"id":"pane","title":"Shell"}]}]}],"agents":[{"id":"agent","kind":"agent"}]});
        assert_eq!(
            panes(&snapshot),
            vec![("pane".into(), "工作区 / 服务 / Shell".into())]
        );
    }

    #[test]
    fn real_snapshot_envelope_joins_flat_workspace_and_tab_records() {
        let snapshot = json!({"result":{"snapshot": {
            "workspaces":[{"workspace_id":"w1","label":"项目"}],
            "tabs":[{"tab_id":"t1","workspace_id":"w1","label":"服务"}],
            "panes":[{"pane_id":"p1","tab_id":"t1","title":"Shell","display_agent":"Codex"}],
            "agents":[{"pane_id":"unrelated","name":"Not a pane"}]
        }, "panes":[{"pane_id":"outside"}]}});
        assert_eq!(
            panes(&snapshot),
            vec![("p1".into(), "项目 / 服务 / Shell · Codex".into())]
        );
    }

    #[test]
    fn flat_contract_panes_are_deduplicated_and_bounded() {
        let snapshot = json!({"panes": (0..300).map(|index|json!({"pane_id":format!("pane-{index}"),"title":"Terminal"})).collect::<Vec<_>>()});
        assert_eq!(panes(&snapshot).len(), 256);
        assert_eq!(
            panes(&json!({"panes":[{"pane_id":"p"},{"pane_id":"p"}]})).len(),
            1
        );
    }
}
