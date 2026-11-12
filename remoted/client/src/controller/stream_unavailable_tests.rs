//! A failed backend attachment cannot disconnect other sessions or claim an exit.
use super::*;

fn pane(stream: &str) -> Pane {
    Pane {
        kind: PaneKind::Terminal,
        herdr_pane_id: None,
        herdr_grant: None,
        programs: Vec::new(),
        stream: stream.into(),
        name: stream.into(),
        controller: true,
        read_only: false,
        exit: None,
        unavailable: None,
        gap: false,
    }
}

#[tokio::test]
async fn interrupted_stream_keeps_its_screen_and_other_streams_live() {
    let (updates, mut received) = mpsc::channel(32);
    let mut actor = Actor::new(updates);
    actor.model.phase = Phase::Ready;
    actor.model.permission = Some(Permission::Operate);
    for (stream, backend) in [("hc-stream", "hc"), ("native-stream", "native")] {
        actor.model.panes.push(pane(stream));
        actor.model.resources.push(Resource {
            id: stream.into(),
            name: stream.into(),
            backend: backend.into(),
            terminal: true,
            herdr: false,
            files: false,
        });
        actor.attachments.insert(
            stream.into(),
            Attachment {
                grant: stream.into(),
                pane: None,
            },
        );
        actor.output.insert(
            stream.into(),
            OutputState {
                decoder: Decoder::default(),
                sequence: Some(3),
            },
        );
    }
    let changed = actor
        .response(Response {
            id: 0,
            event: Event::TerminalUnavailable {
                stream: "hc-stream".into(),
                message: "channel detached; process may still be running".into(),
            },
        })
        .await
        .unwrap();
    assert!(changed);
    assert!(actor.model.ready());
    assert_eq!(actor.model.panes.len(), 2);
    assert!(actor.model.panes[0].unavailable.is_some());
    assert_eq!(actor.model.panes[0].exit, None);
    assert!(!actor.model.panes[0].live());
    assert!(actor.model.panes[1].live() && actor.model.panes[1].controller);
    assert!(!actor.attachments.contains_key("hc-stream"));
    assert!(actor.attachments.contains_key("native-stream"));
    assert!(
        actor
            .validate(&Operation::TerminalInput {
                stream: "hc-stream".into(),
                data: protocol::encode(b"must not send")
            })
            .is_err()
    );
    assert!(
        actor
            .validate(&Operation::TerminalControl {
                stream: "hc-stream".into(),
                take: true
            })
            .is_err()
    );
    assert!(
        actor
            .validate(&Operation::TerminalResize {
                stream: "hc-stream".into(),
                cols: 80,
                rows: 24
            })
            .is_err()
    );
    assert!(
        actor
            .validate(&Operation::TerminalInput {
                stream: "native-stream".into(),
                data: protocol::encode(b"still live")
            })
            .is_ok()
    );
    assert!(
        matches!(received.recv().await, Some(Update::Output { stream, reset: false, .. }) if stream == "hc-stream")
    );

    // A queued late control event must not revive the attachment.
    assert!(
        !actor
            .response(Response {
                id: 0,
                event: Event::TerminalControl {
                    stream: "hc-stream".into(),
                    controller: true
                }
            })
            .await
            .unwrap()
    );
    assert!(!actor.model.panes[0].controller);
    actor.capture_restoration();
    assert_eq!(actor.restoring.len(), 1);
    assert!(actor.restoring.contains_key("native-stream"));

    // Closing the retained view works even though the host's channel is gone.
    // There is deliberately no WorkspaceChannel, so this would fail if sent.
    actor
        .command(Command::Request(Operation::TerminalDetach {
            stream: "hc-stream".into(),
        }))
        .await
        .unwrap();
    assert_eq!(actor.model.panes.len(), 1);
    assert_eq!(actor.model.panes[0].stream, "native-stream");
    assert!(actor.model.ready());
}
