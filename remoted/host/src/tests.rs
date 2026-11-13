use super::*;
use neko_remoted_protocol::{self as protocol, messages::Event};
use std::sync::Mutex;
use std::time::Duration;

pub(crate) fn store() -> (Save, Arc<Mutex<Option<String>>>) {
    let saved = Arc::new(Mutex::new(None));
    let target = saved.clone();
    let save: Save = Arc::new(move |value| {
        let target = target.clone();
        Box::pin(async move {
            *target.lock().unwrap() = Some(value);
            Ok(())
        })
    });
    (save, saved)
}
#[tokio::test]
async fn identity_is_durable_disabled_and_local_has_no_fake_pairing_events() {
    let (save, saved) = store();
    let host = start(None, save.clone()).await.unwrap();
    assert!(!host.snapshot().enabled);
    assert_eq!(host.snapshot().status, "disabled");
    assert!(saved.lock().unwrap().is_some());
    let mut local = host.local().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), local.receive())
            .await
            .is_err()
    );
    let id = local.request(Operation::Resources {}).await.unwrap();
    let response = local.receive().await.unwrap();
    assert_eq!(response.id, id);
    assert!(matches!(response.event, Event::Resources { resources } if resources.is_empty()));
    host.manage(Action::Configure {
        revision: host.snapshot().revision,
        relay: String::new(),
        enabled: false,
    })
    .await
    .unwrap();
    let serialized = saved.lock().unwrap().clone().unwrap();
    let restored = start(Some(&serialized), save).await.unwrap();
    assert_eq!(
        host.0.core.identity.public(),
        restored.0.core.identity.public()
    );
    assert_eq!(host.0.core.route, restored.0.core.route);
    assert_eq!(host.snapshot().status, "disabled");
}
#[tokio::test]
async fn failing_save_blocks_start_and_configuration_and_stale_revision_is_rejected() {
    let fail: Save = Arc::new(|_| Box::pin(async { anyhow::bail!("protected store unavailable") }));
    assert!(start(None, fail).await.is_err());
    let (save, _) = store();
    let host = start(None, save).await.unwrap();
    host.manage(Action::Configure {
        revision: 0,
        relay: "https://neko-relay.czyt.eu.org".into(),
        enabled: false,
    })
    .await
    .unwrap();
    assert!(host.manage(Action::Invite { revision: 0 }).await.is_err());
    assert_eq!(host.snapshot().status, "disabled");
    assert!(!host.snapshot().enabled);
}

// Execute default-shell tests in a fresh process, without loading a user's shell
// configuration or changing environment variables in the parallel test runner.
// Write fixture executables in a short-lived child, whose writable descriptor cannot
// be inherited by other concurrently spawned PTY children (Linux ETXTBSY race).
#[cfg(unix)]
pub(crate) fn write_test_executable(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let status = std::process::Command::new("/bin/sh")
        .args(["-c", "printf '%s' \"$2\" > \"$1\"", "fixture"])
        .arg(path)
        .arg(body)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
#[cfg(unix)]
pub(crate) fn controlled_shell_child(test: &str) -> bool {
    if std::env::var("NEKO_HOST_TEST_CHILD").as_deref() == Ok(test) {
        return false;
    }
    let root = std::env::temp_dir().join(format!("neko-host-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let shell = root.join("shell");
    write_test_executable(&shell, "#!/bin/sh\nexec /bin/sh\n");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--include-ignored", "--nocapture"])
        .env("NEKO_HOST_TEST_CHILD", test)
        .env("SHELL", &shell)
        .env("HOME", &root)
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .current_dir(&root)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(
        output.status.success(),
        "isolated PTY test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;")),
        "isolated PTY test did not execute exactly one test:\n{stdout}"
    );
    true
}

#[cfg(unix)]
#[tokio::test]
async fn actual_local_pty_is_immediately_writable_and_survives_detach() {
    if controlled_shell_child("tests::actual_local_pty_is_immediately_writable_and_survives_detach")
    {
        return;
    }
    let (save, _) = store();
    let host = start(None, save).await.unwrap();
    let id = host
        .create_terminal("isolated host test".into())
        .await
        .unwrap();
    let mut local = host.local().await.unwrap();
    let attached = local
        .request(Operation::TerminalAttach {
            grant: id.clone(),
            pane: None,
        })
        .await
        .unwrap();
    let response = local.receive().await.unwrap();
    assert_eq!(response.id, attached);
    let Event::TerminalAttached {
        stream, read_only, ..
    } = response.event
    else {
        panic!("attach failed")
    };
    assert!(!read_only);
    let input = local
        .request(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"printf 'native-before-control\\n'\r"),
        })
        .await
        .unwrap();
    loop {
        let response = local.receive().await.unwrap();
        if response.id == input {
            assert!(
                matches!(response.event, Event::Ok),
                "local input must work immediately without acquiring control"
            );
            break;
        }
    }
    local
        .request(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await
        .unwrap();
    let command = b"printf '%s%s\\n' native_ pty_ok\r";
    local
        .request(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(command),
        })
        .await
        .unwrap();
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::TerminalOutput { data, .. } = local.receive().await.unwrap().event {
                output.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if output
                    .windows(b"native_pty_ok".len())
                    .any(|b| b == b"native_pty_ok")
                {
                    break;
                }
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "PTY output missing: {:?}",
        String::from_utf8_lossy(&output)
    );
    local.close().await;
    assert!(host.snapshot().resources.iter().any(|r| r.id == id));
    host.close_terminal(&id).await.unwrap();
}

struct EncryptedPeer {
    input: tokio::sync::mpsc::Sender<crate::relay::Input>,
    output: tokio::sync::mpsc::Receiver<protocol::messages::RelayFrame>,
    credits: Arc<tokio::sync::Semaphore>,
    channel: protocol::crypto::Channel,
    cancel: CancellationToken,
    next: u64,
}
impl Drop for EncryptedPeer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl EncryptedPeer {
    async fn open(
        host: &Host,
        identity: &protocol::crypto::Identity,
        mode: protocol::crypto::Mode,
        code: Option<&protocol::code::DeviceCode>,
    ) -> Self {
        use protocol::crypto::Handshake;
        let relay = protocol::RelayAddress::parse(&host.snapshot().relay).unwrap();
        let (link, mut output, credits) = crate::relay::test_link();
        let cancel = link.cancel.clone();
        let (input, incoming) = tokio::sync::mpsc::channel(8);
        let mut handshake = Handshake::new(
            identity,
            &relay,
            &host.0.core.route,
            mode,
            true,
            code.map(|c| c.psk()),
            (mode == protocol::crypto::Mode::Connect).then(|| host.0.core.identity.public()),
        )
        .unwrap();
        tokio::spawn(crate::peer::run(host.0.core.clone(), relay, link, incoming));
        input.send(crate::relay::Input::Hello(mode)).await.unwrap();
        input
            .send(crate::relay::Input::Data(handshake.write().unwrap()))
            .await
            .unwrap();
        let protocol::messages::RelayFrame::Data { data, .. } =
            tokio::time::timeout(Duration::from_secs(5), output.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("handshake frame")
        };
        credits.add_permits(1);
        handshake
            .read(&protocol::decode(&data, 256).unwrap())
            .unwrap();
        if !handshake.finished() {
            input
                .send(crate::relay::Input::Data(handshake.write().unwrap()))
                .await
                .unwrap();
        }
        let (channel, key) = handshake.into_channel().unwrap();
        assert_eq!(&key, host.0.core.identity.public());
        Self {
            input,
            output,
            credits,
            channel,
            cancel,
            next: 1,
        }
    }
    async fn send(&mut self, operation: Operation) -> u64 {
        let id = self.next;
        self.next += 1;
        let bytes = protocol::pack(&protocol::messages::Request { id, operation }).unwrap();
        self.input
            .send(crate::relay::Input::Data(
                self.channel.encrypt(&bytes).unwrap(),
            ))
            .await
            .unwrap();
        id
    }
    async fn receive(&mut self) -> Response {
        let frame = tokio::time::timeout(Duration::from_secs(5), self.output.recv())
            .await
            .unwrap()
            .expect("encrypted peer closed");
        let protocol::messages::RelayFrame::Data { data, .. } = frame else {
            panic!("business frame")
        };
        self.credits.add_permits(1);
        protocol::unpack(
            &self
                .channel
                .decrypt(&protocol::decode(&data, protocol::MAX_CIPHERTEXT).unwrap())
                .unwrap(),
        )
        .unwrap()
    }
    async fn reply(&mut self, id: u64) -> Event {
        loop {
            let response = self.receive().await;
            if let Event::Ping { nonce } = response.event {
                self.send(Operation::Pong { nonce }).await;
                continue;
            }
            if response.id == id {
                return response.event;
            }
        }
    }
}
async fn isolated_host(save: Save) -> Host {
    let core = crate::state::Core::start(None, save).await.unwrap();
    let host = Host(Arc::new(Owner { core }));
    host.manage(Action::Configure {
        revision: 0,
        relay: "https://relay.example".into(),
        enabled: true,
    })
    .await
    .unwrap();
    host
}
async fn pair_host(
    host: &Host,
    identity: &protocol::crypto::Identity,
    permission: Permission,
    resources: Vec<String>,
) -> EncryptedPeer {
    host.manage(Action::Invite {
        revision: host.snapshot().revision,
    })
    .await
    .unwrap();
    let relay = protocol::RelayAddress::parse(&host.snapshot().relay).unwrap();
    let code =
        protocol::code::DeviceCode::parse(&host.snapshot().invitation.unwrap(), &relay).unwrap();
    let mut peer =
        EncryptedPeer::open(host, identity, protocol::crypto::Mode::Pair, Some(&code)).await;
    let identify = peer
        .send(Operation::Identify {
            name: "isolated encrypted test".into(),
        })
        .await;
    assert!(
        matches!(peer.reply(identify).await,Event::Pending{fingerprint} if fingerprint==protocol::fingerprint(identity.public()))
    );
    let pending = host.snapshot().pending[0].id.clone();
    host.manage(Action::Approve {
        revision: host.snapshot().revision,
        pending,
        permission,
        resources,
    })
    .await
    .unwrap();
    assert!(matches!(peer.receive().await.event, Event::Approved { .. }));
    peer.send(Operation::AcknowledgePairing {}).await;
    assert!(matches!(peer.receive().await.event, Event::Ready { .. }));
    let Event::HostInfo { name } = peer.receive().await.event else {
        panic!("host metadata required");
    };
    assert_eq!(name, hostname::get().unwrap().to_string_lossy().trim());
    peer
}
#[tokio::test]
async fn noise_explicit_approval_reconnect_durable_grants_and_revocation() {
    let (save, saved) = store();
    let host = isolated_host(save.clone()).await;
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer = pair_host(&host, &identity, Permission::ReadOnly, vec![]).await;
    let request = peer.send(Operation::Resources {}).await;
    assert!(
        matches!(peer.reply(request).await,Event::Resources{resources} if resources.is_empty())
    );
    let device = host.snapshot().devices[0].id.clone();
    assert!(host.snapshot().devices[0].online);
    drop(peer);
    let serialized = saved.lock().unwrap().clone().unwrap();
    let restored = Host(Arc::new(Owner {
        core: crate::state::Core::start(Some(&serialized), save)
            .await
            .unwrap(),
    }));
    let mut reconnect =
        EncryptedPeer::open(&restored, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(
        reconnect.receive().await.event,
        Event::Ready {
            permission: Permission::ReadOnly,
            ..
        }
    ));
    restored
        .manage(Action::Revoke {
            revision: restored.snapshot().revision,
            device,
        })
        .await
        .unwrap();
    assert!(restored.snapshot().devices.is_empty());
    assert!(reconnect.cancel.is_cancelled());
    let mut denied =
        EncryptedPeer::open(&restored, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(denied.receive().await.event,Event::Error{code,..} if code=="revoked"));
}
#[tokio::test]
async fn failed_approval_is_not_ephemeral_trust_and_stale_approval_fails() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let fail = Arc::new(AtomicBool::new(false));
    let failure = fail.clone();
    let save: Save = Arc::new(move |_| {
        let fail = failure.clone();
        Box::pin(async move {
            ensure!(!fail.load(Ordering::SeqCst), "store locked");
            Ok(())
        })
    });
    let host = isolated_host(save).await;
    host.manage(Action::Invite {
        revision: host.snapshot().revision,
    })
    .await
    .unwrap();
    let relay = protocol::RelayAddress::parse(&host.snapshot().relay).unwrap();
    let code =
        protocol::code::DeviceCode::parse(&host.snapshot().invitation.unwrap(), &relay).unwrap();
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer =
        EncryptedPeer::open(&host, &identity, protocol::crypto::Mode::Pair, Some(&code)).await;
    let id = peer
        .send(Operation::Identify {
            name: "storage failure test".into(),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Pending { .. }));
    let revision = host.snapshot().revision;
    let pending = host.snapshot().pending[0].id.clone();
    fail.store(true, Ordering::SeqCst);
    assert!(
        host.manage(Action::Approve {
            revision,
            pending: pending.clone(),
            permission: Permission::Operate,
            resources: vec![]
        })
        .await
        .is_err()
    );
    assert!(host.snapshot().devices.is_empty());
    assert_eq!(host.snapshot().revision, revision);
    assert!(
        tokio::time::timeout(Duration::from_millis(30), peer.output.recv())
            .await
            .is_err()
    );
    fail.store(false, Ordering::SeqCst);
    assert!(
        host.manage(Action::Approve {
            revision: revision - 1,
            pending: pending.clone(),
            permission: Permission::Operate,
            resources: vec![]
        })
        .await
        .is_err()
    );
    host.manage(Action::Approve {
        revision,
        pending,
        permission: Permission::ReadOnly,
        resources: vec![],
    })
    .await
    .unwrap();
    assert!(matches!(
        peer.receive().await.event,
        Event::Approved {
            permission: Permission::ReadOnly,
            ..
        }
    ));
}
#[tokio::test]
async fn rtt_uses_worst_live_connection_and_expires_after_thirty_seconds() {
    let (save, _) = store();
    let host = isolated_host(save).await;
    let identity = protocol::crypto::Identity::generate().unwrap();
    let _peer = pair_host(&host, &identity, Permission::ReadOnly, vec![]).await;
    let mut state = host.0.core.state.lock().await;
    let id = state.saved.devices[0].id.clone();
    for (key, rtt, age) in [("one", 4, 0), ("two", 19, 0), ("stale", 99, 31)] {
        state.live.insert(
            key.into(),
            crate::state::Live {
                device: id.clone(),
                cancel: CancellationToken::new(),
                rtt: Some((rtt, std::time::Instant::now() - Duration::from_secs(age))),
            },
        );
    }
    host.0.core.publish(&state).unwrap();
    assert_eq!(host.snapshot().devices[0].rtt_ms, Some(19));
    for p in state.live.values_mut() {
        if let Some((_, at)) = &mut p.rtt {
            *at = std::time::Instant::now() - Duration::from_secs(31);
        }
    }
    host.0.core.publish(&state).unwrap();
    assert_eq!(host.snapshot().devices[0].rtt_ms, None);
}

#[tokio::test(start_paused = true)]
async fn encrypted_ping_pong_bypasses_a_blocked_business_operation_and_state_lock() {
    let (save, _) = store();
    let host = isolated_host(save).await;
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer = pair_host(&host, &identity, Permission::ReadOnly, vec![]).await;
    let guard = host.0.core.state.lock().await;
    // This business request cannot finish until the guard is released.
    let blocked = peer.send(Operation::Resources {}).await;
    let ping = peer
        .send(Operation::Ping {
            nonce: "independent-link-ping".into(),
        })
        .await;
    assert!(matches!(peer.reply(ping).await,Event::Pong{nonce} if nonce=="independent-link-ping"));
    tokio::time::advance(Duration::from_secs(10)).await;
    let Event::Ping { nonce } = peer.receive().await.event else {
        panic!("host heartbeat must bypass state lock")
    };
    let pong = peer.send(Operation::Pong { nonce }).await;
    assert!(matches!(peer.reply(pong).await, Event::Ok));
    drop(guard);
    assert!(matches!(peer.reply(blocked).await, Event::Resources { .. }));
}

#[cfg(unix)]
async fn local_reply(local: &mut LocalSession, id: u64) -> Event {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = local.receive().await.unwrap();
            if response.id == id {
                return response.event;
            }
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn noise_and_local_share_real_pty_control_and_enforce_grants() {
    if controlled_shell_child("tests::noise_and_local_share_real_pty_control_and_enforce_grants") {
        return;
    }
    let (save, _) = store();
    let host = isolated_host(save).await;
    let terminal = host
        .create_terminal("shared real PTY".into())
        .await
        .unwrap();
    let ungranted = host.create_terminal("not shared".into()).await.unwrap();
    let mut local = host.local().await.unwrap();
    let id = local
        .request(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await
        .unwrap();
    let Event::TerminalAttached {
        stream: local_stream,
        ..
    } = local_reply(&mut local, id).await
    else {
        panic!("local attach")
    };
    let id = local
        .request(Operation::TerminalControl {
            stream: local_stream.clone(),
            take: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));

    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer = pair_host(
        &host,
        &identity,
        Permission::Operate,
        vec![terminal.clone()],
    )
    .await;
    let id = peer.send(Operation::Resources {}).await;
    assert!(
        matches!(peer.reply(id).await, Event::Resources { resources } if resources.len() == 1 && resources[0].id == terminal)
    );
    let id = peer
        .send(Operation::TerminalAttach {
            grant: ungranted.clone(),
            pane: None,
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Error { .. }));
    let id = peer
        .send(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await;
    let Event::TerminalAttached {
        stream,
        read_only: false,
        ..
    } = peer.reply(id).await
    else {
        panic!("remote attach")
    };
    let id = peer
        .send(Operation::TerminalInput {
            stream: local_stream.clone(),
            data: protocol::encode(b"forbidden\r"),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Error { .. }));
    let id = peer
        .send(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"forbidden\r"),
        })
        .await;
    assert!(
        matches!(peer.reply(id).await, Event::Error { .. }),
        "attach must not take control"
    );
    let id = peer
        .send(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await;
    assert!(matches!(
        peer.reply(id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    let id = local
        .request(Operation::TerminalInput {
            stream: local_stream.clone(),
            data: protocol::encode(b"forbidden\r"),
        })
        .await
        .unwrap();
    assert!(
        matches!(local_reply(&mut local, id).await, Event::Ok),
        "local typing must automatically reclaim this terminal"
    );
    let id = peer
        .send(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"remote-must-not-type-before-taking-control\r"),
        })
        .await;
    assert!(
        matches!(peer.reply(id).await, Event::Error { .. }),
        "local reclamation must revoke remote input authority"
    );
    let id = peer
        .send(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await;
    assert!(matches!(
        peer.reply(id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));

    let id = peer
        .send(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"printf '%s%s\\n' shared_ noise_pty_ok\r"),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Ok));
    let marker = b"shared_noise_pty_ok";
    let mut remote_output = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::TerminalOutput { data, .. } = peer.receive().await.event {
                remote_output.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if remote_output
                    .windows(marker.len())
                    .any(|window| window == marker)
                {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    let mut local_output = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::TerminalOutput { data, .. } = local.receive().await.unwrap().event {
                local_output.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if local_output
                    .windows(marker.len())
                    .any(|window| window == marker)
                {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    let device = host.snapshot().devices[0].id.clone();
    host.manage(Action::SetPermission {
        revision: host.snapshot().revision,
        device: device.clone(),
        permission: Permission::ReadOnly,
        resources: vec![terminal.clone()],
    })
    .await
    .unwrap();
    assert!(peer.cancel.is_cancelled());
    let mut readonly =
        EncryptedPeer::open(&host, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(
        readonly.receive().await.event,
        Event::Ready {
            permission: Permission::ReadOnly,
            ..
        }
    ));
    let id = readonly
        .send(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await;
    let Event::TerminalAttached {
        stream,
        read_only: true,
        ..
    } = readonly.reply(id).await
    else {
        panic!("readonly attach")
    };
    for operation in [
        Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        },
        Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"forbidden\r"),
        },
        Operation::TerminalResize {
            stream,
            cols: 80,
            rows: 24,
        },
    ] {
        let id = readonly.send(operation).await;
        assert!(matches!(readonly.reply(id).await, Event::Error { .. }));
    }
    host.manage(Action::Revoke {
        revision: host.snapshot().revision,
        device,
    })
    .await
    .unwrap();
    assert!(readonly.cancel.is_cancelled());
    let id = local
        .request(Operation::TerminalControl {
            stream: local_stream,
            take: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    assert_eq!(
        host.snapshot().resources.len(),
        2,
        "permission changes must not kill local PTYs"
    );
    local.close().await;
    host.close_terminal(&terminal).await.unwrap();
    host.close_terminal(&ungranted).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn missing_device_pong_closes_the_connection_within_a_bounded_deadline() {
    let (save, _) = store();
    let host = isolated_host(save).await;
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer = pair_host(&host, &identity, Permission::ReadOnly, vec![]).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(matches!(peer.receive().await.event, Event::Ping { .. }));
    tokio::time::advance(Duration::from_secs(30)).await;
    tokio::time::timeout(Duration::from_secs(1), peer.cancel.cancelled())
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn wrong_device_pong_cannot_keep_the_connection_alive() {
    let (save, _) = store();
    let host = isolated_host(save).await;
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer = pair_host(&host, &identity, Permission::ReadOnly, vec![]).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    let Event::Ping { nonce } = peer.receive().await.event else {
        panic!("host ping required")
    };
    peer.send(Operation::Pong {
        nonce: format!("wrong-{nonce}"),
    })
    .await;
    tokio::time::timeout(Duration::from_secs(1), peer.cancel.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn failing_configuration_save_preserves_the_last_durable_state() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let failure = Arc::new(AtomicBool::new(false));
    let fail = failure.clone();
    let save: Save = Arc::new(move |_| {
        let fail = fail.clone();
        Box::pin(async move {
            ensure!(!fail.load(Ordering::SeqCst), "store unavailable");
            Ok(())
        })
    });
    let host = start(None, save).await.unwrap();
    let original = host.snapshot();
    failure.store(true, Ordering::SeqCst);
    assert!(
        host.manage(Action::Configure {
            revision: original.revision,
            relay: "https://relay.example".into(),
            enabled: true
        })
        .await
        .is_err()
    );
    let after = host.snapshot();
    assert_eq!(after.revision, original.revision);
    assert_eq!(after.relay, original.relay);
    assert!(!after.enabled);
    assert_eq!(after.status, "disabled");
}

#[cfg(unix)]
struct IsolatedHerdr {
    directory: std::path::PathBuf,
    wrapper: std::path::PathBuf,
}
#[cfg(unix)]
impl IsolatedHerdr {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "nhh-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        for sub in ["config", "runtime", "state", "data"] {
            std::fs::create_dir(directory.join(sub)).unwrap();
        }
        std::fs::write(directory.join("config.toml"), "onboarding = false\n[terminal]\ndefault_shell = \"/bin/sh\"\nshell_mode = \"non_login\"\nnew_cwd = \"current\"\n[update]\nversion_check = false\nmanifest_check = false\n").unwrap();
        let binary =
            std::env::var("NEKO_TEST_HERDR_BINARY").unwrap_or_else(|_| "/usr/sbin/herdr".into());
        let quoted_binary = binary.replace('\'', "'\\''");
        let wrapper = directory.join("herdr");
        let session = format!("nhh-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let script = format!(
            "#!/bin/sh\nunset HERDR_SOCKET HERDR_SESSION ENV BASH_ENV\nexport HERDR_CONFIG_PATH='{0}/config.toml' XDG_CONFIG_HOME='{0}/config' XDG_RUNTIME_DIR='{0}/runtime' XDG_STATE_HOME='{0}/state' XDG_DATA_HOME='{0}/data' SHELL=/bin/sh\nexec '{quoted_binary}' --session '{session}' \"$@\"\n",
            directory.display()
        );
        write_test_executable(&wrapper, &script);
        Self { directory, wrapper }
    }
}
#[cfg(unix)]
impl Drop for IsolatedHerdr {
    fn drop(&mut self) {
        use std::process::Stdio;
        if let Ok(mut child) = std::process::Command::new(&self.wrapper)
            .args(["server", "stop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Real installed Herdr; only the fixture's unique named session and temporary config are used.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires an installed Herdr TUI binary; starts only an isolated named runtime"]
async fn genuine_local_herdr_start_screen_input_and_noise_readonly() {
    let runtime = IsolatedHerdr::new();
    let (save, _) = store();
    let mut core = crate::state::Core::start(None, save).await.unwrap();
    Arc::get_mut(&mut core).unwrap().herdr =
        Arc::new(crate::herdr::Herdr::with_program(runtime.wrapper.clone()));
    let host = Host(Arc::new(Owner { core }));
    host.manage(Action::RefreshHerdr {
        revision: host.snapshot().revision,
    })
    .await
    .unwrap();
    assert_eq!(host.snapshot().herdr.status, HerdrStatus::Stopped);
    assert!(!host.snapshot().resources.iter().any(|r| r.herdr));
    host.manage(Action::StartHerdr {
        revision: host.snapshot().revision,
    })
    .await
    .unwrap();
    assert_eq!(host.snapshot().herdr.status, HerdrStatus::Ready);
    let resource = host
        .snapshot()
        .resources
        .into_iter()
        .find(|r| r.herdr)
        .unwrap();
    assert!(!resource.terminal && !resource.files);
    let mut local = host.local().await.unwrap();
    let id = local
        .request(Operation::Herdr {
            grant: resource.id.clone(),
            method: "workspace.create".into(),
            params: serde_json::json!({"name":"isolated native host test"}),
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::Herdr { .. }
    ));
    let id = local
        .request(Operation::Herdr {
            grant: resource.id.clone(),
            method: "session.snapshot".into(),
            params: serde_json::json!({}),
        })
        .await
        .unwrap();
    let Event::Herdr { data } = local_reply(&mut local, id).await else {
        panic!("Herdr snapshot")
    };
    let pane = data["result"]["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = local
        .request(Operation::Herdr {
            grant: resource.id.clone(),
            method: "workspace.create".into(),
            params: serde_json::json!({"name":"second isolated workspace"}),
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::Herdr { .. }
    ));
    let id = local
        .request(Operation::Herdr {
            grant: resource.id.clone(),
            method: "session.snapshot".into(),
            params: serde_json::json!({}),
        })
        .await
        .unwrap();
    let Event::Herdr { data } = local_reply(&mut local, id).await else {
        panic!("second snapshot")
    };
    let second = data["result"]["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| p["pane_id"].as_str().filter(|id| *id != pane))
        .unwrap()
        .to_owned();
    let id = local
        .request(Operation::TerminalAttach {
            grant: resource.id.clone(),
            pane: Some(second.clone()),
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::TerminalAttached { .. }
    ));
    let id = local
        .request(Operation::TerminalAttach {
            grant: resource.id.clone(),
            pane: Some(second),
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::Error {code, ..} if code == "REQUEST_FAILED"
    ));
    let id = local
        .request(Operation::TerminalAttach {
            grant: resource.id.clone(),
            pane: Some(pane.clone()),
        })
        .await
        .unwrap();
    let Event::TerminalAttached { stream, .. } = local_reply(&mut local, id).await else {
        panic!("Herdr attach")
    };
    let id = local
        .request(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    let id = local
        .request(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"printf '%s%s\\n' genuine_herdr_ host_ok\r"),
        })
        .await
        .unwrap();
    assert!(matches!(local_reply(&mut local, id).await, Event::Ok));
    let mut output = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::TerminalOutput { data, reset, .. } = local.receive().await.unwrap().event
            {
                if reset {
                    output.clear();
                }
                output.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if output
                    .windows(b"genuine_herdr_host_ok".len())
                    .any(|w| w == b"genuine_herdr_host_ok")
                {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    let id = local
        .request(Operation::TerminalResize {
            stream,
            cols: 100,
            rows: 30,
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::Error { .. }
    ));
    host.manage(Action::Configure {
        revision: host.snapshot().revision,
        relay: "https://relay.example".into(),
        enabled: true,
    })
    .await
    .unwrap();
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut observer = pair_host(
        &host,
        &identity,
        Permission::ReadOnly,
        vec![resource.id.clone()],
    )
    .await;
    let id = observer
        .send(Operation::Herdr {
            grant: resource.id.clone(),
            method: "session.snapshot".into(),
            params: serde_json::json!({}),
        })
        .await;
    assert!(matches!(observer.reply(id).await, Event::Herdr { .. }));
    let id = observer
        .send(Operation::TerminalAttach {
            grant: resource.id.clone(),
            pane: Some(pane.clone()),
        })
        .await;
    let Event::TerminalAttached {
        stream,
        read_only: true,
        ..
    } = observer.reply(id).await
    else {
        panic!("readonly Herdr attach")
    };
    for operation in [
        Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        },
        Operation::TerminalInput {
            stream,
            data: protocol::encode(b"forbidden\r"),
        },
        Operation::Herdr {
            grant: resource.id.clone(),
            method: "pane.send_text".into(),
            params: serde_json::json!({"pane_id":pane,"text":"forbidden"}),
        },
        Operation::Herdr {
            grant: resource.id,
            method: "server.live_handoff".into(),
            params: serde_json::json!({}),
        },
    ] {
        let id = observer.send(operation).await;
        assert!(matches!(observer.reply(id).await, Event::Error { .. }));
    }
    println!(
        "PASS genuine isolated Herdr {:?}: explicit start, two different panes attach, duplicate pane REQUEST_FAILED, local control/input/replacement screen; real Noise readonly denial; no Relay required for local",
        host.snapshot().herdr
    );
    drop(observer);
    local.close().await;
    host.0
        .core
        .herdr
        .request("server.stop", serde_json::json!({}))
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn closing_a_resource_removes_durable_grants_without_granting_replacement_pty() {
    if controlled_shell_child(
        "tests::closing_a_resource_removes_durable_grants_without_granting_replacement_pty",
    ) {
        return;
    }
    let (save, saved) = store();
    let host = isolated_host(save).await;
    let terminal = host
        .create_terminal("closing shared PTY".into())
        .await
        .unwrap();
    let identity = protocol::crypto::Identity::generate().unwrap();
    let peer = pair_host(
        &host,
        &identity,
        Permission::ReadOnly,
        vec![terminal.clone()],
    )
    .await;
    host.close_terminal(&terminal).await.unwrap();
    assert!(peer.cancel.is_cancelled());
    assert!(host.snapshot().devices[0].resources.is_empty());
    let persisted: crate::state::Saved =
        serde_json::from_str(saved.lock().unwrap().as_ref().unwrap()).unwrap();
    assert!(persisted.devices[0].resources.is_empty());
    let replacement = host
        .create_terminal("replacement unshared PTY".into())
        .await
        .unwrap();
    assert_ne!(replacement, terminal);
    let mut reconnect =
        EncryptedPeer::open(&host, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(
        reconnect.receive().await.event,
        Event::Ready { .. }
    ));
    let id = reconnect.send(Operation::Resources {}).await;
    assert!(
        matches!(reconnect.reply(id).await, Event::Resources { resources } if resources.is_empty())
    );
    let id = reconnect
        .send(Operation::TerminalAttach {
            grant: replacement.clone(),
            pane: None,
        })
        .await;
    assert!(matches!(reconnect.reply(id).await, Event::Error { .. }));
    host.close_terminal(&replacement).await.unwrap();
}

// An independent protocol client used only by the explicit public-v3 acceptance test.
// It uses real TLS/WebSocket/Noise and acknowledges each exact inbound token after decoding.
#[cfg(unix)]
type PublicSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
#[cfg(unix)]
struct PublicPeer {
    socket: public_path::WireSocket,
    channel: protocol::crypto::Channel,
    next: u64,
}
#[cfg(unix)]
async fn public_send(socket: &mut PublicSocket, frame: protocol::messages::RelayFrame) {
    use futures_util::SinkExt;
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&frame).unwrap().into(),
        )),
    )
    .await
    .expect("public Relay send timeout")
    .expect("public Relay send failed");
}
#[cfg(unix)]
async fn public_frame(socket: &mut PublicSocket) -> protocol::messages::RelayFrame {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match socket
                .next()
                .await
                .expect("public Relay closed")
                .expect("public Relay read failed")
            {
                Message::Text(text) => {
                    return serde_json::from_str(&text).expect("public Relay frame");
                }
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                Message::Pong(_) => {}
                _ => panic!("public Relay closed or invalid frame"),
            }
        }
    })
    .await
    .expect("public Relay receive timeout")
}
#[cfg(unix)]
#[path = "tests/public_path.rs"]
mod public_path;
#[cfg(unix)]
impl PublicPeer {
    async fn open(
        host: &Host,
        identity: &protocol::crypto::Identity,
        mode: protocol::crypto::Mode,
        code: Option<&protocol::code::DeviceCode>,
    ) -> Self {
        use protocol::{crypto::Handshake, messages::RelayFrame};
        use tokio_tungstenite::{Connector, tungstenite::client::IntoClientRequest};
        let relay = protocol::RelayAddress::parse(&host.snapshot().relay).unwrap();
        let mut request = relay
            .socket("client", &host.0.core.route)
            .unwrap()
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            protocol::RELAY_PROTOCOL.parse().unwrap(),
        );
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
        ))
        .with_no_client_auth();
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(protocol::MAX_RELAY_MESSAGE))
            .max_frame_size(Some(protocol::MAX_RELAY_MESSAGE));
        let (mut socket, response) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async_tls_with_config(
                request,
                Some(config),
                false,
                Some(Connector::Rustls(Arc::new(tls))),
            ),
        )
        .await
        .expect("public TLS connect timeout")
        .expect("public v3 TLS/WebSocket connect failed");
        assert_eq!(
            response.headers()["sec-websocket-protocol"],
            protocol::RELAY_PROTOCOL
        );
        let RelayFrame::Connected { peer } = public_frame(&mut socket).await else {
            panic!("public Relay registration")
        };
        public_send(
            &mut socket,
            RelayFrame::Hello {
                peer: peer.clone(),
                mode,
            },
        )
        .await;
        let mut socket = public_path::WireSocket::new(socket, peer);
        let mut handshake = Handshake::new(
            identity,
            &relay,
            &host.0.core.route,
            mode,
            true,
            code.map(|c| c.psk()),
            (mode == protocol::crypto::Mode::Connect).then(|| host.0.core.identity.public()),
        )
        .unwrap();
        socket.send(handshake.write().unwrap()).await;
        handshake.read(&socket.receive().await).unwrap();
        if !handshake.finished() {
            socket.send(handshake.write().unwrap()).await;
        }
        let (channel, key) = handshake.into_channel().unwrap();
        assert_eq!(&key, host.0.core.identity.public());
        Self {
            socket,
            channel,
            next: 1,
        }
    }
    async fn send(&mut self, operation: Operation) -> u64 {
        let id = self.next;
        self.next += 1;
        let bytes = protocol::pack(&protocol::messages::Request { id, operation }).unwrap();
        self.socket
            .send(self.channel.encrypt(&bytes).unwrap())
            .await;
        id
    }
    async fn receive(&mut self) -> Response {
        loop {
            let data = self.socket.receive().await;
            let response: Response =
                protocol::unpack(&self.channel.decrypt(&data).unwrap()).unwrap();
            if matches!(response.event, Event::HostInfo { .. }) {
                continue;
            }
            if let Event::Ping { nonce } = response.event {
                self.send(Operation::Pong { nonce }).await;
            } else {
                return response;
            }
        }
    }
    async fn reply(&mut self, id: u64) -> Event {
        loop {
            let response = self.receive().await;
            if response.id == id {
                return response.event;
            }
        }
    }
    async fn close(mut self) {
        let _ = self.socket.close(None).await;
    }
    async fn disconnected(&mut self) {
        tokio::time::timeout(Duration::from_secs(15), self.socket.disconnected())
            .await
            .expect("revoked public socket did not disconnect");
    }
    async fn output(&mut self, marker: &[u8]) {
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut bytes = Vec::new();
            loop {
                if let Event::TerminalOutput { data, .. } = self.receive().await.event {
                    bytes.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                    if bytes.windows(marker.len()).any(|w| w == marker) {
                        return;
                    }
                }
            }
        })
        .await
        .expect("remote PTY executed output missing");
    }
}
#[cfg(unix)]
async fn public_wait(host: &Host, predicate: impl Fn(&HostSnapshot) -> bool) {
    let mut snapshots = host.subscribe();
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if predicate(&snapshots.borrow_and_update()) {
                return;
            }
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .expect("public host snapshot deadline (requires deployed v3 Relay)");
}

/// Explicit opt-in: unique in-memory store/route/PTYs, real production host supervisor,
/// real public TLS/WebSocket/Noise. No provisioning, deployment, or persistent pairing.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires NEKO_NATIVE_HOST_PUBLIC_RELAY pointing to an already deployed v3 Relay"]
async fn public_v3_host_pair_shared_pty_readonly_revoke_reconnect_lifecycle() {
    if controlled_shell_child(
        "tests::public_v3_host_pair_shared_pty_readonly_revoke_reconnect_lifecycle",
    ) {
        return;
    }
    let relay = std::env::var("NEKO_NATIVE_HOST_PUBLIC_RELAY")
        .expect("set NEKO_NATIVE_HOST_PUBLIC_RELAY=https://your-deployed-v3-relay");
    let (save, saved) = store();
    let host = start(None, save.clone()).await.unwrap();
    host.manage(Action::Configure {
        revision: host.snapshot().revision,
        relay,
        enabled: true,
    })
    .await
    .unwrap();
    public_wait(&host, |s| s.status == "online").await;
    let terminal = host
        .create_terminal("isolated public-v3 PTY".into())
        .await
        .unwrap();
    let ungranted = host
        .create_terminal("isolated ungranted PTY".into())
        .await
        .unwrap();
    let mut local = host.local().await.unwrap();
    let id = local
        .request(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await
        .unwrap();
    let Event::TerminalAttached {
        stream: local_stream,
        ..
    } = local_reply(&mut local, id).await
    else {
        panic!("local attach")
    };
    host.manage(Action::Invite {
        revision: host.snapshot().revision,
    })
    .await
    .unwrap();
    let address = protocol::RelayAddress::parse(&host.snapshot().relay).unwrap();
    let code =
        protocol::code::DeviceCode::parse(&host.snapshot().invitation.unwrap(), &address).unwrap();
    let identity = protocol::crypto::Identity::generate().unwrap();
    let mut peer =
        PublicPeer::open(&host, &identity, protocol::crypto::Mode::Pair, Some(&code)).await;
    let id = peer
        .send(Operation::Identify {
            name: "public-v3 isolated acceptance".into(),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Pending { .. }));
    public_wait(&host, |s| s.pending.len() == 1).await;
    assert!(host.snapshot().devices.is_empty(), "no implicit approval");
    host.manage(Action::Approve {
        revision: host.snapshot().revision,
        pending: host.snapshot().pending[0].id.clone(),
        permission: Permission::Operate,
        resources: vec![terminal.clone()],
    })
    .await
    .unwrap();
    assert!(matches!(peer.receive().await.event, Event::Approved { .. }));
    peer.send(Operation::AcknowledgePairing {}).await;
    assert!(matches!(
        peer.receive().await.event,
        Event::Ready {
            permission: Permission::Operate,
            ..
        }
    ));
    let device = host.snapshot().devices[0].id.clone();
    let id = peer.send(Operation::Resources {}).await;
    assert!(
        matches!(peer.reply(id).await,Event::Resources {resources} if resources.len()==1 && resources[0].id==terminal)
    );
    let id = peer
        .send(Operation::TerminalAttach {
            grant: ungranted.clone(),
            pane: None,
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Error { .. }));
    let id = peer
        .send(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await;
    let Event::TerminalAttached {
        stream,
        read_only: false,
        ..
    } = peer.reply(id).await
    else {
        panic!("remote attach")
    };
    let id = peer
        .send(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await;
    assert!(matches!(
        peer.reply(id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    let id = local
        .request(Operation::TerminalInput {
            stream: local_stream.clone(),
            data: protocol::encode(b"printf '%s%s\\n' public_v3_ local_reclaimed\r"),
        })
        .await
        .unwrap();
    assert!(
        matches!(local_reply(&mut local, id).await, Event::Ok),
        "local input must reclaim the shared PTY immediately"
    );
    let id = peer
        .send(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"must not run\r"),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Error { .. }));
    let id = peer
        .send(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await;
    assert!(
        matches!(
            peer.reply(id).await,
            Event::TerminalControl {
                controller: true,
                ..
            }
        ),
        "remote input requires reacquiring control after local input"
    );
    let id = peer
        .send(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"printf '%s%s\\n' public_v2_ remote_ok\r"),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Ok));
    peer.output(b"public_v2_remote_ok").await;
    let mut local_bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Event::TerminalOutput { data, .. } = local.receive().await.unwrap().event {
                local_bytes.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if local_bytes
                    .windows(b"public_v2_remote_ok".len())
                    .any(|w| w == b"public_v2_remote_ok")
                {
                    break;
                }
            }
        }
    })
    .await
    .expect("local subscriber must see the same remote PTY output");
    let id = local
        .request(Operation::TerminalControl {
            stream: local_stream.clone(),
            take: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    let id = peer
        .send(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"must not run\r"),
        })
        .await;
    assert!(matches!(peer.reply(id).await, Event::Error { .. }));
    let id = local
        .request(Operation::TerminalInput {
            stream: local_stream.clone(),
            data: protocol::encode(b"printf '%s%s\\n' public_v2_ local_ok\r"),
        })
        .await
        .unwrap();
    assert!(matches!(local_reply(&mut local, id).await, Event::Ok));
    peer.output(b"public_v2_local_ok").await;
    // A deliberate detach/reconnect retains the same PTY and explicit resource grant.
    peer.close().await;
    public_wait(&host, |s| !s.devices[0].online).await;
    let mut peer = PublicPeer::open(&host, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(peer.receive().await.event, Event::Ready { .. }));
    let id = peer
        .send(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await;
    assert!(matches!(
        peer.reply(id).await,
        Event::TerminalAttached { .. }
    ));
    peer.output(b"public_v2_local_ok").await;
    host.manage(Action::SetPermission {
        revision: host.snapshot().revision,
        device: device.clone(),
        permission: Permission::ReadOnly,
        resources: vec![terminal.clone()],
    })
    .await
    .unwrap();
    peer.disconnected().await;
    let mut readonly =
        PublicPeer::open(&host, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(
        readonly.receive().await.event,
        Event::Ready {
            permission: Permission::ReadOnly,
            ..
        }
    ));
    let id = readonly
        .send(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await;
    let Event::TerminalAttached {
        stream,
        read_only: true,
        ..
    } = readonly.reply(id).await
    else {
        panic!("readonly attach")
    };
    readonly.output(b"public_v2_local_ok").await;
    for operation in [
        Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        },
        Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(b"must not run\r"),
        },
        Operation::TerminalResize {
            stream,
            cols: 100,
            rows: 30,
        },
    ] {
        let id = readonly.send(operation).await;
        assert!(matches!(readonly.reply(id).await, Event::Error { .. }));
    }
    host.close_terminal(&terminal).await.unwrap();
    readonly.disconnected().await;
    assert!(host.snapshot().devices[0].resources.is_empty());
    let mut empty = PublicPeer::open(&host, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(empty.receive().await.event, Event::Ready { .. }));
    let id = empty.send(Operation::Resources {}).await;
    assert!(matches!(empty.reply(id).await,Event::Resources {resources} if resources.is_empty()));
    host.manage(Action::Revoke {
        revision: host.snapshot().revision,
        device,
    })
    .await
    .unwrap();
    empty.disconnected().await;
    let mut denied =
        PublicPeer::open(&host, &identity, protocol::crypto::Mode::Connect, None).await;
    assert!(matches!(denied.receive().await.event,Event::Error {code,..} if code=="revoked"));
    denied.close().await;
    let persisted: crate::state::Saved =
        serde_json::from_str(saved.lock().unwrap().as_ref().unwrap()).unwrap();
    assert!(persisted.devices.is_empty());
    host.close_terminal(&ungranted).await.unwrap();
    local.close().await;
    host.manage(Action::Configure {
        revision: host.snapshot().revision,
        relay: host.snapshot().relay,
        enabled: false,
    })
    .await
    .unwrap();
    eprintln!(
        "PASS actual public-v2 production host: TLS/Noise explicit approval, shared real PTY and control, both output directions, reconnect, readonly denial, closed-resource grant cleanup, revoke and reconnect rejection"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_close_delivers_owned_final_exit_after_resource_withdrawal() {
    if controlled_shell_child(
        "tests::local_close_delivers_owned_final_exit_after_resource_withdrawal",
    ) {
        return;
    }
    let (save, _) = store();
    let host = start(None, save).await.unwrap();
    let terminal = host
        .create_terminal("local final exit".into())
        .await
        .unwrap();
    let mut local = host.local().await.unwrap();
    let id = local
        .request(Operation::TerminalAttach {
            grant: terminal.clone(),
            pane: None,
        })
        .await
        .unwrap();
    let Event::TerminalAttached { stream, .. } = local_reply(&mut local, id).await else {
        panic!("local attach")
    };
    host.close_terminal(&terminal).await.unwrap();
    assert!(!host.snapshot().resources.iter().any(|r| r.id == terminal));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::TerminalExit { stream: closed, .. } = local.receive().await.unwrap().event
            {
                assert_eq!(closed, stream);
                break;
            }
        }
    })
    .await
    .expect("explicit close must deliver final owned Exit after withdrawing its resource");
    let id = local
        .request(Operation::TerminalInput {
            stream,
            data: protocol::encode(b"never\r"),
        })
        .await
        .unwrap();
    assert!(matches!(
        local_reply(&mut local, id).await,
        Event::Error { .. }
    ));
}
