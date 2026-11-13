//! A write grant to a local native terminal or local Herdr also permits a sibling
//! native shell as the host login user. HC grants do not establish that identity.
//! Children are saved only for the creating device, so reconnect lists them without
//! replaying creation. No client-selected command, identity or target is accepted.
use crate::{
    session::{Principal, authorize, resources},
    state::{Core, DEADLINE},
};
use anyhow::{Context, Result, ensure};
use neko_remoted_protocol::messages::Resource;
use tokio_util::sync::CancellationToken;

impl Core {
    pub(crate) async fn create_granted_terminal(
        &self,
        principal: &Principal,
        parent: &str,
        cancel: &CancellationToken,
        outcome_unknown: &mut bool,
    ) -> Result<Resource> {
        // Called outside session dispatch's state guard. Keep authorization,
        // creation and grant persistence serialized with local revoke/management.
        let mut state = self.state.lock().await;
        ensure!(!cancel.is_cancelled(), "session cancelled before creation");
        let device = authorize(&state, principal)?;
        ensure!(
            device.is_none_or(|device| device.permission.can_write()),
            "read-only device cannot create terminals"
        );
        ensure!(
            device.is_none_or(|device| device.resources.len() < 32),
            "device resource limit reached"
        );
        let allowed = resources(self, device)?;
        ensure!(
            allowed.iter().any(|resource| resource.id == parent
                && ((resource.backend == "native" && resource.terminal)
                    || (resource.backend == "herdr" && resource.herdr))),
            "parent does not authorize a shell as the host login user"
        );
        let next_revision = state
            .revision
            .checked_add(1)
            .context("host revision exhausted")?;
        *outcome_unknown = true;
        // Registry creation uses the same trusted default profile as the local
        // Native create path. Do not call Core::create_terminal under this guard.
        let id = tokio::time::timeout(DEADLINE, self.terminals.create("Terminal".into(), 80, 24))
            .await
            .context("terminal creation timed out; do not retry")??;
        let resource = Resource {
            id: id.clone(),
            name: "Terminal".into(),
            backend: "native".into(),
            terminal: true,
            herdr: false,
            files: false,
        };
        if let Principal::Device {
            id: device,
            generation,
        } = principal
        {
            let mut saved = state.saved.clone();
            saved
                .devices
                .iter_mut()
                .find(|item| item.id == *device && item.generation == *generation)
                .context("device authorization changed")?
                .resources
                .push(id.clone());
            if let Err(error) = self.persist(&saved).await {
                // A failed grant commit must not expose an ungranted live shell.
                tokio::time::timeout(DEADLINE, self.terminals.close(&id))
                    .await
                    .context("ungranted terminal cleanup timed out")??;
                return Err(error);
            }
            state.saved = saved;
        }
        state.revision = next_revision;
        self.publish(&state)?;
        *outcome_unknown = false;
        Ok(resource)
    }
}

#[cfg(all(test, unix))]
mod tests;
