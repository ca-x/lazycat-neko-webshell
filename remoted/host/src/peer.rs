use crate::{
    relay::{Input, Link},
    session::{self, Principal},
    state::{Core, DEADLINE, Device, Live, Pending, check_name},
};
use anyhow::{Context, Result, bail, ensure};
use neko_remoted_protocol::{
    self as protocol, RelayAddress,
    crypto::{Channel, Handshake, Mode},
    messages::{Event, Operation, Request, Response},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub(crate) async fn run(
    core: Arc<Core>,
    relay: RelayAddress,
    link: Link,
    incoming: mpsc::Receiver<Input>,
) {
    let subscriber = uuid::Uuid::new_v4().to_string();
    let _result = tokio::select! {
        biased;
        () = link.cancel.cancelled() => Ok(()),
        result = run_inner(&core, &relay, &link, &subscriber, incoming) => result,
    };
    link.cancel.cancel();
    core.disconnected(&subscriber).await;
}
async fn next(link: &Link, incoming: &mut mpsc::Receiver<Input>) -> Result<Input> {
    tokio::select! {
        biased;
        () = link.cancel.cancelled() => bail!("peer cancelled"),
        result = tokio::time::timeout(DEADLINE, incoming.recv()) => result.context("peer timed out")?.context("peer disconnected").map(Input::consumed),
    }
}
async fn data(link: &Link, incoming: &mut mpsc::Receiver<Input>) -> Result<Vec<u8>> {
    match next(link, incoming).await? {
        Input::Data(data) => Ok(data),
        _ => bail!("unexpected Hello"),
    }
}
async fn response(link: &Link, channel: &mut Channel, id: u64, event: Event) -> Result<()> {
    link.send(&channel.encrypt(&protocol::pack(&Response { id, event })?)?)
        .await
}
async fn request(
    link: &Link,
    incoming: &mut mpsc::Receiver<Input>,
    channel: &mut Channel,
    last: &mut u64,
) -> Result<Request> {
    let request: Request = protocol::unpack(&channel.decrypt(&data(link, incoming).await?)?)?;
    ensure!(request.id > *last, "request replay");
    *last = request.id;
    Ok(request)
}
async fn run_inner(
    core: &Arc<Core>,
    relay: &RelayAddress,
    link: &Link,
    subscriber: &str,
    mut incoming: mpsc::Receiver<Input>,
) -> Result<()> {
    let Input::Hello(mode) = next(link, &mut incoming).await? else {
        bail!("Hello required")
    };
    let psk = if mode == Mode::Pair {
        let state = core.state.lock().await;
        ensure!(state.saved.enabled, "hosting disabled");
        let invitation = state
            .invitation
            .as_ref()
            .filter(|i| i.expires > Instant::now())
            .context("fresh invitation required")?;
        Some(*invitation.code.psk())
    } else {
        None
    };
    let mut handshake = Handshake::new(
        &core.identity,
        relay,
        &core.route,
        mode,
        false,
        psk.as_ref(),
        None,
    )?;
    handshake.read(&data(link, &mut incoming).await?)?;
    link.send(&handshake.write()?).await?;
    if !handshake.finished() {
        handshake.read(&data(link, &mut incoming).await?)?;
    }
    let (mut channel, key) = handshake.into_channel()?;
    let key = protocol::encode(&key);
    let mut last = 0;
    let device = if let Some(psk) = psk {
        pair_identity(
            core,
            link,
            subscriber,
            &mut incoming,
            &mut channel,
            &mut last,
            PairIdentity { key, psk },
        )
        .await?
    } else {
        let state = core.state.lock().await;
        let device = state
            .saved
            .devices
            .iter()
            .find(|d| d.key == key && state.saved.enabled)
            .cloned();
        drop(state);
        let Some(device) = device else {
            response(
                link,
                &mut channel,
                0,
                Event::Error {
                    code: "revoked".into(),
                    message: "Device is not authorized".into(),
                    outcome_unknown: false,
                },
            )
            .await?;
            bail!("device not authorized");
        };
        device
    };
    let principal = Principal::Device {
        id: device.id.clone(),
        generation: device.generation,
    };
    {
        let mut state = core.state.lock().await;
        session::authorize(&state, &principal)?;
        ensure!(!link.cancel.is_cancelled(), "peer cancelled");
        state.live.insert(
            subscriber.into(),
            Live {
                device: device.id,
                cancel: link.cancel.clone(),
                rtt: None,
            },
        );
        core.publish(&state)?;
    }
    let (requests, input) = mpsc::channel(8);
    let (output, mut responses) = mpsc::channel(32);
    tokio::spawn(session::serve(
        core.clone(),
        principal,
        subscriber.into(),
        input,
        output,
        link.cancel.clone(),
    ));
    let mut ticks = tokio::time::interval(Duration::from_secs(10));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    let mut ping: Option<(String, tokio::time::Instant)> = None;
    let mut last_rtt = None;
    let mut ready = false;
    // Only this loop touches the Noise state. No replay or transport retries occur here.
    loop {
        tokio::select! {
            () = link.cancel.cancelled() => return Ok(()),
            message = incoming.recv() => {
                let Input::Data(data) = message.context("peer disconnected")?.consumed() else { bail!("unexpected Hello") };
                let request: Request = protocol::unpack(&channel.decrypt(&data)?)?;
                ensure!(request.id > last, "request replay"); last = request.id;
                match request.operation {
                    Operation::Pong { nonce } => {
                        let (expected, sent) = ping.take().context("unexpected device pong")?;
                        ensure!(nonce == expected, "device heartbeat mismatch");
                        last_rtt = Some((u64::try_from(sent.elapsed().as_millis()).unwrap_or(u64::MAX), Instant::now()));
                        if let Ok(mut state) = core.state.try_lock() {
                            if let Some(live) = state.live.get_mut(subscriber) { live.rtt = last_rtt; }
                            core.publish(&state)?;
                        }
                        response(link, &mut channel, request.id, Event::Ok).await?;
                    }
                    Operation::EnableDirect { key } => {
                        ensure!(ready, "authentication is not ready");
                        // Device authorization is checked independently of the transport path.
                        link.enable_direct(protocol::decode32(&key)?)?;
                        response(link, &mut channel, request.id, Event::DirectReady {}).await?;
                    }
                    Operation::Ping { nonce } => {
                        ensure!(nonce.len() <= 128, "heartbeat nonce exceeds limit");
                        response(link, &mut channel, request.id, Event::Pong { nonce }).await?;
                    }
                    operation => requests.try_send(Request { id: request.id, operation }).context("business request queue full")?,
                }
            }
            message = responses.recv() => {
                let message = message.context("business session closed")?;
                let becoming_ready = matches!(message.event, Event::Ready { .. });
                if becoming_ready { ready = true; }
                link.send(&channel.encrypt(&protocol::pack(&message)?)?).await?;
                if becoming_ready && let Some(name) = reported_host_name() {
                    response(link, &mut channel, 0, Event::HostInfo { name }).await?;
                }
            }
            _ = ticks.tick(), if ready => {
                ensure!(!ping.as_ref().is_some_and(|(_, sent)| sent.elapsed() >= Duration::from_secs(30)), "device heartbeat timed out");
                if ping.is_none() {
                    let nonce = uuid::Uuid::new_v4().to_string();
                    response(link, &mut channel, 0, Event::Ping { nonce: nonce.clone() }).await?;
                    ping = Some((nonce, tokio::time::Instant::now()));
                }
                if let Ok(mut state) = core.state.try_lock() {
                    if let Some(live) = state.live.get_mut(subscriber) { live.rtt = last_rtt; }
                    core.publish(&state)?;
                }
            }
        }
    }
}
struct PairIdentity {
    key: String,
    psk: [u8; 32],
}
async fn pair_identity(
    core: &Core,
    link: &Link,
    subscriber: &str,
    incoming: &mut mpsc::Receiver<Input>,
    channel: &mut Channel,
    last: &mut u64,
    identity: PairIdentity,
) -> Result<Device> {
    let identify = request(link, incoming, channel, last).await?;
    let Operation::Identify { name } = identify.operation else {
        bail!("Identify required")
    };
    check_name(&name)?;
    let (decision, selected) = oneshot::channel();
    {
        let mut state = core.state.lock().await;
        ensure!(
            state.saved.enabled && !link.cancel.is_cancelled(),
            "hosting disabled"
        );
        let invitation = state
            .invitation
            .as_ref()
            .filter(|i| i.expires > Instant::now())
            .context("invitation expired")?;
        let differs = invitation
            .code
            .psk()
            .iter()
            .zip(identity.psk)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b));
        ensure!(
            differs == 0 && state.pending.len() < 8,
            "invitation changed or pending limit reached"
        );
        ensure!(
            !state.saved.devices.iter().any(|d| d.key == identity.key),
            "device already paired"
        );
        state.invitation = None;
        state.pending.insert(
            subscriber.into(),
            Pending {
                name,
                key: identity.key.clone(),
                decision,
                expires: Instant::now() + Duration::from_secs(120),
                cancel: link.cancel.clone(),
            },
        );
        state.revision += 1;
        core.publish(&state)?;
    }
    response(
        link,
        channel,
        identify.id,
        Event::Pending {
            fingerprint: protocol::fingerprint(&protocol::decode32(&identity.key)?),
        },
    )
    .await?;
    let device = tokio::select! {
        biased;
        () = link.cancel.cancelled() => bail!("pairing cancelled"),
        result = tokio::time::timeout(Duration::from_secs(120), selected) => result.context("approval expired")?.context("approval cancelled")?,
    };
    ensure!(device.key == identity.key, "approval identity mismatch");
    response(
        link,
        channel,
        0,
        Event::Approved {
            host_key: protocol::encode(core.identity.public()),
            device_id: device.id.clone(),
            permission: device.permission,
        },
    )
    .await?;
    let ack = request(link, incoming, channel, last).await?;
    ensure!(
        matches!(ack.operation, Operation::AcknowledgePairing {}),
        "pairing persistence acknowledgement required"
    );
    Ok(device)
}

fn reported_host_name() -> Option<String> {
    let name = hostname::get().ok()?.to_string_lossy().trim().to_owned();
    (!name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)).then_some(name)
}
