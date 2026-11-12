//! Ordered ciphertext transport over a permanent relay and an optional WebRTC path.
//!
//! Feed **every** Noise record through this module, starting with its first handshake
//! record. Enable direct only after authenticating and exchanging a fresh 32-byte
//! signaling key inside Noise. Application encryption and authorization remain with
//! the caller. Carrier acknowledgements must be handled outside this overlay.
mod p2p;
mod signal;
mod wire;

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, interval, timeout},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub use neko_remoted_protocol::{MAX_WIRE_PAYLOAD, messages::ConnectionKind};
use signal::{Cipher, Signal};

/// Maximum ciphertext size, before the overlay header.
pub const MAX_DATA: usize = neko_remoted_protocol::MAX_CIPHERTEXT;
/// Maximum outstanding records in each direction, including unconsumed records.
pub const WINDOW: usize = 8;
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const DIRECT_DEADLINE: Duration = Duration::from_secs(15);
const CARRIER_DEADLINE: Duration = Duration::from_secs(5);
const MAX_EARLY_SIGNALS: usize = signal::MAX_SIGNALS;

/// The existing WebSocket writer completes this once the frame is written.
/// It must not wait for this overlay's consumer ACK to complete the write.
pub struct Write {
    pub data: Vec<u8>,
    pub completed: oneshot::Sender<Result<()>>,
}

/// Bounded application-owned relay queues. A carrier failure ends this connection,
/// including its optional direct path. Reconnection creates a fresh transport.
pub struct Carrier {
    pub outgoing: mpsc::Sender<Write>,
    pub incoming: mpsc::Receiver<Result<Vec<u8>>>,
}

/// The authenticated client initiates; the host responds. Never enable twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Responder,
}
impl Role {
    fn other(self) -> Self {
        match self {
            Self::Initiator => Self::Responder,
            Self::Responder => Self::Initiator,
        }
    }
}

/// Cloneable sending half. Success means this one record is accepted; the caller
/// must never retry it. Waiting for the eight-record window provides backpressure.
#[derive(Clone)]
pub struct Sender {
    commands: mpsc::Sender<Command>,
    window: Arc<Semaphore>,
    direct_requested: Arc<AtomicBool>,
    kind: watch::Receiver<ConnectionKind>,
    terminal: Arc<Terminal>,
}

impl Sender {
    /// Accepted records survive direct failure as their original ciphertext bytes.
    /// Once queued, cancelling this future does not retract the accepted record.
    pub async fn send(&self, data: Vec<u8>) -> Result<()> {
        ensure!(data.len() <= MAX_DATA, "ciphertext exceeds transport limit");
        let permit = self
            .window
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| self.terminal.error())?;
        let (complete, result) = oneshot::channel();
        self.commands
            .send(Command::Send {
                data,
                permit,
                complete,
            })
            .await
            .map_err(|_| self.terminal.error())?;
        result.await.map_err(|_| self.terminal.error())?
    }

    /// Begin the connection's single direct attempt. The key must be random,
    /// unique to this connection and exchanged through authenticated Noise.
    /// This synchronous method never blocks the caller's receive loop.
    pub fn enable_direct(&self, role: Role, key: [u8; 32]) -> Result<()> {
        ensure!(
            self.direct_requested
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "direct already enabled"
        );
        if self
            .commands
            .try_send(Command::Enable {
                role,
                key: Zeroizing::new(key),
            })
            .is_err()
        {
            self.direct_requested.store(false, Ordering::Release);
            bail!("transport closed or command queue full");
        }
        Ok(())
    }

    /// Connecting continues to carry business records over the relay. Direct is
    /// reported only after this connection's authenticated DataChannel opens.
    pub fn kind(&self) -> ConnectionKind {
        *self.kind.borrow()
    }

    /// Changes are coalesced; this is state, not a delivery-event stream.
    pub fn subscribe_kind(&self) -> watch::Receiver<ConnectionKind> {
        self.kind.clone()
    }
}

/// Single consumer. `recv` is cancellation safe: no await occurs between removing
/// a record and notifying the actor of consumption. Ordered unique records drain
/// before the actor's terminal error is returned.
pub struct Receiver {
    incoming: mpsc::Receiver<Delivery>,
    consumed: mpsc::UnboundedSender<u64>,
    terminal: Arc<Terminal>,
}
impl Receiver {
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let record = self
            .incoming
            .recv()
            .await
            .ok_or_else(|| self.terminal.error())?;
        // A terminated actor cannot ACK, but its validated ordered records must
        // still drain. Return the saved terminal error once the queue is empty.
        let _ = self.consumed.send(record.seq);
        Ok(record.data)
    }
}

/// Own this for the connection's lifetime. Dropping it cancels both the actor and
/// ICE/DTLS/SCTP, including callbacks and timers. RTC cleanup runs asynchronously;
/// `shutdown` additionally waits for cleanup. This guard is deliberately not Clone.
pub struct Guard {
    cancel: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
}
impl Guard {
    pub async fn shutdown(mut self) -> Result<()> {
        self.cancel.cancel();
        self.wait().await
    }
    pub async fn join(mut self) -> Result<()> {
        self.wait().await
    }
    async fn wait(&mut self) -> Result<()> {
        self.task
            .take()
            .context("transport task already joined")?
            .await
            .context("transport task panicked")?
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Start on the current Tokio runtime. Keep `Guard` alive until the connection is
/// over, and pump both carrier queues concurrently with Noise send and receive.
pub fn start(carrier: Carrier) -> (Sender, Receiver, Guard) {
    start_with(carrier, p2p::Options::default())
}

fn start_with(carrier: Carrier, options: p2p::Options) -> (Sender, Receiver, Guard) {
    let (commands, receive_commands) = mpsc::channel(16);
    let (deliver, incoming) = mpsc::channel(WINDOW);
    // At most WINDOW consumption notifications can exist: an application cannot
    // receive another record until ACKs have freed slots at the peer.
    let (consumed, receive_consumed) = mpsc::unbounded_channel();
    let (kind_tx, kind) = watch::channel(ConnectionKind::Relay);
    let window = Arc::new(Semaphore::new(WINDOW));
    let terminal = Arc::new(Terminal::default());
    let sender = Sender {
        commands,
        window: window.clone(),
        direct_requested: Arc::new(AtomicBool::new(false)),
        kind,
        terminal: terminal.clone(),
    };
    let receiver = Receiver {
        incoming,
        consumed,
        terminal: terminal.clone(),
    };
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let (events_tx, events_rx) = mpsc::channel(signal::MAX_SIGNALS + WINDOW);
    let mut actor = Actor {
        carrier,
        commands: receive_commands,
        deliver,
        consumed: receive_consumed,
        kind: kind_tx,
        window,
        pending: VecDeque::new(),
        reorder: BTreeMap::new(),
        next_send: 0,
        next_deliver: 0,
        next_consume: 0,
        cipher: None,
        early_signals: Vec::new(),
        engine: None,
        retired_engine: None,
        events_tx,
        events_rx,
        direct_deadline: None,
        direct_disabled: false,
        options,
        terminal: terminal.clone(),
    };
    let task = tokio::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = stop.cancelled() => Ok(()),
            result = actor.run() => result,
        };
        let result = match result {
            Ok(()) => {
                terminal.set(Arc::new(
                    std::io::Error::new(
                        std::io::ErrorKind::ConnectionAborted,
                        "transport cancelled",
                    )
                    .into(),
                ));
                Ok(())
            }
            Err(error) => {
                let error = Arc::new(error);
                terminal.set(error.clone());
                Err(SharedError(error).into())
            }
        };
        actor.window.close();
        actor.kind.send_replace(ConnectionKind::Relay);
        if let Some(engine) = actor.engine.take() {
            engine.shutdown().await;
        }
        if let Some(engine) = actor.retired_engine.take() {
            engine.shutdown().await;
        }
        result
    });
    (
        sender,
        receiver,
        Guard {
            cancel,
            task: Some(task),
        },
    )
}

/// Preserve concrete carrier error types through a cloneable terminal-error slot.
/// `anyhow::Error::chain()` still reaches IO, WebSocket and authentication causes.
#[derive(Debug)]
struct SharedError(Arc<anyhow::Error>);
impl std::fmt::Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}
impl std::error::Error for SharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref().as_ref())
    }
}
#[derive(Default)]
struct Terminal(Mutex<Option<Arc<anyhow::Error>>>);
impl Terminal {
    fn set(&self, error: Arc<anyhow::Error>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
    }
    fn error(&self) -> anyhow::Error {
        match self.0.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(error) => SharedError(error.clone()).into(),
            None => anyhow::anyhow!("transport actor stopped unexpectedly"),
        }
    }
}

enum Command {
    Send {
        data: Vec<u8>,
        permit: OwnedSemaphorePermit,
        complete: oneshot::Sender<Result<()>>,
    },
    Enable {
        role: Role,
        key: Zeroizing<[u8; 32]>,
    },
}
struct Delivery {
    seq: u64,
    data: Vec<u8>,
}
struct Pending {
    seq: u64,
    frame: Vec<u8>,
    direct_since: Option<Instant>,
    _permit: OwnedSemaphorePermit,
}
struct Actor {
    carrier: Carrier,
    commands: mpsc::Receiver<Command>,
    deliver: mpsc::Sender<Delivery>,
    consumed: mpsc::UnboundedReceiver<u64>,
    kind: watch::Sender<ConnectionKind>,
    window: Arc<Semaphore>,
    pending: VecDeque<Pending>,
    reorder: BTreeMap<u64, Vec<u8>>,
    next_send: u64,
    next_deliver: u64,
    next_consume: u64,
    cipher: Option<Cipher>,
    early_signals: Vec<Vec<u8>>,
    engine: Option<p2p::Engine>,
    retired_engine: Option<p2p::Engine>,
    events_tx: mpsc::Sender<p2p::Event>,
    events_rx: mpsc::Receiver<p2p::Event>,
    direct_deadline: Option<Instant>,
    direct_disabled: bool,
    options: p2p::Options,
    terminal: Arc<Terminal>,
}

impl Actor {
    async fn run(&mut self) -> Result<()> {
        let mut clock = interval(Duration::from_millis(50));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = self.deliver.closed() => return Ok(()),
                command = self.commands.recv() => match command {
                    None => return Ok(()),
                    Some(Command::Send { data, permit, complete }) => {
                        let result = self.send(data, permit).await;
                        if let Err(error) = result {
                            let error = Arc::new(error);
                            self.terminal.set(error.clone());
                            let _ = complete.send(Err(SharedError(error.clone()).into()));
                            return Err(SharedError(error).into());
                        }
                        let _ = complete.send(Ok(()));
                    }
                    Some(Command::Enable { role, key }) => self.enable(role, *key).await?,
                },
                incoming = self.carrier.incoming.recv() => {
                    let bytes = incoming.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "relay disconnected"))??;
                    self.receive(&bytes, false).await?;
                }
                consumed = self.consumed.recv() => {
                    let seq = consumed.context("application receiver closed")?;
                    ensure!(seq == self.next_consume, "invalid local consumption order");
                    self.next_consume = self.next_consume.checked_add(1).context("receive sequence exhausted")?;
                    self.relay(wire::ack(self.next_consume)).await?;
                }
                event = self.events_rx.recv(), if self.engine.is_some() => {
                    if let Some(event) = event { self.direct_event(event).await?; }
                }
                _ = clock.tick() => {
                    let now = Instant::now();
                    let timed_out = self.engine.as_ref().is_some_and(p2p::Engine::is_closed)
                        || self.direct_deadline.is_some_and(|deadline| now >= deadline)
                        || self.pending.iter().any(|record| record.direct_since.is_some_and(|sent| now.duration_since(sent) >= ACK_TIMEOUT));
                    if timed_out { self.fallback(true).await?; }
                }
            }
        }
    }

    async fn relay(&self, data: Vec<u8>) -> Result<()> {
        let (completed, result) = oneshot::channel();
        timeout(CARRIER_DEADLINE, async {
            self.carrier
                .outgoing
                .send(Write { data, completed })
                .await
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "relay writer closed")
                })?;
            result.await.map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "relay write cancelled")
            })?
        })
        .await
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error))?
    }

    async fn send(&mut self, data: Vec<u8>, permit: OwnedSemaphorePermit) -> Result<()> {
        let seq = self.next_send;
        self.next_send = seq.checked_add(1).context("send sequence exhausted")?;
        let frame = wire::data(seq, &data);
        let direct = *self.kind.borrow() == ConnectionKind::Direct;
        self.pending.push_back(Pending {
            seq,
            frame: frame.clone(),
            direct_since: direct.then(Instant::now),
            _permit: permit,
        });
        if direct {
            if let Some(engine) = &self.engine
                && engine.commands.try_send(p2p::Command::Data(frame)).is_ok()
            {
                return Ok(());
            }
            self.fallback(true).await
        } else {
            self.relay(frame).await
        }
    }

    async fn receive(&mut self, bytes: &[u8], direct: bool) -> Result<()> {
        match wire::parse(bytes)? {
            wire::Frame::Data(seq, payload) => {
                if seq < self.next_deliver {
                    // Replayed WSS copies must not enter the Noise receive nonce again.
                    self.relay(wire::ack(self.next_consume)).await?;
                    return Ok(());
                }
                ensure!(
                    wire::in_window(seq, self.next_consume),
                    "receive window exceeded"
                );
                if let Some(existing) = self.reorder.get(&seq) {
                    ensure!(existing == payload, "conflicting ciphertext sequence");
                } else {
                    self.reorder.insert(seq, payload.to_vec());
                }
                while let Some(data) = self.reorder.remove(&self.next_deliver) {
                    self.deliver
                        .try_send(Delivery {
                            seq: self.next_deliver,
                            data,
                        })
                        .map_err(|_| anyhow::anyhow!("receive queue closed or window exceeded"))?;
                    self.next_deliver = self
                        .next_deliver
                        .checked_add(1)
                        .context("receive sequence exhausted")?;
                }
            }
            wire::Frame::Ack(consumed) => {
                ensure!(!direct, "ACK must use permanent relay");
                ensure!(consumed <= self.next_send, "ACK exceeds sent sequence");
                while self
                    .pending
                    .front()
                    .is_some_and(|record| record.seq < consumed)
                {
                    self.pending.pop_front();
                }
            }
            wire::Frame::Signal(signal) => {
                ensure!(!direct, "signaling must use permanent relay");
                if self.cipher.is_none() {
                    ensure!(
                        self.early_signals.len() < MAX_EARLY_SIGNALS,
                        "too many early signals"
                    );
                    ensure!(signal.len() <= 32_808, "early signal size limit");
                    let retained: usize = self.early_signals.iter().map(Vec::len).sum();
                    ensure!(
                        retained + signal.len() <= MAX_WIRE_PAYLOAD,
                        "early signal byte budget exceeded"
                    );
                    self.early_signals.push(signal.to_vec());
                } else {
                    self.signal(signal).await?;
                }
            }
        }
        Ok(())
    }

    async fn enable(&mut self, role: Role, key: [u8; 32]) -> Result<()> {
        ensure!(self.cipher.is_none(), "direct already enabled");
        self.cipher = Some(Cipher::new(role, key));
        self.kind.send_replace(ConnectionKind::Connecting);
        self.direct_deadline = Some(Instant::now() + DIRECT_DEADLINE);
        self.engine = Some(p2p::Engine::start(
            role,
            self.options.clone(),
            self.events_tx.clone(),
        ));
        for signal in std::mem::take(&mut self.early_signals) {
            self.signal(&signal).await?;
        }
        Ok(())
    }

    async fn signal(&mut self, encrypted: &[u8]) -> Result<()> {
        if self.direct_disabled {
            return Ok(());
        }
        let opened = self
            .cipher
            .as_mut()
            .context("signal key missing")?
            .open(encrypted);
        match opened {
            Ok(Some(Signal::RelayOnly)) => self.fallback(false).await,
            Ok(Some(signal)) => {
                if let Some(engine) = &self.engine
                    && engine
                        .commands
                        .try_send(p2p::Command::Signal(signal))
                        .is_ok()
                {
                    return Ok(());
                }
                self.fallback(true).await
            }
            Ok(None) => Ok(()),
            // A malformed or wrong-key signal disables only the optimization.
            Err(_) => self.fallback(true).await,
        }
    }

    async fn direct_event(&mut self, event: p2p::Event) -> Result<()> {
        if self.direct_disabled {
            return Ok(());
        }
        match event {
            p2p::Event::Signal(signal) => {
                let sealed = self
                    .cipher
                    .as_mut()
                    .context("signal key missing")?
                    .seal(&signal);
                match sealed {
                    Ok(bytes) => self.relay(wire::signal(&bytes)).await,
                    Err(_) => self.fallback(true).await,
                }
            }
            p2p::Event::Opened => {
                self.direct_deadline = None;
                self.kind.send_replace(ConnectionKind::Direct);
                Ok(())
            }
            p2p::Event::Data(bytes) => self.receive(&bytes, true).await,
            p2p::Event::Failed => self.fallback(true).await,
        }
    }

    async fn fallback(&mut self, notify: bool) -> Result<()> {
        if self.direct_disabled {
            return Ok(());
        }
        self.direct_disabled = true;
        self.direct_deadline = None;
        self.kind.send_replace(ConnectionKind::Relay);
        // Cancellation is immediate; async RTC cleanup must not delay the replay.
        if let Some(engine) = self.engine.take() {
            engine.cancel();
            self.retired_engine = Some(engine);
        }
        if notify
            && let Some(cipher) = &mut self.cipher
            && let Ok(sealed) = cipher.seal(&Signal::RelayOnly)
        {
            self.relay(wire::signal(&sealed)).await?;
        }
        let replay: Vec<_> = self
            .pending
            .iter_mut()
            .map(|record| {
                record.direct_since = None;
                record.frame.clone()
            })
            .collect();
        for frame in replay {
            self.relay(frame).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_nat;
