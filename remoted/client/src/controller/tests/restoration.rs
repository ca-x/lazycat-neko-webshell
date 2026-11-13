//! Exercise the Ready request burst against the real overlay's consumer window.
use super::*;
use neko_remoted_transport::{Carrier, Guard, Receiver, Sender, WINDOW, Write};
use protocol::crypto::{Channel, Handshake, Mode};

struct Endpoint {
    send: Sender,
    receive: Receiver,
    _guard: Guard,
}

fn carrier_pair() -> (Endpoint, Endpoint, [tokio::task::JoinHandle<()>; 2]) {
    let (client_out, mut client_writes) = mpsc::channel::<Write>(WINDOW);
    let (client_in, client_reads) = mpsc::channel(WINDOW);
    let (host_out, mut host_writes) = mpsc::channel::<Write>(WINDOW);
    let (host_in, host_reads) = mpsc::channel(WINDOW);
    let client = neko_remoted_transport::start(Carrier {
        outgoing: client_out,
        incoming: client_reads,
    });
    let host = neko_remoted_transport::start(Carrier {
        outgoing: host_out,
        incoming: host_reads,
    });
    let pumps = [
        tokio::spawn(async move {
            while let Some(write) = client_writes.recv().await {
                if host_in.send(Ok(write.data)).await.is_err() {
                    break;
                }
                let _ = write.completed.send(Ok(()));
            }
        }),
        tokio::spawn(async move {
            while let Some(write) = host_writes.recv().await {
                if client_in.send(Ok(write.data)).await.is_err() {
                    break;
                }
                let _ = write.completed.send(Ok(()));
            }
        }),
    ];
    (
        Endpoint {
            send: client.0,
            receive: client.1,
            _guard: client.2,
        },
        Endpoint {
            send: host.0,
            receive: host.1,
            _guard: host.2,
        },
        pumps,
    )
}

async fn noise(client: &mut Endpoint, host: &mut Endpoint) -> (Channel, Channel) {
    let client_identity = Identity::generate().unwrap();
    let host_identity = Identity::generate().unwrap();
    let relay = saved_binding(1).relay;
    let route = protocol::route(&protocol::random32().unwrap());
    let mut initiator = Handshake::new(
        &client_identity,
        &relay,
        &route,
        Mode::Connect,
        true,
        None,
        Some(host_identity.public()),
    )
    .unwrap();
    let mut responder = Handshake::new(
        &host_identity,
        &relay,
        &route,
        Mode::Connect,
        false,
        None,
        None,
    )
    .unwrap();
    client.send.send(initiator.write().unwrap()).await.unwrap();
    responder.read(&host.receive.recv().await.unwrap()).unwrap();
    host.send.send(responder.write().unwrap()).await.unwrap();
    initiator
        .read(&client.receive.recv().await.unwrap())
        .unwrap();
    (
        initiator.into_channel().unwrap().0,
        responder.into_channel().unwrap().0,
    )
}

async fn burst_through_carrier(operations: Vec<Operation>, expect_blocked: bool) {
    let (mut client, mut host, pumps) = carrier_pair();
    let (mut client_noise, mut host_noise) = noise(&mut client, &mut host).await;
    let attachments = operations.len() - 1;
    let host_task = tokio::spawn(async move {
        for index in 0..=attachments {
            let request: protocol::messages::Request = protocol::unpack(
                &host_noise
                    .decrypt(&host.receive.recv().await.unwrap())
                    .unwrap(),
            )
            .unwrap();
            let event = if index == 0 {
                assert!(matches!(request.operation, Operation::EnableDirect { .. }));
                Event::DirectReady {}
            } else {
                assert!(
                    matches!(request.operation, Operation::TerminalAttach { .. }),
                    "restoration must never send input or control"
                );
                Event::TerminalAttached {
                    stream: format!("restored-{index}"),
                    name: "restored".into(),
                    read_only: false,
                }
            };
            let response = Response {
                id: request.id,
                event,
            };
            host.send
                .send(
                    host_noise
                        .encrypt(&protocol::pack(&response).unwrap())
                        .unwrap(),
                )
                .await
                .unwrap();
            if index != 0 {
                // One attachment can replay more records than the overlay window.
                // The host cannot read the next request until these are consumed.
                for sequence in 0..=WINDOW {
                    let response = Response {
                        id: 0,
                        event: Event::TerminalOutput {
                            stream: format!("restored-{index}"),
                            sequence: sequence as u64,
                            data: protocol::encode(b"replay"),
                            replay: true,
                            reset: sequence == 0,
                            gap: false,
                        },
                    };
                    host.send
                        .send(
                            host_noise
                                .encrypt(&protocol::pack(&response).unwrap())
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                }
            }
        }
        host
    });
    let (complete, mut completed) = tokio::sync::oneshot::channel();
    let sender = client.send.clone();
    let burst = tokio::spawn(async move {
        for (index, operation) in operations.into_iter().enumerate() {
            let request = protocol::messages::Request {
                id: index as u64 + 1,
                operation,
            };
            sender
                .send(
                    client_noise
                        .encrypt(&protocol::pack(&request).unwrap())
                        .unwrap(),
                )
                .await
                .unwrap();
        }
        complete.send(()).unwrap();
        client_noise
    });
    if expect_blocked {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut completed)
                .await
                .is_err(),
            "a burst above the supported pane limit needs interleaved receive"
        );
    } else {
        tokio::time::timeout(Duration::from_secs(1), &mut completed)
            .await
            .expect("supported Ready burst must finish without consuming host replay")
            .unwrap();
    }
    // Draining the real Receiver emits consumer ACKs and releases both windows.
    for _ in 0..1 + attachments * (WINDOW + 2) {
        tokio::time::timeout(Duration::from_secs(1), client.receive.recv())
            .await
            .unwrap()
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), burst)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), host_task)
        .await
        .unwrap()
        .unwrap();
    for pump in pumps {
        pump.abort();
    }
}

#[tokio::test]
async fn eight_confirmed_restores_and_direct_negotiation_do_not_stall_on_real_carrier() {
    let mut actor = opened_actor();
    let template = actor.model.panes[1].clone();
    actor.model.panes.clear();
    actor.attachments.clear();
    for index in 0..8 {
        let stream = format!("confirmed-{index}");
        let pane = format!("pane-{index}");
        let mut entry = template.clone();
        entry.stream = stream.clone();
        entry.herdr_pane_id = Some(pane.clone());
        actor.model.panes.push(entry);
        actor.attachments.insert(
            stream,
            Attachment {
                grant: "herdr".into(),
                pane: Some(pane),
            },
        );
    }
    let error = actor
        .command(Command::Request(Operation::TerminalAttach {
            grant: "terminal".into(),
            pane: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "最多打开 8 个终端");
    let resources = actor.model.resources.clone();
    let target = actor.model.active_binding.clone().unwrap();
    actor.disconnected().await;
    let (session, mut requests) = Session::observed();
    actor
        .reconnect_with(
            target.clone(),
            ReconnectMode::Retry,
            async { Ok(()) },
            async { Ok((Identity::generate()?, session)) },
        )
        .await
        .unwrap();
    actor
        .response(Response {
            id: 0,
            event: Event::Ready {
                device_id: target.device_id,
                permission: Permission::Operate,
                resources,
            },
        })
        .await
        .unwrap();
    let mut operations = Vec::new();
    for index in 0..9 {
        let request = tokio::time::timeout(Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        let attach = matches!(request.operation, Operation::TerminalAttach { .. });
        operations.push(request.operation);
        if attach {
            assert!(
                actor.outstanding.len() <= 4,
                "restores must share the paced business lane"
            );
            actor
                .response(Response {
                    id: request.id,
                    event: Event::TerminalAttached {
                        stream: format!("restored-{index}"),
                        name: "restored".into(),
                        read_only: false,
                    },
                })
                .await
                .unwrap();
        }
    }
    assert!(requests.try_recv().is_err());
    assert!(actor.outstanding.is_empty());
    assert_eq!(actor.model.panes.len(), 8);
    burst_through_carrier(operations, false).await;
}

#[tokio::test]
async fn real_carrier_requires_receive_interleaving_above_the_supported_pane_limit() {
    let mut operations = vec![Operation::EnableDirect {
        key: protocol::encode(&protocol::random32().unwrap()),
    }];
    operations.extend((0..32).map(|index| Operation::TerminalAttach {
        grant: "herdr".into(),
        pane: Some(format!("pane-{index}")),
    }));
    burst_through_carrier(operations, true).await;
}
