//! Durable host mutations, serialized with grant revocation and local management.
use super::Record;
use crate::{
    CreatedSession, SessionBackend,
    state::{Core, State, check_name},
};
use anyhow::{Context, Result, ensure};
use neko_hc_client::Client;
use serde_json::json;

impl Core {
    pub(crate) async fn connect_hc_locked(&self, state: &mut State) -> Result<Client> {
        let (client, next) = self.hc.connect(&state.saved.hc).await?;
        let mut saved = state.saved.clone();
        saved.hc = next;
        self.persist(&saved).await?;
        state.saved = saved;
        self.hc.connected(&client, &state.saved.hc);
        Ok(client)
    }
    pub(crate) async fn create_session(
        &self,
        backend: SessionBackend,
        name: String,
    ) -> Result<CreatedSession> {
        check_name(&name)?;
        match backend {
            SessionBackend::Native => Ok(CreatedSession {
                grant: self.create_terminal(name).await?,
                pane: None,
            }),
            SessionBackend::Herdr => {
                let mut state = self.state.lock().await;
                let result = async {
                    // Creating a workspace is an explicit local start action too.
                    self.herdr.start().await?;
                    self.herdr.create_session(&name).await
                }
                .await;
                state.revision = state
                    .revision
                    .checked_add(1)
                    .context("host revision exhausted")?;
                self.publish(&state)?;
                result
            }
            SessionBackend::Hc => self.create_hc(name).await,
        }
    }
    async fn create_hc(&self, name: String) -> Result<CreatedSession> {
        let mut state = self.state.lock().await;
        let result = async {
            ensure!(state.saved.hc.terminals.len() < 32, "hc session limit reached");
            let mut client = self.connect_hc_locked(&mut state).await?;
            let record = Record { grant: uuid::Uuid::new_v4().to_string(), name, address: None };
            let mut saved = state.saved.clone();
            saved.hc.terminals.push(record.clone());
            // Save the UUID/core binding BEFORE terminal.run, including when its reply is lost.
            self.persist(&saved).await?;
            state.saved = saved;
            self.hc.committed(&state.saved.hc);
            let terminal = client.call("terminal.run", json!({
                "name":format!("n{}", &uuid::Uuid::parse_str(&record.grant)?.simple().to_string()[..30]),
                "labels":state.saved.hc.labels(&record),"no_parent":true,"no_notify":true,
                "outlive_parent":true,"keep":true,
                "env":{"TERM":"xterm-256color","TERM_PROGRAM":"neoko-terminal","COLORTERM":"truecolor"}
            })).await?;
            let address = state.saved.hc.verify(&record, &terminal)?.to_owned();
            let mut saved = state.saved.clone();
            saved.hc.terminals.iter_mut().find(|r| r.grant == record.grant).context("hc intent disappeared")?.address = Some(address);
            self.persist(&saved).await?;
            state.saved = saved;
            self.hc.connected(&client, &state.saved.hc);
            // Drop/close detaches this connection only, independently of core's program.
            client.close().await?;
            Ok(CreatedSession { grant: record.grant, pane: None })
        }.await;
        if let Err(error) = &result {
            self.hc.error(error);
        }
        state.revision = state
            .revision
            .checked_add(1)
            .context("host revision exhausted")?;
        self.publish(&state)?;
        result
    }
    pub(crate) async fn close_hc(&self, grant: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        ensure!(
            state.saved.hc.terminals.iter().any(|r| r.grant == grant),
            "hc resource not found"
        );
        let result = async {
            let mut saved = state.saved.clone();
            let mut affected = Vec::new();
            for device in &mut saved.devices {
                if device.resources.iter().any(|r| r == grant) {
                    device.resources.retain(|r| r != grant);
                    affected.push(device.id.clone());
                }
            }
            self.persist(&saved).await?;
            state.saved = saved;
            for device in affected {
                self.cancel_device(&state, &device);
            }
            let terminal = self.hc.terminate(grant).await?;
            let mut saved = state.saved.clone();
            saved.hc.terminals.retain(|r| r.grant != grant);
            self.persist(&saved).await?;
            state.saved = saved;
            self.hc.committed(&state.saved.hc);
            self.hc.closed(grant, &terminal);
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            self.hc.error(error);
        }
        state.revision = state
            .revision
            .checked_add(1)
            .context("host revision exhausted")?;
        self.publish(&state)?;
        result
    }
}
