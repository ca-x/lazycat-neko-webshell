use super::*;

#[cfg(unix)]
async fn mock_visible_screen(
    read: Arc<Mutex<Value>>,
) -> (Scratch, Arc<Herdr>, tokio::task::JoinHandle<()>) {
    let scratch = Scratch::new();
    let socket = scratch.0.join("screen.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let herdr = Arc::new(Herdr::with_program(
        scratch.script(&json!({"running":true,"socket":socket})),
    ));
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = if request["method"] == "ping" {
                json!({"protocol":22,"version":"test"})
            } else {
                assert_eq!(request["method"], "pane.read");
                assert_eq!(
                    request["params"],
                    json!({"pane_id":"w1:p1","source":"visible","format":"ansi","strip_ansi":false})
                );
                json!({"read":read.lock().unwrap().clone()})
            };
            stream
                .write_all(format!("{}\n", json!({"id":request["id"],"result":result})).as_bytes())
                .await
                .unwrap();
        }
    });
    herdr.refresh().await.unwrap();
    (scratch, herdr, server)
}

#[cfg(unix)]
#[tokio::test]
async fn visible_ansi_screens_preserve_color_only_changes_and_empty_resets_without_history_gaps() {
    let read = Arc::new(Mutex::new(
        json!({"text":"\x1b[31mRED\x1b[0m","source":"visible","format":"ansi","truncated":false}),
    ));
    let (_scratch, herdr, server) = mock_visible_screen(read.clone()).await;
    let endpoint = herdr.endpoint().unwrap();
    let (events, mut incoming) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let polling = tokio::spawn(async move {
        herdr
            .screen(&endpoint, "w1:p1", "screen", &events, &token)
            .await
    });
    let initial = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(initial, Event::TerminalOutput {data,reset:true,gap:false,..} if data == protocol::encode(b"\x1b[31mRED\x1b[0m"))
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), incoming.recv())
            .await
            .is_err()
    );
    read.lock().unwrap()["text"] = json!("\x1b[32mRED\x1b[0m");
    let changed = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(changed, Event::TerminalOutput {data,reset:true,gap:false,..} if data == protocol::encode(b"\x1b[32mRED\x1b[0m"))
    );
    read.lock().unwrap()["text"] = json!("");
    let empty = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(empty, Event::TerminalOutput {data,reset:true,gap:false,..} if data.is_empty())
    );
    cancel.cancel();
    polling.await.unwrap().unwrap();
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn incomplete_visible_screens_fail_without_publishing_a_sliced_replacement() {
    let read = Arc::new(Mutex::new(json!({"text":"partial","truncated":true})));
    let (_scratch, herdr, server) = mock_visible_screen(read.clone()).await;
    let (events, mut incoming) = mpsc::channel(8);
    for value in [
        json!({"text":"partial","truncated":true}),
        json!({"text":"x".repeat(SCREEN_LIMIT+1),"truncated":false}),
    ] {
        *read.lock().unwrap() = value;
        let error = herdr
            .screen(
                &herdr.endpoint().unwrap(),
                "w1:p1",
                "screen",
                &events,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("incomplete or exceeds"));
        assert!(incoming.try_recv().is_err());
    }
    server.abort();
}
#[test]
fn contract_denies_handoff_streaming_unknown_and_preserves_future_protocol_compatibility() {
    assert!(allowed("session.snapshot"));
    assert!(allowed("pane.send_input"));
    for method in [
        "server.live_handoff",
        "events.subscribe",
        "unknown",
        "pane.unknown",
        " pane.read",
    ] {
        assert!(!allowed(method), "{method}");
    }
    assert_eq!(MIN_PROTOCOL, 14);
}
#[tokio::test]
async fn missing_installation_is_not_ready_and_start_never_installs() {
    let herdr = Herdr::default();
    herdr.refresh().await.unwrap();
    assert_eq!(herdr.snapshot().status, HerdrStatus::NotInstalled);
    assert!(herdr.resource().is_none());
    assert!(herdr.start().await.is_err());
    assert_eq!(herdr.snapshot().status, HerdrStatus::NotInstalled);
    assert!(herdr.snapshot().install_command.contains("herdr.dev"));
    assert!(
        herdr
            .request("server.live_handoff", json!({}))
            .await
            .is_err()
    );
}

#[cfg(unix)]
struct Scratch(PathBuf);
#[cfg(unix)]
impl Scratch {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("neko-host-herdr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn script(&self, status: &Value) -> PathBuf {
        let file = self.0.join("herdr");
        let status = status.to_string().replace('\'', "'\\''");
        crate::tests::write_test_executable(
            &file,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'herdr test\\n'; else printf '%s\\n' '{status}'; fi\n"
            ),
        );
        file
    }
}
#[cfg(unix)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn installed_stopped_and_detection_error_are_distinct() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("herdr.sock");
    let script = scratch.script(&json!({"running":false,"socket":socket}));
    let herdr = Herdr::with_program(script.clone());
    herdr.refresh().await.unwrap();
    assert_eq!(herdr.snapshot().status, HerdrStatus::Stopped);
    assert!(herdr.snapshot().version.is_some());
    assert!(herdr.resource().is_none());
    crate::tests::write_test_executable(&script, "#!/bin/sh\nprintf 'broken status'\n");
    assert!(herdr.refresh().await.is_err());
    assert_eq!(herdr.snapshot().status, HerdrStatus::Error);
    assert!(!herdr.snapshot().error.is_empty());
    assert!(herdr.resource().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn ready_higher_protocol_is_usable_and_socket_replacement_is_rejected() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("herdr.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let script = scratch.script(&json!({"running":true,"socket":socket}));
    let replies = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = if request["method"] == "ping" {
                json!({"protocol":999,"version":"future-compatible"})
            } else {
                json!({"snapshot":{"panes":[]}})
            };
            stream
                .write_all(
                    format!("{}\n", json!({"id":"neoko-terminal-host","result":result})).as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let herdr = Herdr::with_program(script);
    herdr.refresh().await.unwrap();
    assert_eq!(herdr.snapshot().status, HerdrStatus::Ready);
    assert_eq!(herdr.snapshot().protocol, Some(999));
    let resource = herdr.resource().unwrap();
    assert!(resource.herdr && !resource.terminal && !resource.files);
    herdr.request("session.snapshot", json!({})).await.unwrap();
    replies.await.unwrap();
    std::fs::remove_file(&socket).unwrap();
    let _replacement = tokio::net::UnixListener::bind(&socket).unwrap();
    assert!(herdr.request("session.snapshot", json!({})).await.is_err());
    assert_eq!(herdr.snapshot().status, HerdrStatus::Error);
    assert!(herdr.resource().is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_socket_and_old_protocol_fail_closed() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("real.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let symlink = scratch.0.join("herdr.sock");
    std::os::unix::fs::symlink(&socket, &symlink).unwrap();
    let herdr = Herdr::with_program(scratch.script(&json!({"running":true,"socket":symlink})));
    assert!(herdr.refresh().await.is_err());
    assert!(herdr.resource().is_none());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream)
            .read_line(&mut line)
            .await
            .unwrap();
        stream.write_all(b"{\"id\":\"neoko-terminal-host\",\"result\":{\"protocol\":13,\"version\":\"old\"}}\n").await.unwrap();
    });
    let herdr = Herdr::with_program(scratch.script(&json!({"running":true,"socket":socket})));
    assert!(herdr.refresh().await.is_err());
    assert_eq!(herdr.snapshot().status, HerdrStatus::Error);
    assert!(herdr.resource().is_none());
    server.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn runtime_replacement_never_retargets_an_existing_input_or_screen() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("herdr.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let herdr = Herdr::with_program(scratch.script(&json!({"running":true,"socket":socket})));
    let reply = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream)
            .read_line(&mut line)
            .await
            .unwrap();
        stream.write_all(b"{\"id\":\"neoko-terminal-host\",\"result\":{\"protocol\":22,\"version\":\"test\"}}\n").await.unwrap();
    });
    herdr.refresh().await.unwrap();
    reply.await.unwrap();
    let old = herdr.endpoint().unwrap();
    let (events, _incoming) = mpsc::channel(8);
    herdr.attachments.lock().unwrap().insert(
        "old-stream".into(),
        Attachment {
            endpoint: old.clone(),
            pane: "pane".into(),
            subscriber: "local".into(),
            read_only: false,
            controller: true,
            events: events.clone(),
            cancel: CancellationToken::new(),
        },
    );
    // Simulate discovery publishing another runtime before cancellation is observed.
    herdr
        .observed
        .lock()
        .unwrap()
        .endpoint
        .as_mut()
        .unwrap()
        .resource = "new-runtime".into();
    assert!(
        herdr
            .input("old-stream", "local", b"echo must-not-run\r".to_vec())
            .await
            .is_err()
    );
    assert!(
        herdr
            .screen(
                &old,
                "pane",
                "old-stream",
                &events,
                &CancellationToken::new()
            )
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn background_transport_failure_withdraws_watched_resource_without_a_request() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("herdr.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let save: crate::Save = Arc::new(|_| Box::pin(async { Ok(()) }));
    let mut core = crate::state::Core::start(None, save).await.unwrap();
    Arc::get_mut(&mut core).unwrap().herdr = Arc::new(Herdr::with_program(
        scratch.script(&json!({"running":true,"socket":socket})),
    ));
    let replies = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let result = if request["method"] == "ping" {
                json!({"protocol":22,"version":"test"})
            } else {
                json!({"snapshot":{"panes":[{"pane_id":"pane"}]}})
            };
            stream
                .write_all(
                    format!("{}\n", json!({"id":"neoko-terminal-host","result":result})).as_bytes(),
                )
                .await
                .unwrap();
        }
        // Listener closes here; the background pane.read must withdraw the ready resource.
    });
    core.herdr.refresh().await.unwrap();
    {
        let state = core.state.lock().await;
        core.publish(&state).unwrap();
    }
    let mut snapshots = core.snapshots.subscribe();
    assert!(
        snapshots
            .borrow_and_update()
            .resources
            .iter()
            .any(|r| r.herdr)
    );
    let watcher = tokio::spawn(crate::watch_herdr(core.clone()));
    let (events, _incoming) = mpsc::channel(8);
    core.herdr
        .attach("pane".into(), "local".into(), events, false)
        .await
        .unwrap();
    replies.await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), snapshots.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshots.borrow().herdr.status, HerdrStatus::Error);
    assert!(!snapshots.borrow().resources.iter().any(|r| r.herdr));
    core.shutdown.cancel();
    watcher.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn mutation_written_without_reply_is_reported_outcome_unknown() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("herdr.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let save: crate::Save = Arc::new(|_| Box::pin(async { Ok(()) }));
    let mut core = crate::state::Core::start(None, save).await.unwrap();
    Arc::get_mut(&mut core).unwrap().herdr = Arc::new(Herdr::with_program(
        scratch.script(&json!({"running":true,"socket":socket})),
    ));
    let server = tokio::spawn(async move {
        for i in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            if i == 0 {
                stream.write_all(b"{\"id\":\"neoko-terminal-host\",\"result\":{\"protocol\":22,\"version\":\"test\"}}\n").await.unwrap();
            } else {
                assert_eq!(request["method"], "workspace.create");
                // The mutation reached the authority. Lose the reply: retry could duplicate it.
            }
        }
    });
    core.herdr.refresh().await.unwrap();
    let resource = core.herdr.resource().unwrap();
    let host = crate::Host(Arc::new(crate::Owner { core }));
    let mut local = host.local().await.unwrap();
    let id = local
        .request(protocol::messages::Operation::Herdr {
            grant: resource.id,
            method: "workspace.create".into(),
            params: json!({"name":"unknown outcome"}),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let response = local.receive().await.unwrap();
            if response.id == id {
                assert!(matches!(
                    response.event,
                    Event::Error {
                        outcome_unknown: true, code, ..
                    } if code == "REQUEST_FAILED"
                ));
                break;
            }
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_local_creation_returns_the_real_herdr_resource_and_root_pane() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("create.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let script = scratch.script(&json!({"running":true,"socket":socket}));
    let replies = tokio::spawn(async move {
        for expected in ["ping", "workspace.create", "session.snapshot"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], expected);
            let result = match expected {
                "ping" => json!({"protocol":22,"version":"test"}),
                "workspace.create" => {
                    assert_eq!(request["params"]["focus"], false);
                    assert_eq!(request["params"]["label"], "new local workspace");
                    json!({"workspace":{"workspace_id":"w1"},"tab":{"tab_id":"w1:t1"},"root_pane":{"pane_id":"w1:p1"}})
                }
                _ => json!({"panes":[{"pane_id":"w1:p1"}]}),
            };
            stream
                .write_all(
                    format!("{}\n", json!({"id":"neoko-terminal-host","result":result})).as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let herdr = Herdr::with_program(script);
    herdr.start().await.unwrap();
    let created = herdr.create_session("new local workspace").await.unwrap();
    assert_eq!(created.grant, herdr.resource().unwrap().id);
    assert_eq!(created.pane.as_deref(), Some("w1:p1"));
    assert_eq!(herdr.resource().unwrap().backend, "herdr");
    replies.await.unwrap();
}
