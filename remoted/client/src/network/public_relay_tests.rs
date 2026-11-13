//! Opt-in acceptance against the deployed Relay using fresh identities and PTYs.
use super::*;
use neko_remoted_protocol::messages::{Event, Permission};
use neoko_terminal_host::{Action, Host, HostSnapshot, Save};
use std::{sync::Arc, time::Instant};

async fn wait(host: &Host, predicate: impl Fn(&HostSnapshot) -> bool) {
    let mut snapshots = host.subscribe();
    timeout(Duration::from_secs(30), async {
        loop {
            if predicate(&snapshots.borrow_and_update()) {
                return;
            }
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .expect("public host state timeout");
}
async fn reply(session: &mut Session, id: u64) -> Event {
    timeout(Duration::from_secs(20), async {
        loop {
            let response = session.receive().await.unwrap();
            if response.id == id {
                return response.event;
            }
        }
    })
    .await
    .expect("public client reply timeout")
}
async fn output(session: &mut Session, marker: &[u8]) {
    timeout(Duration::from_secs(20), async {
        let mut bytes = Vec::new();
        loop {
            if let Event::TerminalOutput { data, .. } = session.receive().await.unwrap().event {
                bytes.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if bytes.windows(marker.len()).any(|text| text == marker) {
                    return;
                }
                assert!(bytes.len() < 1024 * 1024, "unbounded public fixture output");
            }
        }
    })
    .await
    .expect("public PTY output timeout");
}

#[tokio::test]
#[ignore = "uses fresh in-memory host/client identities and PTYs on NEKO_NATIVE_HOST_PUBLIC_RELAY"]
async fn public_relay_client_pty_latency_and_reconnect() {
    const TEST: &str = "network::public_relay_tests::public_relay_client_pty_latency_and_reconnect";
    // Only the child selects a clean shell; never mutate the test runner's environment.
    if std::env::var("NEKO_CLIENT_PUBLIC_TEST_CHILD").as_deref() != Ok(TEST) {
        use std::os::unix::fs::PermissionsExt;
        let scratch = tempfile::tempdir().unwrap();
        let shell = scratch.path().join("shell");
        std::fs::write(&shell, "#!/bin/sh\nexec /bin/sh\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--include-ignored", "--nocapture"])
            .env("NEKO_CLIENT_PUBLIC_TEST_CHILD", TEST)
            .env("SHELL", shell)
            .env_remove("ENV")
            .env_remove("BASH_ENV")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&result.stdout);
        print!("{stdout}");
        assert!(
            result.status.success(),
            "public client failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            stdout.contains("1 passed; 0 failed"),
            "acceptance child did not execute"
        );
        return;
    }
    let relay = RelayAddress::parse(
        &std::env::var("NEKO_NATIVE_HOST_PUBLIC_RELAY").expect("public relay env required"),
    )
    .unwrap();
    let save: Save = Arc::new(|_| Box::pin(async { Ok(()) }));
    let host = neoko_terminal_host::start(None, save).await.unwrap();
    host.manage(Action::Configure {
        revision: host.snapshot().revision,
        relay: relay.origin().into(),
        enabled: true,
    })
    .await
    .unwrap();
    wait(&host, |s| s.status == "online").await;
    let grant = host
        .create_terminal("isolated native client latency".into())
        .await
        .unwrap();
    host.manage(Action::Invite {
        revision: host.snapshot().revision,
    })
    .await
    .unwrap();
    let code = DeviceCode::parse(&host.snapshot().invitation.unwrap(), &relay).unwrap();
    let identity = Identity::generate().unwrap();
    let mut session = Session::pair(&relay, &code, &identity).await.unwrap();
    let host_key = *session.host_key();
    let id = session
        .request(Operation::Identify {
            name: "isolated client latency test".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        reply(&mut session, id).await,
        Event::Pending { .. }
    ));
    wait(&host, |s| s.pending.len() == 1).await;
    host.manage(Action::Approve {
        revision: host.snapshot().revision,
        pending: host.snapshot().pending[0].id.clone(),
        permission: Permission::Operate,
        resources: vec![grant.clone()],
    })
    .await
    .unwrap();
    assert!(matches!(
        session.receive().await.unwrap().event,
        Event::Approved { .. }
    ));
    session
        .request(Operation::AcknowledgePairing {})
        .await
        .unwrap();
    assert!(matches!(
        session.receive().await.unwrap().event,
        Event::Ready { .. }
    ));
    let id = session
        .request(Operation::TerminalAttach {
            grant: grant.clone(),
            pane: None,
        })
        .await
        .unwrap();
    let Event::TerminalAttached { stream, .. } = reply(&mut session, id).await else {
        panic!("public client attach")
    };
    let id = session
        .request(Operation::TerminalControl {
            stream: stream.clone(),
            take: true,
        })
        .await
        .unwrap();
    assert!(matches!(
        reply(&mut session, id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    let mut samples = Vec::new();
    for sample in 0..5 {
        let started = Instant::now();
        let command = format!("printf '%s%s\\n' native_echo_ sample_{sample}\r");
        session
            .request(Operation::TerminalInput {
                stream: stream.clone(),
                data: protocol::encode(command.as_bytes()),
            })
            .await
            .unwrap();
        output(
            &mut session,
            format!("native_echo_sample_{sample}").as_bytes(),
        )
        .await;
        samples.push(started.elapsed().as_millis());
    }
    samples.sort();
    println!(
        "PUBLIC_RELAY_CLIENT_PTY_ECHO_MS samples={samples:?} median={} max={}",
        samples[2], samples[4]
    );
    let route = code.route().to_owned();
    session.close().await;
    wait(&host, |s| !s.devices[0].online).await;
    let mut session = Session::connect(&relay, &route, &identity, &host_key)
        .await
        .unwrap();
    assert!(matches!(
        session.receive().await.unwrap().event,
        Event::Ready { .. }
    ));
    let id = session
        .request(Operation::TerminalAttach {
            grant: grant.clone(),
            pane: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        reply(&mut session, id).await,
        Event::TerminalAttached { .. }
    ));
    output(&mut session, b"native_echo_sample_4").await;
    session.close().await;
    host.close_terminal(&grant).await.unwrap();
    host.manage(Action::Configure {
        revision: host.snapshot().revision,
        relay: relay.origin().into(),
        enabled: false,
    })
    .await
    .unwrap();
}
