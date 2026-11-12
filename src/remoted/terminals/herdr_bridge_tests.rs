//! Exercises real terminal operations and the agent's stdio-to-Unix-socket bridge.
use super::*;
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, sync::Mutex, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

pub(in crate::remoted) struct Scratch(pub(in crate::remoted) PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
pub(in crate::remoted) struct Pane {
    pub(in crate::remoted) input: Vec<u8>,
    size: Option<(u16, u16)>,
    pub(in crate::remoted) sequence: u64,
    methods: Vec<String>,
}

pub(in crate::remoted) async fn fake_herdr(listener: UnixListener, pane: Arc<Mutex<Pane>>) {
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let line = BufReader::new(reader)
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        let method = request["method"].as_str().unwrap();
        let result = {
            let mut pane = pane.lock().unwrap();
            pane.methods.push(method.to_owned());
            if method == "session.snapshot" {
                json!({"type":"session_snapshot", "snapshot": {
                    "workspaces":[{"workspace_id":"w1"}],
                    "tabs":[{"tab_id":"w1:t1","workspace_id":"w1"}],
                    "panes":[{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1"}]
                }})
            } else {
                assert_eq!(request["params"]["pane_id"], "w1:p1");
                match method {
                    "pane.get" => json!({"pane":{"pane_id":"w1:p1"}}),
                    "pane.read" => {
                        json!({"sequence":pane.sequence,"text":format!("ready\n{}", String::from_utf8_lossy(&pane.input))})
                    }
                    "pane.wait_for_output" => json!({"timed_out":true}),
                    "pane.send_input" | "pane.send_text" => {
                        if let Some(text) = request["params"]["text"].as_str() {
                            pane.input.extend_from_slice(text.as_bytes());
                        }
                        if request["params"]["keys"]
                            .as_array()
                            .is_some_and(|keys| keys.iter().any(|key| key == "Enter"))
                        {
                            pane.input.push(b'\n');
                        }
                        pane.sequence += 1;
                        json!({})
                    }
                    "pane.resize" => {
                        pane.size = Some((
                            request["params"]["cols"]
                                .as_u64()
                                .unwrap()
                                .try_into()
                                .unwrap(),
                            request["params"]["rows"]
                                .as_u64()
                                .unwrap()
                                .try_into()
                                .unwrap(),
                        ));
                        json!({})
                    }
                    _ => panic!("unexpected bridge method: {method}"),
                }
            }
        };
        let response = format!("{}\n", json!({"id":request["id"],"result":result}));
        writer.write_all(response.as_bytes()).await.unwrap();
    }
}

async fn bridge_request(
    binary: &PathBuf,
    socket: &PathBuf,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let mut child = tokio::process::Command::new(binary)
        .args([
            "agent",
            "herdr-socket-bridge",
            "--mode",
            "request",
            "--socket",
        ])
        .arg(socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let request = format!(
        "{}\n",
        json!({"id":"bridge-test", "method":method, "params":params})
    );
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(request.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    stdin.shutdown().await.map_err(|e| e.to_string())?;
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let response: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    if let Some(error) = response.get("error") {
        return Err(error.to_string());
    }
    Ok(response)
}

pub(in crate::remoted) fn bridge_adapter(
    state: Arc<AppState>,
    binary: PathBuf,
    socket: PathBuf,
) -> HerdrTerminalAdapter {
    HerdrTerminalAdapter::with_test_rpc(state, move |session, operation| {
        let binary = binary.clone();
        let socket = socket.clone();
        Box::pin(async move {
            if session != "approved-session" {
                return Err("session was not authorized".into());
            }
            let (method, params, _) = crate::herdr::terminal_mcp_operation_request(operation)?;
            if let Some(pane_id) = params.get("pane_id") {
                bridge_request(&binary, &socket, "pane.get", json!({"pane_id":pane_id})).await?;
            }
            bridge_request(&binary, &socket, method, params).await
        })
    })
}

async fn output_contains(incoming: &mut mpsc::Receiver<Event>, text: &[u8]) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match incoming.recv().await.unwrap() {
                Event::TerminalOutput { data, .. } => {
                    let bytes = protocol_bytes(&data);
                    if bytes.windows(text.len()).any(|part| part == text) {
                        break;
                    }
                }
                Event::Error { message, .. } => panic!("terminal stream failed: {message}"),
                _ => panic!("unexpected terminal event"),
            }
        }
    })
    .await
    .expect("terminal output must reach the event stream");
}

fn protocol_bytes(data: &str) -> Vec<u8> {
    neko_remoted_protocol::decode(data, neko_remoted_protocol::FILE_CHUNK).unwrap()
}

fn approved_grant() -> Grant {
    Grant {
        id: "grant".into(),
        session: "approved-session".into(),
        selector: "test@owner".into(),
        login: "user".into(),
        backend: "herdr".into(),
        name: "Herdr".into(),
        target_signature: String::new(),
        root: None,
        root_identity: None,
    }
}

#[tokio::test]
#[ignore = "requires REMOTED_TEST_HERDR_BRIDGE pointing to a local WebShell agent binary"]
async fn real_bridge_attach_control_input_read_and_resize() {
    let binary = PathBuf::from(
        std::env::var_os("REMOTED_TEST_HERDR_BRIDGE").expect("bridge binary required"),
    );
    let scratch =
        Scratch(std::env::temp_dir().join(format!("neko-herdr-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&scratch.0).unwrap();
    let socket = scratch.0.join("herdr.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let pane = Arc::new(Mutex::new(Pane {
        sequence: 1,
        ..Pane::default()
    }));
    let server = tokio::spawn(fake_herdr(listener, pane.clone()));
    let state = Arc::new(AppState::new_for_test(":memory:".into()));
    let adapter = bridge_adapter(state.clone(), binary, socket);
    assert!(adapter.snapshot("other-session").await.is_err());
    let grant = approved_grant();
    let cancel = CancellationToken::new();
    let (events, mut incoming) = mpsc::channel(16);
    assert!(
        attach_herdr(
            adapter.clone(),
            &grant,
            Some("w9:p9".into()),
            "bad".into(),
            events.clone(),
            cancel.child_token()
        )
        .await
        .is_err()
    );
    assert!(
        attach_herdr(
            adapter.clone(),
            &grant,
            None,
            "bad".into(),
            events.clone(),
            cancel.child_token()
        )
        .await
        .is_err()
    );
    let backend = attach_herdr(
        adapter,
        &grant,
        Some("w1:p1".into()),
        "stream".into(),
        events,
        cancel.clone(),
    )
    .await
    .unwrap();
    let mut terminal = Terminal {
        grant,
        backend,
        controller: None,
        cancel: cancel.clone(),
        state,
        control_cancel: None,
    };
    output_contains(&mut incoming, b"ready").await;
    assert!(
        terminal
            .input(b"denied-before-control".to_vec())
            .await
            .is_err()
    );
    assert!(terminal.control(true).unwrap());
    terminal.input(b"bridge-input\n".to_vec()).await.unwrap();
    output_contains(&mut incoming, b"bridge-input").await;
    assert!(terminal.resize(100, 40).await.is_err());
    assert!(!terminal.control(false).unwrap());
    assert!(
        terminal
            .input(b"denied-after-release".to_vec())
            .await
            .is_err()
    );
    cancel.cancel();
    drop(terminal);
    server.abort();
    let _ = server.await;
    let pane = pane.lock().unwrap();
    assert_eq!(pane.input, b"bridge-input\n");
    assert_eq!(pane.size, None);
    for method in [
        "session.snapshot",
        "pane.get",
        "pane.read",
        "pane.send_text",
    ] {
        assert!(pane.methods.iter().any(|seen| seen == method), "{method}");
    }
}
