use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

use crate::database::AgentKeyboardUsage;
use crate::lightos_admin::current_request_account_id;
use crate::state::AppState;
use crate::tty_init::lightos_features_enabled;

#[derive(Deserialize)]
pub(crate) struct UsedShortcut {
    agent: String,
    group: String,
    shortcut: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageSnapshot {
    user_id: String,
    usage: Vec<AgentKeyboardUsage>,
}

fn account(headers: &HeaderMap) -> Result<String, StatusCode> {
    let user_id = if lightos_features_enabled() {
        current_request_account_id(headers)
    } else {
        // Generic mode has no authenticated accounts. Use one local identity and
        // never accept a caller-supplied LightOS user header there.
        Some("local".to_owned())
    };
    user_id
        .filter(|id| id.len() <= 256)
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn valid_shortcut(input: &UsedShortcut) -> bool {
    matches!(input.agent.as_str(), "codex" | "claude" | "pi" | "opencode")
        && matches!(input.group.as_str(), "send" | "control" | "mode" | "edit")
        && !input.shortcut.is_empty()
        && input.shortcut.len() <= 64
        && input
            .shortcut
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<UsageSnapshot>, StatusCode> {
    let user_id = account(&headers)?;
    let usage = state
        .database()
        .load_agent_keyboard_usage(&user_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(UsageSnapshot { user_id, usage }))
}

pub(crate) async fn record(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<UsedShortcut>,
) -> Result<Json<AgentKeyboardUsage>, StatusCode> {
    let user_id = account(&headers)?;
    if !valid_shortcut(&input) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let count = state
        .database()
        .increment_agent_keyboard_usage(&user_id, &input.agent, &input.group, &input.shortcut)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::TOO_MANY_REQUESTS)?;
    Ok(Json(AgentKeyboardUsage {
        agent: input.agent,
        group: input.group,
        shortcut: input.shortcut,
        count,
    }))
}

#[cfg(test)]
mod tests {
    use super::{UsedShortcut, valid_shortcut};

    #[test]
    fn validates_bounded_shortcut_identity() {
        let mut input = UsedShortcut {
            agent: "codex".into(),
            group: "edit".into(),
            shortcut: "shift-left".into(),
        };
        assert!(valid_shortcut(&input));
        input.agent = "other".into();
        assert!(!valid_shortcut(&input));
        input.agent = "codex".into();
        input.shortcut = "../bad".into();
        assert!(!valid_shortcut(&input));
    }
}
