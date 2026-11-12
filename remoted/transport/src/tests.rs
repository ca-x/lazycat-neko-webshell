use super::*;
use neko_remoted_protocol::{
    RelayAddress,
    crypto::{Channel, Handshake, Identity, Mode},
};
use std::sync::Mutex;

struct Endpoint {
    send: Sender,
    recv: Receiver,
    guard: Option<Guard>,
}
struct Link {
    a: Endpoint,
    b: Endpoint,
    drop_acks: Arc<AtomicBool>,
    a_relay: Arc<Mutex<Vec<Vec<u8>>>>,
    pumps: Vec<JoinHandle<()>>,
}
impl Drop for Link {
    fn drop(&mut self) {
        for pump in &self.pumps {
            pump.abort();
        }
    }
}
impl Link {
    fn new() -> Self {
        Self::with_options(
            p2p::Options {
                stun_servers: vec![],
                network: None,
                srflx_only: false,
            },
            p2p::Options {
                stun_servers: vec![],
                network: None,
                srflx_only: false,
            },
        )
    }
    fn with_options(a_options: p2p::Options, b_options: p2p::Options) -> Self {
        let (a_out, a_writes) = mpsc::channel(16);
        let (a_in, a_reads) = mpsc::channel(16);
        let (b_out, b_writes) = mpsc::channel(16);
        let (b_in, b_reads) = mpsc::channel(16);
        let (send, recv, guard) = start_with(
            Carrier {
                outgoing: a_out,
                incoming: a_reads,
            },
            a_options,
        );
        let a = Endpoint {
            send,
            recv,
            guard: Some(guard),
        };
        let (send, recv, guard) = start_with(
            Carrier {
                outgoing: b_out,
                incoming: b_reads,
            },
            b_options,
        );
        let b = Endpoint {
            send,
            recv,
            guard: Some(guard),
        };
        let drop_acks = Arc::new(AtomicBool::new(false));
        let a_relay = Arc::new(Mutex::new(Vec::new()));
        let b_relay = Arc::new(Mutex::new(Vec::new()));
        let pumps = vec![
            pump(a_writes, b_in, drop_acks.clone(), a_relay.clone()),
            pump(b_writes, a_in, drop_acks.clone(), b_relay),
        ];
        Self {
            a,
            b,
            drop_acks,
            a_relay,
            pumps,
        }
    }
    async fn close(&mut self) {
        let (a, b) = tokio::join!(
            self.a.guard.take().unwrap().shutdown(),
            self.b.guard.take().unwrap().shutdown()
        );
        a.unwrap();
        b.unwrap();
    }
    async fn direct(&self) {
        self.b
            .send
            .enable_direct(Role::Responder, [17; 32])
            .unwrap();
        self.a
            .send
            .enable_direct(Role::Initiator, [17; 32])
            .unwrap();
        tokio::try_join!(
            wait_kind(&self.a.send, ConnectionKind::Direct),
            wait_kind(&self.b.send, ConnectionKind::Direct)
        )
        .unwrap();
    }
    async fn noise(&mut self) -> (Channel, Channel) {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let relay = RelayAddress::parse("https://relay.example").unwrap();
        let route = neko_remoted_protocol::route(&[9; 32]);
        let mut ah = Handshake::new(
            &a,
            &relay,
            &route,
            Mode::Connect,
            true,
            None,
            Some(b.public()),
        )
        .unwrap();
        let mut bh = Handshake::new(&b, &relay, &route, Mode::Connect, false, None, None).unwrap();
        self.a.send.send(ah.write().unwrap()).await.unwrap();
        bh.read(&self.b.recv.recv().await.unwrap()).unwrap();
        self.b.send.send(bh.write().unwrap()).await.unwrap();
        ah.read(&self.a.recv.recv().await.unwrap()).unwrap();
        (ah.into_channel().unwrap().0, bh.into_channel().unwrap().0)
    }
}
fn pump(
    mut outgoing: mpsc::Receiver<Write>,
    incoming: mpsc::Sender<Result<Vec<u8>>>,
    drop_acks: Arc<AtomicBool>,
    trace: Arc<Mutex<Vec<Vec<u8>>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(write) = outgoing.recv().await {
            let skip = matches!(wire::parse(&write.data), Ok(wire::Frame::Ack(_)))
                && drop_acks.load(Ordering::Acquire);
            trace.lock().unwrap().push(write.data.clone());
            if !skip && incoming.send(Ok(write.data)).await.is_err() {
                let _ = write.completed.send(Err(anyhow::anyhow!("peer gone")));
                break;
            }
            let _ = write.completed.send(Ok(()));
        }
    })
}
async fn wait_kind(sender: &Sender, expected: ConnectionKind) -> Result<()> {
    let mut kind = sender.subscribe_kind();
    timeout(
        Duration::from_secs(18),
        kind.wait_for(|kind| *kind == expected),
    )
    .await??;
    Ok(())
}

#[tokio::test]
async fn relay_envelopes_cover_noise_handshake_and_consumer_window() {
    let mut link = Link::new();
    let (mut a, mut b) = link.noise().await;
    for seq in 0..WINDOW {
        link.a
            .send
            .send(a.encrypt(&[seq as u8]).unwrap())
            .await
            .unwrap();
    }
    let extra = a.encrypt(b"after window").unwrap();
    assert!(
        timeout(Duration::from_millis(50), link.a.send.send(extra.clone()))
            .await
            .is_err()
    );
    for seq in 0..WINDOW {
        assert_eq!(
            b.decrypt(&link.b.recv.recv().await.unwrap()).unwrap(),
            [seq as u8]
        );
    }
    link.a.send.send(extra).await.unwrap();
    assert_eq!(
        b.decrypt(&link.b.recv.recv().await.unwrap()).unwrap(),
        b"after window"
    );
    {
        let frames = link.a_relay.lock().unwrap();
        assert!(frames.iter().all(|frame| wire::parse(frame).is_ok()));
        assert!(matches!(
            wire::parse(&frames[0]),
            Ok(wire::Frame::Data(0, _))
        ));
    }
    link.close().await;
}

#[tokio::test]
async fn cancelled_receive_keeps_next_record_and_closed_carrier_fails_send() {
    let mut link = Link::new();
    assert!(
        timeout(Duration::from_millis(20), link.b.recv.recv())
            .await
            .is_err()
    );
    link.a.send.send(b"kept".to_vec()).await.unwrap();
    assert_eq!(link.b.recv.recv().await.unwrap(), b"kept");
    assert!(link.a.send.send(vec![0; MAX_DATA + 1]).await.is_err());
    link.close().await;
    assert!(link.a.send.send(vec![1]).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_datachannel_is_ordered_binary_and_preserves_noise_nonces() {
    let mut link = Link::new();
    let (mut a, mut b) = link.noise().await;
    link.direct().await;
    let relay_data_before = link
        .a_relay
        .lock()
        .unwrap()
        .iter()
        .filter(|frame| matches!(wire::parse(frame), Ok(wire::Frame::Data(_, _))))
        .count();
    for seq in 0..32u8 {
        let size = if seq == 31 {
            neko_remoted_protocol::MAX_PLAINTEXT
        } else {
            128
        };
        let plain = vec![seq; size];
        link.a.send.send(a.encrypt(&plain).unwrap()).await.unwrap();
        let ciphertext = timeout(Duration::from_secs(3), link.b.recv.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(b.decrypt(&ciphertext).unwrap(), plain);
        link.b.send.send(b.encrypt(&plain).unwrap()).await.unwrap();
        let ciphertext = timeout(Duration::from_secs(3), link.a.recv.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.decrypt(&ciphertext).unwrap(), plain);
    }
    assert_eq!(link.a.send.kind(), ConnectionKind::Direct);
    let relay_data_after = link
        .a_relay
        .lock()
        .unwrap()
        .iter()
        .filter(|frame| matches!(wire::parse(frame), Ok(wire::Frame::Data(_, _))))
        .count();
    assert_eq!(
        relay_data_before, relay_data_after,
        "business bytes must use actual DataChannel"
    );
    link.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ack_timeout_replays_original_ciphertext_once_and_permanently_uses_relay() {
    let mut link = Link::new();
    let (mut a, mut b) = link.noise().await;
    link.direct().await;
    link.drop_acks.store(true, Ordering::Release);
    let first = a.encrypt(b"one command, once").unwrap();
    link.a.send.send(first.clone()).await.unwrap();
    assert_eq!(
        b.decrypt(
            &timeout(Duration::from_secs(2), link.b.recv.recv())
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        b"one command, once"
    );
    wait_kind(&link.a.send, ConnectionKind::Relay)
        .await
        .unwrap();
    link.drop_acks.store(false, Ordering::Release);
    wait_kind(&link.b.send, ConnectionKind::Relay)
        .await
        .unwrap();
    let second = a.encrypt(b"next Noise nonce").unwrap();
    link.a.send.send(second).await.unwrap();
    assert_eq!(
        b.decrypt(
            &timeout(Duration::from_secs(3), link.b.recv.recv())
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        b"next Noise nonce"
    );
    assert!(
        timeout(Duration::from_millis(100), link.b.recv.recv())
            .await
            .is_err(),
        "duplicate must never reach Noise"
    );
    assert!(
        link.a_relay
            .lock()
            .unwrap()
            .iter()
            .any(|frame| frame == &wire::data(1, &first)),
        "replay must preserve exact ciphertext"
    );
    assert!(
        link.a
            .send
            .enable_direct(Role::Initiator, [23; 32])
            .is_err()
    );
    link.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_signaling_key_falls_back_without_breaking_noise() {
    let mut link = Link::new();
    let (mut a, mut b) = link.noise().await;
    link.b
        .send
        .enable_direct(Role::Responder, [41; 32])
        .unwrap();
    link.a
        .send
        .enable_direct(Role::Initiator, [42; 32])
        .unwrap();
    // First observe actor processing, since initial state also is Relay.
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::try_join!(
        wait_kind(&link.a.send, ConnectionKind::Relay),
        wait_kind(&link.b.send, ConnectionKind::Relay)
    )
    .unwrap();
    link.a
        .send
        .send(a.encrypt(b"relay still authenticated").unwrap())
        .await
        .unwrap();
    assert_eq!(
        b.decrypt(&link.b.recv.recv().await.unwrap()).unwrap(),
        b"relay still authenticated"
    );
    link.close().await;
}

#[test]
fn malformed_and_oversized_envelopes_rejected() {
    for len in 0..13 {
        assert!(wire::parse(&vec![0; len]).is_err());
    }
    assert!(wire::parse(&vec![0; MAX_WIRE_PAYLOAD + 1]).is_err());
    let mut unknown = wire::ack(0);
    unknown[4] = 9;
    assert!(wire::parse(&unknown).is_err());
    assert!(wire::parse(&wire::data(0, &vec![0; MAX_DATA + 1])).is_err());
    let mut bad_ack = wire::ack(1);
    bad_ack.push(1);
    assert!(wire::parse(&bad_ack).is_err());
    assert!(wire::in_window(7, 0));
    assert!(!wire::in_window(8, 0));
    assert!(!wire::in_window(u64::MAX, 0));
    assert!(wire::in_window(u64::MAX, u64::MAX));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_private_nats_use_stun_without_public_peer_or_port_forwarding() {
    let network = test_nat::Network::new(false, false).await.unwrap();
    let options = |index: usize| p2p::Options {
        stun_servers: vec!["stun:1.2.3.4:3478".into()],
        network: Some(network.peers[index].clone()),
        srflx_only: true,
    };
    let mut link = Link::with_options(options(0), options(1));
    let (mut a, mut b) = link.noise().await;
    link.direct().await;
    for seq in 0..16u8 {
        let plain = vec![seq; 2048];
        link.a.send.send(a.encrypt(&plain).unwrap()).await.unwrap();
        assert_eq!(
            b.decrypt(
                &timeout(Duration::from_secs(3), link.b.recv.recv())
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            plain
        );
    }
    assert_eq!(link.a.send.kind(), ConnectionKind::Direct);
    link.close().await;
    network.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn symmetric_private_nats_fall_back_and_keep_the_same_noise_channel() {
    let network = test_nat::Network::new(true, false).await.unwrap();
    let options = |index: usize| p2p::Options {
        stun_servers: vec!["stun:1.2.3.4:3478".into()],
        network: Some(network.peers[index].clone()),
        srflx_only: true,
    };
    let mut link = Link::with_options(options(0), options(1));
    let (mut a, mut b) = link.noise().await;
    link.b
        .send
        .enable_direct(Role::Responder, [27; 32])
        .unwrap();
    link.a
        .send
        .enable_direct(Role::Initiator, [27; 32])
        .unwrap();
    tokio::try_join!(
        wait_kind(&link.a.send, ConnectionKind::Connecting),
        wait_kind(&link.b.send, ConnectionKind::Connecting)
    )
    .unwrap();
    // Relay remains usable while ICE tries to punch the two symmetric NATs.
    link.a
        .send
        .send(a.encrypt(b"during ICE").unwrap())
        .await
        .unwrap();
    assert_eq!(
        b.decrypt(&link.b.recv.recv().await.unwrap()).unwrap(),
        b"during ICE"
    );
    tokio::try_join!(
        wait_kind(&link.a.send, ConnectionKind::Relay),
        wait_kind(&link.b.send, ConnectionKind::Relay)
    )
    .unwrap();
    link.a
        .send
        .send(a.encrypt(b"after ICE fallback").unwrap())
        .await
        .unwrap();
    assert_eq!(
        b.decrypt(&link.b.recv.recv().await.unwrap()).unwrap(),
        b"after ICE fallback"
    );
    assert!(
        link.a
            .send
            .enable_direct(Role::Initiator, [28; 32])
            .is_err()
    );
    link.close().await;
    network.close().await.unwrap();
}

#[tokio::test]
async fn receiver_keeps_carrier_error_chain_and_eof_but_protocol_is_not_io() {
    for eof in [false, true] {
        let (outgoing, _writes) = mpsc::channel(8);
        let (incoming, reads) = mpsc::channel(8);
        let (_send, mut recv, guard) = start(Carrier {
            outgoing,
            incoming: reads,
        });
        if !eof {
            incoming
                .send(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "original carrier reset",
                )
                .into()))
                .await
                .unwrap();
        }
        drop(incoming);
        let error = recv.recv().await.unwrap_err();
        let io = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(
            io.kind(),
            if eof {
                std::io::ErrorKind::UnexpectedEof
            } else {
                std::io::ErrorKind::ConnectionReset
            }
        );
        if !eof {
            assert!(format!("{error:#}").contains("original carrier reset"));
        }
        assert!(guard.join().await.is_err());
    }
    let (outgoing, _writes) = mpsc::channel(8);
    let (incoming, reads) = mpsc::channel(8);
    let (_send, mut recv, guard) = start(Carrier {
        outgoing,
        incoming: reads,
    });
    incoming.send(Ok(b"not an overlay".to_vec())).await.unwrap();
    let error = recv.recv().await.unwrap_err();
    assert!(
        error
            .chain()
            .all(|cause| cause.downcast_ref::<std::io::Error>().is_none())
    );
    assert!(format!("{error:#}").contains("invalid transport envelope"));
    assert!(guard.join().await.is_err());
}

#[tokio::test]
async fn receiver_drains_ordered_records_before_eof_or_original_carrier_error() {
    for eof in [true, false] {
        let (outgoing, _writes) = mpsc::channel(8);
        let (incoming, reads) = mpsc::channel(WINDOW + 1);
        let (_send, mut recv, guard) = start(Carrier {
            outgoing,
            incoming: reads,
        });
        for seq in [1, 0].into_iter().chain(2..WINDOW) {
            incoming
                .send(Ok(wire::data(seq as u64, &seq.to_be_bytes())))
                .await
                .unwrap();
        }
        if !eof {
            incoming
                .send(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "reset after final records",
                )
                .into()))
                .await
                .unwrap();
        }
        drop(incoming);
        // Wait for termination before consuming, deterministically closing the
        // ACK notification channel while a full ordered window remains queued.
        assert!(
            timeout(Duration::from_secs(5), guard.join())
                .await
                .unwrap()
                .is_err()
        );
        for seq in 0..WINDOW {
            assert_eq!(recv.recv().await.unwrap(), seq.to_be_bytes());
        }
        let error = recv.recv().await.unwrap_err();
        let io = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(
            io.kind(),
            if eof {
                std::io::ErrorKind::UnexpectedEof
            } else {
                std::io::ErrorKind::ConnectionReset
            }
        );
        if !eof {
            assert!(format!("{error:#}").contains("reset after final records"));
        }
    }
}

#[tokio::test]
async fn guard_drop_closes_actor_and_carrier_and_releases_window() {
    let (outgoing, mut writes) = mpsc::channel(8);
    let (_incoming, reads) = mpsc::channel(8);
    let (send, mut recv, guard) = start(Carrier {
        outgoing,
        incoming: reads,
    });
    drop(guard);
    timeout(Duration::from_secs(1), async {
        assert!(recv.recv().await.is_err());
        assert!(writes.recv().await.is_none());
        assert!(send.send(vec![1]).await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn out_of_order_carrier_copies_are_delivered_once_and_window_is_bounded() {
    let (outgoing, mut writes) = mpsc::channel::<Write>(16);
    let (incoming, reads) = mpsc::channel(16);
    let (_send, mut recv, guard) = start(Carrier {
        outgoing,
        incoming: reads,
    });
    let drain = tokio::spawn(async move {
        while let Some(write) = writes.recv().await {
            let _ = write.completed.send(Ok(()));
        }
    });
    for (seq, data) in [(2, b"c"), (0, b"a"), (1, b"b"), (0, b"a"), (2, b"c")] {
        incoming.send(Ok(wire::data(seq, data))).await.unwrap();
    }
    for expected in [b"a", b"b", b"c"] {
        assert_eq!(recv.recv().await.unwrap(), expected);
    }
    assert!(
        timeout(Duration::from_millis(50), recv.recv())
            .await
            .is_err()
    );
    incoming
        .send(Ok(wire::data(11, b"outside window")))
        .await
        .unwrap();
    let error = recv.recv().await.unwrap_err();
    assert!(format!("{error:#}").contains("receive window exceeded"));
    assert!(guard.join().await.is_err());
    drain.await.unwrap();
}

#[tokio::test]
async fn unauthorized_early_signal_queue_is_bounded_without_starting_rtc() {
    let (outgoing, _writes) = mpsc::channel(16);
    let (incoming, reads) = mpsc::channel(16);
    let (send, mut recv, guard) = start(Carrier {
        outgoing,
        incoming: reads,
    });
    // Invalid encrypted bytes would fail authentication if decrypted. Before the
    // authorized enable they are only retained, and no negotiation is started.
    for _ in 0..MAX_EARLY_SIGNALS {
        incoming.send(Ok(wire::signal(&[0; 40]))).await.unwrap();
    }
    assert!(
        timeout(Duration::from_millis(20), recv.recv())
            .await
            .is_err()
    );
    assert_eq!(send.kind(), ConnectionKind::Relay);
    incoming.send(Ok(wire::signal(&[0; 40]))).await.unwrap();
    let error = recv.recv().await.unwrap_err();
    assert!(format!("{error:#}").contains("too many early signals"));
    assert!(guard.join().await.is_err());
}

#[tokio::test]
async fn impossible_ack_fails_as_protocol_error() {
    let (outgoing, _writes) = mpsc::channel(8);
    let (incoming, reads) = mpsc::channel(8);
    let (_send, mut recv, guard) = start(Carrier {
        outgoing,
        incoming: reads,
    });
    incoming.send(Ok(wire::ack(1))).await.unwrap();
    let error = recv.recv().await.unwrap_err();
    assert!(format!("{error:#}").contains("ACK exceeds sent sequence"));
    assert!(
        error
            .chain()
            .all(|cause| cause.downcast_ref::<std::io::Error>().is_none())
    );
    assert!(guard.join().await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_guard_with_open_rtc_finishes_actor_cleanup() {
    let mut link = Link::new();
    link.direct().await;
    drop(link.a.guard.take().unwrap());
    // The actor owns its delivery sender until its RTC shutdown has completed.
    assert!(
        timeout(Duration::from_secs(5), link.a.recv.recv())
            .await
            .unwrap()
            .is_err()
    );
    assert!(link.a.send.send(b"after guard".to_vec()).await.is_err());
    // The other actor may already observe its carrier disappearing, either result
    // is allowed; its shutdown still awaits its own RTC workers.
    let _ = link.b.guard.take().unwrap().shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_authenticated_offer_waits_for_authorized_enable_then_connects() {
    let mut link = Link::new();
    let (mut a, mut b) = link.noise().await;
    link.a
        .send
        .enable_direct(Role::Initiator, [65; 32])
        .unwrap();
    wait_kind(&link.a.send, ConnectionKind::Connecting)
        .await
        .unwrap();
    // The responder has not processed the protected EnableDirect yet. Incoming
    // offer/candidate frames must wait without starting its RTC or failing Noise.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(link.b.send.kind(), ConnectionKind::Relay);
    let early = timeout(Duration::from_millis(20), link.b.recv.recv()).await;
    assert!(
        early.is_err(),
        "unexpected early delivery/termination: {early:?}"
    );
    link.b
        .send
        .enable_direct(Role::Responder, [65; 32])
        .unwrap();
    tokio::try_join!(
        wait_kind(&link.a.send, ConnectionKind::Direct),
        wait_kind(&link.b.send, ConnectionKind::Direct)
    )
    .unwrap();
    link.a
        .send
        .send(a.encrypt(b"authorized delayed enable").unwrap())
        .await
        .unwrap();
    assert_eq!(
        b.decrypt(
            &timeout(Duration::from_secs(3), link.b.recv.recv())
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        b"authorized delayed enable"
    );
    link.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn carrier_eof_terminates_even_an_open_direct_connection() {
    let mut link = Link::new();
    link.direct().await;
    // Stop b->a WSS forwarding. Direct is not allowed to outlive the authenticated
    // carrier's revocation/disconnect boundary.
    link.pumps[1].abort();
    let error = timeout(Duration::from_secs(5), link.a.recv.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof)
    }));
    assert!(
        link.a
            .send
            .send(b"after carrier EOF".to_vec())
            .await
            .is_err()
    );
    assert!(link.a.guard.take().unwrap().join().await.is_err());
    let _ = link.b.guard.take().unwrap().shutdown().await;
}
