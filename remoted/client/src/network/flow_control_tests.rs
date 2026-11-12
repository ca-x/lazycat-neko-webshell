use super::*;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

type Server = WebSocketStream<TcpStream>;

async fn link() -> (Transport, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client, server) = tokio::join!(
        tokio_tungstenite::connect_async(format!("ws://{address}")),
        async {
            let (socket, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(socket).await.unwrap()
        }
    );
    (
        Transport::start(client.unwrap().0, "fixture".into()),
        server,
    )
}

async fn frame(server: &mut Server, frame: RelayFrame) {
    server
        .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
        .await
        .unwrap();
}

async fn data(server: &mut Server, sequence: u64) {
    frame(
        server,
        RelayFrame::Data {
            peer: "fixture".into(),
            data: protocol::encode(&sequence.to_be_bytes()),
            sequence: Some(sequence),
        },
    )
    .await;
}

async fn queued(transport: &Transport, count: usize) {
    timeout(Duration::from_secs(1), async {
        while transport.incoming.len() != count {
            assert!(!transport.task.is_finished(), "socket pump stopped");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn next_frame(server: &mut Server) -> RelayFrame {
    let Message::Text(text) = timeout(Duration::from_secs(1), server.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    else {
        panic!("expected relay text frame")
    };
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn paused_consumer_withholds_credits_but_pump_services_writes_and_ping() {
    let (mut transport, mut server) = link().await;
    for sequence in 1..=8 {
        data(&mut server, sequence).await;
    }
    queued(&transport, 8).await;
    assert!(
        timeout(Duration::from_millis(20), server.next())
            .await
            .is_err(),
        "queued frames must not return delivery credits before consumption"
    );
    // A full inbound queue must not block writes or their returned credits.
    for _ in 0..8 {
        timeout(DEADLINE, transport.send(vec![42]))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            next_frame(&mut server).await,
            RelayFrame::Data { .. }
        ));
    }
    frame(
        &mut server,
        RelayFrame::Delivered {
            peer: "fixture".into(),
            count: 8,
        },
    )
    .await;
    timeout(DEADLINE, transport.send(vec![43]))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        next_frame(&mut server).await,
        RelayFrame::Data { .. }
    ));
    server.send(Message::Ping(vec![7].into())).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), server.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Pong(vec![7].into())
    );
    assert_eq!(transport.receive().await.unwrap(), 1u64.to_be_bytes());
    assert!(matches!(
        next_frame(&mut server).await,
        RelayFrame::Acknowledge { sequence: 1, .. }
    ));
    // The sender spends only the single credit returned by consumption.
    data(&mut server, 9).await;
    queued(&transport, 8).await;
    for sequence in 2u64..=9 {
        assert_eq!(transport.receive().await.unwrap(), sequence.to_be_bytes());
    }
    let mut acknowledged = 1;
    while acknowledged < 9 {
        let RelayFrame::Acknowledge { sequence, .. } = next_frame(&mut server).await else {
            panic!("expected consumption acknowledgement")
        };
        assert_eq!(sequence, acknowledged + 1);
        acknowledged = sequence;
    }
    assert!(!transport.task.is_finished());
}

#[tokio::test]
async fn relay_failure_survives_a_full_data_queue() {
    let (mut transport, mut server) = link().await;
    for sequence in 1..=8 {
        data(&mut server, sequence).await;
    }
    queued(&transport, 8).await;
    frame(
        &mut server,
        RelayFrame::Error {
            code: "fixture_failure".into(),
        },
    )
    .await;
    timeout(Duration::from_secs(1), async {
        while !transport.task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for sequence in 1u64..=8 {
        assert_eq!(transport.receive().await.unwrap(), sequence.to_be_bytes());
    }
    let error = transport.receive().await.unwrap_err();
    assert!(error.to_string().contains("fixture_failure"), "{error:#}");
}

#[tokio::test]
async fn cancelled_receive_neither_consumes_nor_acknowledges_a_frame() {
    let (mut transport, mut server) = link().await;
    assert!(
        timeout(Duration::from_millis(1), transport.receive())
            .await
            .is_err()
    );
    data(&mut server, 1).await;
    queued(&transport, 1).await;
    assert!(
        timeout(Duration::from_millis(20), server.next())
            .await
            .is_err()
    );
    // Once a queued frame is removed, receive and credit notification must both
    // finish in this poll, with no cancellation point between them.
    let mut receive = Box::pin(transport.receive());
    assert!(
        matches!(futures_util::poll!(&mut receive), std::task::Poll::Ready(Ok(bytes)) if bytes == 1u64.to_be_bytes())
    );
    drop(receive);
    assert!(matches!(
        next_frame(&mut server).await,
        RelayFrame::Acknowledge { sequence: 1, .. }
    ));
    data(&mut server, 2).await;
    assert_eq!(transport.receive().await.unwrap(), 2u64.to_be_bytes());
    assert!(matches!(
        next_frame(&mut server).await,
        RelayFrame::Acknowledge { sequence: 2, .. }
    ));
}

#[tokio::test]
async fn websocket_close_preserves_code_and_reason() {
    let (mut transport, mut server) = link().await;
    server
        .close(Some(CloseFrame {
            code: CloseCode::Policy,
            reason: "fixture_close_reason".into(),
        }))
        .await
        .unwrap();
    let error = timeout(Duration::from_secs(1), transport.receive())
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("fixture_close_reason"),
        "{error:#}"
    );
    assert!(error.to_string().contains("1008"), "{error:#}");
    assert!(retryable(&error));
}

#[test]
fn health_requires_exact_v2_protocol_and_version() {
    assert!(
        validate_relay_health(&serde_json::json!({
            "protocol": protocol::RELAY_PROTOCOL,
            "version": protocol::RELAY_VERSION,
        }))
        .is_ok()
    );
    for health in [
        serde_json::json!({"protocol": "neko.remoted.v1", "version": 1}),
        serde_json::json!({"protocol": protocol::RELAY_PROTOCOL, "version": 1}),
        serde_json::json!({"protocols": [protocol::RELAY_PROTOCOL], "version": protocol::RELAY_VERSION}),
        serde_json::json!({"protocol": "neko.remoted.v1", "version": 1, "protocols": [protocol::RELAY_PROTOCOL]}),
    ] {
        assert!(validate_relay_health(&health).is_err(), "{health}");
    }
}

#[tokio::test]
async fn peer_close_retires_buffered_credits_without_an_ack_queue_error() {
    let (mut transport, mut server) = link().await;
    for sequence in 1..=8 {
        data(&mut server, sequence).await;
    }
    queued(&transport, 8).await;
    frame(
        &mut server,
        RelayFrame::Closed {
            peer: "fixture".into(),
        },
    )
    .await;
    timeout(Duration::from_secs(1), async {
        while !transport.task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(transport.consumed.is_closed());
    // These frames were already received before peer closure. Their now-retired
    // credits cannot be sent, but must not turn valid buffered data into errors.
    for sequence in 1u64..=8 {
        assert_eq!(transport.receive().await.unwrap(), sequence.to_be_bytes());
    }
    let error = transport.receive().await.unwrap_err();
    assert!(error.is::<LinkClosed>(), "{error:#}");
    assert!(retryable(&error));
    let last = timeout(Duration::from_secs(1), server.next())
        .await
        .unwrap();
    assert!(
        !matches!(last, Some(Ok(Message::Text(_)))),
        "ACK sent after peer closed"
    );
}

#[tokio::test]
async fn cancelled_write_waiting_for_real_credits_never_reaches_the_socket() {
    let (mut transport, mut server) = link().await;
    for value in 1..=8 {
        transport.send(vec![value]).await.unwrap();
        let RelayFrame::Data { data, .. } = next_frame(&mut server).await else {
            panic!("expected submitted write")
        };
        assert_eq!(
            protocol::decode(&data, protocol::MAX_CIPHERTEXT).unwrap(),
            [value]
        );
    }
    // All eight real wire credits are spent. This ninth write stays queued;
    // dropping its future must abort the pump before credits can resume it.
    assert!(
        timeout(Duration::from_millis(20), transport.send(vec![9]))
            .await
            .is_err()
    );
    tokio::task::yield_now().await;
    let delivered = RelayFrame::Delivered {
        peer: "fixture".into(),
        count: 8,
    };
    let _ = server
        .send(Message::Text(
            serde_json::to_string(&delivered).unwrap().into(),
        ))
        .await;
    let last = timeout(Duration::from_secs(1), server.next())
        .await
        .unwrap();
    assert!(
        !matches!(last, Some(Ok(Message::Text(_)))),
        "cancelled write escaped after credits returned"
    );
    assert!(transport.send(vec![10]).await.is_err());
}
