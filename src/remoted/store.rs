use crate::database::AppDatabase;
use anyhow::{Context, Result};
use neko_remoted_protocol::{RelayAddress, crypto::Identity, messages::Permission};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootIdentity {
    pub device: u64,
    pub inode: u64,
    pub uid: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grant {
    pub id: String,
    pub session: String,
    pub selector: String,
    pub login: String,
    pub backend: String,
    pub name: String,
    pub target_signature: String,
    pub root: Option<String>,
    pub root_identity: Option<RootIdentity>,
}
impl Grant {
    pub fn resource(&self) -> neko_remoted_protocol::messages::Resource {
        neko_remoted_protocol::messages::Resource {
            id: self.id.clone(),
            name: self.name.clone(),
            backend: self.backend.clone(),
            terminal: true,
            herdr: self.backend == "herdr",
            files: self.root.is_some(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub public_key: String,
    pub permission: Permission,
    pub grants: Vec<Grant>,
    pub generation: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Saved {
    pub version: u8,
    pub owner: String,
    pub enabled: bool,
    pub relay: Option<RelayAddress>,
    #[serde(default)]
    pub registration_key: Option<String>,
    pub routing_secret: String,
    pub private_key: String,
    pub public_key: String,
    pub devices: Vec<Device>,
}
impl Saved {
    pub fn new() -> Result<Self> {
        let id = Identity::generate()?;
        let (private, public) = id.protected_export();
        Ok(Self {
            version: 1,
            owner: String::new(),
            enabled: false,
            relay: None,
            registration_key: None,
            routing_secret: neko_remoted_protocol::encode(&neko_remoted_protocol::random32()?),
            private_key: neko_remoted_protocol::encode(private),
            public_key: neko_remoted_protocol::encode(public),
            devices: vec![],
        })
    }
    pub fn identity(&self) -> Result<Identity> {
        Ok(Identity::restore(
            neko_remoted_protocol::decode32(&self.private_key)?,
            neko_remoted_protocol::decode32(&self.public_key)?,
        ))
    }
    pub fn route(&self) -> Result<String> {
        Ok(neko_remoted_protocol::route(
            &neko_remoted_protocol::decode32(&self.routing_secret)?,
        ))
    }
}
pub struct Pending {
    pub id: String,
    pub name: String,
    pub key: String,
    pub created: Instant,
    pub decision: oneshot::Sender<Device>,
}
pub struct Peer {
    pub path: Option<neko_remoted_transport::Sender>,
    pub device: Option<String>,
    pub cancel: CancellationToken,
    pub rtt: Option<(u64, Instant)>,
}
pub struct Runtime {
    pub status: String,
    pub error: String,
    pub relay_rtt: Option<(u64, Instant)>,
    pub pending: HashMap<String, Pending>,
    pub peers: HashMap<String, Peer>,
    pub invitation: Option<(neko_remoted_protocol::code::DeviceCode, Instant)>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            status: "disabled".into(),
            error: String::new(),
            relay_rtt: None,
            pending: HashMap::new(),
            peers: HashMap::new(),
            invitation: None,
        }
    }
}
pub struct Remoted {
    pub operations: Mutex<super::admission::Operations>,
    pub mutation: tokio::sync::Mutex<()>,
    pub database: Arc<AppDatabase>,
    pub saved: Mutex<Saved>,
    pub runtime: Mutex<Runtime>,
    pub configuration: watch::Sender<u64>,
}
impl Remoted {
    pub fn load(database: Arc<AppDatabase>) -> Result<Self> {
        let saved = match database.load_kv("remoted-v1", "configuration")? {
            Some(b) => serde_json::from_slice(&b).context("invalid remoted configuration")?,
            None => Saved::new()?,
        };
        let (configuration, _) = watch::channel(0);
        Ok(Self {
            operations: Mutex::new(super::admission::Operations::default()),
            mutation: tokio::sync::Mutex::new(()),
            database,
            saved: Mutex::new(saved),
            runtime: Mutex::new(Runtime::default()),
            configuration,
        })
    }
    pub fn snapshot(&self) -> Result<Saved> {
        Ok(self
            .saved
            .lock()
            .map_err(|_| anyhow::anyhow!("remote settings unavailable"))?
            .clone())
    }
    pub fn persist(&self, saved: Saved) -> Result<()> {
        let mut current = self
            .saved
            .lock()
            .map_err(|_| anyhow::anyhow!("remote settings unavailable"))?;
        self.database
            .store_kv("remoted-v1", "configuration", &serde_json::to_vec(&saved)?)?;
        *current = saved;
        Ok(())
    }
    pub fn restart(&self) {
        self.configuration.send_modify(|v| *v = v.wrapping_add(1));
    }
    pub fn cancel_device(&self, id: &str) {
        if let Ok(rt) = self.runtime.lock() {
            for p in rt
                .peers
                .values()
                .filter(|p| p.device.as_deref() == Some(id))
            {
                p.cancel.cancel();
            }
        }
    }
    pub fn current_device(&self, id: &str, generation: u64) -> Result<Device> {
        let s = self.snapshot()?;
        anyhow::ensure!(s.enabled, "远程控制已关闭");
        s.devices
            .into_iter()
            .find(|d| d.id == id && d.generation == generation)
            .context("设备授权已撤销或变化")
    }
}
