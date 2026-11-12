//! Host-owned persistent hc terminals. Remote peers receive grants, never core endpoints.
mod launch;
mod model;
mod state;
mod stream;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, ensure};
use launch::Launch;
pub(crate) use model::{Record, SavedHc};
use neko_hc_client::Client;
use neko_remoted_protocol::messages::{Event, Resource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
use std::path::PathBuf;
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Availability of the explicitly selected, isolated native hc namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HcStatus {
    Stopped,
    NotInstalled,
    Ready,
    Error,
}
/// Local installation guidance and the observed core identity. No install command is run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HcSnapshot {
    pub status: HcStatus,
    pub version: Option<String>,
    pub core_id: Option<String>,
    pub error: String,
    pub install_url: String,
}
impl Default for HcSnapshot {
    fn default() -> Self {
        Self {
            status: HcStatus::Stopped,
            version: None,
            core_id: None,
            error: String::new(),
            install_url: "https://github.com/herdrdev/herdr-core#installation".into(),
        }
    }
}
struct Observed {
    snapshot: HcSnapshot,
    launch: Option<Launch>,
    saved: SavedHc,
}
struct Attachment {
    grant: String,
    subscriber: String,
    read_only: bool,
    controller: bool,
    commands: mpsc::Sender<Command>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
}
enum Command {
    Input(Vec<u8>, oneshot::Sender<Result<()>>),
    Resize(u16, u16, oneshot::Sender<Result<()>>),
    Focus(bool, Option<oneshot::Sender<Result<()>>>),
}
pub(crate) struct Hc {
    observed: Mutex<Observed>,
    attachments: Mutex<HashMap<String, Attachment>>,
    shutdown: CancellationToken,
    pub changed: tokio::sync::Notify,
    // Only unit tests can supply an executable or endpoint root.
    #[cfg(test)]
    program: Option<PathBuf>,
    #[cfg(test)]
    root: Option<PathBuf>,
}
impl Hc {
    pub fn new(saved: SavedHc, shutdown: CancellationToken) -> Self {
        Self {
            observed: Mutex::new(Observed {
                snapshot: HcSnapshot::default(),
                launch: None,
                saved,
            }),
            attachments: Mutex::new(HashMap::new()),
            shutdown,
            changed: tokio::sync::Notify::new(),
            #[cfg(test)]
            program: None,
            #[cfg(test)]
            root: None,
        }
    }
    #[cfg(test)]
    pub(super) fn with_program(
        saved: SavedHc,
        shutdown: CancellationToken,
        program: PathBuf,
        root: PathBuf,
    ) -> Self {
        Self {
            program: Some(program),
            root: Some(root),
            ..Self::new(saved, shutdown)
        }
    }
    pub fn snapshot(&self) -> HcSnapshot {
        self.observed
            .lock()
            .expect("hc state lock")
            .snapshot
            .clone()
    }
    pub fn resources(&self) -> Vec<Resource> {
        let observed = self.observed.lock().expect("hc state lock");
        if observed.snapshot.status != HcStatus::Ready {
            return Vec::new();
        }
        observed
            .saved
            .terminals
            .iter()
            .filter(|r| r.address.is_some())
            .map(|r| Resource {
                id: r.grant.clone(),
                name: r.name.clone(),
                backend: "hc".into(),
                terminal: true,
                herdr: false,
                files: false,
            })
            .collect()
    }
    pub fn contains(&self, id: &str) -> bool {
        self.observed
            .lock()
            .expect("hc state lock")
            .saved
            .terminals
            .iter()
            .any(|r| r.grant == id)
    }
    pub fn committed(&self, saved: &SavedHc) {
        self.observed.lock().expect("hc state lock").saved = saved.clone();
    }
    pub fn error(&self, error: &anyhow::Error) {
        let mut observed = self.observed.lock().expect("hc state lock");
        if observed.snapshot.status != HcStatus::NotInstalled {
            observed.snapshot.status = HcStatus::Error;
        }
        observed.snapshot.error = error.to_string();
        drop(observed);
        self.changed.notify_one();
    }
    pub fn refresh(&self, saved: &SavedHc) -> Result<()> {
        #[cfg(test)]
        let launch = Launch::discover(
            &saved.namespace,
            self.program.as_deref(),
            self.root.as_deref(),
        )?;
        #[cfg(not(test))]
        let launch = Launch::discover(&saved.namespace, None, None)?;
        let mut observed = self.observed.lock().expect("hc state lock");
        if launch.is_none() {
            observed.snapshot.status = HcStatus::NotInstalled;
        } else if observed.launch.as_ref().map(|l| &l.binary) != launch.as_ref().map(|l| &l.binary)
            || observed.snapshot.status != HcStatus::Ready
        {
            observed.snapshot.status = HcStatus::Stopped;
        }
        observed.snapshot.error.clear();
        observed.launch = launch;
        Ok(())
    }
    /// Only explicit local Connect/create actions call this with permission to start a core.
    pub async fn connect(&self, saved: &SavedHc) -> Result<(Client, SavedHc)> {
        self.refresh(saved)?;
        let launch = self
            .observed
            .lock()
            .expect("hc state lock")
            .launch
            .clone()
            .context("hc is not installed; use the official installation guide")?;
        let mut client = launch.connect(&saved.namespace, true).await?;
        verify_core(&client, saved)?;
        let mut next = saved.clone();
        next.core_id = Some(core_id(&client)?);
        let list = client
            .call(
                "terminal.list",
                json!({"all":true,"labels":{"neoko_host":saved.namespace}}),
            )
            .await?;
        for (index, record) in saved.terminals.iter().enumerate() {
            if let Some(terminal) = saved.resolve(record, &list)? {
                next.terminals[index].address = Some(saved.verify(record, &terminal)?.into());
            }
        }
        // The caller persists next before publishing Ready or performing any terminal.run.
        Ok((client, next))
    }
    pub fn connected(&self, client: &Client, saved: &SavedHc) {
        let mut observed = self.observed.lock().expect("hc state lock");
        observed.saved = saved.clone();
        observed.snapshot.status = HcStatus::Ready;
        observed.snapshot.core_id = saved.core_id.clone();
        observed.snapshot.version = client.hello()["version"].as_str().map(str::to_owned);
        observed.snapshot.error = if saved.terminals.iter().any(|r| r.address.is_none()) {
            "A previous creation outcome is unknown; it was not repeated".into()
        } else {
            String::new()
        };
    }
    async fn connection(&self) -> Result<(Client, SavedHc)> {
        let (launch, saved) = {
            let observed = self.observed.lock().expect("hc state lock");
            ensure!(
                observed.snapshot.status == HcStatus::Ready,
                "hc is not connected; use Connect hc explicitly"
            );
            (
                observed
                    .launch
                    .clone()
                    .context("hc executable unavailable")?,
                observed.saved.clone(),
            )
        };
        let client = launch.connect(&saved.namespace, false).await?;
        verify_core(&client, &saved)?;
        Ok((client, saved))
    }
    pub async fn terminal(&self, grant: &str) -> Result<(Client, SavedHc, Record, Value)> {
        let (mut client, saved) = self.connection().await?;
        let record = saved
            .terminals
            .iter()
            .find(|r| r.grant == grant)
            .cloned()
            .context("hc resource is not owned")?;
        ensure!(
            record.address.is_some(),
            "hc creation result is unknown; reconnect without creating another terminal"
        );
        let list = client
            .call(
                "terminal.list",
                json!({"all":true,"labels":saved.labels(&record)}),
            )
            .await?;
        let terminal = saved
            .resolve(&record, &list)?
            .context("hc terminal is missing; it was not recreated")?;
        Ok((client, saved, record, terminal))
    }
    pub async fn terminate(&self, grant: &str) -> Result<Value> {
        let (mut client, _, _, terminal) = self.terminal(grant).await?;
        let terminal = if terminal["state"] != "finished" {
            let result = client
                .call("terminal.kill", json!({"terminal":terminal["address"]}))
                .await?;
            ensure!(
                result["state"] == "finished",
                "hc did not confirm terminal termination"
            );
            result
        } else {
            terminal
        };
        client.close().await?;
        Ok(terminal)
    }
    pub fn is_stream(&self, stream: &str, subscriber: &str) -> bool {
        self.attachments
            .lock()
            .expect("hc attachments lock")
            .get(stream)
            .is_some_and(|a| a.subscriber == subscriber)
    }
    fn controlled(&self, stream: &str, subscriber: &str) -> bool {
        self.attachments
            .lock()
            .expect("hc attachments lock")
            .get(stream)
            .is_some_and(|a| {
                a.subscriber == subscriber
                    && a.controller
                    && !a.read_only
                    && !a.cancel.is_cancelled()
            })
    }
    pub async fn control(&self, stream: &str, subscriber: &str, take: bool) -> Result<bool> {
        let (done, reply) = oneshot::channel();
        {
            let mut attachments = self.attachments.lock().expect("hc attachments lock");
            let chosen = attachments.get(stream).context("hc stream not found")?;
            ensure!(
                chosen.subscriber == subscriber && !chosen.cancel.is_cancelled(),
                "hc stream is not owned"
            );
            ensure!(!take || !chosen.read_only, "hc stream is read-only");
            let grant = chosen.grant.clone();
            for (id, item) in attachments
                .iter_mut()
                .filter(|(_, item)| item.grant == grant)
            {
                let controller = if take {
                    id == stream
                } else {
                    item.controller && id != stream
                };
                if controller != item.controller {
                    item.controller = controller;
                    if item
                        .events
                        .try_send(Event::TerminalControl {
                            stream: id.clone(),
                            controller,
                        })
                        .is_err()
                    {
                        item.controller = false;
                        item.cancel.cancel();
                    }
                    if id != stream
                        && item
                            .commands
                            .try_send(Command::Focus(controller, None))
                            .is_err()
                    {
                        item.controller = false;
                        item.cancel.cancel();
                    }
                }
            }
            let chosen = attachments.get_mut(stream).context("hc stream not found")?;
            if chosen
                .commands
                .try_send(Command::Focus(take, Some(done)))
                .is_err()
            {
                chosen.controller = false;
                chosen.cancel.cancel();
                anyhow::bail!("hc command queue full");
            }
        }
        reply.await.context("hc control worker stopped")??;
        Ok(self.controlled(stream, subscriber))
    }
    pub async fn input(&self, stream: &str, subscriber: &str, data: Vec<u8>) -> Result<()> {
        ensure!(data.len() <= 16384, "hc input exceeds limit");
        let (done, reply) = oneshot::channel();
        self.command(stream, subscriber, Command::Input(data, done))?;
        reply.await.context("hc input worker stopped")?
    }
    pub async fn resize(&self, stream: &str, subscriber: &str, cols: u16, rows: u16) -> Result<()> {
        ensure!(
            (2..=1000).contains(&cols) && (2..=1000).contains(&rows),
            "invalid hc size"
        );
        let (done, reply) = oneshot::channel();
        self.command(stream, subscriber, Command::Resize(cols, rows, done))?;
        reply.await.context("hc resize worker stopped")?
    }
    fn command(&self, stream: &str, subscriber: &str, command: Command) -> Result<()> {
        ensure!(
            self.controlled(stream, subscriber),
            "take hc terminal control first"
        );
        let attachments = self.attachments.lock().expect("hc attachments lock");
        let item = attachments.get(stream).context("hc stream not found")?;
        item.commands
            .try_send(command)
            .context("hc command queue full or stopped")
    }
    pub fn detach(&self, stream: &str, subscriber: &str) -> Result<()> {
        let mut attachments = self.attachments.lock().expect("hc attachments lock");
        ensure!(
            attachments
                .get(stream)
                .is_some_and(|a| a.subscriber == subscriber),
            "hc stream is not owned"
        );
        if let Some(item) = attachments.remove(stream) {
            item.cancel.cancel();
        }
        Ok(())
    }
    pub fn detach_all(&self, subscriber: &str) {
        self.attachments
            .lock()
            .expect("hc attachments lock")
            .retain(|_, item| {
                if item.subscriber == subscriber {
                    item.cancel.cancel();
                    false
                } else {
                    true
                }
            });
    }
    pub fn closed(&self, grant: &str, terminal: &Value) {
        self.attachments
            .lock()
            .expect("hc attachments lock")
            .retain(|stream, item| {
                if item.grant == grant {
                    let _ = item.events.try_send(Event::TerminalProgramStatus {
                        stream: stream.clone(),
                        records: Vec::new(),
                    });
                    if let Err(mpsc::error::TrySendError::Full(event)) =
                        item.events.try_send(stream::ended(stream, terminal))
                    {
                        let events = item.events.clone();
                        // One bounded retry per retired attachment, after resource withdrawal.
                        tokio::spawn(async move {
                            let _ = stream::notify(&events, event).await;
                        });
                    }
                    item.cancel.cancel();
                    false
                } else {
                    true
                }
            });
    }
}
fn core_id(client: &Client) -> Result<String> {
    let id = client.hello()["core"]["id"]
        .as_str()
        .context("hc omitted core identity")?;
    ensure!(
        !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
        "invalid hc core identity"
    );
    Ok(id.into())
}
fn verify_core(client: &Client, saved: &SavedHc) -> Result<()> {
    let id = core_id(client)?;
    ensure!(
        saved
            .core_id
            .as_ref()
            .is_none_or(|expected| expected == &id),
        "hc core identity changed; existing terminals were not recreated"
    );
    Ok(())
}
