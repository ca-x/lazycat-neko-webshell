use super::super::{Session, Socket, protocol, tests::channels};
use super::*;
use futures_util::{SinkExt, StreamExt};
use protocol::messages::{Event, Operation, RelayFrame, Request, Response};
use std::time::Duration;
use tokio::{
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_tungstenite::{MaybeTlsStream, tungstenite::Message};

#[tokio::test]
async fn burst_input_keeps_receiving_with_both_overlay_windows_full() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client_channel, mut host_channel, remote_key) = channels();
    let (outgoing, mut writes) = mpsc::channel::<Write>(8);
    let (wire, incoming) = mpsc::channel(8);
    let (host_sender, mut host_receiver, guard) =
        neko_remoted_transport::start(Carrier { outgoing, incoming });
    let (finished, hold_open) = oneshot::channel::<()>();
    let host = tokio::spawn(async move {
        for id in 1..=32 {
            let plain = host_channel
                .decrypt(&host_receiver.recv().await.unwrap())
                .unwrap();
            let request: Request = protocol::unpack(&plain).unwrap();
            assert_eq!(request.id, id);
            assert!(
                matches!(request.operation, Operation::TerminalInput { stream, data }
                if stream == "fixture" && protocol::decode(&data, 64).unwrap() == id.to_string().as_bytes())
            );
            for event in [
                Event::Ok,
                Event::TerminalOutput {
                    stream: "fixture".into(),
                    sequence: id * 2 - 1,
                    data: protocol::encode(b"echo"),
                    replay: false,
                    reset: false,
                    gap: false,
                },
                Event::TerminalOutput {
                    stream: "fixture".into(),
                    sequence: id * 2,
                    data: protocol::encode(b"prompt"),
                    replay: false,
                    reset: false,
                    gap: false,
                },
            ] {
                let response = Response {
                    id: if matches!(event, Event::Ok) { id } else { 0 },
                    event,
                };
                let encrypted = host_channel
                    .encrypt(&protocol::pack(&response).unwrap())
                    .unwrap();
                host_sender.send(encrypted).await.unwrap();
            }
        }
        let _ = hold_open.await;
    });
    let _host_abort = super::super::AbortOnDrop(Some(host.abort_handle()));
    let broker = tokio::spawn(async move {
        let _guard = guard;
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut sequence = 0;
        let mut outstanding = 0u8;
        loop {
            tokio::select! {
                write = writes.recv(), if outstanding < 8 => {
                    let Some(write) = write else { return; };
                    sequence += 1;
                    let frame = RelayFrame::Data { peer: "fixture".into(), data: protocol::encode(&write.data), sequence: Some(sequence) };
                    let result = socket.send(Message::Text(serde_json::to_string(&frame).unwrap().into())).await.map_err(anyhow::Error::from);
                    let failed = result.is_err(); let _ = write.completed.send(result);
                    if failed { return; }
                    outstanding += 1;
                }
                message = socket.next() => {
                    let Some(Ok(Message::Text(text))) = message else { return; };
                    match serde_json::from_str::<RelayFrame>(&text).unwrap() {
                        RelayFrame::Data { data, .. } => {
                            wire.send(Ok(protocol::decode(&data, protocol::MAX_WIRE_PAYLOAD).unwrap())).await.unwrap();
                            let frame = RelayFrame::Delivered { peer: "fixture".into(), count: 1 };
                            socket.send(Message::Text(serde_json::to_string(&frame).unwrap().into())).await.unwrap();
                        }
                        RelayFrame::Acknowledge { .. } => outstanding = outstanding.checked_sub(1).unwrap(),
                        other => panic!("unexpected carrier frame {other:?}"),
                    }
                }
            }
        }
    });
    let _broker_abort = super::super::AbortOnDrop(Some(broker.abort_handle()));
    let (socket, _) = tokio_tungstenite::client_async(
        format!("ws://{address}"),
        MaybeTlsStream::Plain(TcpStream::connect(address).await.unwrap()),
    )
    .await
    .unwrap();
    let socket: Socket = socket;
    let mut session = Session {
        transport: Connection::start(Transport::start(socket, "fixture".into())),
        direct_key: None,
        channel: client_channel,
        remote_key,
        next: 1,
    };
    timeout(Duration::from_millis(500), async {
        for id in 1..=32 {
            assert_eq!(
                session
                    .request(Operation::TerminalInput {
                        stream: "fixture".into(),
                        data: protocol::encode(id.to_string().as_bytes())
                    })
                    .await
                    .unwrap(),
                id
            );
        }
    })
    .await
    .expect("burst enqueue must not await the peer's application consumption ACK");
    timeout(Duration::from_secs(5), async {
        let mut accepted = 0;
        let mut output = 0;
        while accepted < 32 || output < 64 {
            let response = session.receive().await.unwrap();
            match response.event {
                Event::Ok => {
                    accepted += 1;
                    assert_eq!(response.id, accepted);
                }
                Event::TerminalOutput { .. } => output += 1,
                event => panic!("unexpected response {event:?}"),
            }
        }
    })
    .await
    .expect("both directions must continue through all 32 inputs and 64 output records");
    let _ = finished.send(());
    host.await.unwrap();
    session.close().await;
}

async fn live_session() -> (
    Session,
    protocol::crypto::Channel,
    tokio_tungstenite::WebSocketStream<TcpStream>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client, server) = tokio::join!(
        tokio_tungstenite::connect_async(format!("ws://{address}")),
        async {
            let (socket, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(socket).await.unwrap()
        }
    );
    let (channel, host, remote_key) = channels();
    (
        Session {
            transport: Connection::start(Transport::start(client.unwrap().0, "fixture".into())),
            direct_key: None,
            channel,
            remote_key,
            next: 1,
        },
        host,
        server,
    )
}

async fn relay_frame(
    socket: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    frame: RelayFrame,
) {
    socket
        .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn queued_authenticated_denial_survives_immediate_relay_eof() {
    let (mut session, mut host, mut socket) = live_session().await;
    let response = Response {
        id: 1,
        event: Event::Error {
            code: "AUTHORIZATION_FAILED".into(),
            message: "fixture readonly upload denied".into(),
            outcome_unknown: false,
        },
    };
    let ciphertext = host.encrypt(&protocol::pack(&response).unwrap()).unwrap();
    let mut data = b"NKT1\0".to_vec();
    data.extend_from_slice(&0u64.to_be_bytes());
    data.extend_from_slice(&ciphertext);
    relay_frame(
        &mut socket,
        RelayFrame::Data {
            peer: "fixture".into(),
            data: protocol::encode(&data),
            sequence: Some(1),
        },
    )
    .await;
    relay_frame(
        &mut socket,
        RelayFrame::Closed {
            peer: "fixture".into(),
        },
    )
    .await;
    // Force the complete EOF/actor shutdown before the application's first read.
    let Connection::Live(path) = &session.transport else {
        unreachable!()
    };
    timeout(Duration::from_secs(1), async {
        while !path.bridge.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reply = session
        .receive()
        .await
        .expect("EOF must not hide queued authenticated denial");
    assert_eq!(reply.id, 1);
    assert!(
        matches!(reply.event, Event::Error { code, outcome_unknown: false, .. } if code == "AUTHORIZATION_FAILED")
    );
    let error = session.receive().await.unwrap_err();
    assert!(error.is::<super::super::LinkClosed>(), "{error:#}");
    assert!(super::super::retryable(&error));
}

#[tokio::test]
async fn full_request_queue_does_not_advance_id_or_noise_nonce() {
    let (mut session, mut host, mut socket) = live_session().await;
    let Connection::Live(path) = &session.transport else {
        unreachable!()
    };
    let mut permits = Vec::new();
    for _ in 0..REQUEST_QUEUE {
        permits.push(path.requests.clone().try_reserve_owned().unwrap());
    }
    let error = session
        .request(Operation::Ping {
            nonce: "not accepted".into(),
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("未入队"), "{error:#}");
    assert_eq!(session.next, 1);
    drop(permits);
    assert_eq!(
        session
            .request(Operation::Ping {
                nonce: "accepted".into()
            })
            .await
            .unwrap(),
        1
    );
    let message = timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = message else {
        panic!("expected encrypted request")
    };
    let RelayFrame::Data { data, .. } = serde_json::from_str(&text).unwrap() else {
        panic!("expected Relay data")
    };
    let bytes = protocol::decode(&data, protocol::MAX_WIRE_PAYLOAD).unwrap();
    assert_eq!(&bytes[..13], b"NKT1\0\0\0\0\0\0\0\0\0");
    let request: Request = protocol::unpack(&host.decrypt(&bytes[13..]).unwrap()).unwrap();
    assert_eq!(request.id, 1);
    assert!(matches!(request.operation, Operation::Ping { nonce } if nonce == "accepted"));
}

#[tokio::test]
async fn background_failure_preserves_authentication_cause_without_replaying_queued_requests() {
    let (mut session, _, mut socket) = live_session().await;
    for id in 0..32 {
        session
            .request(Operation::Ping {
                nonce: id.to_string(),
            })
            .await
            .unwrap();
    }
    relay_frame(
        &mut socket,
        RelayFrame::Error {
            code: "revoked".into(),
        },
    )
    .await;
    let error = timeout(Duration::from_secs(1), session.receive())
        .await
        .unwrap()
        .unwrap_err();
    assert!(super::super::authentication_failure(&error), "{error:#}");
    assert!(!super::super::retryable(&error), "{error:#}");
    // No receive-side error can restart the queued writer. There can be at most
    // one eight-record window already sent before the revocation arrived.
    let Connection::Live(path) = &mut session.transport else {
        unreachable!()
    };
    let _ = (&mut path.bridge).await;
    let mut sent = 0;
    while let Some(Ok(Message::Text(text))) = timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
    {
        if matches!(
            serde_json::from_str::<RelayFrame>(&text).unwrap(),
            RelayFrame::Data { .. }
        ) {
            sent += 1;
        }
    }
    assert!(
        sent <= neko_remoted_transport::WINDOW,
        "queued requests replayed after revocation"
    );
}
