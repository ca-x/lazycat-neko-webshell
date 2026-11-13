#[test]
fn saved_intents_round_trip_and_reject_cross_core_labels() {
    use super::*;
    let mut saved = SavedHc {
        core_id: Some("core-test".into()),
        ..SavedHc::default()
    };
    let record = Record {
        grant: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        address: Some("t1".into()),
    };
    saved.terminals.push(record.clone());
    saved.validate().unwrap();
    let terminal = serde_json::json!({"address":"t1","labels":saved.labels(&record)});
    assert_eq!(saved.verify(&record, &terminal).unwrap(), "t1");
    let mut wrong = terminal.clone();
    wrong["labels"]["neoko_host"] = "different".into();
    assert!(saved.verify(&record, &wrong).is_err());
    wrong = terminal;
    wrong["address"] = "other/t1".into();
    assert!(saved.verify(&record, &wrong).is_err());
}

#[cfg(unix)]
mod unix {
    use super::super::*;
    use crate::{Action, Host, Operation, Owner, Save, SessionBackend, state::Core};
    use neko_remoted_protocol::{
        self as protocol,
        messages::{Event, ProgramState, Request},
    };
    use std::{sync::Arc, time::Duration};

    struct Fixture {
        root: PathBuf,
        program: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("neko-hc-fixture-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let program = root.join("hc");
            crate::tests::write_test_executable(&program, include_str!("fixture.py"));
            Self { root, program }
        }
        fn mode(&self, mode: &str) {
            std::fs::write(self.root.join("store/mode"), mode).unwrap();
        }
        fn log(&self) -> Vec<Value> {
            std::fs::read_to_string(self.root.join("store/requests.jsonl"))
                .unwrap_or_default()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
        fn runs(&self) -> usize {
            self.log()
                .iter()
                .filter(|v| v["method"] == "terminal.run")
                .count()
        }
        async fn host(&self, saved: Option<&str>) -> Host {
            let root = self.root.join("store");
            std::fs::create_dir_all(&root).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let target = root.join("host-saved.json");
            let save: Save = Arc::new(move |value| {
                let target = target.clone();
                Box::pin(async move {
                    std::fs::write(target, value)?;
                    Ok(())
                })
            });
            let mut core = Core::start(saved, save).await.unwrap();
            let saved = core.state.lock().await.saved.hc.clone();
            let hc = Hc::with_program(
                saved,
                core.shutdown.child_token(),
                self.program.clone(),
                root,
            );
            Arc::get_mut(&mut core).unwrap().hc = Arc::new(hc);
            Host(Arc::new(Owner { core }))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(text) = std::fs::read_to_string(self.root.join("store/host-saved.json"))
                && let Ok(saved) = serde_json::from_str::<Value>(&text)
                && let Some(namespace) = saved["hc"]["namespace"]
                    .as_str()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
            {
                let _ = std::fs::remove_dir_all(format!("/tmp/nkhc-{}", namespace.simple()));
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    async fn response(session: &mut crate::LocalSession, id: u64) -> Event {
        loop {
            let response = tokio::time::timeout(Duration::from_secs(4), session.receive())
                .await
                .unwrap()
                .unwrap();
            if response.id == id {
                return response.event;
            }
        }
    }
    async fn attach(host: &Host, grant: &str) -> (crate::LocalSession, String) {
        let mut local = host.local().await.unwrap();
        let stream = attach_to(&mut local, grant).await;
        (local, stream)
    }
    async fn attach_to(local: &mut crate::LocalSession, grant: &str) -> String {
        let request = local
            .request(Operation::TerminalAttach {
                grant: grant.into(),
                pane: None,
            })
            .await
            .unwrap();
        match response(local, request).await {
            Event::TerminalAttached {
                stream,
                read_only: false,
                ..
            } => stream,
            other => panic!("unexpected attachment response: {other:?}"),
        }
    }
    #[tokio::test]
    async fn explicit_create_framed_control_status_detach_and_confirmed_close() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        assert!(fixture.log().is_empty(), "starting a host must not run hc");
        host.manage(Action::RefreshHc {
            revision: host.snapshot().revision,
        })
        .await
        .unwrap();
        assert!(fixture.log().is_empty(), "refresh must not connect");
        assert_eq!(host.snapshot().hc.status, HcStatus::Stopped);
        let created = host
            .create_session(SessionBackend::Hc, "persistent fixture".into())
            .await
            .unwrap();
        assert!(created.pane.is_none());
        assert_eq!(host.snapshot().hc.status, HcStatus::Ready);
        assert!(
            host.snapshot()
                .resources
                .iter()
                .any(|r| r.id == created.grant && r.backend == "hc")
        );
        let (mut local, stream) = attach(&host, &created.grant).await;
        let mut statuses = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), local.receive())
                .await
                .unwrap()
                .unwrap()
                .event;
            if let Event::TerminalProgramStatus { records, .. } = event {
                if let Some(record) = records.first() {
                    statuses.push(record.state);
                }
                if statuses.last() == Some(&ProgramState::Done) {
                    break;
                }
            }
        }
        let request = local
            .request(Operation::TerminalInput {
                stream: stream.clone(),
                data: protocol::encode(b"typed"),
            })
            .await
            .unwrap();
        assert!(
            matches!(response(&mut local, request).await, Event::Ok),
            "a local HC terminal accepts input immediately after attach"
        );
        let request = local
            .request(Operation::TerminalControl {
                stream: stream.clone(),
                take: true,
            })
            .await
            .unwrap();
        assert!(matches!(
            response(&mut local, request).await,
            Event::TerminalControl {
                controller: true,
                ..
            }
        ));
        let request = local
            .request(Operation::TerminalResize {
                stream: stream.clone(),
                cols: 120,
                rows: 40,
            })
            .await
            .unwrap();
        assert!(matches!(response(&mut local, request).await, Event::Ok));
        let request = local
            .request(Operation::TerminalInput {
                stream: stream.clone(),
                data: protocol::encode(b"typed"),
            })
            .await
            .unwrap();
        assert!(matches!(response(&mut local, request).await, Event::Ok));
        let request = local
            .request(Operation::TerminalDetach { stream })
            .await
            .unwrap();
        assert!(matches!(response(&mut local, request).await, Event::Ok));
        assert_eq!(fixture.runs(), 1);
        let (_local, _stream) = attach(&host, &created.grant).await;
        host.close_terminal(&created.grant).await.unwrap();
        assert!(
            !host
                .snapshot()
                .resources
                .iter()
                .any(|r| r.id == created.grant)
        );
        assert_eq!(fixture.runs(), 1);
        assert!(
            !fixture
                .log()
                .iter()
                .any(|v| v["kind"] == 0 && v["data"] != json!([116, 121, 112, 101, 100])),
            "the native renderer must never answer the replayed VT query"
        );
    }
    #[tokio::test]
    async fn ambiguous_run_is_durable_and_restart_recovers_without_a_second_run() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        fixture.mode("run_lost");
        assert!(
            host.create_session(SessionBackend::Hc, "ambiguous".into())
                .await
                .is_err()
        );
        assert_eq!(fixture.runs(), 1);
        let saved = std::fs::read_to_string(fixture.root.join("store/host-saved.json")).unwrap();
        let parsed: Value = serde_json::from_str(&saved).unwrap();
        assert!(parsed["hc"]["terminals"][0]["address"].is_null());
        let grant = parsed["hc"]["terminals"][0]["grant"]
            .as_str()
            .unwrap()
            .to_owned();
        drop(host);
        fixture.mode("");
        let restored = fixture.host(Some(&saved)).await;
        assert_eq!(fixture.runs(), 1);
        assert!(
            restored.snapshot().resources.is_empty(),
            "restoring must not connect automatically"
        );
        restored
            .manage(Action::ConnectHc {
                revision: restored.snapshot().revision,
            })
            .await
            .unwrap();
        assert_eq!(fixture.runs(), 1);
        assert!(restored.snapshot().resources.iter().any(|r| r.id == grant));
        let (_local, _) = attach(&restored, &grant).await;
        restored.close_terminal(&grant).await.unwrap();
    }
    #[tokio::test]
    async fn core_identity_change_refuses_attach_create_and_kill() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "identity".into())
            .await
            .unwrap();
        let path = fixture.root.join("store/fixture.json");
        let mut state: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        state["core_id"] = "replaced-core".into();
        std::fs::write(path, state.to_string()).unwrap();
        let mut local = host.local().await.unwrap();
        let request = local
            .request(Operation::TerminalAttach {
                grant: created.grant.clone(),
                pane: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            response(&mut local, request).await,
            Event::Error { .. }
        ));
        assert!(
            host.create_session(SessionBackend::Hc, "must not run".into())
                .await
                .is_err()
        );
        assert!(host.close_terminal(&created.grant).await.is_err());
        assert_eq!(fixture.runs(), 1);
        assert!(!fixture.log().iter().any(|v| v["method"] == "terminal.kill"));
    }
    #[tokio::test]
    async fn read_only_remote_grant_cannot_control_resize_or_input_and_drop_detaches() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "read only".into())
            .await
            .unwrap();
        let device = crate::state::Device {
            id: uuid::Uuid::new_v4().to_string(),
            name: "observer".into(),
            key: protocol::encode(protocol::crypto::Identity::generate().unwrap().public()),
            permission: crate::Permission::ReadOnly,
            resources: vec![created.grant.clone()],
            generation: 1,
        };
        host.0.core.state.lock().await.saved.enabled = true;
        host.0
            .core
            .state
            .lock()
            .await
            .saved
            .devices
            .push(device.clone());
        let (requests, rx) = mpsc::channel(8);
        let (events, mut responses) = mpsc::channel(32);
        let cancel = host.0.core.shutdown.child_token();
        let subscriber = uuid::Uuid::new_v4().to_string();
        let task = tokio::spawn(crate::session::serve(
            host.0.core.clone(),
            crate::session::Principal::Device {
                id: device.id.clone(),
                generation: 1,
            },
            subscriber,
            rx,
            events,
            cancel.clone(),
        ));
        assert!(matches!(
            responses.recv().await.unwrap().event,
            Event::Ready { .. }
        ));
        requests
            .send(Request {
                id: 1,
                operation: Operation::TerminalAttach {
                    grant: created.grant.clone(),
                    pane: None,
                },
            })
            .await
            .unwrap();
        let Event::TerminalAttached {
            stream,
            read_only: true,
            ..
        } = responses.recv().await.unwrap().event
        else {
            panic!("observer attach failed")
        };
        let operations = [
            Operation::TerminalControl {
                stream: stream.clone(),
                take: true,
            },
            Operation::TerminalResize {
                stream: stream.clone(),
                cols: 90,
                rows: 30,
            },
            Operation::TerminalInput {
                stream: stream.clone(),
                data: protocol::encode(b"not authorized"),
            },
        ];
        for (index, operation) in operations.into_iter().enumerate() {
            let id = index as u64 + 2;
            requests.send(Request { id, operation }).await.unwrap();
            loop {
                let response = tokio::time::timeout(Duration::from_secs(4), responses.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if response.id == id {
                    assert!(matches!(response.event, Event::Error { .. }));
                    break;
                }
            }
        }
        // Revoke the generation, then request again: no queued output may cross it.
        host.0.core.state.lock().await.saved.devices[0].generation += 1;
        requests
            .send(Request {
                id: 5,
                operation: Operation::Resources {},
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(4), task)
            .await
            .unwrap()
            .unwrap();
        assert!(host.0.core.hc.attachments.lock().unwrap().is_empty());
        assert_eq!(fixture.runs(), 1);
        assert!(
            !fixture
                .log()
                .iter()
                .any(|v| matches!(v["kind"].as_u64(), Some(0 | 3 | 4)))
        );
    }
    #[tokio::test]
    async fn missing_hc_reports_install_guidance_without_spawning() {
        let (save, _) = crate::tests::store();
        let host = crate::start(None, save).await.unwrap();
        host.manage(Action::RefreshHc {
            revision: host.snapshot().revision,
        })
        .await
        .unwrap();
        assert_eq!(host.snapshot().hc.status, HcStatus::NotInstalled);
        assert!(
            host.create_session(SessionBackend::Hc, "missing".into())
                .await
                .is_err()
        );
        assert_eq!(host.snapshot().hc.status, HcStatus::NotInstalled);
        assert!(!host.snapshot().hc.error.is_empty());
        assert!(host.snapshot().hc.install_url.starts_with("https://"));
    }
    #[tokio::test]
    async fn recoverable_close_and_slow_consumer_never_report_successful_exit() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "recoverable".into())
            .await
            .unwrap();
        fixture.mode("holder_lost");
        let (mut local, stream) = attach(&host, &created.grant).await;
        let request = local
            .request(Operation::TerminalControl {
                stream: stream.clone(),
                take: true,
            })
            .await
            .unwrap();
        let _ = response(&mut local, request).await;
        loop {
            let response = tokio::time::timeout(Duration::from_secs(4), local.receive())
                .await
                .unwrap()
                .unwrap();
            match response.event {
                Event::TerminalExit { .. } => {
                    panic!("holder loss must not become a successful exit")
                }
                Event::TerminalUnavailable {
                    stream: ended,
                    message,
                } => {
                    assert_eq!(ended, stream);
                    assert!(message.contains("holder_lost"));
                    break;
                }
                _ => {}
            }
        }
        fixture.mode("burst");
        // Exercise a consumer that actually stops draining. LocalSession now
        // propagates backpressure and its bounded response queue absorbs part
        // of the burst before this attachment-level delivery budget applies.
        let (events, _slow) = tokio::sync::mpsc::channel(1);
        host.0
            .core
            .hc
            .attach(&created.grant, "slow fixture".into(), events, false)
            .await
            .unwrap();
        // An undrained receiver exhausts the three-second output drain and
        // then the separate three-second final notification deadline.
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if host.0.core.hc.attachments.lock().unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            fixture.runs(),
            1,
            "slow consumers must not replace the core terminal"
        );
        fixture.mode("");
        host.close_terminal(&created.grant).await.unwrap();
    }

    #[tokio::test]
    async fn failed_intent_save_prevents_terminal_run() {
        let fixture = Fixture::new();
        let mut host = fixture.host(None).await;
        // Replace only the protected-store callback before any worker owns Core.
        let owner = Arc::get_mut(&mut host.0).unwrap();
        let core = Arc::get_mut(&mut owner.core).unwrap();
        let original = core.save.clone();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        core.save = Arc::new(move |value| {
            let index = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let original = original.clone();
            Box::pin(async move {
                if index == 1 {
                    anyhow::bail!("intent store failed");
                }
                original(value).await
            })
        });
        assert!(
            host.create_session(SessionBackend::Hc, "not sent".into())
                .await
                .is_err()
        );
        assert_eq!(fixture.runs(), 0);
        assert!(host.0.core.state.lock().await.saved.hc.terminals.is_empty());
    }

    #[tokio::test]
    async fn hc_stream_failures_preserve_sibling_native_pty_and_do_not_recreate() {
        if crate::tests::controlled_shell_child(
            "hc::tests::unix::hc_stream_failures_preserve_sibling_native_pty_and_do_not_recreate",
        ) {
            return;
        }
        for mode in ["holder_lost", "unknown_exit", "worker_lost"] {
            let fixture = Fixture::new();
            let host = fixture.host(None).await;
            let native = host
                .create_session(SessionBackend::Native, "sibling PTY".into())
                .await
                .unwrap();
            let created = host
                .create_session(SessionBackend::Hc, mode.into())
                .await
                .unwrap();
            let (mut local, native_stream) = attach(&host, &native.grant).await;
            let hc_stream = attach_to(&mut local, &created.grant).await;
            let id = local
                .request(Operation::TerminalControl {
                    stream: native_stream.clone(),
                    take: true,
                })
                .await
                .unwrap();
            assert!(matches!(
                response(&mut local, id).await,
                Event::TerminalControl {
                    controller: true,
                    ..
                }
            ));
            fixture.mode(mode);
            let id = local
                .request(Operation::TerminalControl {
                    stream: hc_stream.clone(),
                    take: true,
                })
                .await
                .unwrap();
            assert!(matches!(
                response(&mut local, id).await,
                Event::TerminalControl {
                    controller: true,
                    ..
                }
            ));
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match local.receive().await.unwrap().event {
                        Event::TerminalUnavailable { stream, message } => {
                            assert_eq!(stream, hc_stream);
                            let expected = match mode {
                                "unknown_exit" => "no numeric exit status",
                                "holder_lost" => "holder_lost",
                                _ => "hc EOF",
                            };
                            assert!(message.contains(expected), "{mode}: {message}");
                            break;
                        }
                        Event::TerminalExit { .. } => panic!("{mode}: exit code was fabricated"),
                        Event::Error { code, message, .. } => {
                            panic!("{mode}: unscoped {code}: {message}")
                        }
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            // The retired ID cannot receive input or be detached a second time.
            for operation in [
                Operation::TerminalInput {
                    stream: hc_stream.clone(),
                    data: protocol::encode(b"never replay"),
                },
                Operation::TerminalDetach { stream: hc_stream },
            ] {
                let id = local.request(operation).await.unwrap();
                assert!(matches!(
                    response(&mut local, id).await,
                    Event::Error { .. }
                ));
            }
            let id = local
                .request(Operation::TerminalInput {
                    stream: native_stream.clone(),
                    data: protocol::encode(b"printf 'SIBLING-%s\\n' still-alive\n"),
                })
                .await
                .unwrap();
            assert!(matches!(response(&mut local, id).await, Event::Ok));
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut output = Vec::new();
                while !String::from_utf8_lossy(&output).contains("SIBLING-still-alive") {
                    match local.receive().await.unwrap().event {
                        Event::TerminalOutput { stream, data, .. } => {
                            assert_eq!(stream, native_stream);
                            output
                                .extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                        }
                        Event::TerminalUnavailable { message, .. }
                        | Event::Error { message, .. } => panic!("sibling PTY failed: {message}"),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            fixture.mode("");
            let _reattached = attach_to(&mut local, &created.grant).await;
            assert_eq!(
                fixture.runs(),
                1,
                "reattach must reuse the saved hc terminal"
            );
            host.close_terminal(&created.grant).await.unwrap();
            host.close_terminal(&native.grant).await.unwrap();
        }
    }

    #[tokio::test]
    async fn worker_failure_is_delivered_after_a_temporarily_full_event_queue() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "full queue".into())
            .await
            .unwrap();
        fixture.mode("attach_lost");
        let (events, mut incoming) = mpsc::channel(1);
        events
            .send(Event::Pong {
                nonce: "queue occupied".into(),
            })
            .await
            .unwrap();
        let (stream, _) = host
            .0
            .core
            .hc
            .attach(&created.grant, "slow fixture".into(), events, false)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !fixture
                .log()
                .iter()
                .any(|entry| entry["disconnected"] == true)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // Keep the event queue full while the worker observes the closed pipe.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(matches!(incoming.recv().await, Some(Event::Pong { .. })));
        let mut output_seen = false;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match incoming.recv().await.unwrap() {
                    Event::TerminalOutput { .. } => output_seen = true,
                    Event::TerminalUnavailable {
                        stream: ended,
                        message,
                    } => {
                        assert_eq!(ended, stream);
                        assert!(message.contains("hc EOF"), "{message}");
                        assert!(output_seen, "queued output must precede stream retirement");
                        break;
                    }
                    Event::TerminalExit { .. } | Event::Error { .. } => {
                        panic!("worker failure escaped its stream")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(fixture.runs(), 1);
        fixture.mode("");
        host.close_terminal(&created.grant).await.unwrap();
    }

    #[tokio::test]
    async fn blocked_delivery_reports_incomplete_output_and_retires_only_its_attachment() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "incomplete output".into())
            .await
            .unwrap();
        fixture.mode("burst");
        let (events, mut incoming) = mpsc::channel(1);
        events
            .send(Event::Pong {
                nonce: "queue occupied".into(),
            })
            .await
            .unwrap();
        let (stream, _) = host
            .0
            .core
            .hc
            .attach(&created.grant, "blocked fixture".into(), events, false)
            .await
            .unwrap();
        // The bounded delivery drain expires while the receiver stays blocked.
        tokio::time::sleep(Duration::from_millis(3300)).await;
        assert!(matches!(incoming.recv().await, Some(Event::Pong { .. })));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match incoming.recv().await.unwrap() {
                    Event::TerminalUnavailable {
                        stream: ended,
                        message,
                    } => {
                        assert_eq!(ended, stream);
                        assert!(
                            message.contains("queued output may be incomplete"),
                            "{message}"
                        );
                        break;
                    }
                    Event::TerminalExit { .. } | Event::Error { .. } => {
                        panic!("a blocked consumer is not a process exit")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while host.0.core.hc.is_stream(&stream, "blocked fixture") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            host.snapshot()
                .resources
                .iter()
                .any(|resource| resource.id == created.grant)
        );
        assert_eq!(fixture.runs(), 1);
        fixture.mode("");
        host.close_terminal(&created.grant).await.unwrap();
    }

    #[tokio::test]
    async fn explicit_close_without_exit_code_survives_a_full_event_queue() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "unknown close".into())
            .await
            .unwrap();
        let (events, mut incoming) = mpsc::channel(1);
        events
            .send(Event::Pong {
                nonce: "queue occupied".into(),
            })
            .await
            .unwrap();
        let (stream, _) = host
            .0
            .core
            .hc
            .attach(&created.grant, "closing fixture".into(), events, false)
            .await
            .unwrap();
        fixture.mode("kill_unknown");
        host.close_terminal(&created.grant).await.unwrap();
        assert!(
            !host
                .snapshot()
                .resources
                .iter()
                .any(|resource| resource.id == created.grant)
        );
        assert!(matches!(incoming.recv().await, Some(Event::Pong { .. })));
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match incoming.recv().await.unwrap() {
                    Event::TerminalUnavailable {
                        stream: ended,
                        message,
                    } => {
                        assert_eq!(ended, stream);
                        assert!(message.contains("no numeric exit status"));
                        break;
                    }
                    Event::TerminalExit { .. } | Event::Error { .. } => {
                        panic!("close must not fabricate a successful exit")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn oversized_status_table_cannot_disconnect_the_workspace() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "large status".into())
            .await
            .unwrap();
        fixture.mode("oversized_status");
        let (mut local, stream) = attach(&host, &created.grant).await;
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match local.receive().await.unwrap().event {
                    Event::TerminalUnavailable {
                        stream: ended,
                        message,
                    } => {
                        assert_eq!(ended, stream);
                        assert!(!message.is_empty());
                        break;
                    }
                    Event::Error { .. } | Event::TerminalExit { .. } => {
                        panic!("large metadata escaped the hc stream")
                    }
                    Event::TerminalControl {
                        controller: true, ..
                    } => {
                        panic!("failed initial status must never claim control")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(
            host.0
                .core
                .hc
                .attachments
                .lock()
                .unwrap()
                .get(&stream)
                .is_none_or(|item| !item.controller || item.cancel.is_cancelled()),
            "failed attachment retained control"
        );
        let id = local
            .request(Operation::Ping {
                nonce: "still connected".into(),
            })
            .await
            .unwrap();
        assert!(matches!(response(&mut local, id).await, Event::Pong { .. }));
        fixture.mode("");
        host.close_terminal(&created.grant).await.unwrap();
    }

    #[tokio::test]
    async fn old_saved_hosts_gain_a_durable_hc_namespace_without_changing_identity() {
        let (save, saved) = crate::tests::store();
        let host = crate::start(None, save.clone()).await.unwrap();
        let mut legacy: Value =
            serde_json::from_str(saved.lock().unwrap().as_ref().unwrap()).unwrap();
        legacy.as_object_mut().unwrap().remove("hc");
        let restored = crate::start(Some(&legacy.to_string()), save.clone())
            .await
            .unwrap();
        assert_eq!(
            host.0.core.identity.public(),
            restored.0.core.identity.public()
        );
        assert_eq!(host.0.core.route, restored.0.core.route);
        let namespace = restored
            .0
            .core
            .state
            .lock()
            .await
            .saved
            .hc
            .namespace
            .clone();
        let serialized = saved.lock().unwrap().clone().unwrap();
        let restarted = crate::start(Some(&serialized), save).await.unwrap();
        assert_eq!(
            restarted.0.core.state.lock().await.saved.hc.namespace,
            namespace
        );
        assert_eq!(restarted.snapshot().hc.status, HcStatus::Stopped);
        assert!(restarted.snapshot().resources.is_empty());
    }

    #[tokio::test]
    async fn unavailable_after_withdrawal_is_scoped_to_the_authenticated_session() {
        let fixture = Fixture::new();
        let host = fixture.host(None).await;
        let created = host
            .create_session(SessionBackend::Hc, "scoped".into())
            .await
            .unwrap();
        let (mut local, stream) = attach(&host, &created.grant).await;
        let mut ready_status = false;
        let mut ready_control = false;
        loop {
            match local.receive().await.unwrap().event {
                Event::TerminalProgramStatus { records, .. } => {
                    ready_status |= records
                        .iter()
                        .any(|record| record.state == ProgramState::Done);
                }
                Event::TerminalControl {
                    stream: controlled,
                    controller: true,
                } => {
                    assert_eq!(controlled, stream);
                    ready_control = true;
                }
                _ => {}
            }
            if ready_status && ready_control {
                break;
            }
        }
        let (_other, foreign) = attach(&host, &created.grant).await;
        let events = host.0.core.hc.attachments.lock().unwrap()[&stream]
            .events
            .clone();
        let mut withdrawn = host.0.core.state.lock().await.saved.hc.clone();
        withdrawn.terminals.clear();
        host.0.core.hc.committed(&withdrawn);
        events
            .send(Event::TerminalUnavailable {
                stream: foreign,
                message: "must not cross sessions".into(),
            })
            .await
            .unwrap();
        events
            .send(Event::TerminalOutput {
                stream: stream.clone(),
                sequence: 999,
                data: protocol::encode(b"must not cross withdrawal"),
                replay: false,
                reset: false,
                gap: false,
            })
            .await
            .unwrap();
        events
            .send(Event::TerminalUnavailable {
                stream: stream.clone(),
                message: "owned stream retired".into(),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match local.receive().await.unwrap().event {
                    Event::TerminalUnavailable {
                        stream: ended,
                        message,
                    } => {
                        assert_eq!(ended, stream, "foreign stream crossed the session boundary");
                        assert_eq!(message, "owned stream retired");
                        break;
                    }
                    Event::TerminalOutput { sequence: 999, .. } => {
                        panic!("output crossed resource withdrawal")
                    }
                    // Initial output/status may already be queued before resource withdrawal.
                    Event::TerminalOutput { .. } | Event::TerminalProgramStatus { .. } => {}
                    Event::TerminalControl {
                        stream: controlled,
                        controller: false,
                    } => {
                        assert_eq!(
                            controlled, stream,
                            "foreign control crossed the session boundary"
                        );
                    }
                    other => panic!("unexpected event after withdrawal: {other:?}"),
                }
            }
        })
        .await
        .unwrap();
        events
            .send(Event::TerminalUnavailable {
                stream,
                message: "duplicate must be dropped".into(),
            })
            .await
            .unwrap();
        let id = local
            .request(Operation::Ping {
                nonce: "still connected".into(),
            })
            .await
            .unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(4), local.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply.id, id);
        assert!(matches!(reply.event, Event::Pong { .. }));
    }

    struct RealCoreCleanup {
        binary: PathBuf,
        root: PathBuf,
        namespace: String,
    }
    impl Drop for RealCoreCleanup {
        fn drop(&mut self) {
            let socket = format!(
                "/tmp/nkhc-{}/core.sock",
                uuid::Uuid::parse_str(&self.namespace).unwrap().simple()
            );
            // The CLI is fenced to this test's private endpoint and state, even on panic.
            let mut child = std::process::Command::new(&self.binary)
                .args(["server", "stop"])
                .env("HC_SOCKET", socket)
                .env("HC_SESSION", "neoko-terminal")
                .env("XDG_STATE_HOME", &self.root)
                .env("XDG_CONFIG_HOME", self.root.join("config"))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while child.try_wait().unwrap().is_none() {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    #[tokio::test]
    #[ignore = "requires explicit NEKO_TEST_HC binary; uses a private endpoint, HOME and shell"]
    async fn actual_isolated_hc_pty_status_resize_restart_attach_and_exit() {
        if crate::tests::controlled_shell_child(
            "hc::tests::unix::actual_isolated_hc_pty_status_resize_restart_attach_and_exit",
        ) {
            return;
        }
        let binary = std::env::var_os("NEKO_TEST_HC")
            .map(PathBuf::from)
            .expect("set NEKO_TEST_HC to an absolute installed hc binary");
        assert!(binary.is_absolute());
        let mut fixture = Fixture::new();
        fixture.program = binary.clone();
        let host = fixture.host(None).await;
        let namespace = host.0.core.state.lock().await.saved.hc.namespace.clone();
        let _cleanup = RealCoreCleanup {
            binary,
            root: fixture.root.join("store"),
            namespace,
        };
        let created = host
            .create_session(SessionBackend::Hc, "real isolated hc".into())
            .await
            .unwrap();
        let (mut local, stream) = attach(&host, &created.grant).await;
        let request = local
            .request(Operation::TerminalControl {
                stream: stream.clone(),
                take: true,
            })
            .await
            .unwrap();
        assert!(matches!(
            response(&mut local, request).await,
            Event::TerminalControl {
                controller: true,
                ..
            }
        ));
        let request = local
            .request(Operation::TerminalResize {
                stream: stream.clone(),
                cols: 111,
                rows: 37,
            })
            .await
            .unwrap();
        assert!(matches!(response(&mut local, request).await, Event::Ok));
        let request = local.request(Operation::TerminalInput { stream, data: protocol::encode(b"printf '\\033]7501;state=working:app=fixture\\007'; printf 'REAL-HC-PTY\\n'; stty size\n") }).await.unwrap();
        assert!(matches!(response(&mut local, request).await, Event::Ok));
        let mut text = Vec::new();
        let mut status = false;
        tokio::time::timeout(Duration::from_secs(8), async {
            while !status || !String::from_utf8_lossy(&text).contains("37 111") {
                match local.receive().await.unwrap().event {
                    Event::TerminalOutput { data, .. } => {
                        text.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap())
                    }
                    Event::TerminalProgramStatus { records, .. } => {
                        status |= records.iter().any(|r| {
                            r.state == ProgramState::Working && r.app.as_deref() == Some("fixture")
                        })
                    }
                    Event::Error { code, message, .. } => panic!("{code}: {message}"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        let saved = std::fs::read_to_string(fixture.root.join("store/host-saved.json")).unwrap();
        drop(local);
        drop(host);
        let restored = fixture.host(Some(&saved)).await;
        restored
            .manage(Action::ConnectHc {
                revision: restored.snapshot().revision,
            })
            .await
            .unwrap();
        assert_eq!(
            restored
                .snapshot()
                .resources
                .iter()
                .filter(|r| r.backend == "hc")
                .count(),
            1
        );
        let (mut local, stream) = attach(&restored, &created.grant).await;
        let request = local
            .request(Operation::TerminalControl {
                stream: stream.clone(),
                take: true,
            })
            .await
            .unwrap();
        assert!(matches!(
            response(&mut local, request).await,
            Event::TerminalControl {
                controller: true,
                ..
            }
        ));
        let request = local
            .request(Operation::TerminalInput {
                stream,
                data: protocol::encode(b"exit 7\n"),
            })
            .await
            .unwrap();
        assert!(matches!(response(&mut local, request).await, Event::Ok));
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match local.receive().await.unwrap().event {
                    Event::TerminalExit { code, .. } => {
                        assert_eq!(code, 7);
                        break;
                    }
                    Event::Error { code, message, .. } => panic!("{code}: {message}"),
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        // A finished real core record must preserve the same numeric code on reattach.
        let (mut finished, finished_stream) = attach(&restored, &created.grant).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match finished.receive().await.unwrap().event {
                    Event::TerminalExit { stream, code } => {
                        assert_eq!(stream, finished_stream);
                        assert_eq!(code, 7);
                        break;
                    }
                    Event::TerminalUnavailable { message, .. } | Event::Error { message, .. } => {
                        panic!("finished reattach lost its real code: {message}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        restored.close_terminal(&created.grant).await.unwrap();
        // Exercise an explicit kill of a still-running core-owned PTY as well.
        let running = restored
            .create_session(SessionBackend::Hc, "real explicit close".into())
            .await
            .unwrap();
        let stream = attach_to(&mut local, &running.grant).await;
        restored.close_terminal(&running.grant).await.unwrap();
        assert!(
            !restored
                .snapshot()
                .resources
                .iter()
                .any(|r| r.id == running.grant)
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match local.receive().await.unwrap().event {
                    Event::TerminalExit { stream: ended, .. }
                    | Event::TerminalUnavailable { stream: ended, .. } => {
                        assert_eq!(ended, stream);
                        break;
                    }
                    Event::Error { message, .. } => {
                        panic!("explicit close escaped its stream: {message}")
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        let id = local
            .request(Operation::TerminalInput {
                stream,
                data: protocol::encode(b"never after close"),
            })
            .await
            .unwrap();
        assert!(matches!(
            response(&mut local, id).await,
            Event::Error { .. }
        ));
    }
}
