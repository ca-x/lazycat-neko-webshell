//! Current-user Herdr discovery and allowlisted SockAPI, independent of LightOS.
#[path = "../../../../src/herdr/native_input.rs"]
mod native_input;
pub(crate) mod transport;

use anyhow::{Context, Result, bail, ensure};
use neko_remoted_protocol::{
    self as protocol,
    messages::{Event, Resource},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

const RPC_LIMIT: usize = 512 * 1024;
const RPC_TIMEOUT: Duration = Duration::from_secs(3);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);
const MIN_PROTOCOL: u64 = 14;
const SCREEN_LIMIT: usize = 256 * 1024;
/// Local Herdr availability. Detection failures never become an empty ready session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HerdrStatus {
    Checking,
    NotInstalled,
    Stopped,
    Ready,
    Error,
}
/// UI status and installation guidance; commands are text only, never executed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HerdrSnapshot {
    pub status: HerdrStatus,
    pub version: Option<String>,
    pub server_version: Option<String>,
    pub protocol: Option<u64>,
    pub error: String,
    pub install_url: String,
    pub install_command: String,
}
impl Default for HerdrSnapshot {
    fn default() -> Self {
        Self {
            status: HerdrStatus::Checking,
            version: None,
            server_version: None,
            protocol: None,
            error: String::new(),
            install_url: "https://herdr.dev/tui/".into(),
            install_command: if cfg!(windows) {
                "irm https://herdr.dev/install.ps1 | iex"
            } else {
                "curl -fsSL https://herdr.dev/install.sh | sh"
            }
            .into(),
        }
    }
}
#[derive(Deserialize)]
struct Contract {
    methods: Vec<String>,
    stream_methods: Vec<String>,
}
static CONTRACT: LazyLock<Contract> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../src/herdr_socket_contract.json"))
        .expect("bundled Herdr contract")
});
pub(crate) fn allowed(method: &str) -> bool {
    method != "server.live_handoff"
        && CONTRACT.methods.iter().any(|m| m == method)
        && !CONTRACT.stream_methods.iter().any(|m| m == method)
}
#[derive(Clone)]
struct Endpoint {
    binary: PathBuf,
    socket: PathBuf,
    resource: String,
    signature: String,
}
struct Observed {
    snapshot: HerdrSnapshot,
    endpoint: Option<Endpoint>,
}
struct Attachment {
    endpoint: Endpoint,
    pane: String,
    subscriber: String,
    read_only: bool,
    controller: bool,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
}
/// No endpoint is accepted from a caller. Only the installed CLI's local status selects it.
pub(crate) struct Herdr {
    observed: Mutex<Observed>,
    pub(crate) changed: tokio::sync::Notify,
    inspection: tokio::sync::Mutex<()>,
    attachments: Mutex<HashMap<String, Attachment>>,
    #[cfg(test)]
    program: Option<PathBuf>,
}
impl Default for Herdr {
    fn default() -> Self {
        Self {
            observed: Mutex::new(Observed {
                snapshot: HerdrSnapshot::default(),
                endpoint: None,
            }),
            changed: tokio::sync::Notify::new(),
            inspection: tokio::sync::Mutex::new(()),
            attachments: Mutex::new(HashMap::new()),
            #[cfg(test)]
            program: None,
        }
    }
}
impl Herdr {
    #[cfg(all(test, unix))]
    pub(crate) fn with_program(program: PathBuf) -> Self {
        Self {
            program: Some(program),
            ..Self::default()
        }
    }
    pub fn snapshot(&self) -> HerdrSnapshot {
        self.observed
            .lock()
            .expect("Herdr state lock")
            .snapshot
            .clone()
    }
    pub fn resource(&self) -> Option<Resource> {
        let state = self.observed.lock().ok()?;
        if state.snapshot.status != HerdrStatus::Ready {
            return None;
        }
        Some(Resource {
            id: state.endpoint.as_ref()?.resource.clone(),
            name: "Herdr".into(),
            backend: "herdr".into(),
            terminal: false,
            herdr: true,
            files: false,
        })
    }
    fn fail(&self, error: &anyhow::Error) {
        let mut state = self.observed.lock().expect("Herdr state lock");
        state.snapshot.status = HerdrStatus::Error;
        state.snapshot.error = error.to_string();
        // Endpoint remains private for rediscovery; no resource is advertised in Error.
        drop(state);
        for attachment in self
            .attachments
            .lock()
            .expect("Herdr attachment lock")
            .values()
        {
            attachment.cancel.cancel();
        }
        self.changed.notify_one();
    }
    pub async fn refresh(&self) -> Result<()> {
        let _guard = self.inspection.lock().await;
        let result = tokio::time::timeout(DISCOVERY_TIMEOUT, self.discover())
            .await
            .context("Herdr detection timed out")
            .and_then(|r| r);
        if let Err(error) = &result {
            self.fail(error);
        }
        result
    }
    async fn discover(&self) -> Result<()> {
        #[cfg(test)]
        let binary = self.program.clone();
        #[cfg(not(test))]
        let binary = find_program()?;
        let Some(binary) = binary else {
            let mut observed = self.observed.lock().expect("Herdr state lock");
            observed.snapshot = HerdrSnapshot {
                status: HerdrStatus::NotInstalled,
                ..HerdrSnapshot::default()
            };
            observed.endpoint = None;
            drop(observed);
            for attachment in self
                .attachments
                .lock()
                .expect("Herdr attachment lock")
                .values()
            {
                attachment.cancel.cancel();
            }
            return Ok(());
        };
        transport::validate_binary(&binary)?;
        let version = run(&binary, &["--version"]).await?;
        let version = String::from_utf8(version)?.trim().to_owned();
        let status: Value =
            serde_json::from_slice(&run(&binary, &["status", "server", "--json"]).await?)?;
        let socket = PathBuf::from(
            status["socket"]
                .as_str()
                .context("Herdr status has no socket")?,
        );
        ensure!(
            socket.is_absolute(),
            "Herdr status returned a nonlocal socket"
        );
        let running = status["running"]
            .as_bool()
            .context("Herdr status missing running flag")?;
        let signature = if running {
            transport::identity(&socket)?
        } else {
            "stopped".into()
        };
        let previous = self
            .observed
            .lock()
            .expect("Herdr state lock")
            .endpoint
            .clone();
        let resource = previous
            .filter(|e| e.socket == socket && e.signature == signature)
            .map_or_else(|| uuid::Uuid::new_v4().to_string(), |e| e.resource);
        let endpoint = Endpoint {
            binary,
            socket,
            resource,
            signature,
        };
        let mut snapshot = HerdrSnapshot {
            status: HerdrStatus::Stopped,
            version: Some(version),
            ..HerdrSnapshot::default()
        };
        if running {
            let ping = rpc(&endpoint, "ping", json!({})).await?;
            let ping = ping.get("result").unwrap_or(&ping);
            let protocol = ping["protocol"]
                .as_u64()
                .context("Herdr ping missing protocol")?;
            ensure!(
                protocol >= MIN_PROTOCOL,
                "Herdr protocol is below the supported minimum"
            );
            snapshot.protocol = Some(protocol);
            snapshot.server_version = ping["version"].as_str().map(str::to_owned);
            snapshot.status = HerdrStatus::Ready;
        }
        if self
            .observed
            .lock()
            .expect("Herdr state lock")
            .endpoint
            .as_ref()
            .is_some_and(|old| old.resource != endpoint.resource)
            || !running
        {
            for attachment in self
                .attachments
                .lock()
                .expect("Herdr attachment lock")
                .values()
            {
                attachment.cancel.cancel();
            }
        }
        *self.observed.lock().expect("Herdr state lock") = Observed {
            snapshot,
            endpoint: Some(endpoint),
        };
        Ok(())
    }
    pub async fn start(&self) -> Result<()> {
        let _guard = self.inspection.lock().await;
        let result = tokio::time::timeout(Duration::from_secs(9), self.start_inner())
            .await
            .context("Herdr startup timed out")
            .and_then(|r| r);
        if let Err(error) = &result
            && self.snapshot().status != HerdrStatus::NotInstalled
        {
            self.fail(error);
        }
        result
    }
    async fn start_inner(&self) -> Result<()> {
        self.discover().await?;
        let snapshot = self.snapshot();
        if snapshot.status == HerdrStatus::Ready {
            return Ok(());
        }
        ensure!(
            snapshot.status == HerdrStatus::Stopped,
            "Herdr is not installed; use the installation guide"
        );
        let endpoint = self
            .observed
            .lock()
            .expect("Herdr state lock")
            .endpoint
            .clone()
            .context("Herdr binary unavailable")?;
        transport::validate_binary(&endpoint.binary)?;
        // Inherit this user's config/environment. No HOME/cwd changes, no install or handoff.
        let child = Command::new(&endpoint.binary)
            .arg("server")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut starting = Starting(Some(child));
        loop {
            if let Some(exit) = starting.0.as_mut().expect("starting child").try_wait()? {
                bail!("Herdr server exited during startup: {exit}");
            }
            self.discover().await?;
            if self.snapshot().status == HerdrStatus::Ready {
                let mut child = starting.0.take().expect("ready child");
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    fn endpoint(&self) -> Result<Endpoint> {
        let state = self
            .observed
            .lock()
            .map_err(|_| anyhow::anyhow!("Herdr state lock poisoned"))?;
        ensure!(
            state.snapshot.status == HerdrStatus::Ready,
            "Herdr is not ready"
        );
        state.endpoint.clone().context("Herdr endpoint unavailable")
    }
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        ensure!(
            allowed(method),
            "Herdr method is not in the supported nonstreaming allowlist"
        );
        let endpoint = self.endpoint()?;
        self.request_at(&endpoint, method, params).await
    }
    async fn request_at(&self, endpoint: &Endpoint, method: &str, params: Value) -> Result<Value> {
        ensure!(
            self.endpoint()?.resource == endpoint.resource,
            "Herdr runtime changed; reattach required"
        );
        let result = rpc(endpoint, method, params).await;
        if let Err(error) = &result {
            // Only transport failures withdraw availability; method errors leave the runtime usable.
            if !error.is::<MethodError>() {
                self.fail(error);
            }
        }
        result
    }
    /// An explicit local action; workspace.create owns its initial tab and PTY.
    pub async fn create_session(&self, name: &str) -> Result<crate::CreatedSession> {
        crate::state::check_name(name)?;
        let endpoint = self.endpoint()?;
        let created = self
            .request_at(
                &endpoint,
                "workspace.create",
                json!({"label":name,"focus":false}),
            )
            .await?;
        let result = created.get("result").unwrap_or(&created);
        let pane = result["root_pane"]["pane_id"].as_str().context(
            "Herdr created a workspace but omitted its root pane; creation was not retried",
        )?;
        ensure!(
            !pane.is_empty() && pane.len() <= 256 && !pane.chars().any(char::is_control),
            "Herdr returned an invalid pane identity"
        );
        // Validate membership without sending input, focus or a layout resize.
        let snapshot = self
            .request_at(&endpoint, "session.snapshot", json!({}))
            .await?;
        let result = snapshot.get("result").unwrap_or(&snapshot);
        let panes = result.get("snapshot").unwrap_or(result)["panes"]
            .as_array()
            .context("Herdr snapshot has no panes")?;
        ensure!(
            panes.iter().any(|p| p["pane_id"].as_str() == Some(pane)),
            "Herdr created pane is not in the pinned runtime"
        );
        Ok(crate::CreatedSession {
            grant: endpoint.resource,
            pane: Some(pane.into()),
        })
    }
    pub async fn attach(
        self: &Arc<Self>,
        pane: String,
        subscriber: String,
        events: mpsc::Sender<Event>,
        read_only: bool,
    ) -> Result<(String, String)> {
        let endpoint = self.endpoint()?;
        let snapshot = self
            .request_at(&endpoint, "session.snapshot", json!({}))
            .await?;
        let result = snapshot.get("result").unwrap_or(&snapshot);
        let panes = result
            .get("snapshot")
            .unwrap_or(result)
            .get("panes")
            .and_then(Value::as_array)
            .context("Herdr snapshot has no panes")?;
        ensure!(
            panes.iter().any(|p| p["pane_id"].as_str() == Some(&pane)),
            "Herdr pane is not in the current session"
        );
        let stream = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        self.attachments
            .lock()
            .expect("Herdr attachment lock")
            .insert(
                stream.clone(),
                Attachment {
                    endpoint: endpoint.clone(),
                    pane: pane.clone(),
                    subscriber,
                    read_only,
                    controller: false,
                    events: events.clone(),
                    cancel: cancel.clone(),
                },
            );
        let host = self.clone();
        let id = stream.clone();
        let name = pane.clone();
        tokio::spawn(async move {
            if host
                .screen(&endpoint, &pane, &id, &events, &cancel)
                .await
                .is_err()
            {
                let _ = events.try_send(Event::Error {
                    code: "herdr_unavailable".into(),
                    message: "Herdr screen unavailable; refresh local Herdr status".into(),
                    outcome_unknown: false,
                });
            }
            host.attachments
                .lock()
                .expect("Herdr attachment lock")
                .remove(&id);
        });
        Ok((stream, name))
    }
    async fn screen(
        &self,
        endpoint: &Endpoint,
        pane: &str,
        stream: &str,
        events: &mpsc::Sender<Event>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut previous = None;
        let mut sequence = 0_u64;
        loop {
            let response = tokio::select! { biased; () = cancel.cancelled() => return Ok(()), result = self.request_at(endpoint, "pane.read", json!({"pane_id":pane,"source":"visible","format":"ansi","strip_ansi":false})) => result? };
            let result = response.get("result").unwrap_or(&response);
            let read = result.get("read").unwrap_or(result);
            let text = read
                .get("text")
                .and_then(Value::as_str)
                .context("Herdr read missing text")?;
            ensure!(
                text.len() <= SCREEN_LIMIT && read["truncated"].as_bool() != Some(true),
                "Herdr visible screen is incomplete or exceeds the snapshot size limit"
            );
            ensure!(
                read.get("format").is_none_or(|format| format == "ansi")
                    && read.get("source").is_none_or(|source| source == "visible"),
                "Herdr did not return the requested visible ANSI screen"
            );
            if previous.as_deref() != Some(text) {
                let bytes = text.as_bytes();
                // Wire chunks are a byte stream; clients decode after assembling terminal bytes.
                for (i, chunk) in bytes.chunks(16 * 1024).enumerate() {
                    sequence = sequence
                        .checked_add(1)
                        .context("Herdr screen sequence exhausted")?;
                    let event = Event::TerminalOutput {
                        stream: stream.into(),
                        sequence,
                        data: protocol::encode(chunk),
                        replay: false,
                        reset: i == 0,
                        gap: false,
                    };
                    tokio::select! { () = cancel.cancelled() => return Ok(()), result = events.send(event) => result? };
                }
                if bytes.is_empty() {
                    sequence = sequence
                        .checked_add(1)
                        .context("Herdr screen sequence exhausted")?;
                    let event = Event::TerminalOutput {
                        stream: stream.into(),
                        sequence,
                        data: String::new(),
                        replay: false,
                        reset: true,
                        gap: false,
                    };
                    tokio::select! { () = cancel.cancelled() => return Ok(()), result = events.send(event) => result? };
                }
                previous = Some(text.to_owned());
            }
            tokio::select! { () = cancel.cancelled() => return Ok(()), () = tokio::time::sleep(Duration::from_millis(250)) => {} }
        }
    }
    pub fn is_stream(&self, stream: &str, subscriber: &str) -> bool {
        self.attachments
            .lock()
            .is_ok_and(|all| all.get(stream).is_some_and(|a| a.subscriber == subscriber))
    }
    pub fn control(&self, stream: &str, subscriber: &str, take: bool) -> Result<bool> {
        let mut all = self
            .attachments
            .lock()
            .map_err(|_| anyhow::anyhow!("Herdr attachment lock"))?;
        let chosen = all.get(stream).context("Herdr stream not found")?;
        ensure!(
            chosen.subscriber == subscriber && !chosen.cancel.is_cancelled(),
            "Herdr stream is not owned"
        );
        ensure!(!take || !chosen.read_only, "Herdr stream is read-only");
        let pane = chosen.pane.clone();
        for (id, item) in all.iter_mut().filter(|(_, a)| a.pane == pane) {
            let controller = if take {
                id == stream
            } else {
                item.controller && id != stream
            };
            if item.controller != controller {
                item.controller = controller;
                if item
                    .events
                    .try_send(Event::TerminalControl {
                        stream: id.clone(),
                        controller,
                    })
                    .is_err()
                {
                    item.cancel.cancel();
                    item.controller = false;
                }
            }
        }
        Ok(all
            .get(stream)
            .is_some_and(|a| a.controller && !a.cancel.is_cancelled()))
    }
    pub fn require_control(&self, pane: &str, subscriber: &str) -> Result<()> {
        ensure!(
            self.attachments
                .lock()
                .map_err(|_| anyhow::anyhow!("Herdr attachment lock"))?
                .values()
                .any(|a| a.pane == pane
                    && a.subscriber == subscriber
                    && a.controller
                    && !a.read_only
                    && !a.cancel.is_cancelled()),
            "take Herdr pane control first"
        );
        Ok(())
    }
    pub async fn input(&self, stream: &str, subscriber: &str, data: Vec<u8>) -> Result<()> {
        let (pane, endpoint) = {
            let all = self
                .attachments
                .lock()
                .map_err(|_| anyhow::anyhow!("Herdr attachment lock"))?;
            let a = all.get(stream).context("Herdr stream not found")?;
            ensure!(
                a.subscriber == subscriber
                    && a.controller
                    && !a.read_only
                    && !a.cancel.is_cancelled(),
                "Herdr input is not authorized"
            );
            (a.pane.clone(), a.endpoint.clone())
        };
        let (method, params) = native_input::request(&pane, &data).map_err(anyhow::Error::msg)?;
        self.request_at(&endpoint, method, params).await?;
        Ok(())
    }
    pub fn detach(&self, stream: &str, subscriber: &str) -> Result<()> {
        let mut all = self
            .attachments
            .lock()
            .map_err(|_| anyhow::anyhow!("Herdr attachment lock"))?;
        ensure!(
            all.get(stream).is_some_and(|a| a.subscriber == subscriber),
            "Herdr stream is not owned"
        );
        if let Some(a) = all.remove(stream) {
            a.cancel.cancel();
        }
        Ok(())
    }
    pub fn detach_all(&self, subscriber: &str) {
        self.attachments
            .lock()
            .expect("Herdr attachment lock")
            .retain(|_, a| {
                if a.subscriber == subscriber {
                    a.cancel.cancel();
                    false
                } else {
                    true
                }
            });
    }
}
struct Starting(Option<Child>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.start_kill();
        }
    }
}
#[derive(Debug)]
struct MethodError(String);
impl std::fmt::Display for MethodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MethodError {}
async fn rpc(endpoint: &Endpoint, method: &str, params: Value) -> Result<Value> {
    let bytes =
        serde_json::to_vec(&json!({"id":"neoko-terminal-host","method":method,"params":params}))?;
    ensure!(bytes.len() <= 64 * 1024, "Herdr request exceeds limit");
    tokio::time::timeout(RPC_TIMEOUT, async {
        ensure!(
            transport::identity(&endpoint.socket)? == endpoint.signature,
            "Herdr endpoint changed; refresh status before attaching"
        );
        let mut socket = transport::connect(&endpoint.socket).await?;
        socket.write_all(&bytes).await?;
        socket.write_all(b"\n").await?;
        let mut bytes = Vec::new();
        BufReader::new(socket)
            .take((RPC_LIMIT + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= RPC_LIMIT && bytes.ends_with(b"\n"),
            "Herdr response missing or exceeds limit"
        );
        let response: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            response["id"].as_str() == Some("neoko-terminal-host"),
            "Herdr response ID mismatch"
        );
        if let Some(error) = response.get("error") {
            return Err(MethodError(error.to_string()).into());
        }
        Ok(response)
    })
    .await
    .context("Herdr socket request timed out")?
}
async fn run(binary: &Path, args: &[&str]) -> Result<Vec<u8>> {
    tokio::time::timeout(RPC_TIMEOUT, async {
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().context("Herdr stdout missing")?;
        let stderr = child.stderr.take().context("Herdr stderr missing")?;
        let out = async {
            let mut data = Vec::new();
            stdout.take(32769).read_to_end(&mut data).await?;
            ensure!(data.len() <= 32768, "Herdr status exceeds limit");
            Ok::<_, anyhow::Error>(data)
        };
        let err = async {
            let mut data = Vec::new();
            stderr.take(4097).read_to_end(&mut data).await?;
            ensure!(data.len() <= 4096, "Herdr error exceeds limit");
            Ok::<_, anyhow::Error>(())
        };
        let (out, (), status) = tokio::try_join!(out, err, async {
            Ok::<_, anyhow::Error>(child.wait().await?)
        })?;
        ensure!(status.success(), "Herdr status command failed");
        Ok(out)
    })
    .await
    .context("Herdr status command timed out")?
}
#[cfg(not(test))]
fn find_program() -> Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    for dir in std::env::split_paths(&path)
        .take(64)
        .filter(|p| p.is_absolute())
    {
        let file = dir.join(if cfg!(windows) { "herdr.exe" } else { "herdr" });
        if file.is_file() {
            return Ok(Some(file.canonicalize()?));
        }
    }
    Ok(None)
}
#[cfg(test)]
fn terminal_mcp_input_request(pane: &str, data: &str) -> Result<(&'static str, Value), String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| e.to_string())?;
    native_input::request(pane, &bytes)
}
#[cfg(test)]
mod tests;
