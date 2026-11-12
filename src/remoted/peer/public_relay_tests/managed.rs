//! Same existing PTY/output buffer and control hub used by managed Web terminals.
use super::*;
use crate::{state::SessionRecord, terminal_manager::TerminalEvent};

struct ExistingPty {
    state: Arc<AppState>,
    session: String,
    web: String,
}
impl Drop for ExistingPty {
    fn drop(&mut self) {
        let _ = self
            .state
            .terminal_control
            .disconnect(&self.session, &self.web);
        self.state.sessions.close_terminal_and_output(&self.session);
        self.state.sessions.write().unwrap().remove(&self.session);
    }
}

async fn shared_output(
    source: &mut tokio::sync::broadcast::Receiver<TerminalEvent>,
    needle: &[u8],
) {
    timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        loop {
            if let TerminalEvent::Output(frame) = source.recv().await.unwrap() {
                bytes.extend_from_slice(&frame.data);
                if bytes.windows(needle.len()).any(|part| part == needle) {
                    return;
                }
            }
        }
    })
    .await
    .expect("existing Web output subscriber must receive the same PTY bytes");
}

async fn control(client: &mut Client, stream: &str) {
    let id = client
        .request(Operation::TerminalControl {
            stream: stream.into(),
            take: true,
        })
        .await;
    assert!(matches!(
        client.response(id).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
}

async fn input(client: &mut Client, stream: &str, command: &[u8]) -> Event {
    let id = client
        .request(Operation::TerminalInput {
            stream: stream.into(),
            data: protocol::encode(command),
        })
        .await;
    client.response(id).await
}

pub(super) async fn run(
    state: &Arc<AppState>,
    relay: &RelayAddress,
    adapter: crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter,
) {
    let (record, terminal, web, _cleanup) = existing_pty(state);
    let mut web_output = terminal.subscribe();
    let mut notices = state.terminal_control.subscribe(&record.id).unwrap();
    terminal
        .write_input(b"printf 'shared-%s-ready\\n' pty\n".to_vec())
        .unwrap();
    shared_output(&mut web_output, b"shared-pty-ready").await;
    let grant = Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: record.id.clone(),
        selector: record.selector.clone(),
        login: "test-user".into(),
        backend: "ssh".into(),
        name: "existing ordinary PTY".into(),
        target_signature: String::new(),
        root: None,
        root_identity: None,
    };
    let _target = test_target::Scope::new(state.clone(), grant.clone(), adapter);
    let mut client = pair_and_approve(state, relay, &grant, Permission::Operate).await;
    let id = client
        .request(Operation::TerminalAttach {
            grant: grant.id.clone(),
            pane: None,
        })
        .await;
    let Event::TerminalAttached { stream, .. } = client.response(id).await else {
        panic!("existing ordinary PTY must attach")
    };
    client.output(&stream, b"shared-pty-ready").await;
    assert!(Arc::ptr_eq(
        &terminal,
        &state.sessions.terminal(&record.id).unwrap().unwrap()
    ));
    control(&mut client, &stream).await;
    timeout(Duration::from_secs(2), notices.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !state
            .terminal_control
            .is_controller(&record.id, &web.connection_id)
    );
    assert!(matches!(
        input(
            &mut client,
            &stream,
            b"printf 'native-%s-output\\n' shared\n"
        )
        .await,
        Event::Ok
    ));
    client.output(&stream, b"native-shared-output").await;
    shared_output(&mut web_output, b"native-shared-output").await;
    state
        .terminal_control
        .take_control(&record.id, &web.connection_id)
        .unwrap();
    assert!(matches!(
        input(&mut client, &stream, b"NATIVE-WRITE-MUST-BE-DENIED\n").await,
        Event::Error {
            outcome_unknown: false,
            ..
        }
    ));
    terminal
        .write_input(b"printf 'web-%s-output\\n' shared\n".to_vec())
        .unwrap();
    client.output(&stream, b"web-shared-output").await;
    control(&mut client, &stream).await;
    let id = client.request(Operation::TerminalDetach { stream }).await;
    assert!(matches!(client.response(id).await, Event::Ok));
    assert!(Arc::ptr_eq(
        &terminal,
        &state.sessions.terminal(&record.id).unwrap().unwrap()
    ));
    assert!(
        state
            .terminal_control
            .is_controller(&record.id, &web.connection_id)
    );
    terminal
        .write_input(b"printf 'after-%s-detach\\n' native\n".to_vec())
        .unwrap();
    shared_output(&mut web_output, b"after-native-detach").await;
    let _ = client.socket.close(None).await;
    eprintln!(
        "PASS public encrypted ordinary attach reuses exact existing ManagedTerminal Arc; native input reaches existing Web subscriber; Web input reaches native; shared hub take/reclaim denies native writes; detach preserves existing PTY"
    );
    eprintln!(
        "BOUNDARY ordinary: real local PTY + host manager + shared Web control hub; no browser WebSocket or LightOS agent transport exercised"
    );
}

fn existing_pty(
    state: &Arc<AppState>,
) -> (
    SessionRecord,
    Arc<crate::terminal_manager::ManagedTerminal>,
    crate::terminal_control::TerminalControlSnapshot,
    ExistingPty,
) {
    let record = SessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        host: "isolated-local-pty".into(),
        selector: "isolated@owner".into(),
        status: "running".into(),
        cols: 80,
        rows: 24,
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "stty -echo; exec /bin/sh".into()],
        metadata: std::collections::HashMap::from([("sessionBackend".into(), "ssh".into())]),
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
    let web = state.terminal_control.connect(&record.id).unwrap();
    let cleanup = ExistingPty {
        state: state.clone(),
        session: record.id.clone(),
        web: web.connection_id.clone(),
    };
    assert!(web.is_controller);
    (record, terminal, web, cleanup)
}
