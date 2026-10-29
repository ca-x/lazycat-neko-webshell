use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, anyhow, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod runtime;
pub use runtime::register;
pub mod cloudflare_quick;
pub mod ngrok_provider;

pub const PLUGIN_ID: &str = "public-tunnel";

const DEFAULT_PROVIDER: &str = "cloudflare-quick";
const START_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Default)]
pub struct TunnelManager {
    sessions: Mutex<HashMap<String, TunnelHandle>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelSessionInfo {
    pub id: String,
    pub provider: String,
    pub public_url: String,
    pub upstream_url: String,
    pub status: String,
    pub created_at_ms: u64,
}

#[derive(Debug)]
pub struct StartTunnelRequest {
    pub id: String,
    pub provider: String,
    pub upstream_url: String,
    pub ngrok_authtoken: Option<String>,
}

pub struct TunnelPluginResponse {
    pub status: String,
    pub content_type: String,
    pub payload: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

struct TunnelHandle {
    info: TunnelSessionInfo,
    status: Arc<Mutex<String>>,
    stop_tx: tokio::sync::watch::Sender<bool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl TunnelManager {
    pub fn stop_all(&self) -> Result<(), neko_plugin::PluginError> {
        let handles = self
            .sessions
            .lock()
            .map_err(|_| neko_plugin::PluginError::internal("tunnel session store lock poisoned"))?
            .drain()
            .map(|(_, handle)| handle)
            .collect::<Vec<_>>();
        for handle in &handles {
            let _ = handle.stop_tx.send(true);
        }
        for handle in handles {
            handle.stop_and_wait();
        }
        Ok(())
    }

    pub fn invoke_metadata(
        &self,
        operation: &str,
        metadata: &HashMap<String, String>,
    ) -> anyhow::Result<TunnelPluginResponse> {
        match operation.trim() {
            "" | "list" | "status" | "default" => self.list(),
            "start" => self.start(metadata),
            "stop" => self.stop(metadata),
            operation => bail!("unsupported public tunnel operation: {operation}"),
        }
    }

    fn start(&self, metadata: &HashMap<String, String>) -> anyhow::Result<TunnelPluginResponse> {
        let start = parse_start_request(metadata)?;
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let (ready_tx, ready_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new("starting".to_owned()));

        let worker = match start.provider.as_str() {
            "cloudflare-quick" => {
                cloudflare_quick::spawn(start, stop_rx, ready_tx, Arc::clone(&status))
            }
            "ngrok" => ngrok_provider::spawn(start, stop_rx, ready_tx, Arc::clone(&status)),
            provider => bail!("unsupported tunnel provider: {provider}"),
        };

        let info = match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(info)) => info,
            result => {
                let _ = stop_tx.send(true);
                let _ = worker.join();
                return Err(match result {
                    Ok(Err(error)) => anyhow!(error),
                    _ => anyhow!("tunnel provider did not become ready"),
                });
            }
        };
        let handle = TunnelHandle {
            info: info.clone(),
            status,
            stop_tx,
            worker: Some(worker),
        };
        self.sessions
            .lock()
            .map_err(|_| anyhow!("tunnel session store lock poisoned"))?
            .insert(info.id.clone(), handle);
        json_response(
            "complete",
            &serde_json::json!({
                "session": info,
                "providers": provider_descriptors(),
            }),
        )
    }

    fn list(&self) -> anyhow::Result<TunnelPluginResponse> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("tunnel session store lock poisoned"))?
            .values()
            .map(TunnelHandle::snapshot)
            .collect::<Vec<_>>();
        json_response(
            "complete",
            &serde_json::json!({
                "sessions": sessions,
                "providers": provider_descriptors(),
            }),
        )
    }

    fn stop(&self, metadata: &HashMap<String, String>) -> anyhow::Result<TunnelPluginResponse> {
        let tunnel_id = required_metadata(metadata, "tunnelId")?;
        let handle = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("tunnel session store lock poisoned"))?
            .remove(tunnel_id)
            .ok_or_else(|| anyhow!("tunnel session not found: {tunnel_id}"))?;
        let mut info = handle.snapshot();
        "stopping".clone_into(&mut info.status);
        handle.stop_and_wait();
        json_response(
            "complete",
            &serde_json::json!({
                "session": info,
                "providers": provider_descriptors(),
            }),
        )
    }
}

impl Drop for TunnelHandle {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
    }
}

impl TunnelHandle {
    fn stop_and_wait(mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn snapshot(&self) -> TunnelSessionInfo {
        let mut info = self.info.clone();
        info.status = self
            .status
            .lock()
            .map_or_else(|_| "unknown".to_owned(), |status| status.clone());
        info
    }
}

pub(crate) fn mark_status(status: &Arc<Mutex<String>>, value: &str) {
    if let Ok(mut status) = status.lock() {
        value.clone_into(&mut status);
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

fn parse_start_request(metadata: &HashMap<String, String>) -> anyhow::Result<StartTunnelRequest> {
    let provider = metadata
        .get("provider")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER)
        .to_owned();
    let upstream_url = if let Some(upstream_url) = metadata
        .get("upstreamUrl")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_upstream_url(upstream_url)?;
        upstream_url.to_owned()
    } else {
        let host = metadata
            .get("host")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("127.0.0.1");
        validate_host(host)?;
        let port = metadata
            .get("port")
            .map(String::as_str)
            .map(str::trim)
            .ok_or_else(|| anyhow!("metadata.port is required"))?
            .parse::<u16>()
            .context("metadata.port must be a number from 1 to 65535")?;
        if port == 0 {
            bail!("metadata.port must be a number from 1 to 65535");
        }
        let path = metadata
            .get("path")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("/");
        build_http_url(host, port, path)?
    };
    let ngrok_authtoken = metadata
        .get("ngrokAuthtoken")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(StartTunnelRequest {
        id: Uuid::new_v4().to_string(),
        provider,
        upstream_url,
        ngrok_authtoken,
    })
}

fn validate_host(host: &str) -> anyhow::Result<()> {
    if host.contains('/') || host.contains('\\') || host.contains('@') {
        bail!("metadata.host must be an IP address or hostname without a scheme");
    }
    if !is_local_host(host) {
        bail!("metadata.host must be localhost or 127.0.0.1 by default");
    }
    Ok(())
}

fn validate_upstream_url(value: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(value).map_err(|err| anyhow!("invalid upstream URL: {err}"))?;
    if url.scheme() != "http" {
        bail!("metadata.upstreamUrl must use http");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("metadata.upstreamUrl must include a host"))?;
    if !is_local_host(host) {
        bail!("metadata.upstreamUrl must point to localhost or 127.0.0.1");
    }
    if url.port_or_known_default().is_none() {
        bail!("metadata.upstreamUrl must include a port");
    }
    Ok(())
}

fn is_local_host(host: &str) -> bool {
    matches!(
        host.trim_matches(['[', ']']),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn build_http_url(host: &str, port: u16, path: &str) -> anyhow::Result<String> {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let value = format!("http://{host}:{port}{path}");
    reqwest::Url::parse(&value).map_err(|err| anyhow!("invalid upstream URL: {err}"))?;
    Ok(value)
}

fn required_metadata<'a>(
    metadata: &'a HashMap<String, String>,
    key: &str,
) -> anyhow::Result<&'a str> {
    metadata
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("metadata.{key} is required"))
}

fn json_response(
    status: &str,
    payload: &serde_json::Value,
) -> anyhow::Result<TunnelPluginResponse> {
    Ok(TunnelPluginResponse {
        status: status.to_owned(),
        content_type: "application/json".to_owned(),
        payload: serde_json::to_vec(payload)?,
        metadata: HashMap::new(),
    })
}

fn provider_descriptors() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "id": "cloudflare-quick",
            "label": "Cloudflare Quick Tunnel",
            "requiresToken": false,
            "experimental": true,
        }),
        serde_json::json!({
            "id": "ngrok",
            "label": "ngrok",
            "requiresToken": true,
            "experimental": false,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_stop_waits_for_tunnel_worker_cleanup() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !*stop_rx.borrow() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "stop was not signaled"
                );
                std::thread::yield_now();
            }
            std::thread::sleep(Duration::from_millis(20));
            worker_finished.store(true, Ordering::Release);
        });
        let manager = TunnelManager::default();
        manager
            .sessions
            .lock()
            .unwrap()
            .insert("test".into(), test_handle(stop_tx, worker));
        manager.stop_all().unwrap();
        assert!(finished.load(Ordering::Acquire));
        assert!(manager.sessions.lock().unwrap().is_empty());
    }

    fn test_handle(
        stop_tx: tokio::sync::watch::Sender<bool>,
        worker: std::thread::JoinHandle<()>,
    ) -> TunnelHandle {
        TunnelHandle {
            info: TunnelSessionInfo {
                id: "test".into(),
                provider: "test".into(),
                public_url: String::new(),
                upstream_url: String::new(),
                status: "running".into(),
                created_at_ms: 0,
            },
            status: Arc::new(Mutex::new("running".into())),
            stop_tx,
            worker: Some(worker),
        }
    }

    #[test]
    fn dropping_a_tunnel_signals_stop_without_waiting_for_its_worker() {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || release_rx.recv().unwrap());
        let handle = test_handle(stop_tx, worker);
        let (closed_tx, closed_rx) = mpsc::channel();
        let cleanup = std::thread::spawn(move || {
            drop(handle);
            closed_tx.send(()).unwrap();
        });
        let closed = closed_rx.recv_timeout(Duration::from_millis(250));
        release_tx.send(()).unwrap();
        cleanup.join().unwrap();
        assert!(closed.is_ok(), "tunnel drop waited for its worker");
        assert!(*stop_rx.borrow(), "tunnel drop must request cancellation");
    }
}
