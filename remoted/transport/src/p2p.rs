//! A single, disposable ICE/DTLS/SCTP attempt. No application authentication lives here.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use tokio::{sync::mpsc, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;
use webrtc::{
    api::{APIBuilder, setting_engine::SettingEngine},
    data_channel::{RTCDataChannel, data_channel_init::RTCDataChannelInit},
    ice::network_type::NetworkType,
    ice_transport::{ice_candidate::RTCIceCandidateInit, ice_server::RTCIceServer},
    peer_connection::{
        RTCPeerConnection, configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
    },
};

use crate::{MAX_WIRE_PAYLOAD, Role, WINDOW, signal::Signal};

const LABEL: &str = "neoko-stream";
const PROTOCOL: &str = "neoko-stream-v1";
const MAX_CANDIDATES: usize = 64;
const SEND_TIMEOUT: Duration = Duration::from_millis(500);

pub(crate) enum Command {
    Signal(Signal),
    Data(Vec<u8>),
}
pub(crate) enum Event {
    Signal(Signal),
    Opened,
    Data(Vec<u8>),
    Failed,
}

#[derive(Clone)]
pub(crate) struct Options {
    pub(crate) stun_servers: Vec<String>,
    #[cfg(test)]
    pub(crate) network: Option<Arc<webrtc::util::vnet::net::Net>>,
    #[cfg(test)]
    pub(crate) srflx_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stun_servers: vec!["stun:stun.l.google.com:19302".into()],
            #[cfg(test)]
            network: None,
            #[cfg(test)]
            srflx_only: false,
        }
    }
}

pub(crate) struct Engine {
    pub(crate) commands: mpsc::Sender<Command>,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    closed: Arc<AtomicBool>,
}

impl Engine {
    pub(crate) fn start(role: Role, options: Options, events: mpsc::Sender<Event>) -> Self {
        // All bounded early signals may be replayed before RTC initialization is
        // polled. Reserve their complete budget plus the application window.
        let (commands, receive) = mpsc::channel(crate::signal::MAX_SIGNALS + WINDOW);
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let finished = closed.clone();
        let task = tokio::spawn(async move {
            // Always close the RTC object, including initialization/SDP failures.
            #[cfg(test)]
            let srflx_only = options.srflx_only;
            #[cfg(not(test))]
            let srflx_only = false;
            let peer = make_peer(options).await;
            if let Ok(peer) = peer {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => {},
                    _ = run(&peer, role, receive, &events, &stop, srflx_only) => {},
                }
                let _ = timeout(Duration::from_secs(2), peer.close()).await;
            }
            finished.store(true, Ordering::Release);
            let _ = events.try_send(Event::Failed);
        });
        Self {
            commands,
            cancel,
            task: Some(task),
            closed,
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn cancel(&self) {
        self.cancel.cancel();
    }

    pub(crate) async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            // The RTC close deadline is two seconds. Initialization also has a bound.
            let _ = task.await;
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn make_peer(options: Options) -> Result<Arc<RTCPeerConnection>> {
    let mut settings = SettingEngine::default();
    settings.set_network_types(vec![NetworkType::Udp4, NetworkType::Udp6]);
    settings.set_ice_timeouts(
        Some(Duration::from_secs(3)),
        Some(Duration::from_secs(8)),
        Some(Duration::from_secs(1)),
    );
    #[cfg(test)]
    settings.set_vnet(options.network);
    let api = APIBuilder::new().with_setting_engine(settings).build();
    let config = RTCConfiguration {
        ice_servers: if options.stun_servers.is_empty() {
            vec![]
        } else {
            vec![RTCIceServer {
                urls: options.stun_servers,
                ..Default::default()
            }]
        },
        ..Default::default()
    };
    Ok(Arc::new(
        timeout(Duration::from_secs(3), api.new_peer_connection(config)).await??,
    ))
}

fn attach(
    channel: &Arc<RTCDataChannel>,
    events: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> Result<()> {
    ensure!(
        channel.label() == LABEL && channel.protocol() == PROTOCOL,
        "unexpected DataChannel"
    );
    ensure!(
        channel.ordered()
            && channel.max_retransmits().is_none()
            && channel.max_packet_lifetime().is_none(),
        "DataChannel must be reliable and ordered"
    );
    let opened = events.clone();
    let open_stop = cancel.clone();
    channel.on_open(Box::new(move || {
        Box::pin(async move {
            publish(&opened, Event::Opened, &open_stop);
        })
    }));
    let closed = events.clone();
    let close_stop = cancel.clone();
    channel.on_close(Box::new(move || {
        let closed = closed.clone();
        let close_stop = close_stop.clone();
        Box::pin(async move {
            publish(&closed, Event::Failed, &close_stop);
        })
    }));
    let failed = events.clone();
    let error_stop = cancel.clone();
    channel.on_error(Box::new(move |_| {
        let failed = failed.clone();
        let error_stop = error_stop.clone();
        Box::pin(async move {
            publish(&failed, Event::Failed, &error_stop);
        })
    }));
    let receive = events.clone();
    let receive_stop = cancel.clone();
    channel.on_message(Box::new(move |message| {
        let receive = receive.clone();
        let receive_stop = receive_stop.clone();
        Box::pin(async move {
            let event = if message.is_string || message.data.len() > MAX_WIRE_PAYLOAD {
                Event::Failed
            } else {
                Event::Data(message.data.to_vec())
            };
            publish(&receive, event, &receive_stop);
        })
    }));
    Ok(())
}

// Callbacks never park SCTP/ICE workers behind the application actor. A full
// bounded queue cancels this attempt; the overlay observes engine completion.
fn publish(events: &mpsc::Sender<Event>, event: Event, cancel: &CancellationToken) {
    if events.try_send(event).is_err() {
        cancel.cancel();
    }
}

async fn run(
    peer: &Arc<RTCPeerConnection>,
    role: Role,
    mut commands: mpsc::Receiver<Command>,
    events: &mpsc::Sender<Event>,
    cancel: &CancellationToken,
    srflx_only: bool,
) -> Result<()> {
    let candidates = events.clone();
    let candidate_stop = cancel.clone();
    peer.on_ice_candidate(Box::new(move |candidate| {
        let candidates = candidates.clone();
        let candidate_stop = candidate_stop.clone();
        Box::pin(async move {
            if let Some(candidate) = candidate {
                let event = match candidate.to_json() {
                    Ok(value) => {
                        if srflx_only && !value.candidate.contains(" typ srflx") {
                            return;
                        }
                        Event::Signal(Signal::Candidate(value))
                    }
                    Err(_) => Event::Failed,
                };
                publish(&candidates, event, &candidate_stop);
            }
        })
    }));
    let state_events = events.clone();
    let state_stop = cancel.clone();
    peer.on_peer_connection_state_change(Box::new(move |state| {
        let state_events = state_events.clone();
        let state_stop = state_stop.clone();
        Box::pin(async move {
            if matches!(
                state,
                RTCPeerConnectionState::Disconnected
                    | RTCPeerConnectionState::Failed
                    | RTCPeerConnectionState::Closed
            ) {
                publish(&state_events, Event::Failed, &state_stop);
            }
        })
    }));
    let (channel_tx, mut channel_rx) = mpsc::channel(1);
    let remote_events = events.clone();
    let remote_stop = cancel.clone();
    peer.on_data_channel(Box::new(move |channel| {
        let channel_tx = channel_tx.clone();
        let remote_events = remote_events.clone();
        let remote_stop = remote_stop.clone();
        Box::pin(async move {
            if role != Role::Responder
                || attach(&channel, &remote_events, &remote_stop).is_err()
                || channel_tx.try_send(channel).is_err()
            {
                publish(&remote_events, Event::Failed, &remote_stop);
            }
        })
    }));
    let mut channel = None;
    if role == Role::Initiator {
        let dc = peer
            .create_data_channel(
                LABEL,
                Some(RTCDataChannelInit {
                    ordered: Some(true),
                    max_retransmits: None,
                    max_packet_life_time: None,
                    protocol: Some(PROTOCOL.into()),
                    ..Default::default()
                }),
            )
            .await?;
        attach(&dc, events, cancel)?;
        channel = Some(dc);
        let offer = peer.create_offer(None).await?;
        let sdp = offer.sdp.clone();
        peer.set_local_description(offer).await?;
        events.send(Event::Signal(Signal::Offer(sdp))).await?;
    }
    let mut has_remote = false;
    let mut remote_candidates: Vec<RTCIceCandidateInit> = Vec::new();
    let mut candidate_count = 0;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            incoming = channel_rx.recv(), if channel.is_none() => {
                channel = Some(incoming.context("DataChannel callback closed")?);
                channel_rx.close();
            }
            command = commands.recv() => match command.context("direct command channel closed")? {
                Command::Data(data) => {
                    ensure!(data.len() <= MAX_WIRE_PAYLOAD, "oversized DataChannel frame");
                    let dc = channel.as_ref().context("DataChannel not open")?;
                    ensure!(dc.buffered_amount().await + data.len() <= WINDOW * MAX_WIRE_PAYLOAD, "DataChannel buffer limit");
                    timeout(SEND_TIMEOUT, dc.send(&Bytes::from(data))).await??;
                }
                Command::Signal(signal) => match signal {
                    Signal::Candidate(candidate) => {
                        candidate_count += 1;
                        ensure!(candidate_count <= MAX_CANDIDATES, "ICE candidate count limit");
                        if has_remote { peer.add_ice_candidate(candidate).await?; }
                        else { remote_candidates.push(candidate); }
                    }
                    Signal::Offer(sdp) if role == Role::Responder && !has_remote => {
                        peer.set_remote_description(RTCSessionDescription::offer(sdp)?).await?;
                        has_remote = true;
                        for candidate in remote_candidates.drain(..) { peer.add_ice_candidate(candidate).await?; }
                        let answer = peer.create_answer(None).await?;
                        let sdp = answer.sdp.clone();
                        peer.set_local_description(answer).await?;
                        events.send(Event::Signal(Signal::Answer(sdp))).await?;
                    }
                    Signal::Answer(sdp) if role == Role::Initiator && !has_remote => {
                        peer.set_remote_description(RTCSessionDescription::answer(sdp)?).await?;
                        has_remote = true;
                        for candidate in remote_candidates.drain(..) { peer.add_ice_candidate(candidate).await?; }
                    }
                    _ => bail!("unexpected direct signal"),
                }
            }
        }
    }
}
