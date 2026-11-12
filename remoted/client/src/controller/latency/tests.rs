use super::*;
use protocol::messages::Request;

fn path(kind: ConnectionKind, generation: u64) -> Option<Path> {
    Some(Path { generation, kind })
}

#[test]
fn samples_require_both_request_id_and_nonce_and_use_the_actual_round_trip() {
    let now = Instant::now();
    let path = path(ConnectionKind::Relay, 1);
    let mut latency = Latency::default();
    latency.sync(path, now);
    assert!(latency.due(now));
    latency.sent(7, "noise-probe".into(), now);
    let received = now + Duration::from_millis(42);
    assert!(!latency.pong(8, "noise-probe", received));
    assert!(!latency.pong(7, "relay-ping", received));
    assert_eq!(latency.millis(path, received), None);
    assert!(latency.pong(7, "noise-probe", received));
    assert_eq!(latency.millis(path, received), Some(42));
    assert!(!latency.pong(7, "noise-probe", received));
    assert!(!latency.due(now + INTERVAL - Duration::from_nanos(1)));
    assert!(latency.due(now + INTERVAL));
}

#[test]
fn only_one_probe_is_pending_until_its_thirty_second_expiry() {
    let now = Instant::now();
    let path = path(ConnectionKind::Relay, 1);
    let mut latency = Latency::default();
    latency.sync(path, now);
    latency.sent(1, "old".into(), now);
    for seconds in [10, 20, 29] {
        let at = now + Duration::from_secs(seconds);
        latency.sync(path, at);
        assert!(!latency.due(at));
    }
    let expired = now + EXPIRY;
    assert!(!latency.pong(1, "old", expired));
    latency.sync(path, expired);
    assert!(latency.due(expired));
    latency.sent(2, "fresh".into(), expired);
    assert!(!latency.pong(1, "old", expired));
    assert!(latency.pong(2, "fresh", expired + Duration::from_millis(3)));
}

#[test]
fn samples_expire_even_without_another_tick_or_response() {
    let now = Instant::now();
    let path = path(ConnectionKind::Direct, 1);
    let mut latency = Latency::default();
    latency.sync(path, now);
    latency.sent(1, "probe".into(), now);
    let received = now + Duration::from_millis(2);
    assert!(latency.pong(1, "probe", received));
    assert_eq!(latency.millis(path, received + EXPIRY), None);
    assert_eq!(latency.millis(None, received), None);
    assert_eq!(
        latency.millis(self::path(ConnectionKind::Relay, 1), received),
        None
    );
    latency.sync(path, received + EXPIRY);
    assert!(latency.sample.is_none());
}

#[test]
fn every_path_or_generation_change_discards_samples_and_pending_pongs() {
    let now = Instant::now();
    let mut latency = Latency::default();
    let mut id = 0;
    for target in [
        path(ConnectionKind::Relay, 1),
        path(ConnectionKind::Connecting, 1),
        path(ConnectionKind::Direct, 1),
        path(ConnectionKind::Relay, 1),
        path(ConnectionKind::Relay, 2),
    ] {
        latency.sync(target, now);
        assert_eq!(latency.millis(target, now), None);
        assert!(latency.due(now));
        assert!(!latency.pong(id, "old-path", now));
        id += 1;
        latency.sent(id, "sample".into(), now);
        assert!(latency.pong(id, "sample", now + Duration::from_millis(5)));
        assert_eq!(
            latency.millis(target, now + Duration::from_millis(5)),
            Some(5)
        );
        id += 1;
        latency.sent(id, "old-path".into(), now + INTERVAL);
    }
    latency.sync(None, now);
    assert!(!latency.due(now));
    assert!(latency.pending.is_none() && latency.sample.is_none());
}

fn actor() -> (Actor, mpsc::Receiver<Request>) {
    let mut actor = Actor::new(mpsc::channel(32).0);
    let (session, requests) = Session::observed();
    actor.session = Some(session.into());
    actor.model.phase = Phase::Ready;
    (actor, requests)
}

#[tokio::test]
async fn client_sends_an_encrypted_ping_and_only_its_matching_noise_pong_updates_the_model() {
    let (mut actor, mut requests) = actor();
    let now = Instant::now();
    actor.latency_tick(now).await.unwrap();
    // Session::observed decrypts the request on the remote Noise endpoint.
    let request = requests.recv().await.unwrap();
    let Operation::Ping { nonce } = request.operation else {
        panic!("latency must use the authenticated application Ping")
    };
    assert_ne!(protocol::decode32(&nonce).unwrap(), [0; 32]);
    for (id, nonce) in [
        (request.id + 1, nonce.clone()),
        (request.id, "relay".into()),
    ] {
        actor
            .response(Response {
                id,
                event: Event::Pong { nonce },
            })
            .await
            .unwrap();
        assert_eq!(actor.model.latency_ms, None);
    }
    assert!(
        actor
            .response(Response {
                id: request.id,
                event: Event::Pong { nonce },
            })
            .await
            .unwrap()
    );
    assert!(actor.model.latency_ms.is_some());
    assert!(actor.outstanding.is_empty() && !actor.unknown_outcome);
    actor
        .latency_tick(now + INTERVAL - Duration::from_millis(1))
        .await
        .unwrap();
    assert!(requests.try_recv().is_err());
    actor.latency_tick(now + INTERVAL).await.unwrap();
    assert!(matches!(
        requests.recv().await.unwrap().operation,
        Operation::Ping { .. }
    ));
}

#[tokio::test]
async fn missing_pongs_expire_without_disconnect_retry_or_unknown_business_operations() {
    let (mut actor, mut requests) = actor();
    let now = Instant::now();
    actor.latency_tick(now).await.unwrap();
    let old = requests.recv().await.unwrap();
    for seconds in [10, 20, 29] {
        actor
            .latency_tick(now + Duration::from_secs(seconds))
            .await
            .unwrap();
        assert!(requests.try_recv().is_err());
    }
    actor.latency_tick(now + EXPIRY).await.unwrap();
    let fresh = requests.recv().await.unwrap();
    let (Operation::Ping { nonce: old_nonce }, Operation::Ping { nonce }) =
        (old.operation, fresh.operation)
    else {
        panic!("only Ping probes are expected")
    };
    assert_ne!(old_nonce, nonce);
    assert!(!actor.latency_pong(old.id, &old_nonce, now + EXPIRY));
    assert!(actor.latency_pong(fresh.id, &nonce, now + EXPIRY + Duration::from_millis(7)));
    assert_eq!(actor.model.latency_ms, Some(7));
    assert!(actor.model.ready() && actor.session.is_some());
    assert!(!actor.unknown_outcome && actor.outstanding.is_empty() && actor.retry.at.is_none());
}

#[tokio::test]
async fn unconfirmed_business_requests_drain_before_a_probe_is_added_to_the_window() {
    let (mut actor, mut requests) = actor();
    let now = Instant::now();
    actor.outstanding.insert(
        99,
        Outstanding {
            kind: Pending::Ordinary,
            expires: now + EXPIRY,
        },
    );
    actor.latency_tick(now).await.unwrap();
    assert!(requests.try_recv().is_err());
    assert!(actor.latency.pending.is_none());
    actor.outstanding.clear();
    actor.latency_tick(now).await.unwrap();
    assert!(matches!(
        requests.recv().await.unwrap().operation,
        Operation::Ping { .. }
    ));
    assert!(actor.outstanding.is_empty());
}

#[tokio::test]
async fn local_offline_and_superseded_generations_never_send_probes_or_show_stale_latency() {
    let (mut actor, mut requests) = actor();
    let now = Instant::now();
    actor.model.local = true;
    actor.model.latency_ms = Some(12);
    actor.latency_tick(now).await.unwrap();
    assert_eq!(actor.model.latency_ms, None);
    actor.model.local = false;
    actor.model.phase = Phase::AwaitingReady;
    actor.latency_tick(now).await.unwrap();
    actor.model.phase = Phase::Ready;
    actor.scope.advance().unwrap();
    actor.latency_tick(now).await.unwrap();
    assert!(requests.try_recv().is_err());
    assert!(actor.latency.pending.is_none());
}

#[tokio::test]
async fn disconnect_and_generation_changes_invalidate_samples_before_publication() {
    let (mut actor, mut requests) = actor();
    let now = Instant::now();
    actor.latency_tick(now).await.unwrap();
    let request = requests.recv().await.unwrap();
    let Operation::Ping { nonce } = request.operation else {
        panic!("expected Ping")
    };
    assert!(actor.latency_pong(request.id, &nonce, now + Duration::from_millis(8)));
    assert_eq!(
        actor.reported_latency(now + Duration::from_millis(8)),
        Some(8)
    );
    actor.scope.advance().unwrap();
    assert_eq!(actor.reported_latency(now + Duration::from_millis(8)), None);
    actor.disconnected().await;
    assert!(actor.model.latency_ms.is_none() && actor.latency.pending.is_none());
    assert!(!actor.latency_pong(request.id, &nonce, now + INTERVAL));
    assert!(!actor.unknown_outcome);
}
