use crate::{
    Action, DeviceSummary, HostSnapshot, PendingDevice, Permission, Resource, Save,
    terminals::TerminalRegistry,
};
use anyhow::{Context, Result, ensure};
use neko_remoted_protocol::{self as protocol, RelayAddress, code::DeviceCode, crypto::Identity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Notify, oneshot, watch};
use tokio_util::sync::CancellationToken;

pub(crate) const DEADLINE: Duration = Duration::from_secs(10);
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Device {
    pub id: String,
    pub name: String,
    pub key: String,
    pub permission: Permission,
    pub resources: Vec<String>,
    pub generation: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    #[serde(default)]
    pub hc: crate::hc::SavedHc,
    schema: u8,
    pub enabled: bool,
    pub relay: String,
    private_key: String,
    public_key: String,
    route_secret: String,
    pub devices: Vec<Device>,
}
pub(crate) struct Invitation {
    pub code: DeviceCode,
    pub expires: Instant,
}
pub(crate) struct Pending {
    pub name: String,
    pub key: String,
    pub expires: Instant,
    pub decision: oneshot::Sender<Device>,
    pub cancel: CancellationToken,
}
pub(crate) struct Live {
    pub device: String,
    pub cancel: CancellationToken,
    pub rtt: Option<(u64, Instant)>,
}
pub(crate) struct State {
    pub saved: Saved,
    pub revision: u64,
    pub generation: u64,
    pub status: String,
    pub error: String,
    pub invitation: Option<Invitation>,
    pub pending: HashMap<String, Pending>,
    pub live: HashMap<String, Live>,
    pub relay_cancel: CancellationToken,
}
pub(crate) struct Core {
    pub state: Mutex<State>,
    pub snapshots: watch::Sender<HostSnapshot>,
    pub terminals: TerminalRegistry,
    pub herdr: Arc<crate::herdr::Herdr>,
    pub hc: Arc<crate::hc::Hc>,
    pub identity: Identity,
    pub route: String,
    pub route_secret: String,
    pub save: Save,
    pub changed: Notify,
    pub shutdown: CancellationToken,
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 160 && !name.chars().any(char::is_control)
}
pub(crate) fn check_name(name: &str) -> Result<()> {
    ensure!(valid_name(name), "invalid display name");
    Ok(())
}
impl Core {
    pub async fn start(input: Option<&str>, save: Save) -> Result<Arc<Self>> {
        let saved = match input {
            Some(input) => {
                ensure!(input.len() <= 128 * 1024, "saved host state exceeds limit");
                serde_json::from_str::<Saved>(input).context("invalid saved host state")?
            }
            None => {
                let identity = Identity::generate()?;
                let (private, public) = identity.protected_export();
                Saved {
                    hc: crate::hc::SavedHc::default(),
                    schema: 1,
                    enabled: false,
                    relay: String::new(),
                    private_key: protocol::encode(private),
                    public_key: protocol::encode(public),
                    route_secret: protocol::encode(&protocol::random32()?),
                    devices: Vec::new(),
                }
            }
        };
        saved.hc.validate()?;
        ensure!(
            saved.schema == 1 && saved.devices.len() <= protocol::MAX_DEVICES,
            "unsupported saved host state"
        );
        if !saved.relay.is_empty() {
            ensure!(
                RelayAddress::parse(&saved.relay)?.origin() == saved.relay,
                "noncanonical saved relay"
            );
        }
        ensure!(
            !saved.enabled || !saved.relay.is_empty(),
            "enabled host requires a relay"
        );
        let private = protocol::decode32(&saved.private_key)?;
        let public = protocol::decode32(&saved.public_key)?;
        ensure!(
            x25519_dalek::x25519(private, x25519_dalek::X25519_BASEPOINT_BYTES) == public,
            "saved host key mismatch"
        );
        let secret = protocol::decode32(&saved.route_secret)?;
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        for device in &saved.devices {
            check_name(&device.name)?;
            ensure!(
                uuid::Uuid::parse_str(&device.id).is_ok()
                    && ids.insert(&device.id)
                    && keys.insert(&device.key)
                    && device.generation > 0,
                "invalid saved device identity"
            );
            protocol::decode32(&device.key)?;
            ensure!(
                device.resources.len() <= 32
                    && device
                        .resources
                        .iter()
                        .all(|id| uuid::Uuid::parse_str(id).is_ok())
                    && device.resources.iter().collect::<HashSet<_>>().len()
                        == device.resources.len(),
                "invalid saved grants"
            );
        }
        let encoded = serde_json::to_string(&saved)?;
        tokio::time::timeout(DEADLINE, save(encoded))
            .await
            .context("host identity save timed out")??;
        let snapshot = HostSnapshot {
            revision: 0,
            herdr: crate::HerdrSnapshot::default(),
            hc: crate::HcSnapshot::default(),
            enabled: saved.enabled,
            relay: saved.relay.clone(),
            status: if saved.enabled {
                "connecting"
            } else {
                "disabled"
            }
            .into(),
            error: String::new(),
            invitation: None,
            pending: Vec::new(),
            devices: Vec::new(),
            resources: Vec::new(),
        };
        let shutdown = CancellationToken::new();
        let hc = Arc::new(crate::hc::Hc::new(saved.hc.clone(), shutdown.child_token()));
        let core = Arc::new(Self {
            identity: Identity::restore(private, public),
            route: protocol::route(&secret),
            route_secret: protocol::encode(&secret),
            state: Mutex::new(State {
                saved,
                revision: 0,
                generation: 0,
                status: snapshot.status.clone(),
                error: String::new(),
                invitation: None,
                pending: HashMap::new(),
                live: HashMap::new(),
                relay_cancel: shutdown.child_token(),
            }),
            snapshots: watch::channel(snapshot).0,
            terminals: TerminalRegistry::default(),
            herdr: Arc::new(crate::herdr::Herdr::default()),
            hc,
            save,
            changed: Notify::new(),
            shutdown,
        });
        {
            let state = core.state.lock().await;
            core.publish(&state)?;
        }
        Ok(core)
    }
    pub fn resources(&self) -> Result<Vec<Resource>> {
        let mut resources: Vec<_> = self
            .terminals
            .list()?
            .into_iter()
            .map(|terminal| Resource {
                id: terminal.id,
                name: terminal.name,
                backend: "native".into(),
                terminal: true,
                herdr: false,
                files: false,
            })
            .collect();
        if let Some(resource) = self.herdr.resource() {
            resources.push(resource);
        }
        resources.extend(self.hc.resources());
        Ok(resources)
    }
    pub fn publish(&self, state: &State) -> Result<()> {
        let mut pending: Vec<_> = state
            .pending
            .iter()
            .filter(|(_, p)| p.expires > Instant::now() && !p.cancel.is_cancelled())
            .map(|(id, p)| PendingDevice {
                id: id.clone(),
                name: p.name.clone(),
                fingerprint: protocol::fingerprint(
                    &protocol::decode32(&p.key).expect("validated Noise key"),
                ),
            })
            .collect();
        pending.sort_by(|a, b| a.id.cmp(&b.id));
        let devices = state
            .saved
            .devices
            .iter()
            .map(|device| {
                let live: Vec<_> = state
                    .live
                    .values()
                    .filter(|p| p.device == device.id && !p.cancel.is_cancelled())
                    .collect();
                DeviceSummary {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    fingerprint: protocol::fingerprint(
                        &protocol::decode32(&device.key).expect("validated saved key"),
                    ),
                    permission: device.permission,
                    resources: device.resources.clone(),
                    online: !live.is_empty(),
                    rtt_ms: live
                        .iter()
                        .filter_map(|p| {
                            p.rtt
                                .filter(|(_, at)| at.elapsed() < Duration::from_secs(30))
                                .map(|(rtt, _)| rtt)
                        })
                        .max(),
                }
            })
            .collect();
        self.snapshots.send_replace(HostSnapshot {
            revision: state.revision,
            enabled: state.saved.enabled,
            relay: state.saved.relay.clone(),
            status: state.status.clone(),
            error: state.error.clone(),
            invitation: state
                .invitation
                .as_ref()
                .filter(|i| i.expires > Instant::now())
                .map(|i| i.code.display()),
            pending,
            devices,
            resources: self.resources()?,
            herdr: self.herdr.snapshot(),
            hc: self.hc.snapshot(),
        });
        Ok(())
    }
    pub(crate) async fn persist(&self, saved: &Saved) -> Result<()> {
        tokio::time::timeout(DEADLINE, (self.save)(serde_json::to_string(saved)?))
            .await
            .context("host state save timed out")??;
        Ok(())
    }
    pub async fn manage(&self, action: Action) -> Result<()> {
        let mut state = self.state.lock().await;
        ensure!(state.revision == action.revision(), "stale host revision");
        let mut saved = state.saved.clone();
        match action {
            Action::RefreshHc { .. } | Action::ConnectHc { .. } => {
                let result = if matches!(action, Action::ConnectHc { .. }) {
                    self.connect_hc_locked(&mut state).await.map(|_| ())
                } else {
                    self.hc.refresh(&state.saved.hc)
                };
                if let Err(error) = &result {
                    self.hc.error(error);
                }
                state.revision = state
                    .revision
                    .checked_add(1)
                    .context("host revision exhausted")?;
                self.publish(&state)?;
                return result;
            }
            Action::RefreshHerdr { .. } | Action::StartHerdr { .. } => {
                let result = if matches!(action, Action::StartHerdr { .. }) {
                    self.herdr.start().await
                } else {
                    self.herdr.refresh().await
                };
                state.revision = state
                    .revision
                    .checked_add(1)
                    .context("host revision exhausted")?;
                self.publish(&state)?;
                return result;
            }
            Action::Configure { relay, enabled, .. } => {
                saved.relay = if relay.trim().is_empty() {
                    ensure!(!enabled, "Relay address required");
                    String::new()
                } else {
                    RelayAddress::parse(&relay)?.origin().into()
                };
                saved.enabled = enabled;
                self.persist(&saved).await?;
                state.saved = saved;
                state.generation = state
                    .generation
                    .checked_add(1)
                    .context("host generation exhausted")?;
                state.relay_cancel.cancel();
                state.invitation = None;
                for p in state.pending.values() {
                    p.cancel.cancel();
                }
                state.pending.clear();
                for p in state.live.values() {
                    p.cancel.cancel();
                }
                state.status = if enabled { "connecting" } else { "disabled" }.into();
                state.error.clear();
                self.changed.notify_one();
            }
            Action::Invite { .. } => {
                ensure!(saved.enabled, "hosting is disabled");
                for p in state.pending.values() {
                    p.cancel.cancel();
                }
                state.pending.clear();
                let code = DeviceCode::generate(&self.route, &RelayAddress::parse(&saved.relay)?)?;
                state.invitation = Some(Invitation {
                    code,
                    expires: Instant::now() + Duration::from_secs(300),
                });
            }
            Action::Approve {
                pending,
                permission,
                resources,
                ..
            } => {
                self.validate_grants(&resources)?;
                ensure!(
                    saved.enabled && saved.devices.len() < protocol::MAX_DEVICES,
                    "host disabled or device limit reached"
                );
                let selected = state
                    .pending
                    .get(&pending)
                    .context("pending device no longer available")?;
                ensure!(
                    selected.expires > Instant::now()
                        && !selected.cancel.is_cancelled()
                        && !selected.decision.is_closed(),
                    "pairing expired"
                );
                ensure!(
                    !saved.devices.iter().any(|d| d.key == selected.key),
                    "device already paired; reconnect instead"
                );
                let device = Device {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: selected.name.clone(),
                    key: selected.key.clone(),
                    permission,
                    resources,
                    generation: 1,
                };
                saved.devices.push(device.clone());
                self.persist(&saved).await?;
                state.saved = saved;
                let selected = state
                    .pending
                    .remove(&pending)
                    .context("pending device disappeared")?;
                let _ = selected.decision.send(device);
            }
            Action::Revoke { device, .. } => {
                ensure!(
                    saved.devices.iter().any(|d| d.id == device),
                    "device not found"
                );
                saved.devices.retain(|d| d.id != device);
                self.persist(&saved).await?;
                state.saved = saved;
                self.cancel_device(&state, &device);
            }
            Action::SetPermission {
                device,
                permission,
                resources,
                ..
            } => {
                self.validate_grants(&resources)?;
                let selected = saved
                    .devices
                    .iter_mut()
                    .find(|d| d.id == device)
                    .context("device not found")?;
                selected.permission = permission;
                selected.resources = resources;
                selected.generation = selected
                    .generation
                    .checked_add(1)
                    .context("device generation exhausted")?;
                self.persist(&saved).await?;
                state.saved = saved;
                self.cancel_device(&state, &device);
            }
        }
        state.revision = state
            .revision
            .checked_add(1)
            .context("host revision exhausted")?;
        self.publish(&state)
    }
    fn validate_grants(&self, grants: &[String]) -> Result<()> {
        let resources = self.resources()?;
        ensure!(
            grants.len() <= 32
                && grants.iter().collect::<HashSet<_>>().len() == grants.len()
                && grants
                    .iter()
                    .all(|id| resources.iter().any(|r| r.id == *id)),
            "select explicit existing resources"
        );
        Ok(())
    }
    pub(crate) fn cancel_device(&self, state: &State, id: &str) {
        for peer in state.live.values().filter(|p| p.device == id) {
            peer.cancel.cancel();
        }
    }
    pub async fn create_terminal(&self, name: String) -> Result<String> {
        check_name(&name)?;
        let mut state = self.state.lock().await;
        ensure!(self.terminals.list()?.len() < 32, "terminal limit reached");
        let id = tokio::time::timeout(DEADLINE, self.terminals.create(name, 80, 24))
            .await
            .context("terminal creation timed out")??;
        state.revision += 1;
        self.publish(&state)?;
        Ok(id)
    }
    pub async fn close_terminal(&self, id: &str) -> Result<()> {
        if self.hc.contains(id) {
            return self.close_hc(id).await;
        }
        let mut state = self.state.lock().await;
        ensure!(
            self.terminals.list()?.iter().any(|t| t.id == id),
            "terminal not found"
        );
        let mut saved = state.saved.clone();
        let affected: Vec<_> = saved
            .devices
            .iter_mut()
            .filter_map(|d| {
                if d.resources.iter().any(|r| r == id) {
                    d.resources.retain(|r| r != id);
                    Some(d.id.clone())
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            self.persist(&saved).await?;
            state.saved = saved;
        }
        for device in affected {
            self.cancel_device(&state, &device);
        }
        let result = tokio::time::timeout(DEADLINE, self.terminals.close(id))
            .await
            .context("terminal close timed out")?;
        state.revision += 1;
        self.publish(&state)?;
        result
    }
    pub async fn disconnected(&self, subscriber: &str) {
        self.terminals.detach_all(subscriber);
        self.herdr.detach_all(subscriber);
        self.hc.detach_all(subscriber);
        let mut state = self.state.lock().await;
        state.live.remove(subscriber);
        if state.pending.remove(subscriber).is_some() {
            state.revision += 1;
        }
        let _ = self.publish(&state);
    }
}
