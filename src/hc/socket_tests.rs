//! Socket boundary checks: a localhost browser and the production framed client.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::routing::get;
use futures::StreamExt as _;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite};

use crate::state::{AppState, SessionRecord};

use super::super::model::{EventLines, program_records};
use super::super::protocol::{CLOSE, CREDIT, Client, DATA, Frame};
use super::super::session::Session;
use super::{AttachRequest, Stream, core_frame, replay_finished};

const DEADLINE: Duration = Duration::from_secs(5);
type Browser = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn socket_pair() -> (WebSocket, Browser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let router = Router::new().route(
        "/",
        get(move |ws: WebSocketUpgrade| {
            let sender = Arc::clone(&sender);
            async move {
                ws.on_upgrade(move |socket| async move {
                    sender.lock().await.take().unwrap().send(socket).unwrap();
                })
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (browser, _) = timeout(DEADLINE, connect_async(format!("ws://{address}/")))
        .await
        .unwrap()
        .unwrap();
    let socket = timeout(DEADLINE, rx).await.unwrap().unwrap();
    // Accepted upgraded sockets are independent of the listening server.
    server.abort();
    (socket, browser)
}

struct Fixture {
    session: Session,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("neko-hc-ws-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let state = Arc::new(AppState::new_for_test(root.join("test.db")));
        let record = SessionRecord {
            id: "socket-test".into(),
            host: "test".into(),
            selector: "test@owner".into(),
            status: "running".into(),
            cols: 80,
            rows: 24,
            command: String::new(),
            args: Vec::new(),
            metadata: HashMap::new(),
        };
        state
            .sessions
            .write()
            .unwrap()
            .insert(record.id.clone(), record.clone());
        Self {
            session: Session {
                state,
                record,
                provider: "test-provider".into(),
                scope: "test-scope".into(),
                login_user: "test-user".into(),
            },
            root,
        }
    }

    fn status(&self) -> String {
        self.session.state.sessions.read().unwrap()[&self.session.record.id]
            .status
            .clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stream(session: &Session, keep: bool) -> Stream<'_> {
    Stream {
        session,
        terminal: 7,
        events: 8,
        records: program_records(&json!({"status":{"state":"working","app":"codex"}})),
        lines: EventLines {
            sequence: 40,
            ..EventLines::default()
        },
        keep,
    }
}

async fn peer() -> Client {
    let mut command = Command::new("python3");
    command.args(["-u", "-c", PEER]);
    Client::start(command).await.unwrap()
}

async fn next(browser: &mut Browser) -> tungstenite::Message {
    timeout(DEADLINE, browser.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

async fn text(browser: &mut Browser) -> Value {
    let message = next(browser).await;
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

fn frame(channel: u32, kind: u8, payload: impl Into<Vec<u8>>) -> Frame {
    Frame {
        channel,
        kind,
        payload: payload.into(),
    }
}

async fn credit(client: &mut Client, channel: u32, length: usize) {
    let echoed = timeout(DEADLINE, client.next_frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        echoed,
        frame(
            channel,
            CREDIT,
            u32::try_from(length).unwrap().to_le_bytes()
        )
    );
}

fn event(sequence: u64, state: &str) -> String {
    format!(
        "{}\n",
        json!({"method":"event","params":{"seq":sequence,"type":"status","data":{"status":{"state":state,"app":"codex"}}}})
    )
}

#[tokio::test]
async fn split_and_coalesced_events_ignore_old_snapshot_and_emit_only_changed_status() {
    let fixture = Fixture::new();
    let (mut socket, mut browser) = socket_pair().await;
    let mut client = peer().await;
    let mut stream = stream(&fixture.session, false);
    let events = [
        event(39, "error"),
        event(41, "working"),
        event(42, "blocked"),
        event(42, "error"),
        event(43, "blocked"),
        event(44, "done"),
    ]
    .concat();
    let split = 17;
    for payload in [&events.as_bytes()[..split], &events.as_bytes()[split..]] {
        assert!(
            core_frame(
                &mut socket,
                &mut client,
                frame(8, DATA, payload),
                &mut stream
            )
            .await
            .unwrap()
        );
        credit(&mut client, 8, payload.len()).await;
    }
    assert_eq!(stream.lines.sequence, 44);
    assert!(stream.lines.partial.is_empty());
    super::send(&mut socket, json!({"type":"barrier"}))
        .await
        .unwrap();
    for state in ["blocked", "done"] {
        assert_eq!(
            text(&mut browser).await,
            json!({"type":"program-status","records":[{"id":"","state":state,"app":"codex"}]})
        );
    }
    assert_eq!(
        text(&mut browser).await["type"],
        "barrier",
        "unchanged and stale statuses must not be sent"
    );
    assert_eq!(fixture.status(), "running");
    client.close().await.unwrap();
}

#[tokio::test]
async fn missed_events_and_kept_holder_loss_are_retryable_without_process_exit() {
    for (keep, incoming) in [
        (false, frame(8, DATA, b"{\"method\":\"missed\"}\n")),
        (false, frame(8, CLOSE, b"{}")),
        (
            true,
            frame(
                7,
                CLOSE,
                br#"{"reason":"holder_lost","exit_code":null,"signal":null}"#,
            ),
        ),
    ] {
        let fixture = Fixture::new();
        let (mut socket, mut browser) = socket_pair().await;
        let mut client = peer().await;
        let mut stream = stream(&fixture.session, keep);
        assert!(
            !core_frame(&mut socket, &mut client, incoming, &mut stream)
                .await
                .unwrap()
        );
        super::send(&mut socket, json!({"type":"barrier"}))
            .await
            .unwrap();
        let error = text(&mut browser).await;
        assert_eq!(error["type"], "error");
        assert_eq!(error["fatal"], false);
        assert_eq!(error["retryable"], true);
        assert_eq!(
            text(&mut browser).await["type"],
            "barrier",
            "transient loss must not emit an exit"
        );
        assert_eq!(fixture.status(), "running");
        client.close().await.unwrap();
    }
}

#[tokio::test]
async fn final_signal_preserves_null_exit_clears_status_and_marks_session_exited() {
    let fixture = Fixture::new();
    let (mut socket, mut browser) = socket_pair().await;
    let mut client = peer().await;
    let mut stream = stream(&fixture.session, false);
    assert!(
        core_frame(
            &mut socket,
            &mut client,
            frame(7, DATA, b"last output\r\n"),
            &mut stream
        )
        .await
        .unwrap()
    );
    credit(&mut client, 7, 13).await;
    assert!(
        !core_frame(
            &mut socket,
            &mut client,
            frame(
                7,
                CLOSE,
                br#"{"reason":"signaled","exit_code":null,"signal":15}"#
            ),
            &mut stream
        )
        .await
        .unwrap()
    );
    assert_eq!(
        next(&mut browser).await.into_data().as_ref(),
        b"last output\r\n"
    );
    assert_eq!(
        text(&mut browser).await,
        json!({"type":"program-status","records":[]})
    );
    assert_eq!(
        text(&mut browser).await,
        json!({"type":"process-exit","exit_code":null,"signal":15,"reason":"signaled"})
    );
    assert_eq!(fixture.status(), "exited");
    client.close().await.unwrap();
}

fn request() -> AttachRequest {
    AttachRequest {
        session_id: "socket-test".into(),
        pane_id: "pane-test".into(),
        cols: 80,
        rows: 24,
        single_controller: false,
        history_lines: 100,
        display: json!({}),
    }
}

async fn assert_replay(browser: &mut Browser, expected: &[u8], exit_code: Value) {
    let start = text(browser).await;
    assert_eq!(start["type"], "replay-start");
    assert_eq!(start["replay_mode"], "snapshot");
    assert_eq!(text(browser).await["type"], "ready");
    assert_eq!(text(browser).await["type"], "replay-complete");
    let output = next(browser).await;
    assert!(output.is_binary(), "snapshot must remain terminal bytes");
    let bytes = output.into_data();
    assert!(
        bytes.windows(expected.len()).any(|bytes| bytes == expected),
        "missing final screen: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        text(browser).await,
        json!({"type":"program-status","records":[]})
    );
    let exit = text(browser).await;
    assert_eq!(exit["type"], "process-exit");
    assert_eq!(exit["exit_code"], exit_code);
}

#[tokio::test]
async fn finished_snapshot_delivers_final_output_before_real_exit() {
    let fixture = Fixture::new();
    let (mut socket, mut browser) = socket_pair().await;
    let mut client = peer().await;
    let terminal =
        json!({"address":"t7","state":"finished","exit_code":7,"signal":null,"reason":"exited"});
    replay_finished(
        &mut socket,
        &fixture.session,
        &request(),
        &mut client,
        &terminal,
    )
    .await
    .unwrap();
    assert_replay(&mut browser, b"final screen\r\nlast line\r\n", json!(7)).await;
    assert_eq!(fixture.status(), "exited");
    client.close().await.unwrap();
}

// This peer only supplies the real Client's required handshake, echoes credits,
// and implements final terminal.read. Socket behavior remains production code.
const PEER: &str = r"
import json, struct, sys
r = sys.stdin.buffer
w = sys.stdout.buffer
hello = json.loads(r.readline())
result = {'generation':1, 'framed':True, 'features':['attach','framed','status'], 'methods':['server.status','terminal.run','terminal.list','terminal.get','terminal.read','terminal.kill','terminal.attach','events.subscribe']}
w.write((json.dumps({'jsonrpc':'2.0','id':hello['id'],'result':result})+'\n').encode()); w.flush()
def exact(n):
    data = b''
    while len(data) < n:
        part = r.read(n-len(data))
        if not part: sys.exit(0)
        data += part
    return data
while True:
    ch, kind, n = struct.unpack('<IBI', exact(9))
    payload = exact(n)
    if ch == 0:
        req = json.loads(payload)
        assert req['method'] == 'terminal.read'
        assert req['params'] == {'terminal':'t7','lines':100}
        payload = json.dumps({'jsonrpc':'2.0','id':req['id'],'result':{'text':'final screen\nlast line\n','complete':True}}).encode()
    else:
        assert kind == 1 and len(payload) == 4
    w.write(struct.pack('<IBI', ch, kind, len(payload)) + payload); w.flush()
";

#[cfg(target_os = "linux")]
mod real {
    use std::os::unix::fs::DirBuilderExt as _;
    use std::path::PathBuf;
    use std::process::Stdio;

    use anyhow::{Context as _, Result, ensure};

    use super::{
        Client, DEADLINE, Fixture, assert_replay, json, replay_finished, request, socket_pair,
        timeout,
    };

    struct Core {
        binary: PathBuf,
        root: PathBuf,
    }

    impl Core {
        fn command(&self) -> tokio::process::Command {
            let mut command = tokio::process::Command::new(&self.binary);
            command
                .env_clear()
                .env("HOME", self.root.join("home"))
                .env("XDG_CONFIG_HOME", self.root.join("config"))
                .env("XDG_STATE_HOME", self.root.join("state"))
                .env("XDG_RUNTIME_DIR", self.root.join("run"))
                .env("XDG_DATA_HOME", self.root.join("data"))
                .env("XDG_CACHE_HOME", self.root.join("cache"))
                .env("HC_SOCKET", self.root.join("s"))
                .env("HC_SESSION", "neko-ws-test")
                .env("PATH", "/usr/bin:/bin")
                .env("SHELL", "/bin/sh")
                .env("TERM", "xterm-256color")
                .env("LANG", "C.UTF-8")
                .current_dir(&self.root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            command
        }
    }

    impl Drop for Core {
        fn drop(&mut self) {
            // Keep cleanup bounded even if an assertion unwinds the async test.
            let command = self.command();
            let configured = command.as_std();
            let mut cleanup = std::process::Command::new("/usr/bin/timeout");
            cleanup
                .args(["--signal=KILL", "5s"])
                .arg(&self.binary)
                .args(["server", "stop"])
                .env_clear()
                .envs(
                    configured
                        .get_envs()
                        .filter_map(|(key, value)| value.map(|value| (key, value))),
                )
                .current_dir(&self.root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if cleanup.status().is_ok_and(|status| status.success()) {
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires NEKO_HC_TEST_BINARY pointing to a trusted Linux hc executable"]
    async fn official_hc_final_snapshot_reaches_browser_before_exit() -> Result<()> {
        let binary = std::fs::canonicalize(
            std::env::var_os("NEKO_HC_TEST_BINARY").context("set NEKO_HC_TEST_BINARY")?,
        )?;
        let root = std::env::temp_dir().join(format!("nhw-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        let core = Core { binary, root };
        for directory in ["home", "config", "state", "run", "data", "cache"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(core.root.join(directory))?;
        }
        let mut command = core.command();
        command.arg("connect");
        let mut client = Client::start(command).await?;
        let terminal = client
            .call(
                "terminal.run",
                json!({
                    "argv":["/bin/sh","-c","printf 'NEKO_FINAL_SCREEN\\n'; exit 7"],
                    "name":"neko-ws-final", "no_parent":true, "no_notify":true, "keep":false
                }),
            )
            .await?;
        let finished = timeout(DEADLINE, async {
            loop {
                let terminal = client
                    .call("terminal.get", json!({"terminal":terminal["address"]}))
                    .await?;
                if terminal["state"] == "finished" {
                    return Ok::<_, anyhow::Error>(terminal);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .context("official hc terminal did not finish")??;
        ensure!(
            finished["exit_code"] == 7,
            "unexpected final terminal: {finished}"
        );
        let fixture = Fixture::new();
        let (mut socket, mut browser) = socket_pair().await;
        replay_finished(
            &mut socket,
            &fixture.session,
            &request(),
            &mut client,
            &finished,
        )
        .await?;
        assert_replay(&mut browser, b"NEKO_FINAL_SCREEN", json!(7)).await;
        ensure!(
            fixture.status() == "exited",
            "final replay left session running"
        );
        client.close().await?;
        Ok(())
    }
}
