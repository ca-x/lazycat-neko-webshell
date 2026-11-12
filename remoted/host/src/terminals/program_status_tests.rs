use super::*;
use protocol::messages::{BlockedKind, ProgramState};

fn subscriber(state: &mut State, stream: &str) -> mpsc::Receiver<Event> {
    let (events, incoming) = mpsc::channel(32);
    let (stop, _) = watch::channel(Stop::Running);
    state.attachments.insert(
        stream.into(),
        Subscriber {
            owner: "viewer".into(),
            read_only: true,
            events,
            stop,
            sequence: 0,
        },
    );
    incoming
}

#[test]
fn split_probe_replies_before_fanout_and_changed_tables_preserve_metadata() {
    let mut state = State::default();
    let mut first = subscriber(&mut state, "first");
    let mut second = subscriber(&mut state, "second");
    accept_output(&mut state, b"\x1b]7501;", |_| {
        panic!("incomplete probe replied")
    })
    .unwrap();
    assert!(matches!(first.try_recv(), Ok(Event::TerminalOutput { .. })));
    second.try_recv().unwrap();
    let mut replies = Vec::new();
    let bytes = b"?\x07\x1b]7501;state=blocked:kind=permission:progress=42:app=codex:title=5rWL6K+V:msg=QXBwcm92ZQ==:id=agent/sub\x07";
    accept_output(&mut state, bytes, |reply| {
        assert!(first.try_recv().is_err());
        assert!(second.try_recv().is_err());
        replies.push(reply);
        Ok(())
    })
    .unwrap();
    assert_eq!(replies, [b"\x1b]7501;?\x07".to_vec()]);
    for (stream, incoming) in [("first", &mut first), ("second", &mut second)] {
        assert!(matches!(
            incoming.try_recv(),
            Ok(Event::TerminalOutput { replay: false, .. })
        ));
        let Event::TerminalProgramStatus {
            stream: actual,
            records,
        } = incoming.try_recv().unwrap()
        else {
            panic!("expected replacement");
        };
        assert_eq!(actual, stream);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.id, "agent/sub");
        assert_eq!(record.app.as_deref(), Some("codex"));
        assert_eq!(record.state, ProgramState::Blocked);
        assert_eq!(record.kind, Some(BlockedKind::Permission));
        assert_eq!(record.progress, Some(42));
        assert_eq!(record.title.as_deref(), Some("测试"));
        assert_eq!(record.msg.as_deref(), Some("Approve"));
    }
    accept_output(&mut state, &bytes[2..], |_| {
        panic!("report generated reply")
    })
    .unwrap();
    publish_program_status(&mut state);
    assert!(matches!(first.try_recv(), Ok(Event::TerminalOutput { .. })));
    assert!(first.try_recv().is_err());
    accept_output(&mut state, b"\x1b]7501;state=clear\x07", |_| Ok(())).unwrap();
    first.try_recv().unwrap();
    assert!(
        matches!(first.try_recv(), Ok(Event::TerminalProgramStatus { records, .. }) if records.is_empty())
    );
    publish_program_status(&mut state);
    assert!(first.try_recv().is_err());
}

#[test]
fn reply_queue_failure_prevents_output_publication() {
    let mut state = State::default();
    let mut events = subscriber(&mut state, "s");
    assert!(
        accept_output(&mut state, b"\x1b]7501;?\x07", |_| bail!(
            "reply queue full"
        ))
        .is_err()
    );
    assert!(events.try_recv().is_err());
    assert!(state.replay.is_empty());
}

#[cfg(unix)]
struct Fixture {
    registry: TerminalRegistry,
    id: String,
}

#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(terminal) = self.registry.terminal(&self.id)
            && let Ok(mut killer) = terminal.killer.lock()
        {
            let _ = killer.kill();
        }
    }
}

#[cfg(unix)]
async fn table(
    events: &mut mpsc::Receiver<Event>,
    expected_replay: bool,
) -> Vec<protocol::messages::ProgramRecord> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap() {
                Event::TerminalOutput { replay, .. } => assert_eq!(replay, expected_replay),
                Event::TerminalProgramStatus { records, .. } => return records,
                Event::TerminalControl { .. } => {}
                event => panic!("unexpected event: {event:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_probe_once_replay_replacement_clear_and_no_unchanged_tables() {
    let registry = TerminalRegistry::default();
    let script = r#"
stty raw -echo min 0 time 3
printf '\033]7501;'
sleep 0.05
printf '?\007\033]7501;state=working:app=codex:id=agent\007'
reply=$(dd bs=1 count=9 2>/dev/null)
[ "$reply" = "$(printf '\033]7501;?\007')" ] || exit 91
extra=$(dd bs=1 count=1 2>/dev/null)
[ -z "$extra" ] || exit 92
stty min 1 time 0
printf '\nreply-once\n'
next=$(dd bs=1 count=1 2>/dev/null)
[ "$next" = x ] || exit 93
printf '\033]7501;state=clear\007'
dd bs=1 count=1 >/dev/null 2>&1
"#;
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", script]);
    let id = registry
        .create_with_command("status fixture".into(), 80, 24, command)
        .await
        .unwrap();
    let fixture = Fixture { registry, id };
    let (events, mut initial) = mpsc::channel(128);
    let attachment = fixture
        .registry
        .attach(&fixture.id, "first".into(), events, true)
        .await
        .unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match initial.recv().await.unwrap() {
                Event::TerminalOutput { data, .. } => {
                    bytes.extend(protocol::decode(&data, CHUNK).unwrap());
                    if bytes
                        .windows(b"reply-once".len())
                        .any(|window| window == b"reply-once")
                    {
                        break;
                    }
                }
                Event::TerminalProgramStatus { .. } => {}
                event => panic!("probe fixture failed: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
    fixture
        .registry
        .detach(&attachment.stream, "first")
        .unwrap();
    let (events, mut incoming) = mpsc::channel(128);
    let attachment = fixture
        .registry
        .attach(&fixture.id, "second".into(), events, false)
        .await
        .unwrap();
    let records = table(&mut incoming, true).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].app.as_deref(), Some("codex"));
    assert_eq!(records[0].state, ProgramState::Working);
    assert!(
        tokio::time::timeout(Duration::from_millis(650), incoming.recv())
            .await
            .is_err()
    );
    fixture
        .registry
        .control(&attachment.stream, "second", true)
        .unwrap();
    fixture
        .registry
        .input(&attachment.stream, "second", b"x".to_vec())
        .await
        .unwrap();
    assert!(table(&mut incoming, false).await.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(650), incoming.recv())
            .await
            .is_err()
    );
    fixture
        .registry
        .detach(&attachment.stream, "second")
        .unwrap();
    let (events, mut incoming) = mpsc::channel(128);
    fixture
        .registry
        .attach(&fixture.id, "third".into(), events, true)
        .await
        .unwrap();
    assert!(table(&mut incoming, true).await.is_empty());
    fixture.registry.close(&fixture.id).await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_foreground_detection_stays_in_its_own_real_pty() {
    let registry = TerminalRegistry::default();
    let mut agent = CommandBuilder::new("bash");
    agent.args(["--noprofile", "--norc", "-c", "exec -a codex sleep 30"]);
    let agent = Fixture {
        id: registry
            .create_with_command("agent".into(), 80, 24, agent)
            .await
            .unwrap(),
        registry: registry.clone(),
    };
    let mut other = CommandBuilder::new("/bin/sh");
    other.args(["-c", "exec sleep 30"]);
    let other = Fixture {
        id: registry
            .create_with_command("other".into(), 80, 24, other)
            .await
            .unwrap(),
        registry,
    };
    let (events, mut detected) = mpsc::channel(128);
    agent
        .registry
        .attach(&agent.id, "agent-viewer".into(), events, true)
        .await
        .unwrap();
    let records = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::TerminalProgramStatus { records, .. } = detected.recv().await.unwrap()
                && !records.is_empty()
            {
                break records;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].app.as_deref(), Some("codex"));
    let (events, mut isolated) = mpsc::channel(128);
    other
        .registry
        .attach(&other.id, "other-viewer".into(), events, true)
        .await
        .unwrap();
    assert!(table(&mut isolated, true).await.is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(650), isolated.recv())
            .await
            .is_err()
    );
    agent.registry.close(&agent.id).await.unwrap();
    other.registry.close(&other.id).await.unwrap();
}
