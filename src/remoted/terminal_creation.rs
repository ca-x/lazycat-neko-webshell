//! Operate on an approved terminal/Herdr target also permits a fresh ordinary
//! terminal as that exact target's login user. The child has no file delegation.
//! Persist its grant for only that device: reconnect discovers, never recreates.
//! Physical clients and `MicroVMs` need request-local platform tickets and cannot
//! derive creation authority from these persistent grants.
use super::{
    admission::Admission,
    peer,
    store::{Device, Grant},
};
use crate::{
    config::{DEFAULT_COLS, DEFAULT_OUTPUT_FRAME_LIMIT, DEFAULT_ROWS},
    state::{AppState, METADATA_LOGIN_USER, SessionRecord},
    workspace::{self, SessionBackend, WorkspaceTerminalDefaults},
};
use anyhow::{Context, Result, ensure};
use neko_remoted_protocol::messages::Resource;
use std::{collections::HashMap, sync::Arc, time::Duration};

fn backend(parent: &Grant) -> Result<SessionBackend> {
    ensure!(
        parent.selector.trim() == parent.selector,
        "noncanonical parent target"
    );
    if parent.backend == "ssh" {
        ensure!(
            crate::ssh_backend::is_ssh_selector(&parent.selector),
            "SSH parent target changed"
        );
        return Ok(SessionBackend::Ssh);
    }
    ensure!(
        matches!(parent.backend.as_str(), "webshell" | "herdr" | "zellij"),
        "parent cannot create ordinary terminals"
    );
    ensure!(
        !crate::lightos_admin::is_client_selector(&parent.selector)
            && !crate::ssh_backend::is_ssh_selector(&parent.selector),
        "target requires separate creation authority"
    );
    crate::validation::validate_selector(&parent.selector)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(SessionBackend::Webshell)
}

fn child_grant(parent: &Grant, session: &SessionRecord) -> Result<Grant> {
    let expected = if parent.backend == "ssh" {
        "ssh"
    } else {
        "webshell"
    };
    ensure!(
        session.selector == parent.selector
            && session
                .metadata
                .get(METADATA_LOGIN_USER)
                .map_or("", String::as_str)
                == parent.login
            && session.metadata.get("sessionBackend").map(String::as_str) == Some(expected)
            && session.id != parent.session,
        "created session target does not match parent"
    );
    Ok(Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: session.id.clone(),
        selector: parent.selector.clone(),
        login: parent.login.clone(),
        backend: expected.into(),
        name: "Terminal".into(),
        target_signature: parent.target_signature.clone(),
        root: None,
        root_identity: None,
    })
}

pub(super) async fn create(
    state: &Arc<AppState>,
    device: &Device,
    parent: &Grant,
    admission: &Admission,
    outcome_unknown: &mut bool,
) -> Result<Resource> {
    let backend = backend(parent)?;
    let defaults = WorkspaceTerminalDefaults::new(
        state,
        &parent.selector,
        DEFAULT_COLS,
        DEFAULT_ROWS,
        DEFAULT_OUTPUT_FRAME_LIMIT,
        false,
        &parent.login,
        backend,
    )
    .map_err(anyhow::Error::msg)?;
    // Validate the full target/login immediately before submitting the mutation.
    // Do not clone parent commands or metadata into a new shell profile.
    let current = peer::grant(state, device, &parent.id).await?;
    ensure!(
        current.same_authority(parent),
        "parent authority changed before creation"
    );
    admission.submit_creation(parent)?;
    *outcome_unknown = true;
    // Keep provider session identities in the provider workspace. Never migrate
    // them into an Agent workspace or manufacture physical-client selectors.
    let session =
        workspace::create_workspace_session(state, &parent.selector, &defaults, HashMap::new())
            .map_err(|error| anyhow::anyhow!("session creation failed: {error:?}"))?
            .session;
    let result = async {
        let child = child_grant(parent, &session)?;
        let target = state.clone();
        let spec = session.terminal_spec(DEFAULT_COLS, DEFAULT_ROWS);
        // The same session manager used by the provider's terminal endpoint.
        // Await this owned worker after dispatch even if the connection closes.
        tokio::task::spawn_blocking(move || target.sessions.open_terminal(spec, true, false))
            .await
            .context("terminal creation worker stopped")??;
        state.sessions.mark_status(&session.id, "running");
        // Creation can outlive a revoke or target change. Recheck target identity
        // before publishing, then commit atomically with device management.
        let current = tokio::time::timeout(
            Duration::from_secs(30),
            peer::grant(state, device, &parent.id),
        )
        .await
        .context("parent authorization timed out after creation")??;
        ensure!(
            current.same_authority(parent),
            "parent authority changed during creation"
        );
        let _guard = state.remoted.mutation.lock().await;
        state
            .remoted
            .persist_created_grant(device, parent, child.clone())?;
        Ok::<_, anyhow::Error>(child.resource())
    }
    .await;
    if result.is_err() {
        // Only the freshly created session is eligible for cleanup. A failure is
        // kept explicit; never claim a successful rollback without confirmation.
        let closed = workspace::close_workspace_session(state, &session.id)
            .map_err(|error| anyhow::anyhow!("ungranted terminal cleanup failed: {error:?}"))?;
        state
            .sessions
            .close_sessions(closed.closed_session_ids.iter().map(String::as_str));
    }
    if result.is_ok() {
        *outcome_unknown = false;
    }
    result
}

#[cfg(test)]
mod tests;
