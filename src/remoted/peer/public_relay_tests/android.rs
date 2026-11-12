//! Interactive, isolated host for the task-owned real Android client.
//!
//! Parent wiring (owned by the main agent):
//! `#[path = "public_relay_tests/android.rs"] mod android;`
//!
//! Run with `REMOTED_ANDROID_STATE=/tmp/<fresh-name>.json` and optionally
//! `REMOTED_ANDROID_SECONDS=600` (30..=900). Pair within the normal five-minute
//! invitation lifetime. No scripted native client or Herdr runtime is involved.
use super::*;
use crate::{
    plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter, state::SessionRecord,
    terminal_manager::TerminalEvent,
};
use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
};

/// Only this test's session and shell are removed, even on panic/cancellation.
struct PtyCleanup {
    state: Arc<AppState>,
    session: String,
    observer: String,
}
impl Drop for PtyCleanup {
    fn drop(&mut self) {
        let _ = self
            .state
            .terminal_control
            .disconnect(&self.session, &self.observer);
        self.state.sessions.close_terminal_and_output(&self.session);
        self.state.sessions.write().unwrap().remove(&self.session);
    }
}

fn private_file(path: &std::path::Path) -> File {
    assert!(
        path.is_absolute() && path.starts_with(std::env::temp_dir()),
        "artifact must be an absolute temporary path"
    );
    let parent = path
        .parent()
        .unwrap()
        .canonicalize()
        .expect("artifact parent must exist");
    assert!(
        parent.starts_with(std::env::temp_dir().canonicalize().unwrap()),
        "artifact parent must resolve inside temp dir"
    );
    // Never truncate an existing artifact or follow an existing symlink.
    let file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .expect("fresh private artifact path required");
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .unwrap();
    file
}

fn write_status(file: &mut File, status: &serde_json::Value) {
    file.seek(SeekFrom::Start(0)).unwrap();
    serde_json::to_writer_pretty(&mut *file, status).unwrap();
    file.write_all(b"\n").unwrap();
    let length = file.stream_position().unwrap();
    file.set_len(length).unwrap();
    file.flush().unwrap();
}

struct Config {
    path: PathBuf,
    status_file: File,
    output_path: PathBuf,
    output_file: File,
    seconds: u64,
    expected: String,
}

struct LocalPty {
    _cleanup: PtyCleanup,
    record: SessionRecord,
    terminal: Arc<crate::terminal_manager::ManagedTerminal>,
    observer: crate::terminal_control::TerminalControlSnapshot,
}

struct Harness {
    // Drop the host before removing its authorization seam, PTY and scratch dir.
    host: tokio_util::task::AbortOnDropHandle<()>,
    _target: test_target::Scope,
    pty: LocalPty,
    _scratch: Scratch,
    state: Arc<AppState>,
    config: Config,
    grant: Grant,
    status: serde_json::Value,
    output: tokio::sync::broadcast::Receiver<TerminalEvent>,
    control: tokio::sync::broadcast::Receiver<()>,
}

struct Proof {
    approved: bool,
    expected_seen: bool,
    control_changes: u64,
    output_gap: bool,
}

fn configuration() -> Config {
    let path = PathBuf::from(
        std::env::var_os("REMOTED_ANDROID_STATE").expect("REMOTED_ANDROID_STATE required"),
    );
    let status_file = private_file(&path);
    let output_path = path.with_extension("pty.log");
    let output_file = private_file(&output_path);
    let seconds: u64 = std::env::var("REMOTED_ANDROID_SECONDS").map_or(600, |value| {
        value.parse().expect("duration must be an integer")
    });
    assert!(
        (30..=900).contains(&seconds),
        "duration must be 30..=900 seconds"
    );
    let expected = std::env::var("REMOTED_ANDROID_EXPECT_OUTPUT")
        .unwrap_or_else(|_| "ANDROID-CJK-中文成功".into());
    assert!(
        !expected.is_empty() && expected.len() <= 4096,
        "expected output must be 1..=4096 bytes"
    );
    Config {
        path,
        status_file,
        output_path,
        output_file,
        seconds,
        expected,
    }
}

fn existing_pty(state: &Arc<AppState>, scratch: &Scratch) -> LocalPty {
    // Same SessionRecord/open_terminal/control-hub path as managed::existing_pty.
    // That sibling helper is private; no visibility or production changes needed.
    let record = SessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        host: "task-owned Android integration PTY".into(),
        selector: format!("android-test@{}", uuid::Uuid::new_v4()),
        status: "running".into(),
        cols: 80,
        rows: 24,
        command: "/bin/sh".into(),
        // Pass the cwd as argv, never interpolate it into shell source or change
        // the host process's cwd/HOME. The shell lives in this test's temp dir.
        args: vec![
            "-c".into(),
            "stty -echo; cd -- \"$1\" || exit; exec /bin/sh".into(),
            "android-harness".into(),
            scratch.0.to_string_lossy().into_owned(),
        ],
        metadata: HashMap::from([("sessionBackend".into(), "ssh".into())]),
    };
    state
        .sessions
        .write()
        .unwrap()
        .insert(record.id.clone(), record.clone());
    let terminal = state
        .sessions
        .open_terminal(record.terminal_spec(80, 24), true, false)
        .unwrap();
    let observer = state.terminal_control.connect(&record.id).unwrap();
    let cleanup = PtyCleanup {
        state: state.clone(),
        session: record.id.clone(),
        observer: observer.connection_id.clone(),
    };
    LocalPty {
        _cleanup: cleanup,
        record,
        terminal,
        observer,
    }
}

async fn setup() -> Harness {
    crate::tls::initialize();
    let relay = RelayAddress::parse("https://neko-relay.czyt.eu.org").unwrap();
    let mut config = configuration();
    let Config {
        path,
        status_file,
        output_path,
        seconds,
        expected,
        ..
    } = &mut config;
    let scratch =
        Scratch(std::env::temp_dir().join(format!("neko-android-pty-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&scratch.0).unwrap();
    std::fs::set_permissions(&scratch.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = Arc::new(AppState::new_for_test(":memory:".into()));
    // new_for_test owns a separate in-memory database and a fresh Remoted identity.
    let mut saved = state.remoted.snapshot().unwrap();
    assert!(saved.devices.is_empty());
    saved.owner = format!("android-test-owner-{}", uuid::Uuid::new_v4());
    saved.enabled = true;
    saved.relay = Some(relay.clone());
    state.remoted.persist(saved.clone()).unwrap();

    let pty = existing_pty(&state, &scratch);
    let LocalPty {
        record, terminal, ..
    } = &pty;
    let output = terminal.subscribe();
    let control = state.terminal_control.subscribe(&record.id).unwrap();
    let grant = Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: record.id.clone(),
        selector: record.selector.clone(),
        login: "isolated-test-user".into(),
        backend: "ssh".into(),
        name: "Android live shell · 实际终端".into(),
        target_signature: String::new(),
        root: None,
        root_identity: None,
    };
    let target = test_target::Scope::new(
        state.clone(),
        grant.clone(),
        HerdrTerminalAdapter::new(state.clone()),
    );
    let mut host =
        tokio_util::task::AbortOnDropHandle::new(crate::remoted::relay::start(state.clone()));
    let mut status = serde_json::json!({
        "schema": 1, "status": "starting", "pid": std::process::id(),
        "relay": relay.origin(), "hostKey": saved.public_key, "route": saved.route().unwrap(),
        "session": record.id, "grant": grant.id, "permission": "operate",
        "outputPath": output_path, "scratchPath": scratch.0,
        "durationSeconds": seconds, "pairWithinSeconds": 300,
        "expectedOutput": expected,
        "suggestedCommand": format!("printf '%s\\n' '{}'", expected.replace('\'', "'\\''")),
        "boundary": "Real public WSS/Noise + Android APK + existing local ManagedTerminal and control hub; platform ownership is a state-scoped test seam; no Herdr or user session.",
    });
    write_status(status_file, &status);
    timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                result = &mut host => panic!("isolated host exited before readiness: {result:?}"),
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    let runtime = state.remoted.runtime.lock().unwrap();
                    if runtime.status == "online" && runtime.relay_rtt.is_some() { break; }
                }
            }
        }
    })
    .await
    .expect("public host registration/RTT failed; no pairing secret has been published");
    let code = DeviceCode::generate(&saved.route().unwrap(), &relay).unwrap();
    status["deviceCode"] = code.display().into();
    state.remoted.runtime.lock().unwrap().invitation = Some((code, Instant::now()));
    status["status"] = "awaiting_android_pair".into();
    write_status(status_file, &status);
    eprintln!(
        "ANDROID_HARNESS_READY state={} output={} (pairing secret only in 0600 JSON)",
        path.display(),
        output_path.display()
    );
    terminal
        .write_input(b"printf 'ANDROID-HARNESS-READY\\n'\n".to_vec())
        .unwrap();
    Harness {
        host,
        _target: target,
        pty,
        _scratch: scratch,
        state,
        config,
        grant,
        status,
        output,
        control,
    }
}

fn approve_invitation(
    state: &Arc<AppState>,
    grant: &Grant,
    status: &mut serde_json::Value,
) -> bool {
    // This private state has exactly one fresh invitation. peer::pair_device
    // consumes its PSK-validated invitation before inserting Pending.
    let pending = {
        let mut runtime = state.remoted.runtime.lock().unwrap();
        assert!(
            runtime.pending.len() <= 1,
            "unexpected second pairing request"
        );
        if let Some(id) = runtime.pending.keys().next().cloned() {
            assert!(
                runtime.invitation.is_none(),
                "invitation must have been consumed by authenticated pairing"
            );
            Some(runtime.pending.remove(&id).unwrap())
        } else {
            None
        }
    };
    if let Some(pending) = pending {
        assert!(
            !pending.decision.is_closed(),
            "Android disconnected before approval"
        );
        let device = Device {
            id: uuid::Uuid::new_v4().to_string(),
            name: pending.name,
            public_key: pending.key,
            permission: Permission::Operate,
            grants: vec![grant.clone()],
            generation: 1,
        };
        let mut current = state.remoted.snapshot().unwrap();
        assert!(
            current.devices.is_empty(),
            "only the fresh invite may be approved"
        );
        current.devices.push(device.clone());
        state.remoted.persist(current).unwrap();
        status["approvedDeviceId"] = device.id.clone().into();
        assert!(
            pending.decision.send(device).is_ok(),
            "Android disconnected during approval"
        );
        eprintln!(
            "ANDROID_HARNESS approved only the fresh invitation; awaiting real UI operations"
        );
        true
    } else {
        false
    }
}

impl Harness {
    async fn monitor(&mut self) -> Proof {
        let Self {
            state,
            pty,
            config,
            grant,
            host,
            status,
            output,
            control,
            ..
        } = self;
        let Config {
            path,
            status_file,
            output_file,
            seconds,
            expected,
            ..
        } = config;
        let LocalPty {
            record,
            terminal,
            observer,
            ..
        } = pty;
        let started = Instant::now();
        let mut approved = false;
        let mut seen = HashSet::new();
        let mut output_bytes = 0u64;
        let mut output_tail = Vec::new();
        let mut expected_seen = false;
        let mut control_changes = 0u64;
        let mut output_gap = false;
        let mut ticks = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                result = &mut *host => panic!("isolated host unexpectedly exited: {result:?}"),
                event = output.recv() => match event {
                    Ok(TerminalEvent::Output(frame)) => {
                        output_file.write_all(&frame.data).unwrap();
                        output_file.flush().unwrap();
                        output_bytes += frame.data.len() as u64;
                        output_tail.extend_from_slice(&frame.data);
                        // A literal in the echoed command is not execution evidence.
                        expected_seen |= String::from_utf8_lossy(&output_tail).lines()
                            .any(|line| line.trim_end_matches('\r') == expected.as_str());
                        if output_tail.len() > 65536 {
                            output_tail.drain(..output_tail.len() - 65536);
                        }
                    }
                    Ok(TerminalEvent::Exit(_)) => panic!("task-owned shell exited before harness deadline"),
                    Ok(TerminalEvent::Error(_)) => panic!("task-owned PTY reported an error; do not claim successful evidence"),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => output_gap = true,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => panic!("PTY output channel closed"),
                },
                notice = control.recv() => {
                    match notice {
                        Ok(()) => control_changes += 1,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => { control_changes += 1; status["controlNoticeGap"] = true.into(); }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => panic!("control hub closed"),
                    }
                },
                _ = ticks.tick() => {
                    if !approved { approved = approve_invitation(state, grant, status); }
                    {
                        let runtime = state.remoted.runtime.lock().unwrap();
                        for (id, peer) in &runtime.peers {
                            if peer.device.is_some() { seen.insert(id.clone()); }
                        }
                        status["relayStatus"] = runtime.status.clone().into();
                        status["activeAuthenticatedPeers"] = runtime.peers.values().filter(|peer| peer.device.is_some()).count().into();
                    }
                    status["status"] = if approved { "android_paired" } else if started.elapsed().as_secs() >= 300 { "invitation_expired" } else { "awaiting_android_pair" }.into();
                    status["elapsedSeconds"] = started.elapsed().as_secs().into();
                    status["authenticatedConnectionsSeen"] = seen.len().into();
                    status["controlChanges"] = control_changes.into();
                    status["observerHasControl"] = state.terminal_control.is_controller(&record.id, &observer.connection_id).into();
                    status["ptyOutputBytes"] = output_bytes.into();
                    status["expectedOutputSeen"] = expected_seen.into();
                    status["outputGap"] = output_gap.into();
                    assert!(Arc::ptr_eq(terminal, &state.sessions.terminal(&record.id).unwrap().expect("same existing PTY must survive reconnect")));
                    write_status(status_file, status);
                    if started.elapsed() >= Duration::from_secs(*seconds)
                        || (expected_seen && control_changes > 0 && path.with_extension("done").is_file())
                    { break; }
                }
            }
        }
        Proof {
            approved,
            expected_seen,
            control_changes,
            output_gap,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "interactive public Relay host; requires a real Android client and fresh REMOTED_ANDROID_STATE path"]
async fn android_managed_terminal_harness() {
    let mut harness = setup().await;
    let proof = harness.monitor().await;
    harness.host.abort();
    let _ = (&mut harness.host).await;
    harness.status["status"] = "completed".into();
    harness.status["passed"] =
        (proof.approved && proof.expected_seen && proof.control_changes > 0 && !proof.output_gap)
            .into();
    write_status(&mut harness.config.status_file, &harness.status);
    assert!(
        proof.approved,
        "no real Android client paired within harness lifetime"
    );
    assert!(
        proof.control_changes > 0,
        "no real Android control transition observed"
    );
    assert!(
        proof.expected_seen,
        "expected real shell output absent; inspect private PTY log"
    );
    assert!(
        !proof.output_gap,
        "local PTY observer lost output; do not claim complete evidence"
    );
    eprintln!("ANDROID_HARNESS completed; private JSON records output/control/reconnect evidence");
}
