use super::*;

struct Pty {
    registry: TerminalRegistry,
    id: String,
}

impl Drop for Pty {
    fn drop(&mut self) {
        if let Ok(terminal) = self.registry.terminal(&self.id)
            && let Ok(mut killer) = terminal.killer.lock()
        {
            let _ = killer.kill();
        }
    }
}

impl Pty {
    async fn new(script: &str) -> Self {
        let registry = TerminalRegistry::default();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", script]);
        let id = registry
            .create_with_command("resize regression".into(), 80, 24, command)
            .await
            .unwrap();
        Self { registry, id }
    }

    async fn attach(&self, owner: &str, read_only: bool) -> (Attachment, mpsc::Receiver<Event>) {
        let (events, incoming) = mpsc::channel(128);
        let attachment = self
            .registry
            .attach(&self.id, owner.into(), events, read_only)
            .await
            .unwrap();
        (attachment, incoming)
    }
}

async fn line(incoming: &mut mpsc::Receiver<Event>, expected: &str) {
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match incoming.recv().await.expect("PTY event stream closed") {
                Event::TerminalOutput { data, .. } => {
                    bytes.extend(protocol::decode(&data, CHUNK).unwrap());
                    if String::from_utf8_lossy(&bytes)
                        .lines()
                        .any(|line| line.trim_end_matches('\r') == expected)
                    {
                        return;
                    }
                }
                Event::Error { message, .. } => panic!("PTY failed: {message}"),
                Event::TerminalExit { code, .. } => panic!("PTY exited {code} before {expected}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "missing {expected}; output={:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
}

async fn exit(incoming: &mut mpsc::Receiver<Event>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match incoming.recv().await.expect("PTY event stream closed") {
                Event::TerminalExit { code, .. } => {
                    assert_eq!(code, 7);
                    return;
                }
                Event::Error { message, .. } => panic!("PTY failed: {message}"),
                _ => {}
            }
        }
    })
    .await
    .expect("PTY exit not delivered");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_in_band_resize_reports_once_without_output_or_replay() {
    let initial = replies::with(80, 24, |vt| vt.observe(b"\x1b[?2048h")).unwrap();
    let resized = replies::with(80, 24, |vt| {
        vt.observe(b"\x1b[?2048h")?;
        vt.resize(40, 12)
    })
    .unwrap();
    let resized_hex = resized
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let script = format!(
        r#"
stty raw -echo min 0 time 10
printf '\033[?2048h'
dd bs=1 count={initial_len} >/dev/null 2>&1
printf '\nIDLE_RESIZE_READY\n'
received=$(dd bs=1 count={resized_len} 2>/dev/null | od -An -tx1 | tr -d '[:space:]')
[ "$received" = '{resized_hex}' ] || exit 91
printf '\nIDLE_RESIZE_RECEIVED\n'
stty min 1 time 0
[ "$(dd bs=1 count=1 2>/dev/null)" = x ] || exit 92
stty min 0 time 3
extra=$(dd bs=1 count=1 2>/dev/null)
[ -z "$extra" ] || exit 93
printf '\nNO_DUPLICATE_RESIZE_OR_REPLAY_REPLY\n'
exit 7
"#,
        initial_len = initial.len(),
        resized_len = resized.len(),
    );
    let pty = Pty::new(&script).await;
    let (first, mut incoming) = pty.attach("first", false).await;
    line(&mut incoming, "IDLE_RESIZE_READY").await;
    pty.registry.control(&first.stream, "first", true).unwrap();
    pty.registry
        .resize(&first.stream, "first", 40, 12)
        .await
        .unwrap();
    line(&mut incoming, "IDLE_RESIZE_RECEIVED").await;
    let (observer, _replay) = pty.attach("observer", true).await;
    pty.registry
        .resize(&first.stream, "first", 40, 12)
        .await
        .unwrap();
    pty.registry.detach(&first.stream, "first").unwrap();
    pty.registry.detach(&observer.stream, "observer").unwrap();
    let (reattached, mut incoming) = pty.attach("reattached", false).await;
    pty.registry
        .control(&reattached.stream, "reattached", true)
        .unwrap();
    pty.registry
        .input(&reattached.stream, "reattached", b"x".to_vec())
        .await
        .unwrap();
    line(&mut incoming, "NO_DUPLICATE_RESIZE_OR_REPLAY_REPLY").await;
    exit(&mut incoming).await;
    pty.registry.close(&pty.id).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_create_is_rejected_before_spawning_a_child() {
    let registry = TerminalRegistry::default();
    let invalid = registry
        .create_with_command(
            "oversized".into(),
            400,
            400,
            CommandBuilder::new("/nonexistent-neoko-resize-regression-command"),
        )
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("bounded screen capacity"));
    let state = registry.inner.state.lock().unwrap();
    assert!(state.terminals.is_empty());
    assert_eq!(state.creating, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_resize_is_rejected_before_changing_the_pty() {
    let pty = Pty::new(
        r#"
stty raw -echo min 1 time 0
printf '\nSIZE_LIMIT_READY\n'
[ "$(dd bs=1 count=1 2>/dev/null)" = a ] || exit 91
stty min 0 time 10
printf '\033[18t'
received=$(dd bs=1 count=10 2>/dev/null | od -An -tx1 | tr -d '[:space:]')
[ "$received" = 1b5b383b32343b383074 ] || exit 92
printf '\nREJECTED_RESIZE_PRESERVED_GEOMETRY\n'
stty min 1 time 0
[ "$(dd bs=1 count=1 2>/dev/null)" = b ] || exit 93
stty min 0 time 10
printf '\033[18t'
received=$(dd bs=1 count=10 2>/dev/null | od -An -tx1 | tr -d '[:space:]')
[ "$received" = 1b5b383b31323b343074 ] || exit 94
printf '\nVALID_RESIZE_STILL_WORKS\n'
exit 7
"#,
    )
    .await;
    let (attachment, mut incoming) = pty.attach("controller", false).await;
    line(&mut incoming, "SIZE_LIMIT_READY").await;
    pty.registry
        .control(&attachment.stream, "controller", true)
        .unwrap();
    let invalid = pty
        .registry
        .resize(&attachment.stream, "controller", 400, 400)
        .await
        .unwrap_err();
    assert!(invalid.to_string().contains("bounded screen capacity"));
    let terminal = pty.registry.terminal(&pty.id).unwrap();
    assert_eq!(terminal.shared.state.lock().unwrap().dimensions, (80, 24));
    pty.registry
        .input(&attachment.stream, "controller", b"a".to_vec())
        .await
        .unwrap();
    line(&mut incoming, "REJECTED_RESIZE_PRESERVED_GEOMETRY").await;
    pty.registry
        .resize(&attachment.stream, "controller", 40, 12)
        .await
        .unwrap();
    pty.registry
        .input(&attachment.stream, "controller", b"b".to_vec())
        .await
        .unwrap();
    line(&mut incoming, "VALID_RESIZE_STILL_WORKS").await;
    exit(&mut incoming).await;
    pty.registry.close(&pty.id).await.unwrap();
}
