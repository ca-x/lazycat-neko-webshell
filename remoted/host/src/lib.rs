//! Native desktop host. Persist the opaque saved string in a host-specific protected
//! credential slot. Local PTYs share this registry with all authorized remote peers.
mod hc;
mod herdr;
mod peer;
mod relay;
mod session;
mod state;
pub mod terminals;

use anyhow::{Context, Result, ensure};
use futures_util::future::BoxFuture;
pub use neko_remoted_protocol::messages::{
    Operation, Permission, Resource, Response, SessionBackend,
};
use state::Core;
use std::{fmt, sync::Arc};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

pub use hc::{HcSnapshot, HcStatus};
pub use herdr::{HerdrSnapshot, HerdrStatus};

/// The granted resource and optional Herdr pane created by an explicit local action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedSession {
    pub grant: String,
    pub pane: Option<String>,
}

pub type Save = Arc<dyn Fn(String) -> BoxFuture<'static, Result<()>> + Send + Sync>;

#[derive(Clone)]
pub struct HostSnapshot {
    pub revision: u64,
    pub herdr: HerdrSnapshot,
    pub hc: HcSnapshot,
    pub enabled: bool,
    pub relay: String,
    pub status: String,
    pub error: String,
    pub invitation: Option<String>,
    pub pending: Vec<PendingDevice>,
    pub devices: Vec<DeviceSummary>,
    pub resources: Vec<Resource>,
}
impl fmt::Debug for HostSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostSnapshot")
            .field("revision", &self.revision)
            .field("herdr", &self.herdr)
            .field("hc", &self.hc)
            .field("enabled", &self.enabled)
            .field("relay", &self.relay)
            .field("status", &self.status)
            .field("error", &self.error)
            .field(
                "invitation",
                &self.invitation.as_ref().map(|_| "[redacted]"),
            )
            .field("pending", &self.pending)
            .field("devices", &self.devices)
            .field("resources", &self.resources)
            .finish()
    }
}
#[derive(Clone, Debug)]
pub struct PendingDevice {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}
#[derive(Clone, Debug)]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub permission: Permission,
    pub resources: Vec<String>,
    pub online: bool,
    pub rtt_ms: Option<u64>,
}
#[derive(Debug)]
pub enum Action {
    RefreshHc {
        revision: u64,
    },
    ConnectHc {
        revision: u64,
    },
    RefreshHerdr {
        revision: u64,
    },
    StartHerdr {
        revision: u64,
    },
    Configure {
        revision: u64,
        relay: String,
        enabled: bool,
    },
    Invite {
        revision: u64,
    },
    Approve {
        revision: u64,
        pending: String,
        permission: Permission,
        resources: Vec<String>,
    },
    Revoke {
        revision: u64,
        device: String,
    },
    SetPermission {
        revision: u64,
        device: String,
        permission: Permission,
        resources: Vec<String>,
    },
}
impl Action {
    fn revision(&self) -> u64 {
        match self {
            Self::RefreshHc { revision }
            | Self::ConnectHc { revision }
            | Self::RefreshHerdr { revision }
            | Self::StartHerdr { revision }
            | Self::Configure { revision, .. }
            | Self::Invite { revision }
            | Self::Approve { revision, .. }
            | Self::Revoke { revision, .. }
            | Self::SetPermission { revision, .. } => *revision,
        }
    }
}

#[derive(Clone)]
pub struct Host(Arc<Owner>);
struct Owner {
    core: Arc<Core>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.core.shutdown.cancel();
    }
}

/// Initialize and durably save an independent identity before exposing any host
/// state or opening a Relay. Hosting defaults to disabled; local terminals work
/// without configuring or contacting a Relay.
pub async fn start(saved: Option<&str>, save: Save) -> Result<Host> {
    let core = Core::start(saved, save).await?;
    let host = Host(Arc::new(Owner { core: core.clone() }));
    tokio::spawn(relay::supervise(core.clone()));
    tokio::spawn(watch_herdr(core.clone()));
    tokio::spawn(watch_hc(core.clone()));
    // Unit tests discover only explicitly isolated fixtures, never the user's live session.
    #[cfg(not(test))]
    tokio::spawn(async move {
        let _ = core.herdr.refresh().await;
        let mut state = core.state.lock().await;
        state.revision = state.revision.saturating_add(1);
        let _ = core.publish(&state);
    });
    Ok(host)
}
// Only failure notifications originate in background screen readers. No periodic probing.
async fn watch_herdr(core: Arc<Core>) {
    loop {
        tokio::select! {
            biased;
            () = core.shutdown.cancelled() => return,
            () = core.herdr.changed.notified() => {}
        }
        let mut state = core.state.lock().await;
        state.revision = state.revision.saturating_add(1);
        let _ = core.publish(&state);
    }
}
async fn watch_hc(core: Arc<Core>) {
    loop {
        tokio::select! {
            biased;
            () = core.shutdown.cancelled() => return,
            () = core.hc.changed.notified() => {}
        }
        let mut state = core.state.lock().await;
        state.revision = state.revision.saturating_add(1);
        let _ = core.publish(&state);
    }
}
impl Host {
    pub fn snapshot(&self) -> HostSnapshot {
        self.0.core.snapshots.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<HostSnapshot> {
        self.0.core.snapshots.subscribe()
    }
    pub async fn manage(&self, action: Action) -> Result<()> {
        self.0.core.manage(action).await
    }
    /// Returns a bounded local-only channel without pairing or Ready events.
    pub async fn local(&self) -> Result<LocalSession> {
        ensure!(!self.0.core.shutdown.is_cancelled(), "host stopped");
        let (requests, incoming) = mpsc::channel(8);
        let (outgoing, responses) = mpsc::channel(32);
        let cancel = self.0.core.shutdown.child_token();
        let subscriber = uuid::Uuid::new_v4().to_string();
        tokio::spawn(session::serve(
            self.0.core.clone(),
            session::Principal::Local,
            subscriber,
            incoming,
            outgoing,
            cancel.clone(),
        ));
        Ok(LocalSession {
            _host: self.clone(),
            requests,
            responses,
            cancel,
            next: 1,
        })
    }
    pub async fn create_terminal(&self, name: String) -> Result<String> {
        self.0.core.create_terminal(name).await
    }
    /// Create a session only from an explicit local user action.
    pub async fn create_session(
        &self,
        backend: SessionBackend,
        name: String,
    ) -> Result<CreatedSession> {
        self.0.core.create_session(backend, name).await
    }
    pub async fn close_terminal(&self, id: &str) -> Result<()> {
        self.0.core.close_terminal(id).await
    }
}
/// Dropping or closing detaches all streams; the registry keeps the PTYs alive.
pub struct LocalSession {
    _host: Host,
    requests: mpsc::Sender<neko_remoted_protocol::messages::Request>,
    responses: mpsc::Receiver<Response>,
    cancel: CancellationToken,
    next: u64,
}
impl LocalSession {
    pub async fn request(&mut self, operation: Operation) -> Result<u64> {
        ensure!(!self.cancel.is_cancelled(), "local session closed");
        let id = self.next;
        self.next = self.next.checked_add(1).context("request IDs exhausted")?;
        let request = neko_remoted_protocol::messages::Request { id, operation };
        neko_remoted_protocol::pack(&request)?;
        self.requests
            .try_send(request)
            .context("local request queue full or closed")?;
        Ok(id)
    }
    pub async fn receive(&mut self) -> Result<Response> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => anyhow::bail!("local session closed"),
            response = self.responses.recv() => response.context("local session closed"),
        }
    }
    pub async fn close(self) {
        self.cancel.cancel();
    }
}
impl Drop for LocalSession {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests;
