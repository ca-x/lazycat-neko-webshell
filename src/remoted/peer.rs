use super::{
    admission::Admission,
    authority,
    files::Helper,
    heartbeat::Heartbeat,
    relay::{Input, Link},
    store::{Device, Grant, Pending},
    terminals::Terminal,
};
use crate::state::AppState;
use anyhow::{Context, Result, bail, ensure};
use neko_remoted_protocol::{
    self as protocol,
    crypto::{Channel, Handshake, Mode},
    messages::{Event, Operation, Request, Response},
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, Weak},
    time::{Duration, Instant},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::timeout,
};
static TRANSFERS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(8)));
static DEVICE_TRANSFERS: LazyLock<Mutex<HashMap<String, Weak<Semaphore>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn transfer_permits(device: &str) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
    let mut limits = DEVICE_TRANSFERS
        .lock()
        .map_err(|_| anyhow::anyhow!("transfer limits unavailable"))?;
    limits.retain(|_, semaphore| semaphore.strong_count() > 0);
    let limit = limits
        .get(device)
        .and_then(Weak::upgrade)
        .unwrap_or_else(|| {
            let limit = Arc::new(Semaphore::new(2));
            limits.insert(device.to_owned(), Arc::downgrade(&limit));
            limit
        });
    let device = limit
        .try_acquire_owned()
        .context("device transfer capacity reached")?;
    let host = TRANSFERS
        .clone()
        .try_acquire_owned()
        .context("host transfer capacity reached")?;
    Ok((device, host))
}

struct PendingCleanup<'a> {
    manager: &'a super::store::Remoted,
    id: String,
}
impl Drop for PendingCleanup<'_> {
    fn drop(&mut self) {
        if let Ok(mut runtime) = self.manager.runtime.lock() {
            runtime.pending.remove(&self.id);
        }
    }
}

#[derive(Debug)]
struct AuthorizationError(String);
impl std::fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for AuthorizationError {}

fn require_write(device: &Device) -> Result<()> {
    if !device.permission.can_write() {
        return Err(AuthorizationError("只读设备不能执行写操作".into()).into());
    }
    Ok(())
}

fn require_herdr_control(method: &str, controlled: bool) -> Result<()> {
    if matches!(
        method,
        "pane.send_text" | "pane.send_keys" | "pane.send_input" | "pane.resize"
    ) && !controlled
    {
        return Err(AuthorizationError("请先附着该Herdr pane并取得终端控制权".into()).into());
    }
    Ok(())
}

struct Transfer {
    helper: Helper,
    grant: Grant,
    token: String,
    size: u64,
    offset: u64,
    upload: bool,
    _permit: OwnedSemaphorePermit,
    _device_permit: OwnedSemaphorePermit,
}

pub async fn run(
    state: &Arc<AppState>,
    link: &Link,
    mut incoming: mpsc::Receiver<Input>,
) -> Result<()> {
    let saved = state.remoted.snapshot()?;
    ensure!(saved.enabled, "remote control disabled");
    let relay = saved.relay.as_ref().context("relay missing")?;
    let Input::Hello(mode) = receive(link, &mut incoming).await? else {
        bail!("expected mode");
    };
    let psk = if mode == Mode::Pair {
        let rt = state
            .remoted
            .runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("remote state unavailable"))?;
        let (code, at) = rt.invitation.as_ref().context("no pairing invitation")?;
        ensure!(
            at.elapsed() < Duration::from_mins(5),
            "pairing invitation expired"
        );
        Some(*code.psk())
    } else {
        None
    };
    let mut hs = Handshake::new(
        &saved.identity()?,
        relay,
        &saved.route()?,
        mode,
        false,
        psk.as_ref(),
        None,
    )?;
    hs.read(&handshake_data(link, &mut incoming).await?)?;
    link.send(&hs.write()?).await?;
    if !hs.finished() {
        hs.read(&handshake_data(link, &mut incoming).await?)?;
    }
    let (mut channel, key) = hs.into_channel()?;
    let key = protocol::encode(&key);
    let mut last = 0;
    let device = if mode == Mode::Pair {
        pair_device(
            state,
            link,
            &mut incoming,
            &mut channel,
            &mut last,
            PairingIdentity {
                key: &key,
                psk: psk.as_ref().context("PSK missing")?,
                host_key: &saved.public_key,
            },
        )
        .await?
    } else {
        saved
            .devices
            .into_iter()
            .find(|d| d.public_key == key)
            .context("device is not paired")?
    };
    state
        .remoted
        .current_device(&device.id, device.generation)?;
    if let Ok(mut rt) = state.remoted.runtime.lock()
        && let Some(p) = rt.peers.get_mut(&link.peer)
    {
        p.device = Some(device.id.clone());
    }
    send(
        link,
        &mut channel,
        0,
        Event::Ready {
            device_id: device.id.clone(),
            permission: device.permission,
            resources: device.grants.iter().map(Grant::resource).collect(),
        },
    )
    .await?;
    serve_authenticated(state, link, incoming, channel, last, device, spawn_work).await
}

struct PairingIdentity<'a> {
    key: &'a str,
    psk: &'a [u8; 32],
    host_key: &'a str,
}
async fn pair_device(
    state: &Arc<AppState>,
    link: &Link,
    incoming: &mut mpsc::Receiver<Input>,
    channel: &mut Channel,
    last: &mut u64,
    identity: PairingIdentity<'_>,
) -> Result<Device> {
    let PairingIdentity { key, psk, host_key } = identity;
    {
        let mut rt = state
            .remoted
            .runtime
            .lock()
            .map_err(|_| anyhow::anyhow!("remote state unavailable"))?;
        let (code, at) = rt
            .invitation
            .as_ref()
            .context("pairing invitation no longer available")?;
        let mut diff = 0;
        for (a, b) in code.psk().iter().zip(psk) {
            diff |= a ^ b;
        }
        ensure!(
            diff == 0 && at.elapsed() < Duration::from_mins(5),
            "pairing invitation changed or expired"
        );
        rt.pending.retain(|_, pending| {
            !pending.decision.is_closed() && pending.created.elapsed() < Duration::from_mins(2)
        });
        ensure!(rt.pending.len() < 8, "too many pending approvals");
        rt.invitation = None;
    }
    let request = read_request(link, incoming, channel, last).await?;
    let Operation::Identify { name } = request.operation else {
        bail!("identify required");
    };
    ensure!(
        !name.trim().is_empty() && name.len() <= 160 && !name.chars().any(char::is_control),
        "invalid device name"
    );
    let (id, decision) = (uuid::Uuid::new_v4().to_string(), oneshot::channel());
    state
        .remoted
        .runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("remote state unavailable"))?
        .pending
        .insert(
            id.clone(),
            Pending {
                id: id.clone(),
                name,
                key: key.to_owned(),
                created: Instant::now(),
                decision: decision.0,
            },
        );
    let _pending_cleanup = PendingCleanup {
        manager: &state.remoted,
        id,
    };
    send(
        link,
        channel,
        request.id,
        Event::Pending {
            fingerprint: protocol::fingerprint(&protocol::decode32(key)?),
        },
    )
    .await?;
    let selected = tokio::select! {biased;()=link.cancel.cancelled()=>bail!("pairing cancelled"),r=timeout(Duration::from_mins(2),decision.1)=>r.context("pairing approval expired")?.context("pairing rejected")?};
    ensure!(selected.public_key == key, "pairing identity mismatch");
    send(
        link,
        channel,
        0,
        Event::Approved {
            host_key: host_key.to_owned(),
            device_id: selected.id.clone(),
            permission: selected.permission,
        },
    )
    .await?;
    let ack = read_request(link, incoming, channel, last).await?;
    ensure!(
        matches!(ack.operation, Operation::AcknowledgePairing {}),
        "pairing persistence acknowledgement required"
    );
    Ok(selected)
}
#[derive(Default)]
struct Resources {
    terminals: HashMap<String, Terminal>,
    transfers: HashMap<String, Transfer>,
}
impl Resources {
    async fn close(&mut self) {
        self.terminals.clear();
        for (_, mut transfer) in self.transfers.drain() {
            transfer.helper.close().await;
        }
    }
}
struct Work {
    state: Arc<AppState>,
    device: Device,
    cancel: tokio_util::sync::CancellationToken,
    request: Request,
    interrupt: tokio_util::sync::CancellationToken,
    events: mpsc::Sender<Event>,
    resources: Resources,
    admission: Admission,
}
struct Completed {
    id: u64,
    event: Event,
    denied: bool,
    resources: Resources,
}

async fn finish_target(
    cancel: &tokio_util::sync::CancellationToken,
    admission: &Admission,
    target: impl std::future::Future<Output = Result<Event>>,
) -> Result<Event> {
    tokio::pin!(target);
    tokio::select! {
        biased;
        () = cancel.cancelled() => {
            if admission.submitted() { target.await }
            else { Err(AuthorizationError("operation cancelled before dispatch".into()).into()) }
        },
        result = &mut target => result,
    }
}

async fn finish_interruptible(
    cancel: &tokio_util::sync::CancellationToken,
    interrupt: &tokio_util::sync::CancellationToken,
    admission: &Admission,
    target: impl std::future::Future<Output = Result<Event>>,
) -> Result<Event> {
    let target = finish_target(cancel, admission, target);
    tokio::pin!(target);
    tokio::select! {
        biased;
        () = interrupt.cancelled() => {
            if admission.mutation_submitted() { target.await }
            else { anyhow::bail!("file operation cancelled before mutation dispatch") }
        },
        result = &mut target => result,
    }
}

fn transfer_id(operation: &Operation) -> Option<&str> {
    match operation {
        Operation::FileChunk { transfer, .. }
        | Operation::FileCommit { transfer }
        | Operation::TransferAcknowledge { transfer, .. } => Some(transfer),
        _ => None,
    }
}

fn spawn_work(mut work: Work) -> tokio::task::JoinHandle<Completed> {
    tokio::spawn(async move {
        let mut outcome_unknown = false;
        let result = {
            let context = OperationContext {
                state: &work.state,
                cancel: &work.cancel,
                device: &work.device,
                events: &work.events,
                admission: &work.admission,
            };
            let operation = operation(
                &context,
                work.request.operation,
                &mut work.resources,
                &mut outcome_unknown,
            );
            finish_interruptible(&work.cancel, &work.interrupt, &work.admission, operation).await
        };
        work.admission.finish(result.is_ok(), outcome_unknown);
        let poisoned: Vec<_> = work
            .resources
            .transfers
            .iter()
            .filter(|(_, transfer)| transfer.helper.is_poisoned())
            .map(|(id, _)| id.clone())
            .collect();
        for id in poisoned {
            if let Some(mut transfer) = work.resources.transfers.remove(&id) {
                transfer.helper.close().await;
            }
        }
        let denied = result
            .as_ref()
            .err()
            .is_some_and(anyhow::Error::is::<AuthorizationError>);
        let event = match result {
            Ok(event) => event,
            Err(error) => Event::Error {
                code: if denied {
                    "AUTHORIZATION_FAILED"
                } else {
                    "REQUEST_FAILED"
                }
                .into(),
                message: error.to_string(),
                outcome_unknown,
            },
        };
        if work.cancel.is_cancelled() {
            work.resources.close().await;
        }
        Completed {
            id: work.request.id,
            event,
            denied,
            resources: work.resources,
        }
    })
}

async fn serve_authenticated(
    state: &Arc<AppState>,
    link: &Link,
    mut incoming: mpsc::Receiver<Input>,
    mut channel: Channel,
    mut last: u64,
    device: Device,
    worker: impl Fn(Work) -> tokio::task::JoinHandle<Completed>,
) -> Result<()> {
    let (events, mut output) = mpsc::channel(8);
    let mut resources = Some(Resources::default());
    let mut streams = std::collections::HashSet::new();
    let mut deferred = std::collections::VecDeque::new();
    let mut queued: std::collections::VecDeque<Request> = std::collections::VecDeque::new();
    let mut pending: Option<tokio::task::JoinHandle<Completed>> = None;
    let mut active_transfer: Option<(String, tokio_util::sync::CancellationToken)> = None;
    let mut auditing: Option<tokio::task::JoinHandle<Result<()>>> = None;
    let mut ticks = tokio::time::interval(Duration::from_secs(10));
    let mut heartbeat = Heartbeat::default();
    let result = async {
        loop {
            tokio::select! {
                () = link.cancel.cancelled() => break,
                completed = async { pending.as_mut().expect("pending work").await }, if pending.is_some() => {
                    pending = None;
                    active_transfer = None;
                    let completed = completed.context("remote operation worker failed")?;
                    resources = Some(completed.resources);
                    streams = resources.as_ref().context("remote resources unavailable")?.terminals.keys().cloned().collect();
                    state.remoted.current_device(&device.id, device.generation)?;
                    send(link, &mut channel, completed.id, completed.event).await?;
                    if completed.denied { break; }
                },
                checked = async { auditing.as_mut().expect("pending audit").await }, if auditing.is_some() => {
                    auditing = None;
                    if let Err(error) = checked.context("remote authorization audit failed")? {
                        send(link, &mut channel, 0, Event::Error { code: "AUTHORIZATION_FAILED".into(), message: error.to_string(), outcome_unknown: false }).await?;
                        break;
                    }
                },
                packet = incoming.recv() => {
                    let Some(Input::Data(cipher)) = packet else { break; };
                    let request: Request = protocol::unpack(&channel.decrypt(&cipher)?)?;
                    ensure!(request.id > last, "request sequence replayed");
                    last = request.id;
                    state.remoted.current_device(&device.id, device.generation)?;
                    match request.operation {
                        Operation::Pong { nonce } => {
                            if let Some(rtt) = heartbeat.pong(&nonce)
                                && let Ok(mut runtime) = state.remoted.runtime.lock()
                                && let Some(peer) = runtime.peers.get_mut(&link.peer) { peer.rtt = Some((rtt, Instant::now())); }
                        },
                        Operation::Ping { nonce } => {
                            ensure!(nonce.len() <= 64, "nonce too long");
                            send(link, &mut channel, request.id, Event::Pong { nonce }).await?;
                        },
                        operation => enqueue_business(link, &mut channel, Request { id: request.id, operation }, &mut queued, active_transfer.as_ref()).await?,
                    }
                },
                () = async {}, if pending.is_none() && deferred.is_empty() && !queued.is_empty() => {
                            let request = queued.pop_front().context("business queue unavailable")?;
                            let id = request.id;
                            let interrupt = link.cancel.child_token();
                            let admission = match admit_request(state, &device, link, &request, interrupt.clone()) {
                                Ok(admission) => admission,
                                Err(error) => {
                                    let busy = error.is::<super::admission::Busy>();
                                    send(link, &mut channel, id, Event::Error { code: if busy { "DEVICE_BUSY" } else { "AUTHORIZATION_FAILED" }.into(), message: error.to_string(), outcome_unknown: false }).await?;
                                    if !busy { break; }
                                    continue;
                                }
                            };
                            active_transfer = transfer_id(&request.operation).map(|id| (id.to_owned(), interrupt.clone()));
                            pending = Some(worker(Work { state: state.clone(), device: device.clone(), cancel: link.cancel.clone(),
                                request, interrupt, events: events.clone(),
                                resources: resources.take().context("remote resources unavailable")?, admission }));
                },
                Some(event) = output.recv(), if deferred.len() < 8 && (pending.is_some() || deferred.is_empty()) => {
                    state.remoted.current_device(&device.id, device.generation)?;
                    let stream = match &event { Event::TerminalOutput { stream, .. } | Event::TerminalExit { stream, .. } => Some(stream), _ => None };
                    if stream.is_some_and(|stream| !streams.contains(stream)) {
                        if pending.is_some() { deferred.push_back(event); }
                    } else { send(link, &mut channel, 0, event).await?; }
                },
                () = async {}, if pending.is_none() && !deferred.is_empty() => {
                    let event = deferred.pop_front().context("deferred output unavailable")?;
                    let stream = match &event { Event::TerminalOutput { stream, .. } | Event::TerminalExit { stream, .. } => Some(stream), _ => None };
                    if stream.is_none_or(|stream| streams.contains(stream)) { send(link, &mut channel, 0, event).await?; }
                },
                _ = ticks.tick() => {
                    state.remoted.current_device(&device.id, device.generation)?;
                    if let Some(nonce) = heartbeat.tick()? { send(link, &mut channel, 0, Event::Ping { nonce }).await?; }
                    if auditing.is_none() {
                        auditing = Some(spawn_audit(state.clone(), device.clone(), link.cancel.clone()));
                    }
                },
            }
        }
        Ok(())
    }.await;
    close_connection(link, auditing, resources).await;
    result
}

async fn enqueue_business(
    link: &Link,
    channel: &mut Channel,
    request: Request,
    queued: &mut std::collections::VecDeque<Request>,
    active: Option<&(String, tokio_util::sync::CancellationToken)>,
) -> Result<()> {
    let cancelling = matches!(request.operation, Operation::Cancel { .. });
    // Reserve one bounded slot so a full business FIFO cannot block cancellation.
    if queued.len() >= if cancelling { 9 } else { 8 } {
        send(
            link,
            channel,
            request.id,
            Event::Error {
                code: "QUEUE_FULL".into(),
                message: "business request queue is full; this request was not admitted".into(),
                outcome_unknown: false,
            },
        )
        .await?;
    } else if cancelling {
        if let Operation::Cancel { transfer } = &request.operation
            && let Some((id, interrupt)) = active
            && id == transfer
        {
            interrupt.cancel();
        }
        queued.push_front(request);
    } else {
        queued.push_back(request);
    }
    Ok(())
}

async fn close_connection(
    link: &Link,
    auditing: Option<tokio::task::JoinHandle<Result<()>>>,
    resources: Option<Resources>,
) {
    link.cancel.cancel();
    if let Some(audit) = auditing {
        audit.abort();
    }
    if let Some(mut resources) = resources {
        resources.close().await;
    }
}

fn admit_request(
    state: &Arc<AppState>,
    device: &Device,
    link: &Link,
    request: &Request,
    interrupt: tokio_util::sync::CancellationToken,
) -> Result<Admission> {
    let admission = state
        .remoted
        .admit(device, &link.peer, request.id, interrupt)?;
    admission.describe(match &request.operation {
        Operation::TerminalAttach { .. } => "terminal_attach",
        Operation::TerminalDetach { .. } => "terminal_detach",
        Operation::TerminalControl { .. } => "terminal_control",
        Operation::TerminalInput { .. } => "terminal_input",
        Operation::TerminalResize { .. } => "terminal_resize",
        Operation::Herdr { .. } => "herdr",
        Operation::FileList { .. } => "file_list",
        Operation::FileDownload { .. } => "file_download",
        Operation::FileUpload { .. } => "file_upload",
        Operation::FileChunk { .. } => "file_chunk",
        Operation::FileCommit { .. } => "file_commit",
        Operation::TransferAcknowledge { .. } => "download_acknowledge",
        Operation::Cancel { .. } => "transfer_cancel",
        _ => "business",
    })?;

    Ok(admission)
}
fn spawn_audit(
    state: Arc<AppState>,
    device: Device,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(async move {
        tokio::select! {
            () = cancel.cancelled() => Ok(()),
            checked = timeout(Duration::from_secs(30), async {
                for resource in &device.grants { grant(&state, &device, &resource.id).await?; }
                Ok::<_, anyhow::Error>(())
            }) => checked.context("authorization audit timed out")?,
        }
    })
}

async fn receive(link: &Link, incoming: &mut mpsc::Receiver<Input>) -> Result<Input> {
    tokio::select! {()=link.cancel.cancelled()=>bail!("connection cancelled"),r=timeout(Duration::from_secs(10),incoming.recv())=>r.context("handshake timed out")?.context("connection closed")}
}
async fn handshake_data(link: &Link, incoming: &mut mpsc::Receiver<Input>) -> Result<Vec<u8>> {
    match receive(link, incoming).await? {
        Input::Data(b) => {
            ensure!(b.len() <= 256, "handshake too large");
            Ok(b)
        }
        Input::Hello(_) => bail!("unexpected handshake frame"),
    }
}
async fn read_request(
    link: &Link,
    incoming: &mut mpsc::Receiver<Input>,
    channel: &mut Channel,
    last: &mut u64,
) -> Result<Request> {
    let Input::Data(cipher) = receive(link, incoming).await? else {
        bail!("expected encrypted message")
    };
    let r: Request = protocol::unpack(&channel.decrypt(&cipher)?)?;
    ensure!(r.id > *last, "request replayed");
    *last = r.id;
    Ok(r)
}
async fn send(link: &Link, channel: &mut Channel, id: u64, event: Event) -> Result<()> {
    let bytes = protocol::pack(&Response { id, event })?;
    timeout(Duration::from_secs(5), link.send(&channel.encrypt(&bytes)?))
        .await
        .context("remote output stalled")?
}
async fn grant(state: &Arc<AppState>, device: &Device, id: &str) -> Result<Grant> {
    let result = async {
        let grant = device
            .grants
            .iter()
            .find(|g| g.id == id)
            .cloned()
            .context("resource was not authorized")?;
        authority::authorize(state, &grant, &state.remoted.snapshot()?.owner).await?;
        state
            .remoted
            .current_device(&device.id, device.generation)?;
        Ok::<_, anyhow::Error>(grant)
    }
    .await;
    result.map_err(|error| AuthorizationError(error.to_string()).into())
}
fn submit(admission: &Admission, mutating: bool) -> Result<()> {
    admission
        .submit(mutating)
        .map_err(|error| AuthorizationError(error.to_string()).into())
}

struct OperationContext<'a> {
    state: &'a Arc<AppState>,
    cancel: &'a tokio_util::sync::CancellationToken,
    device: &'a Device,
    events: &'a mpsc::Sender<Event>,
    admission: &'a Admission,
}
async fn operation(
    context: &OperationContext<'_>,
    operation: Operation,
    resources: &mut Resources,
    outcome_unknown: &mut bool,
) -> Result<Event> {
    match operation {
        Operation::Resources {} => Ok(Event::Resources {
            resources: context.device.grants.iter().map(Grant::resource).collect(),
        }),
        operation @ (Operation::TerminalAttach { .. }
        | Operation::TerminalDetach { .. }
        | Operation::TerminalControl { .. }
        | Operation::TerminalInput { .. }
        | Operation::TerminalResize { .. }) => {
            Box::pin(terminal_operation(
                context,
                operation,
                &mut resources.terminals,
                outcome_unknown,
            ))
            .await
        }
        operation @ Operation::Herdr { .. } => {
            Box::pin(herdr_operation(
                context,
                operation,
                &resources.terminals,
                outcome_unknown,
            ))
            .await
        }
        operation @ (Operation::FileList { .. }
        | Operation::FileDownload { .. }
        | Operation::FileUpload { .. }) => {
            Box::pin(file_start(
                context,
                operation,
                &mut resources.transfers,
                outcome_unknown,
            ))
            .await
        }
        operation @ (Operation::FileChunk { .. }
        | Operation::FileCommit { .. }
        | Operation::TransferAcknowledge { .. }
        | Operation::Cancel { .. }) => {
            Box::pin(transfer_operation(
                context,
                operation,
                &mut resources.transfers,
                outcome_unknown,
            ))
            .await
        }
        _ => bail!("operation is not valid in the current state"),
    }
}

async fn terminal_operation(
    context: &OperationContext<'_>,
    operation: Operation,
    terminals: &mut HashMap<String, Terminal>,
    outcome_unknown: &mut bool,
) -> Result<Event> {
    let state = context.state;
    let cancel = context.cancel;
    let device = context.device;
    let events = context.events;
    let admission = context.admission;
    match operation {
        Operation::TerminalAttach { grant: id, pane } => {
            ensure!(terminals.len() < 8, "terminal capacity reached");
            let grant = grant(state, device, &id).await?;
            let stream = uuid::Uuid::new_v4().to_string();
            let name = grant.name.clone();
            submit(admission, false)?;
            let terminal = Terminal::attach(
                state.clone(),
                grant,
                pane,
                !device.permission.can_write(),
                stream.clone(),
                events.clone(),
                cancel.child_token(),
            )
            .await?;
            terminals.insert(stream.clone(), terminal);
            Ok(Event::TerminalAttached {
                stream,
                name,
                read_only: !device.permission.can_write(),
            })
        }
        Operation::TerminalDetach { stream } => {
            submit(admission, false)?;
            terminals
                .remove(&stream)
                .context("unknown terminal stream")?;
            Ok(Event::Ok)
        }
        Operation::TerminalControl { stream, take } => {
            if take {
                require_write(device)?;
            }
            let t = terminals
                .get_mut(&stream)
                .context("unknown terminal stream")?;
            grant(state, device, &t.grant.id).await?;
            submit(admission, take)?;
            let controller = t.control(take)?;
            Ok(Event::TerminalControl { stream, controller })
        }
        Operation::TerminalInput { stream, data } => {
            require_write(device)?;
            let bytes = protocol::decode(&data, 16 * 1024)?;
            let t = terminals
                .get_mut(&stream)
                .context("unknown terminal stream")?;
            grant(state, device, &t.grant.id).await?;
            ensure!(t.controls(), "请先取得终端控制权");
            submit(admission, true)?;
            *outcome_unknown = true;
            t.input(bytes).await?;
            Ok(Event::Ok)
        }
        Operation::TerminalResize { stream, cols, rows } => {
            require_write(device)?;
            let t = terminals
                .get_mut(&stream)
                .context("unknown terminal stream")?;
            grant(state, device, &t.grant.id).await?;
            ensure!(t.controls(), "请先取得终端控制权");
            crate::validation::validate_size(cols, rows)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            submit(admission, true)?;
            *outcome_unknown = true;
            t.resize(cols, rows).await?;
            Ok(Event::Ok)
        }

        _ => bail!("invalid terminal_operation request"),
    }
}

async fn herdr_operation(
    context: &OperationContext<'_>,
    operation: Operation,
    terminals: &HashMap<String, Terminal>,
    outcome_unknown: &mut bool,
) -> Result<Event> {
    let state = context.state;
    let device = context.device;
    let admission = context.admission;
    match operation {
        Operation::Herdr {
            grant: id,
            method,
            params,
        } => {
            if matches!(method.as_str(), "events.subscribe" | "agent.prompt")
                || !protocol::messages::herdr_allowed(device.permission, &method)
            {
                return Err(AuthorizationError("Herdr方法没有远程授权".into()).into());
            }
            let g = grant(state, device, &id).await?;
            ensure!(g.backend == "herdr", "resource is not Herdr");
            if method.starts_with("pane.") && method != "pane.list" {
                let pane = params["pane_id"].as_str().context("pane_id required")?;
                submit(admission, false)?;
                let panes =
                    crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter::new(
                        state.clone(),
                    )
                    .list_panes(&g.session)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                ensure!(
                    panes.iter().any(|p| p.pane_id == pane),
                    "Herdr pane不属于授权会话"
                );
                if matches!(
                    method.as_str(),
                    "pane.send_text" | "pane.send_keys" | "pane.send_input" | "pane.resize"
                ) {
                    let controlled = terminals.values().any(|terminal| {
                        terminal.grant.id == g.id
                            && matches!(&terminal.backend, super::terminals::Backend::Herdr(attached) if attached == pane)
                            && terminal.controls()
                    });
                    require_herdr_control(&method, controlled)?;
                }
            }
            state
                .remoted
                .current_device(&device.id, device.generation)?;
            let mutating = matches!(
                method.as_str(),
                "pane.send_text"
                    | "pane.send_keys"
                    | "pane.send_input"
                    | "pane.resize"
                    | "agent.prompt"
            );
            submit(admission, mutating)?;
            *outcome_unknown = mutating;
            let data = crate::herdr::run_remoted_request(&g.selector, &method, params)
                .await
                .map_err(anyhow::Error::msg)?;
            Ok(Event::Herdr { data })
        }

        _ => bail!("invalid herdr_operation request"),
    }
}

async fn file_start(
    context: &OperationContext<'_>,
    operation: Operation,
    transfers: &mut HashMap<String, Transfer>,
    outcome_unknown: &mut bool,
) -> Result<Event> {
    let state = context.state;
    let device = context.device;
    let admission = context.admission;
    match operation {
        Operation::FileList { grant: id, path } => {
            let g = grant(state, device, &id).await?;
            submit(admission, false)?;
            let mut helper = Helper::start(state, &g, true).await?;
            submit(admission, false)?;
            let reply = helper.call(json!({"action":"list","path":path})).await?;
            helper.close().await;
            Ok(Event::FileList {
                path,
                entries: serde_json::from_value(reply["entries"].clone())?,
                truncated: reply["truncated"].as_bool().unwrap_or(true),
            })
        }
        Operation::FileDownload { grant: id, path } => {
            ensure!(transfers.len() < 2, "device transfer capacity reached");
            let (device_permit, permit) = transfer_permits(&device.id)?;
            let g = grant(state, device, &id).await?;
            submit(admission, false)?;
            let mut helper = Helper::start(state, &g, true).await?;
            submit(admission, false)?;
            let reply = helper
                .call(json!({"action":"download","path":path}))
                .await?;
            let token = reply["transfer"]
                .as_str()
                .context("transfer ID missing")?
                .to_owned();
            let size = reply["size"].as_u64().context("transfer size missing")?;
            let id = uuid::Uuid::new_v4().to_string();
            transfers.insert(
                id.clone(),
                Transfer {
                    helper,
                    grant: g,
                    token,
                    size,
                    offset: 0,
                    upload: false,
                    _permit: permit,
                    _device_permit: device_permit,
                },
            );
            Ok(Event::TransferStarted { transfer: id, size })
        }
        Operation::FileUpload {
            grant: id,
            path,
            size,
            sha256,
            overwrite,
        } => {
            require_write(device)?;
            ensure!(transfers.len() < 2, "device transfer capacity reached");
            let (device_permit, permit) = transfer_permits(&device.id)?;
            let g = grant(state, device, &id).await?;
            submit(admission, false)?;
            let mut helper = Helper::start(state, &g, false).await?;
            submit(admission, true)?;
            *outcome_unknown = true;
            let reply=helper.call(json!({"action":"upload","path":path,"size":size,"sha256":sha256,"overwrite":overwrite})).await?;
            let token = reply["transfer"]
                .as_str()
                .context("transfer ID missing")?
                .to_owned();
            let id = uuid::Uuid::new_v4().to_string();
            transfers.insert(
                id.clone(),
                Transfer {
                    helper,
                    grant: g,
                    token,
                    size,
                    offset: 0,
                    upload: true,
                    _permit: permit,
                    _device_permit: device_permit,
                },
            );
            Ok(Event::TransferStarted { transfer: id, size })
        }

        _ => bail!("invalid file_start request"),
    }
}

async fn transfer_operation(
    context: &OperationContext<'_>,
    operation: Operation,
    transfers: &mut HashMap<String, Transfer>,
    outcome_unknown: &mut bool,
) -> Result<Event> {
    let state = context.state;
    let device = context.device;
    let admission = context.admission;
    match operation {
        Operation::FileChunk {
            transfer,
            offset,
            data,
        } => {
            require_write(device)?;
            protocol::decode(&data, protocol::FILE_CHUNK)?;
            let t = transfers.get_mut(&transfer).context("unknown transfer")?;
            ensure!(t.upload && t.offset == offset, "transfer offset mismatch");
            grant(state, device, &t.grant.id).await?;
            submit(admission, true)?;
            *outcome_unknown = true;
            let result = t
                .helper
                .call(json!({"action":"chunk","transfer":t.token,"offset":offset,"data":data}))
                .await?;
            t.offset = result["offset"].as_u64().context("offset missing")?;
            Ok(Event::TransferProgress {
                transfer,
                offset: t.offset,
            })
        }
        Operation::FileCommit { transfer } => {
            require_write(device)?;
            let mut t = transfers.remove(&transfer).context("unknown transfer")?;
            ensure!(t.upload && t.offset == t.size, "upload is incomplete");
            grant(state, device, &t.grant.id).await?;
            submit(admission, true)?;
            *outcome_unknown = true;
            let result = t
                .helper
                .call(json!({"action":"commit","transfer":t.token}))
                .await?;
            t.helper.close().await;
            Ok(Event::TransferComplete {
                transfer,
                size: t.size,
                sha256: result["sha256"].as_str().context("digest missing")?.into(),
            })
        }
        operation @ Operation::TransferAcknowledge { .. } => {
            download_chunk(context, operation, transfers).await
        }
        Operation::Cancel { transfer } => {
            submit(admission, false)?;
            if let Some(mut t) = transfers.remove(&transfer) {
                let _ = t
                    .helper
                    .call(json!({"action":"cancel","transfer":t.token}))
                    .await;
                t.helper.close().await;
            }
            Ok(Event::Cancelled { transfer })
        }

        _ => bail!("invalid transfer_operation request"),
    }
}

async fn download_chunk(
    context: &OperationContext<'_>,
    operation: Operation,
    transfers: &mut HashMap<String, Transfer>,
) -> Result<Event> {
    let state = context.state;
    let device = context.device;
    let admission = context.admission;
    match operation {
        Operation::TransferAcknowledge { transfer, offset } => {
            let t = transfers.get_mut(&transfer).context("unknown transfer")?;
            ensure!(!t.upload && offset == t.offset, "download offset mismatch");
            grant(state, device, &t.grant.id).await?;
            submit(admission, false)?;
            let reply = t
                .helper
                .call(json!({"action":"read","transfer":t.token,"offset":offset}))
                .await?;
            if reply["done"] == true {
                let mut t = transfers.remove(&transfer).context("transfer missing")?;
                t.helper.close().await;
                grant(state, device, &t.grant.id).await?;
                Ok(Event::TransferComplete {
                    transfer,
                    size: t.size,
                    sha256: reply["sha256"].as_str().context("digest missing")?.into(),
                })
            } else {
                let data = reply["data"].as_str().context("data missing")?.to_owned();
                let bytes = protocol::decode(&data, protocol::FILE_CHUNK)?;
                t.offset = t
                    .offset
                    .checked_add(u64::try_from(bytes.len())?)
                    .context("offset overflow")?;
                ensure!(t.offset <= t.size, "download exceeds advertised size");
                Ok(Event::FileChunk {
                    transfer,
                    offset,
                    data,
                })
            }
        }

        _ => bail!("invalid download_chunk request"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neko_remoted_protocol::{
        RelayAddress,
        crypto::Identity,
        messages::{Permission, RelayFrame},
    };
    use tokio_util::sync::CancellationToken;

    fn channels() -> (Channel, Channel) {
        let client = Identity::generate().unwrap();
        let host = Identity::generate().unwrap();
        let relay = RelayAddress::parse("https://relay.example").unwrap();
        let route = protocol::route(&protocol::random32().unwrap());
        let mut initiator = Handshake::new(
            &client,
            &relay,
            &route,
            Mode::Connect,
            true,
            None,
            Some(host.public()),
        )
        .unwrap();
        let mut responder =
            Handshake::new(&host, &relay, &route, Mode::Connect, false, None, None).unwrap();
        responder.read(&initiator.write().unwrap()).unwrap();
        initiator.read(&responder.write().unwrap()).unwrap();
        (
            initiator.into_channel().unwrap().0,
            responder.into_channel().unwrap().0,
        )
    }

    async fn request(
        input: &mpsc::Sender<Input>,
        channel: &mut Channel,
        id: u64,
        operation: Operation,
    ) {
        let bytes = protocol::pack(&Request { id, operation }).unwrap();
        input
            .send(Input::Data(channel.encrypt(&bytes).unwrap()))
            .await
            .unwrap();
    }

    async fn response(
        output: &mut mpsc::Receiver<RelayFrame>,
        channel: &mut Channel,
        credits: &Semaphore,
    ) -> Response {
        let frame = timeout(Duration::from_secs(2), output.recv())
            .await
            .unwrap()
            .unwrap();
        let RelayFrame::Data { data, .. } = frame else {
            panic!("expected ciphertext");
        };
        credits.add_permits(1);
        protocol::unpack(
            &channel
                .decrypt(&protocol::decode(&data, protocol::MAX_CIPHERTEXT).unwrap())
                .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "End-to-end encrypted transport exercises FIFO, overflow, heartbeat and revoke outcomes together"
    )]
    async fn encrypted_dispatch_keeps_heartbeats_output_and_queued_inputs_live_during_target_call()
    {
        for (revoke, overflow) in [(false, false), (true, false), (false, true)] {
            let state = Arc::new(AppState::new_for_test(":memory:".into()));
            let device = Device {
                id: "device".into(),
                name: "device".into(),
                public_key: String::new(),
                permission: Permission::Operate,
                grants: vec![],
                generation: 1,
            };
            let mut saved = state.remoted.snapshot().unwrap();
            saved.enabled = true;
            saved.devices.push(device.clone());
            state.remoted.persist(saved).unwrap();
            let (mut client, host) = channels();
            let (input, incoming) = mpsc::channel(8);
            let (outgoing, mut output) = mpsc::channel(128);
            let cancel = CancellationToken::new();
            let credits = Arc::new(Semaphore::new(8));
            let link = Link {
                peer: "peer".into(),
                outgoing,
                credits: credits.clone(),
                cancel: cancel.clone(),
            };
            state.remoted.runtime.lock().unwrap().peers.insert(
                "peer".into(),
                super::super::store::Peer {
                    device: Some(device.id.clone()),
                    cancel: cancel.clone(),
                    rtt: None,
                },
            );
            let started = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let finished = Arc::new(tokio::sync::Notify::new());
            let received = Arc::new(Mutex::new(Vec::new()));
            let received_work = received.clone();
            let server_state = state.clone();
            let start_work = started.clone();
            let release_work = release.clone();
            let finished_work = finished.clone();
            let server = tokio::spawn(async move {
                serve_authenticated(
                    &server_state,
                    &link,
                    incoming,
                    host,
                    0,
                    device,
                    move |mut work| {
                        let started = start_work.clone();
                        let release = release_work.clone();
                        let finished = finished_work.clone();
                        let received = received_work.clone();
                        tokio::spawn(async move {
                            work.admission.submit(true).unwrap();
                            let Operation::TerminalInput { data, .. } = work.request.operation
                            else {
                                panic!("expected terminal input");
                            };
                            received
                                .lock()
                                .unwrap()
                                .push(protocol::decode(&data, 16384).unwrap());
                            if work.request.id == 1 {
                                started.notify_one();
                                work.events
                                    .send(Event::Error {
                                        code: "STREAM_PROGRESS".into(),
                                        message: "stream is live".into(),
                                        outcome_unknown: false,
                                    })
                                    .await
                                    .unwrap();
                                release.notified().await;
                            }
                            work.admission.finish(true, false);
                            if work.request.id == 1 {
                                finished.notify_one();
                            }
                            Completed {
                                id: work.request.id,
                                event: Event::Ok,
                                denied: false,
                                resources: work.resources,
                            }
                        })
                    },
                )
                .await
            });
            request(
                &input,
                &mut client,
                1,
                Operation::TerminalInput {
                    stream: "simulated".into(),
                    data: protocol::encode(b"first"),
                },
            )
            .await;
            timeout(Duration::from_secs(2), started.notified())
                .await
                .unwrap();
            let mut progress = false;
            let nonce = loop {
                match response(&mut output, &mut client, &credits).await.event {
                    Event::Ping { nonce } => break nonce,
                    Event::Error { code, .. } if code == "STREAM_PROGRESS" => progress = true,
                    _ => panic!("unexpected response before host heartbeat"),
                }
            };
            request(&input, &mut client, 2, Operation::Pong { nonce }).await;
            request(
                &input,
                &mut client,
                3,
                Operation::Ping {
                    nonce: "client-heartbeat".into(),
                },
            )
            .await;
            request(
                &input,
                &mut client,
                4,
                Operation::TerminalInput {
                    stream: "simulated".into(),
                    data: protocol::encode(b"second"),
                },
            )
            .await;
            request(
                &input,
                &mut client,
                5,
                Operation::Ping {
                    nonce: "queue-observed".into(),
                },
            )
            .await;
            let mut pong = false;
            let mut queued = false;
            while !(pong && queued && progress) {
                let message = response(&mut output, &mut client, &credits).await;
                match message.event {
                    Event::Pong { nonce } => {
                        if message.id == 3 {
                            assert_eq!(nonce, "client-heartbeat");
                            pong = true;
                        } else {
                            assert_eq!(message.id, 5);
                            assert_eq!(nonce, "queue-observed");
                            queued = true;
                        }
                    }
                    Event::Error { code, .. } if code == "STREAM_PROGRESS" => progress = true,
                    _ => panic!("unexpected response during pending target call"),
                }
            }
            assert!(
                state.remoted.runtime.lock().unwrap().peers["peer"]
                    .rtt
                    .is_some()
            );
            assert_eq!(*received.lock().unwrap(), vec![b"first".to_vec()]);
            if overflow {
                for id in 6..=12 {
                    request(
                        &input,
                        &mut client,
                        id,
                        Operation::TerminalInput {
                            stream: "simulated".into(),
                            data: protocol::encode(format!("queued-{id}").as_bytes()),
                        },
                    )
                    .await;
                }
                request(
                    &input,
                    &mut client,
                    13,
                    Operation::TerminalInput {
                        stream: "simulated".into(),
                        data: protocol::encode(b"rejected"),
                    },
                )
                .await;
                request(
                    &input,
                    &mut client,
                    14,
                    Operation::Ping {
                        nonce: "overflow-observed".into(),
                    },
                )
                .await;
                let rejected = response(&mut output, &mut client, &credits).await;
                assert_eq!(rejected.id, 13);
                assert!(
                    matches!(rejected.event, Event::Error { code, outcome_unknown: false, .. } if code == "QUEUE_FULL")
                );
                let pong = response(&mut output, &mut client, &credits).await;
                assert_eq!(pong.id, 14);
                assert!(
                    matches!(pong.event, Event::Pong { nonce } if nonce == "overflow-observed")
                );
                assert_eq!(*received.lock().unwrap(), vec![b"first".to_vec()]);
            }
            if revoke {
                let mut saved = state.remoted.snapshot().unwrap();
                saved.devices.clear();
                state.remoted.persist(saved).unwrap();
                assert_eq!(
                    state.remoted.operation_status().unwrap().in_flight_unknown,
                    1
                );
                cancel.cancel();
                timeout(Duration::from_secs(2), server)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(state.remoted.operation_status().unwrap().active.len(), 1);
                release.notify_one();
                timeout(Duration::from_secs(2), finished.notified())
                    .await
                    .unwrap();
                assert_eq!(*received.lock().unwrap(), vec![b"first".to_vec()]);
            } else {
                release.notify_one();
                let mut expected_ids = vec![1, 4];
                let mut expected_bytes = vec![b"first".to_vec(), b"second".to_vec()];
                if overflow {
                    expected_ids.extend(6..=12);
                    expected_bytes.extend((6..=12).map(|id| format!("queued-{id}").into_bytes()));
                }
                for expected in expected_ids {
                    let completed = response(&mut output, &mut client, &credits).await;
                    assert_eq!(completed.id, expected);
                    assert!(matches!(completed.event, Event::Ok));
                }
                assert_eq!(*received.lock().unwrap(), expected_bytes);
                cancel.cancel();
                timeout(Duration::from_secs(2), server)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
            }
            let status = state.remoted.operation_status().unwrap();
            assert!(status.active.is_empty());
            assert_eq!(
                status.recent[0].outcome.result.as_deref(),
                Some("confirmed")
            );
            assert!(!status.recent[0].outcome.outcome_unknown);
        }
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "Encrypted regression verifies cancellation ordering across an active read and queued business request"
    )]
    async fn transfer_cancel_overtakes_fifo_and_interrupts_pending_read() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let device = Device {
            id: "device".into(),
            name: "device".into(),
            public_key: String::new(),
            permission: Permission::Operate,
            grants: vec![],
            generation: 1,
        };
        let mut saved = state.remoted.snapshot().unwrap();
        saved.enabled = true;
        saved.devices.push(device.clone());
        state.remoted.persist(saved).unwrap();
        let (mut client, host) = channels();
        let (input, incoming) = mpsc::channel(8);
        let (outgoing, mut output) = mpsc::channel(128);
        let cancel = CancellationToken::new();
        let credits = Arc::new(Semaphore::new(8));
        let link = Link {
            peer: "peer".into(),
            outgoing,
            credits: credits.clone(),
            cancel: cancel.clone(),
        };
        let started = Arc::new(tokio::sync::Notify::new());
        let start_work = started.clone();
        let server = tokio::spawn(async move {
            serve_authenticated(&state, &link, incoming, host, 0, device, move |mut work| {
                let started = start_work.clone();
                tokio::spawn(async move {
                    let event = match work.request.operation {
                        Operation::TransferAcknowledge { .. } => {
                            work.admission.submit(false).unwrap();
                            started.notify_one();
                            let result = finish_interruptible(
                                &work.cancel,
                                &work.interrupt,
                                &work.admission,
                                std::future::pending::<Result<Event>>(),
                            )
                            .await;
                            assert!(result.is_err());
                            Event::Error {
                                code: "CANCELLED".into(),
                                message: "read cancelled".into(),
                                outcome_unknown: false,
                            }
                        }
                        Operation::Cancel { transfer } => Event::Cancelled { transfer },
                        _ => Event::Ok,
                    };
                    work.admission.finish(true, false);
                    Completed {
                        id: work.request.id,
                        event,
                        denied: false,
                        resources: work.resources,
                    }
                })
            })
            .await
        });
        request(
            &input,
            &mut client,
            1,
            Operation::TransferAcknowledge {
                transfer: "download".into(),
                offset: 0,
            },
        )
        .await;
        timeout(Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        for id in 2..10 {
            request(
                &input,
                &mut client,
                id,
                Operation::FileList {
                    grant: "grant".into(),
                    path: String::new(),
                },
            )
            .await;
        }
        request(
            &input,
            &mut client,
            10,
            Operation::Cancel {
                transfer: "download".into(),
            },
        )
        .await;
        let mut order = Vec::new();
        while order.len() < 10 {
            let reply = response(&mut output, &mut client, &credits).await;
            if reply.id != 0 {
                if reply.id == 10 {
                    assert!(matches!(reply.event, Event::Cancelled { .. }));
                }
                order.push(reply.id);
            }
        }
        assert_eq!(order, [1, 10, 2, 3, 4, 5, 6, 7, 8, 9]);
        cancel.cancel();
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn file_cancel_awaits_submitted_mutation_and_reports_interruption() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let device = Device {
            id: "device".into(),
            name: "device".into(),
            public_key: String::new(),
            permission: Permission::Operate,
            grants: vec![],
            generation: 1,
        };
        let mut saved = state.remoted.snapshot().unwrap();
        saved.enabled = true;
        saved.devices.push(device.clone());
        state.remoted.persist(saved).unwrap();
        let cancel = CancellationToken::new();
        let interrupt = cancel.child_token();
        let mut admission = state
            .remoted
            .admit(&device, "peer", 1, interrupt.clone())
            .unwrap();
        admission.submit(true).unwrap();
        interrupt.cancel();
        assert_eq!(
            state.remoted.operation_status().unwrap().in_flight_unknown,
            1
        );
        let result = {
            let release = tokio::sync::Notify::new();
            let target = finish_interruptible(&cancel, &interrupt, &admission, async {
                release.notified().await;
                anyhow::bail!("commit acknowledgement lost")
            });
            tokio::pin!(target);
            assert!(
                timeout(Duration::from_millis(10), &mut target)
                    .await
                    .is_err()
            );
            release.notify_one();
            target.await
        };
        assert!(result.is_err());
        admission.finish(false, true);
        assert!(
            state.remoted.operation_status().unwrap().recent[0]
                .outcome
                .outcome_unknown
        );
        assert!(!cancel.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_unsubmitted_target_is_not_polled_but_dispatched_target_is_awaited() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let device = Device {
            id: "device".into(),
            name: "device".into(),
            public_key: String::new(),
            permission: Permission::Operate,
            grants: vec![],
            generation: 1,
        };
        let mut saved = state.remoted.snapshot().unwrap();
        saved.enabled = true;
        saved.devices.push(device.clone());
        state.remoted.persist(saved).unwrap();
        for submitted in [false, true] {
            let cancel = CancellationToken::new();
            let mut admission = state
                .remoted
                .admit(&device, "peer", 1, cancel.clone())
                .unwrap();
            if submitted {
                admission.submit(true).unwrap();
            }
            cancel.cancel();
            let polled = std::sync::atomic::AtomicBool::new(false);
            let target = async {
                polled.store(true, std::sync::atomic::Ordering::Relaxed);
                tokio::task::yield_now().await;
                Ok(Event::Ok)
            };
            let result = finish_target(&cancel, &admission, target).await;
            assert_eq!(result.is_ok(), submitted);
            assert_eq!(polled.load(std::sync::atomic::Ordering::Relaxed), submitted);
            admission.finish(result.is_ok(), false);
        }
    }

    #[test]
    fn raw_herdr_pane_writes_require_explicit_control() {
        for method in [
            "pane.send_text",
            "pane.send_keys",
            "pane.send_input",
            "pane.resize",
        ] {
            assert!(
                require_herdr_control(method, false)
                    .unwrap_err()
                    .is::<AuthorizationError>()
            );
            assert!(require_herdr_control(method, true).is_ok());
        }
        assert!(require_herdr_control("pane.read", false).is_ok());
    }

    #[test]
    fn transfer_limit_is_shared_across_connections_and_released() {
        let device = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let first = transfer_permits(&device).unwrap();
        let second = transfer_permits(&device).unwrap();
        assert!(transfer_permits(&device).is_err());
        let separate = transfer_permits(&other).unwrap();
        drop(first);
        assert!(transfer_permits(&device).is_ok());
        drop((second, separate));
        let host: Vec<_> = (0..8)
            .map(|_| transfer_permits(&uuid::Uuid::new_v4().to_string()).unwrap())
            .collect();
        assert!(transfer_permits(&uuid::Uuid::new_v4().to_string()).is_err());
        drop(host);
        assert!(transfer_permits(&device).is_ok());
    }

    #[test]
    fn dropping_pending_guard_removes_request_and_closes_decision() {
        let database = Arc::new(
            crate::database::AppDatabase::open(std::path::PathBuf::from(":memory:")).unwrap(),
        );
        let manager = super::super::store::Remoted::load(database).unwrap();
        let (decision, mut response) = oneshot::channel();
        manager.runtime.lock().unwrap().pending.insert(
            "pending".into(),
            Pending {
                id: "pending".into(),
                name: "device".into(),
                key: String::new(),
                created: Instant::now(),
                decision,
            },
        );
        drop(PendingCleanup {
            manager: &manager,
            id: "pending".into(),
        });
        assert!(manager.runtime.lock().unwrap().pending.is_empty());
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }
}
