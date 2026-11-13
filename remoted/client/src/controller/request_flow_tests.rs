use super::*;

fn writable_streams(actor: &mut Actor, streams: &[String]) {
    actor.model.phase = Phase::Ready;
    actor.model.permission = Some(Permission::Operate);
    actor.model.panes = streams
        .iter()
        .map(|stream| Pane {
            kind: PaneKind::Terminal,
            herdr_pane_id: None,
            herdr_grant: None,
            programs: Vec::new(),
            stream: stream.clone(),
            name: "shell".into(),
            controller: true,
            read_only: false,
            exit: None,
            unavailable: None,
            gap: false,
        })
        .collect();
}

#[tokio::test]
async fn keyboard_burst_is_paced_below_the_hosts_business_window_without_loss() {
    let (updates, _receiver) = mpsc::channel(256);
    let mut actor = Actor::new(updates);
    writable_streams(&mut actor, &["burst".into()]);
    let (session, mut requests) = Session::observed();
    actor.session = Some(session.into());
    let expected = "终端输入🙂".repeat(30);
    for character in expected.chars() {
        actor
            .request(
                Operation::TerminalInput {
                    stream: "burst".into(),
                    data: protocol::encode(character.to_string().as_bytes()),
                },
                Pending::Ordinary,
            )
            .await
            .expect("typing must be admitted locally without disconnecting");
        assert!(
            actor.outstanding.len() <= 4,
            "the sender must pace business requests before the host's eight-slot FIFO"
        );
    }
    let mut received = Vec::new();
    while let Ok(request) = requests.try_recv() {
        let Operation::TerminalInput { stream, data } = request.operation else {
            panic!("not terminal input")
        };
        assert_eq!(stream, "burst");
        received.extend(protocol::decode(&data, 16 * 1024).unwrap());
        actor
            .response(Response {
                id: request.id,
                event: Event::Ok,
            })
            .await
            .unwrap();
    }
    assert_eq!(received, expected.as_bytes());
    assert!(actor.outstanding.is_empty());
    assert!(actor.session.is_some());
    assert!(!actor.unknown_outcome);
}

#[tokio::test]
async fn cancel_bypasses_busy_input_and_retired_or_disconnected_input_is_never_replayed() {
    let (updates, _receiver) = mpsc::channel(256);
    let mut actor = Actor::new(updates);
    writable_streams(
        &mut actor,
        &(0..8)
            .map(|index| format!("stream-{index}"))
            .collect::<Vec<_>>(),
    );
    let (session, mut requests) = Session::observed();
    actor.session = Some(session.into());
    for index in 0..8 {
        actor
            .request(
                Operation::TerminalInput {
                    stream: format!("stream-{index}"),
                    data: protocol::encode(b"input"),
                },
                Pending::Ordinary,
            )
            .await
            .unwrap();
    }
    assert_eq!(actor.outstanding.len(), 4);
    for _ in 0..4 {
        requests.recv().await.unwrap();
    }
    actor
        .request(
            Operation::Cancel {
                transfer: "file".into(),
            },
            Pending::Cancel("file".into()),
        )
        .await
        .unwrap();
    assert!(matches!(
        requests.recv().await.unwrap().operation,
        Operation::Cancel { .. }
    ));
    actor.retire_stream("stream-4");
    let id = *actor.outstanding.keys().min().unwrap();
    actor
        .response(Response {
            id,
            event: Event::Ok,
        })
        .await
        .unwrap();
    // Cancellation still occupies a reserved slot; releasing it pumps the next live stream.
    let cancel = *actor
        .outstanding
        .iter()
        .find(|(_, v)| matches!(v.kind, Pending::Cancel(_)))
        .unwrap()
        .0;
    actor
        .response(Response {
            id: cancel,
            event: Event::Ok,
        })
        .await
        .unwrap();
    assert!(
        matches!(requests.recv().await.unwrap().operation, Operation::TerminalInput { stream, .. } if stream == "stream-5")
    );
    actor.clear_workspace();
    let (session, mut replacement) = Session::observed();
    actor.session = Some(session.into());
    actor.flush_requests().await.unwrap();
    assert!(
        replacement.try_recv().is_err(),
        "unsent input cannot migrate to a replacement connection"
    );
}

#[tokio::test]
async fn losing_remote_control_rejects_queued_composer_input_before_transmission() {
    let (updates, _receiver) = mpsc::channel(256);
    let mut actor = Actor::new(updates);
    writable_streams(&mut actor, &["s".into()]);
    let (session, mut requests) = Session::observed();
    actor.session = Some(session.into());
    for index in 0..8 {
        actor
            .submit_input("s".into(), format!("command-{index}"))
            .await
            .unwrap();
    }
    let mut sent = Vec::new();
    while let Ok(request) = requests.try_recv() {
        sent.push(request);
    }
    assert_eq!(sent.len(), 4);
    actor
        .response(Response {
            id: 0,
            event: Event::TerminalControl {
                stream: "s".into(),
                controller: false,
            },
        })
        .await
        .unwrap();
    actor
        .response(Response {
            id: sent[0].id,
            event: Event::Ok,
        })
        .await
        .unwrap();
    assert!(requests.try_recv().is_err());
    let history = actor.model.interaction.entries().collect::<Vec<_>>();
    assert!(
        history[4..]
            .iter()
            .all(|entry| entry.state == interaction::SubmissionState::Rejected)
    );
    assert_eq!(history[0].state, interaction::SubmissionState::Accepted);
}

#[tokio::test]
async fn full_authoritative_screen_clears_a_previous_gap_and_publishes_the_change() {
    let (updates, _receiver) = mpsc::channel(32);
    let mut actor = Actor::new(updates);
    actor.model.panes.push(Pane {
        kind: PaneKind::Terminal,
        herdr_pane_id: None,
        herdr_grant: None,
        programs: Vec::new(),
        stream: "s".into(),
        name: "shell".into(),
        controller: false,
        read_only: false,
        exit: None,
        unavailable: None,
        gap: true,
    });
    actor.output.insert(
        "s".into(),
        OutputState {
            decoder: Decoder::default(),
            sequence: Some(2),
        },
    );
    assert!(
        actor
            .response(Response {
                id: 0,
                event: Event::TerminalOutput {
                    stream: "s".into(),
                    sequence: 9,
                    data: protocol::encode(b"complete"),
                    replay: false,
                    reset: true,
                    gap: false
                }
            })
            .await
            .unwrap()
    );
    assert!(!actor.model.panes[0].gap);
}

#[tokio::test]
async fn remote_creation_requires_operate_and_attaches_only_a_requested_child_once() {
    let (updates, _receiver) = mpsc::channel(64);
    let mut actor = Actor::new(updates);
    actor.model.phase = Phase::Ready;
    actor.model.permission = Some(Permission::ReadOnly);
    actor.model.resources.push(Resource {
        id: "parent".into(),
        name: "Herdr".into(),
        backend: "herdr".into(),
        herdr: true,
        terminal: false,
        files: false,
    });
    let (session, mut requests) = Session::observed();
    actor.session = Some(session.into());
    let create = || {
        Command::Request(Operation::TerminalCreate {
            grant: "parent".into(),
        })
    };
    assert!(actor.command(create()).await.is_err());
    assert!(requests.try_recv().is_err());
    actor.model.permission = Some(Permission::Operate);
    actor.command(create()).await.unwrap();
    let request = requests.recv().await.unwrap();
    assert!(
        actor.command(create()).await.is_err(),
        "a pending create cannot be retried"
    );
    let child = Resource {
        id: "child".into(),
        name: "shell".into(),
        backend: "native".into(),
        herdr: false,
        terminal: true,
        files: false,
    };
    assert!(
        actor
            .response(Response {
                id: request.id + 100,
                event: Event::TerminalCreated {
                    resource: child.clone()
                }
            })
            .await
            .is_err()
    );
    actor
        .response(Response {
            id: request.id,
            event: Event::TerminalCreated { resource: child },
        })
        .await
        .unwrap();
    assert!(
        matches!(requests.recv().await.unwrap().operation, Operation::TerminalAttach { grant, pane: None } if grant == "child")
    );
    assert!(requests.try_recv().is_err());
}
