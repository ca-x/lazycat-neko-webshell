use super::*;
use crate::terminal_reply_authority::TerminalReplyAuthority;
use neko_remoted_protocol::messages::{BlockedKind, ProgramState};

struct PtyCleanup {
    state: Arc<AppState>,
    session: String,
}

impl Drop for PtyCleanup {
    fn drop(&mut self) {
        self.state.sessions.close_terminal_and_output(&self.session);
    }
}

async fn next_table(
    events: &mut mpsc::Receiver<Event>,
    replay: bool,
) -> Vec<neko_remoted_protocol::messages::ProgramRecord> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events.recv().await.expect("terminal event stream closed") {
                Event::TerminalProgramStatus { records, .. } => return records,
                Event::TerminalOutput { replay: actual, .. } => assert_eq!(actual, replay),
                event => panic!("unexpected terminal event: {event:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_pty_attach_replays_then_replaces_and_live_clears_status() {
    let state = Arc::new(AppState::new_for_test(":memory:".into()));
    let record = crate::state::SessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        host: "local-test".into(),
        selector: "test@owner".into(),
        status: "running".into(),
        cols: 80,
        rows: 24,
        command: "/bin/sh".into(),
        args: vec!["-c".into(), r"stty -echo; printf '\033]7501;state=working:app=codex:id=agent\007'; read first; printf '\033]7501;state=clear\007'; read last".into()],
        metadata: std::collections::HashMap::from([("sessionBackend".into(), "ssh".into())]),
    };
    state
        .sessions
        .write()
        .unwrap()
        .insert(record.id.clone(), record.clone());
    let _cleanup = PtyCleanup {
        state: state.clone(),
        session: record.id.clone(),
    };
    let terminal = state
        .sessions
        .open_terminal(record.terminal_spec(80, 24), true, false)
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if terminal
                .program_status_since(None)
                .is_some_and(|(_, records)| !records.is_empty())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let grant = Grant {
        id: "grant".into(),
        session: record.id.clone(),
        selector: record.selector.clone(),
        login: "user".into(),
        backend: "ssh".into(),
        name: "terminal".into(),
        target_signature: String::new(),
        root: None,
        root_identity: None,
    };
    let (events, mut incoming) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let _backend = attach_managed(&state, &grant, "s".into(), events, &cancel).unwrap();
    let records = next_table(&mut incoming, true).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].app.as_deref(), Some("codex"));
    assert_eq!(records[0].state, ProgramState::Working);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(600), incoming.recv())
            .await
            .is_err()
    );
    terminal.write_input(b"continue\n".to_vec()).unwrap();
    assert!(next_table(&mut incoming, false).await.is_empty());
    cancel.cancel();
    let (events, mut incoming) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let _backend = attach_managed(&state, &grant, "s2".into(), events, &cancel).unwrap();
    assert!(next_table(&mut incoming, true).await.is_empty());
    cancel.cancel();
}

#[tokio::test]
async fn replay_precedes_current_table_without_reobserving_or_replying_to_probes() {
    let authority = TerminalReplyAuthority::new(80, 24).unwrap();
    let bytes = b"\x1b]7501;?\x07\x1b]7501;state=blocked:kind=permission:progress=42:app=codex:title=5rWL6K+V:msg=QXBwcm92ZQ==:id=agent/sub\x07";
    let mut replies = Vec::new();
    let replay = authority
        .process_output(
            bytes.to_vec(),
            |reply| {
                replies.extend(reply);
                Ok(())
            },
            |output| output,
        )
        .unwrap();
    assert_eq!(replies, b"\x1b]7501;?\x07");
    let (events, mut incoming) = mpsc::channel(8);
    let mut output = OutputStream::new("s".into(), events, CancellationToken::new());
    output.write(&[], true, true, false).await.unwrap();
    output.write(&replay, true, false, false).await.unwrap();
    let mut revision = None;
    output
        .program_status(authority.program_status_since(revision), &mut revision)
        .await
        .unwrap();
    assert!(matches!(
        incoming.recv().await,
        Some(Event::TerminalOutput {
            replay: true,
            reset: true,
            ..
        })
    ));
    assert!(matches!(
        incoming.recv().await,
        Some(Event::TerminalOutput {
            replay: true,
            reset: false,
            ..
        })
    ));
    let Some(Event::TerminalProgramStatus { stream, records }) = incoming.recv().await else {
        panic!("expected replacement after replay");
    };
    assert_eq!(stream, "s");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, "agent/sub");
    assert_eq!(record.app.as_deref(), Some("codex"));
    assert_eq!(record.state, ProgramState::Blocked);
    assert_eq!(record.kind, Some(BlockedKind::Permission));
    assert_eq!(record.progress, Some(42));
    assert_eq!(record.title.as_deref(), Some("测试"));
    assert_eq!(record.msg.as_deref(), Some("Approve"));
    output
        .program_status(authority.program_status_since(revision), &mut revision)
        .await
        .unwrap();
    assert!(incoming.try_recv().is_err());
    assert_eq!(replies, b"\x1b]7501;?\x07");
}

#[tokio::test]
async fn live_only_sends_changes_and_explicit_empty_replacements() {
    let authority = TerminalReplyAuthority::new(80, 24).unwrap();
    let (events, mut incoming) = mpsc::channel(8);
    let output = OutputStream::new("s".into(), events, CancellationToken::new());
    let mut revision = None;
    output
        .program_status(authority.program_status_since(revision), &mut revision)
        .await
        .unwrap();
    assert!(
        matches!(incoming.recv().await, Some(Event::TerminalProgramStatus { records, .. }) if records.is_empty())
    );
    for (bytes, expected) in [
        (
            b"\x1b]7501;state=working:app=claude:id=agent\x07".as_slice(),
            Some(ProgramState::Working),
        ),
        (
            b"\x1b]7501;state=working:app=claude:id=agent\x07".as_slice(),
            None,
        ),
        (
            b"\x1b]7501;state=done:app=claude:id=agent\x07".as_slice(),
            Some(ProgramState::Done),
        ),
    ] {
        authority
            .process_output(bytes.to_vec(), |_| Ok(()), |_| ())
            .unwrap();
        output
            .program_status(authority.program_status_since(revision), &mut revision)
            .await
            .unwrap();
        if let Some(expected) = expected {
            let Some(Event::TerminalProgramStatus { records, .. }) = incoming.recv().await else {
                panic!("expected changed table");
            };
            assert_eq!(records[0].state, expected);
            assert_eq!(records[0].app.as_deref(), Some("claude"));
        } else {
            assert!(incoming.try_recv().is_err());
        }
    }
    authority
        .process_output(b"\x1b]7501;state=clear\x07".to_vec(), |_| Ok(()), |_| ())
        .unwrap();
    output
        .program_status(authority.program_status_since(revision), &mut revision)
        .await
        .unwrap();
    assert!(
        matches!(incoming.recv().await, Some(Event::TerminalProgramStatus { records, .. }) if records.is_empty())
    );
    let mut reattached_revision = None;
    output
        .program_status(
            authority.program_status_since(reattached_revision),
            &mut reattached_revision,
        )
        .await
        .unwrap();
    assert!(
        matches!(incoming.recv().await, Some(Event::TerminalProgramStatus { records, .. }) if records.is_empty())
    );
}

#[tokio::test]
async fn cancelled_or_closed_status_delivery_does_not_advance_revision() {
    for cancelled in [false, true] {
        let (events, incoming) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        if cancelled {
            cancel.cancel();
        }
        drop(incoming);
        let output = OutputStream::new("s".into(), events, cancel);
        let mut revision = None;
        assert!(
            output
                .program_status(Some((7, Vec::new())), &mut revision)
                .await
                .is_err()
        );
        assert_eq!(revision, None);
    }
}
