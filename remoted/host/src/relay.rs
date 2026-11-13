mod path;
use crate::{
    peer,
    state::{Core, DEADLINE},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use neko_remoted_protocol::{self as protocol, RelayAddress, crypto::Mode, messages::RelayFrame};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, mpsc};
use tokio_tungstenite::{
    Connector,
    tungstenite::{Message, client::IntoClientRequest},
};
use tokio_util::sync::CancellationToken;

pub(crate) enum Input {
    Hello(Mode),
    Data(Vec<u8>),
    CreditedData { data: Vec<u8>, receipt: Receipt },
}
impl Input {
    pub(crate) fn consumed(self) -> Self {
        match self {
            Self::CreditedData { data, receipt } => {
                drop(receipt);
                Self::Data(data)
            }
            input => input,
        }
    }
}
/// Exactly one acknowledgement when the peer consumes or discards this frame.
pub(crate) struct Receipt {
    sequence: u64,
    completed: mpsc::UnboundedSender<u64>,
}
impl Drop for Receipt {
    fn drop(&mut self) {
        let _ = self.completed.send(self.sequence);
    }
}
#[derive(Clone)]
pub(crate) struct Link {
    transport: Arc<std::sync::OnceLock<neko_remoted_transport::Sender>>,
    pub peer: String,
    outgoing: mpsc::Sender<RelayFrame>,
    credits: Arc<Semaphore>,
    pub cancel: CancellationToken,
}
impl Link {
    pub async fn send(&self, data: &[u8]) -> Result<()> {
        if let Some(sender) = self.transport.get() {
            return sender.send(data.to_vec()).await;
        }
        ensure!(
            data.len() <= protocol::MAX_CIPHERTEXT,
            "encrypted message exceeds bound"
        );
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => bail!("peer closed"),
            result = tokio::time::timeout(DEADLINE, async {
                let permit = self.credits.clone().acquire_owned().await?;
                self.outgoing.send(RelayFrame::Data { peer: self.peer.clone(), data: protocol::encode(data), sequence: None }).await?;
                permit.forget();
                Ok::<(), anyhow::Error>(())
            }) => result.context("Relay send timed out")?,
        }
    }
}
struct Inbound {
    incoming: mpsc::Sender<Input>,
    credits: Arc<Semaphore>,
    cancel: CancellationToken,
}
struct Peers(HashMap<String, Inbound>);
impl Drop for Peers {
    fn drop(&mut self) {
        for peer in self.0.values() {
            peer.cancel.cancel();
        }
    }
}

pub(crate) async fn supervise(core: Arc<Core>) {
    let mut delay = 1;
    loop {
        let (relay, generation, cancel) = {
            let mut state = core.state.lock().await;
            if !state.saved.enabled {
                drop(state);
                tokio::select! { biased; () = core.shutdown.cancelled() => return, () = core.changed.notified() => continue }
            }
            state.relay_cancel = core.shutdown.child_token();
            (
                state.saved.relay.clone(),
                state.generation,
                state.relay_cancel.clone(),
            )
        };
        status(&core, generation, "connecting", "").await;
        let result = connection(core.clone(), &relay, generation, cancel.clone()).await;
        cancel.cancel();
        if core.shutdown.is_cancelled() {
            return;
        }
        {
            let state = core.state.lock().await;
            if state.generation != generation || !state.saved.enabled {
                delay = 1;
                continue;
            }
        }
        if result.is_err() {
            status(
                &core,
                generation,
                "reconnecting",
                "Relay connection unavailable",
            )
            .await;
        }
        tokio::select! {
            biased;
            () = core.shutdown.cancelled() => return,
            () = core.changed.notified() => { delay = 1; },
            () = tokio::time::sleep(Duration::from_secs(delay)) => { delay = (delay * 2).min(30); },
        }
    }
}
async fn status(core: &Core, generation: u64, status: &str, error: &str) {
    let mut state = core.state.lock().await;
    if state.generation == generation && state.saved.enabled {
        state.status = status.into();
        state.error = error.into();
        let _ = core.publish(&state);
    }
}
async fn connection(
    core: Arc<Core>,
    origin: &str,
    generation: u64,
    cancel: CancellationToken,
) -> Result<()> {
    let relay = RelayAddress::parse(origin)?;
    let mut request = relay.socket("host", &core.route)?.into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", core.route_secret).parse()?,
    );
    request
        .headers_mut()
        .insert("sec-websocket-protocol", protocol::RELAY_PROTOCOL.parse()?);
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Explicit per-connection selection: embedding applications may enable both providers.
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(protocol::MAX_RELAY_MESSAGE))
        .max_frame_size(Some(protocol::MAX_RELAY_MESSAGE));
    let (mut socket, response) = tokio::select! {
        biased;
        () = cancel.cancelled() => return Ok(()),
        result = tokio::time::timeout(DEADLINE, tokio_tungstenite::connect_async_tls_with_config(
            request, Some(config), true, Some(Connector::Rustls(Arc::new(tls))))) => result.context("Relay connect timed out")??,
    };
    ensure!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .is_some_and(|v| v == protocol::RELAY_PROTOCOL),
        "Relay protocol not accepted"
    );
    let (outgoing, mut frames) = mpsc::channel::<RelayFrame>(128);
    let (finished, mut completions) = mpsc::channel::<String>(protocol::MAX_PEERS);
    let mut peers = Peers(HashMap::new());
    let mut registered = false;
    let registered_deadline = tokio::time::sleep(DEADLINE);
    tokio::pin!(registered_deadline);
    // Outstanding receipts are bounded by each peer's eight-frame Relay window.
    let (completed, mut acknowledgements) = mpsc::unbounded_channel::<u64>();
    let mut ticks = tokio::time::interval(Duration::from_secs(10));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heartbeat: Option<(String, Instant)> = None;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return Ok(()),
            () = &mut registered_deadline, if !registered => bail!("Relay registration timed out"),
            Some(peer) = completions.recv() => {
                if let Some(peer) = peers.0.remove(&peer) { peer.cancel.cancel(); }
                outgoing.try_send(RelayFrame::Closed { peer }).context("Relay control queue full")?;
            }
            Some(sequence) = acknowledgements.recv() => {
                let frame = RelayFrame::Acknowledge { peer: "host".into(), sequence };
                tokio::time::timeout(DEADLINE, socket.send(Message::Text(serde_json::to_string(&frame)?.into()))).await??;
            }
            Some(frame) = frames.recv() => {
                let text = serde_json::to_string(&frame)?;
                ensure!(text.len() <= protocol::MAX_RELAY_MESSAGE, "Relay frame exceeds limit");
                tokio::time::timeout(DEADLINE, socket.send(Message::Text(text.into()))).await??;
            }
            message = socket.next() => {
                let message = message.context("Relay socket closed")??;
                let frame = match message {
                    Message::Text(text) => serde_json::from_str::<RelayFrame>(&text)?,
                    Message::Ping(data) => { tokio::time::timeout(DEADLINE, socket.send(Message::Pong(data))).await??; continue; }
                    Message::Pong(_) => continue,
                    _ => bail!("Relay socket closed or invalid frame"),
                };
                match frame {
                    RelayFrame::Connected { peer } => {
                        ensure!(!registered && peer == "host", "invalid registration");
                        registered = true; status(&core, generation, "online", "").await;
                    }
                    RelayFrame::Opened { peer } => {
                        ensure!(registered && peer.len() <= 128, "invalid peer opening");
                        if peers.0.len() >= protocol::MAX_PEERS || peers.0.contains_key(&peer) {
                            outgoing.try_send(RelayFrame::Closed { peer })?; continue;
                        }
                        let (input, incoming) = mpsc::channel(9);
                        let token = cancel.child_token();
                        let credits = Arc::new(Semaphore::new(8));
                        let link = Link { transport: Arc::new(std::sync::OnceLock::new()), peer: peer.clone(), outgoing: outgoing.clone(), credits: credits.clone(), cancel: token.clone() };
                        peers.0.insert(peer.clone(), Inbound { incoming: input, credits, cancel: token });
                        let incoming = link.start_transport(incoming)?;
                        let core = core.clone(); let relay = relay.clone(); let finished = finished.clone();
                        tokio::spawn(async move {
                            peer::run(core, relay, link, incoming).await;
                            let _ = finished.send(peer).await;
                        });
                    }
                    RelayFrame::Hello { peer, mode } => if let Some(p) = peers.0.get(&peer)
                        && p.incoming.try_send(Input::Hello(mode)).is_err() { p.cancel.cancel(); },
                    RelayFrame::Data { peer, data, sequence: Some(sequence) } => {
                        ensure!(registered && sequence > 0, "invalid Relay sequence");
                        let receipt = Receipt { sequence, completed: completed.clone() };
                        if let Some(p) = peers.0.get(&peer) {
                            match protocol::decode(&data, protocol::MAX_WIRE_PAYLOAD) {
                                Ok(data) => if p.incoming.try_send(Input::CreditedData { data, receipt }).is_err() { p.cancel.cancel(); },
                                Err(_) => p.cancel.cancel(),
                            }
                        }
                    }
                    RelayFrame::Delivered { peer, count } => if let Some(p) = peers.0.get(&peer) {
                        if count > 0 && p.credits.available_permits() + usize::from(count) <= 8 { p.credits.add_permits(usize::from(count)); }
                        else { p.cancel.cancel(); }
                    },
                    RelayFrame::Closed { peer } => if let Some(p) = peers.0.remove(&peer) { p.cancel.cancel(); },
                    RelayFrame::Pong { nonce } => {
                        if let Some((expected, _)) = heartbeat.take() { ensure!(nonce == expected, "Relay heartbeat mismatch"); }
                        else { bail!("unexpected Relay pong"); }
                    }
                    RelayFrame::Error { .. } => bail!("Relay rejected connection"),
                    _ => bail!("unexpected Relay frame"),
                }
            }
            _ = ticks.tick(), if registered => {
                if heartbeat.as_ref().is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(30)) { bail!("Relay heartbeat timed out"); }
                if heartbeat.is_none() {
                    let nonce = uuid::Uuid::new_v4().to_string();
                    outgoing.try_send(RelayFrame::Ping { nonce: nonce.clone() })?;
                    heartbeat = Some((nonce, Instant::now()));
                }
                let Ok(mut state) = core.state.try_lock() else { continue };
                let expired = state.invitation.as_ref().is_some_and(|i| i.expires <= Instant::now());
                if expired { state.invitation = None; }
                let previous = state.pending.len();
                for p in state.pending.values().filter(|p| p.expires <= Instant::now()) { p.cancel.cancel(); }
                state.pending.retain(|_, p| !p.cancel.is_cancelled());
                if expired || previous != state.pending.len() { state.revision += 1; }
                core.publish(&state)?;
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn test_link() -> (Link, mpsc::Receiver<RelayFrame>, Arc<Semaphore>) {
    let (outgoing, incoming) = mpsc::channel(128);
    let credits = Arc::new(Semaphore::new(8));
    (
        Link {
            transport: Arc::new(std::sync::OnceLock::new()),
            peer: "isolated-fixture".into(),
            outgoing,
            credits: credits.clone(),
            cancel: CancellationToken::new(),
        },
        incoming,
        credits,
    )
}

#[cfg(test)]
mod credit_tests {
    use super::*;

    fn frame(sequence: u64, completed: &mpsc::UnboundedSender<u64>) -> Input {
        Input::CreditedData {
            data: vec![sequence as u8],
            receipt: Receipt {
                sequence,
                completed: completed.clone(),
            },
        }
    }

    #[tokio::test]
    async fn slow_consumer_reserves_hello_and_returns_only_consumed_exact_credit() {
        let (completed, mut acknowledgements) = mpsc::unbounded_channel();
        let (input, mut incoming) = mpsc::channel(9);
        input
            .try_send(Input::Hello(Mode::Pair))
            .unwrap_or_else(|_| panic!("Hello slot"));
        for sequence in 1..=8 {
            input
                .try_send(frame(sequence, &completed))
                .unwrap_or_else(|_| panic!("window overflow"));
        }
        assert!(
            acknowledgements.try_recv().is_err(),
            "enqueue must not acknowledge"
        );
        assert!(matches!(
            incoming.recv().await.unwrap().consumed(),
            Input::Hello(_)
        ));
        assert!(
            acknowledgements.try_recv().is_err(),
            "Hello must not return data credit"
        );
        for sequence in 1..=64 {
            assert!(matches!(
                incoming.recv().await.unwrap().consumed(),
                Input::Data(_)
            ));
            assert_eq!(acknowledgements.recv().await, Some(sequence));
            if sequence + 8 <= 64 {
                input
                    .try_send(frame(sequence + 8, &completed))
                    .unwrap_or_else(|_| panic!("consumer overflow"));
            }
        }
        assert!(acknowledgements.try_recv().is_err());
    }

    #[tokio::test]
    async fn two_peers_return_exact_out_of_order_receipts_without_cross_credit() {
        let (completed, mut acknowledgements) = mpsc::unbounded_channel();
        let (slow, mut slow_in) = mpsc::channel(9);
        let (fast, mut fast_in) = mpsc::channel(9);
        let mut outstanding = HashMap::new();
        for sequence in 1..=16 {
            let owner = if sequence % 2 == 1 { "slow" } else { "fast" };
            outstanding.insert(sequence, owner);
            let tx = if owner == "slow" { &slow } else { &fast };
            tx.try_send(frame(sequence, &completed))
                .unwrap_or_else(|_| panic!("peer window overflow"));
        }
        assert!(acknowledgements.try_recv().is_err());
        for sequence in (2..=16).step_by(2) {
            fast_in.recv().await.unwrap().consumed();
            let exact = acknowledgements.recv().await.unwrap();
            assert_eq!(exact, sequence);
            assert_eq!(outstanding.remove(&exact), Some("fast"));
        }
        assert_eq!(outstanding.len(), 8);
        assert!(outstanding.values().all(|owner| *owner == "slow"));
        // Dropping a cancelled peer discards its queued frames, releasing only its receipts.
        slow_in.close();
        drop(slow_in);
        for sequence in (1..=16).step_by(2) {
            let exact = acknowledgements.recv().await.unwrap();
            assert_eq!(exact, sequence);
            assert_eq!(outstanding.remove(&exact), Some("slow"));
        }
        assert!(outstanding.is_empty());
        assert!(acknowledgements.try_recv().is_err());
    }
}
