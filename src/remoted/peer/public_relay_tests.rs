//! Public Relay business tests with scoped platform ownership and isolated Herdr runtimes.
use super::*;
use crate::remoted::terminals::herdr_bridge_tests::{Pane, Scratch, bridge_adapter, fake_herdr};
use futures::{SinkExt, StreamExt};
use protocol::{
    RelayAddress,
    code::DeviceCode,
    crypto::Identity,
    messages::{Permission, RelayFrame},
};
use std::{collections::VecDeque, path::PathBuf};
use tokio::net::{TcpStream, UnixListener};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

#[path = "public_relay_tests/android.rs"]
mod android;
#[path = "public_relay_tests/files.rs"]
mod files;
#[path = "public_relay_tests/keyboard.rs"]
mod keyboard;
#[path = "public_relay_tests/managed.rs"]
mod managed;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn frame(socket: &mut Socket) -> RelayFrame {
    loop {
        let message = timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("public Relay receive timeout")
            .expect("public Relay closed")
            .expect("public Relay transport failed");
        match message {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
            Message::Pong(_) => {}
            other => panic!("unexpected public Relay frame: {other:?}"),
        }
    }
}

async fn send_frame(socket: &mut Socket, frame: RelayFrame) {
    timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(serde_json::to_string(&frame).unwrap().into())),
    )
    .await
    .unwrap()
    .unwrap();
}

async fn payload(socket: &mut Socket, peer: &str) -> Vec<u8> {
    loop {
        match frame(socket).await {
            RelayFrame::Data {
                data,
                sequence: Some(sequence),
                ..
            } => {
                send_frame(
                    socket,
                    RelayFrame::Acknowledge {
                        peer: peer.into(),
                        sequence,
                    },
                )
                .await;
                return protocol::decode(&data, protocol::MAX_CIPHERTEXT).unwrap();
            }
            RelayFrame::Delivered { .. } => {}
            other => panic!("unexpected public Relay payload: {other:?}"),
        }
    }
}

struct Client {
    socket: Socket,
    peer: String,
    channel: Channel,
    next: u64,
    deferred: VecDeque<Response>,
}
impl Client {
    async fn pair(relay: &RelayAddress, code: &DeviceCode, identity: &Identity) -> Self {
        let mut request = relay
            .socket("client", code.route())
            .unwrap()
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "neko.remoted.v1".parse().unwrap());
        let (mut socket, response) = timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            response.headers()["sec-websocket-protocol"],
            "neko.remoted.v1"
        );
        let RelayFrame::Connected { peer } = frame(&mut socket).await else {
            panic!("Relay must allocate client")
        };
        send_frame(
            &mut socket,
            RelayFrame::Hello {
                peer: peer.clone(),
                mode: Mode::Pair,
            },
        )
        .await;
        let mut handshake = Handshake::new(
            identity,
            relay,
            code.route(),
            Mode::Pair,
            true,
            Some(code.psk()),
            None,
        )
        .unwrap();
        send_frame(
            &mut socket,
            RelayFrame::Data {
                peer: peer.clone(),
                data: protocol::encode(&handshake.write().unwrap()),
                sequence: None,
            },
        )
        .await;
        handshake.read(&payload(&mut socket, &peer).await).unwrap();
        if !handshake.finished() {
            send_frame(
                &mut socket,
                RelayFrame::Data {
                    peer: peer.clone(),
                    data: protocol::encode(&handshake.write().unwrap()),
                    sequence: None,
                },
            )
            .await;
        }
        let (channel, _) = handshake.into_channel().unwrap();
        Self {
            socket,
            peer,
            channel,
            next: 1,
            deferred: VecDeque::new(),
        }
    }

    async fn request(&mut self, operation: Operation) -> u64 {
        eprintln!(
            "TRACE send business id={} kind={:?}",
            self.next,
            std::mem::discriminant(&operation)
        );
        let id = self.next;
        self.next += 1;
        let plaintext = protocol::pack(&Request { id, operation }).unwrap();
        let data = protocol::encode(&self.channel.encrypt(&plaintext).unwrap());
        send_frame(
            &mut self.socket,
            RelayFrame::Data {
                peer: self.peer.clone(),
                data,
                sequence: None,
            },
        )
        .await;
        id
    }

    async fn receive(&mut self) -> Response {
        loop {
            let bytes = payload(&mut self.socket, &self.peer).await;
            let response: Response =
                protocol::unpack(&self.channel.decrypt(&bytes).unwrap()).unwrap();
            if let Event::Ping { nonce } = response.event {
                self.request(Operation::Pong { nonce }).await;
            } else {
                eprintln!(
                    "TRACE receive id={} kind={:?}",
                    response.id,
                    std::mem::discriminant(&response.event)
                );
                return response;
            }
        }
    }

    async fn response(&mut self, id: u64) -> Event {
        if let Some(index) = self.deferred.iter().position(|response| response.id == id) {
            return self.deferred.remove(index).unwrap().event;
        }
        loop {
            let response = self.receive().await;
            if response.id == id {
                return response.event;
            }
            self.deferred.push_back(response);
            assert!(self.deferred.len() <= 16);
        }
    }

    async fn output(&mut self, stream_id: &str, needle: &[u8]) {
        loop {
            let response = match self.deferred.pop_front() {
                Some(response) => response,
                None => self.receive().await,
            };
            let Event::TerminalOutput { stream, data, .. } = response.event else {
                panic!("expected terminal output")
            };
            assert_eq!(stream, stream_id);
            let bytes = protocol::decode(&data, protocol::FILE_CHUNK).unwrap();
            if bytes.windows(needle.len()).any(|part| part == needle) {
                return;
            }
        }
    }
}

async fn pair_and_approve(
    state: &Arc<AppState>,
    relay: &RelayAddress,
    grant: &Grant,
    permission: Permission,
) -> Client {
    let saved = state.remoted.snapshot().unwrap();
    let code = DeviceCode::generate(&saved.route().unwrap(), relay).unwrap();
    let copy = DeviceCode::parse(&code.display(), relay).unwrap();
    state.remoted.runtime.lock().unwrap().invitation = Some((code, Instant::now()));
    let identity = Identity::generate().unwrap();
    let mut client = Client::pair(relay, &copy, &identity).await;
    let id = client
        .request(Operation::Identify {
            name: "isolated public Relay integration".into(),
        })
        .await;
    assert!(matches!(client.response(id).await, Event::Pending { .. }));
    let pending = {
        let mut runtime = state.remoted.runtime.lock().unwrap();
        let id = runtime
            .pending
            .values()
            .find(|pending| pending.key == protocol::encode(identity.public()))
            .unwrap()
            .id
            .clone();
        runtime.pending.remove(&id).unwrap()
    };
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: pending.name,
        public_key: pending.key,
        permission,
        grants: vec![grant.clone()],
        generation: 1,
    };
    let mut saved = state.remoted.snapshot().unwrap();
    saved.devices.push(device.clone());
    let expected_host_key = saved.public_key.clone();
    state.remoted.persist(saved).unwrap();
    assert!(pending.decision.send(device).is_ok());
    let Event::Approved {
        host_key,
        permission: approved,
        ..
    } = client.response(0).await
    else {
        panic!("pairing must be approved")
    };
    assert_eq!(host_key, expected_host_key);
    assert_eq!(approved, permission);
    client.request(Operation::AcknowledgePairing {}).await;
    let Event::Ready {
        resources,
        permission: ready,
        ..
    } = client.response(0).await
    else {
        panic!("paired client must become ready")
    };
    assert_eq!(ready, permission);
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].id, grant.id);
    assert_eq!(resources[0].herdr, grant.backend == "herdr");
    assert_eq!(resources[0].terminal, grant.backend != "herdr");
    eprintln!("PASS encrypted pair -> approved -> ready -> scoped Herdr resource ({permission:?})");
    client
}

async fn attach(client: &mut Client, grant: &Grant, read_only: bool, pane: &str) -> String {
    let id = client
        .request(Operation::TerminalAttach {
            grant: grant.id.clone(),
            pane: Some(pane.into()),
        })
        .await;
    let Event::TerminalAttached {
        stream,
        read_only: actual,
        ..
    } = client.response(id).await
    else {
        panic!("authorized pane must attach")
    };
    assert_eq!(actual, read_only);
    eprintln!("PASS authorized Herdr pane attached; awaiting initial PTY output");
    client.output(&stream, b"ready").await;
    eprintln!("PASS initial Herdr PTY output received through encrypted Relay");
    stream
}

async fn operate(client: &mut Client, grant: &Grant, pane: &str, genuine: bool) {
    let id = client.request(Operation::Resources {}).await;
    let Event::Resources { resources } = client.response(id).await else {
        panic!("resource list expected")
    };
    assert_eq!(resources[0].id, grant.id);
    let stream = attach(client, grant, false, pane).await;
    let id = client
        .request(Operation::TerminalControl {
            stream: stream.clone(),
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
    if genuine {
        Box::pin(keyboard::run(client, &stream)).await;
    }
    let nonce = uuid::Uuid::new_v4().to_string();
    let (command, expected) = if genuine {
        (
            format!("printf 'public-relay-output-%s\\n' '{nonce}'\n"),
            format!("public-relay-output-{nonce}"),
        )
    } else {
        ("public-relay-input\n".into(), "public-relay-input".into())
    };
    let id = client
        .request(Operation::TerminalInput {
            stream: stream.clone(),
            data: protocol::encode(command.as_bytes()),
        })
        .await;
    assert!(matches!(client.response(id).await, Event::Ok));
    client.output(&stream, expected.as_bytes()).await;
    eprintln!("PASS actual PTY execution output: {expected}");
    let id = client
        .request(Operation::TerminalResize {
            stream: stream.clone(),
            cols: 100,
            rows: 40,
        })
        .await;
    assert!(matches!(client.response(id).await, Event::Error { .. }));
    let id = client
        .request(Operation::TerminalControl {
            stream: stream.clone(),
            take: false,
        })
        .await;
    assert!(matches!(
        client.response(id).await,
        Event::TerminalControl {
            controller: false,
            ..
        }
    ));
    let id = client.request(Operation::TerminalDetach { stream }).await;
    assert!(matches!(client.response(id).await, Event::Ok));
    eprintln!(
        "PASS encrypted resource -> Herdr attach -> control -> input -> read output -> unsupported cell resize rejected -> release -> detach"
    );
}

type Runtime = (
    Scratch,
    PathBuf,
    Option<Arc<Mutex<Pane>>>,
    Option<tokio_util::task::AbortOnDropHandle<()>>,
    String,
);
fn runtime(genuine: bool) -> Runtime {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("neko-public-relay-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&scratch.0).unwrap();
    let (socket, pane, server) = if genuine {
        let socket = PathBuf::from(
            std::env::var_os("REMOTED_TEST_HERDR_SOCKET")
                .expect("isolated real Herdr socket required"),
        );
        assert!(
            socket.starts_with("/tmp"),
            "only isolated temp socket allowed"
        );
        (socket, None, None)
    } else {
        let socket = scratch.0.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let mut initial = Pane::default();
        initial.sequence = 1;
        let pane = Arc::new(Mutex::new(initial));
        let server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(fake_herdr(
            listener,
            pane.clone(),
        )));
        (socket, Some(pane), Some(server))
    };
    let pane_id = if genuine {
        std::env::var("REMOTED_TEST_HERDR_PANE")
            .expect("pane ID from isolated workspace response required")
    } else {
        "w1:p1".into()
    };
    (scratch, socket, pane, server, pane_id)
}

async fn workflow(genuine: bool) {
    crate::tls::initialize();
    let address = std::env::var("REMOTED_TEST_RELAY").expect("public Relay required");
    assert_eq!(address, "https://neko-relay.czyt.eu.org");
    let relay = RelayAddress::parse(&address).unwrap();
    let binary = PathBuf::from(
        std::env::var_os("REMOTED_TEST_HERDR_BRIDGE").expect("local agent bridge required"),
    );
    let (_scratch, socket, pane, server, pane_id) = runtime(genuine);
    let state = Arc::new(AppState::new_for_test(":memory:".into()));
    let mut saved = state.remoted.snapshot().unwrap();
    saved.owner = "isolated-test-owner".into();
    saved.enabled = true;
    saved.relay = Some(relay.clone());
    state.remoted.persist(saved).unwrap();
    let grant = Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: "approved-session".into(),
        selector: "isolated@owner".into(),
        login: "test-user".into(),
        backend: "herdr".into(),
        name: "isolated Herdr".into(),
        target_signature: String::new(),
        root: None,
        root_identity: None,
    };
    let adapter = bridge_adapter(state.clone(), binary, socket);
    if genuine {
        let read = adapter
            .read(&grant.session, &pane_id, 0, 0, 262_144)
            .await
            .unwrap();
        assert!(
            !read.frames.is_empty(),
            "genuine Herdr adapter must read initial PTY bytes"
        );
        eprintln!(
            "PASS genuine Herdr adapter read {} initial frame(s)",
            read.frames.len()
        );
    }
    let target_scope = test_target::Scope::new(state.clone(), grant.clone(), adapter.clone());
    let mut host =
        tokio_util::task::AbortOnDropHandle::new(crate::remoted::relay::start(state.clone()));
    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut host => panic!("host exited: {result:?}"),
                () = tokio::time::sleep(Duration::from_millis(20)) => {
                    let runtime = state.remoted.runtime.lock().unwrap();
                    if runtime.status == "online" && runtime.relay_rtt.is_some() { break; }
                }
            }
        }
    })
    .await
    .expect("host must register and receive real public Relay RTT");
    eprintln!("PASS real public host business WebSocket registered, heartbeat RTT measured");
    let mut operator = pair_and_approve(&state, &relay, &grant, Permission::Operate).await;
    operate(&mut operator, &grant, &pane_id, genuine).await;
    operator.socket.close(None).await.unwrap();
    let mut observer = pair_and_approve(&state, &relay, &grant, Permission::ReadOnly).await;
    let stream = attach(&mut observer, &grant, true, &pane_id).await;
    let id = observer
        .request(Operation::TerminalInput {
            stream,
            data: protocol::encode(b"READONLY-MUST-NOT-REACH-HERDR"),
        })
        .await;
    assert!(
        matches!(observer.response(id).await, Event::Error {code,outcome_unknown:false,..} if code == "AUTHORIZATION_FAILED")
    );
    check_readonly(pane, &adapter, &grant, &pane_id).await;
    eprintln!("PASS read-only encrypted input rejected before Herdr; backend input unchanged");
    let _ = observer.socket.close(None).await;
    drop(target_scope);
    if genuine {
        Box::pin(managed::run(&state, &relay, adapter.clone())).await;
        Box::pin(files::run(&state, &relay, adapter)).await;
    }
    host.abort();
    let _ = host.await;
    if let Some(server) = server {
        server.abort();
        let _ = server.await;
    }
    eprintln!(
        "HERDR_RUNTIME: {}",
        if genuine {
            "genuine isolated /usr/sbin/herdr PTY"
        } else {
            "controlled fake socket"
        }
    );
    eprintln!(
        "BOUNDARY: public WSS + Noise + pairing + grants + admission + peer dispatch + terminal control are real; LightOS ownership and target dispatch are scoped fixtures; Herdr uses a real local agent bridge"
    );
}

#[tokio::test]
#[ignore = "requires the real public Relay and REMOTED_TEST_HERDR_BRIDGE; uses only fresh identities/in-memory data"]
async fn public_relay_encrypted_pair_herdr_and_readonly() {
    timeout(Duration::from_secs(90), Box::pin(workflow(false)))
        .await
        .expect("public Relay integration exceeded deadline");
}

#[tokio::test]
#[ignore = "requires isolated real Herdr socket/pane, public Relay and local agent bridge"]
async fn public_relay_genuine_herdr_pty_and_readonly() {
    timeout(Duration::from_mins(4), Box::pin(workflow(true)))
        .await
        .expect("genuine Herdr public Relay integration exceeded deadline");
}

async fn check_readonly(
    pane: Option<Arc<Mutex<Pane>>>,
    adapter: &crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter,
    grant: &Grant,
    pane_id: &str,
) {
    if let Some(pane) = pane {
        assert_eq!(pane.lock().unwrap().input, b"public-relay-input\n");
    } else {
        let read = adapter
            .read(&grant.session, pane_id, 0, 0, 262_144)
            .await
            .unwrap();
        let text: Vec<u8> = read
            .frames
            .iter()
            .flat_map(|frame| {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .decode(&frame.data_base64)
                    .unwrap()
            })
            .collect();
        assert!(!String::from_utf8_lossy(&text).contains("READONLY-MUST-NOT-REACH-HERDR"));
        eprintln!(
            "PASS genuine Herdr PTY executed printf sentinel (not an echoed command); readonly payload absent from real pane"
        );
    }
}

#[tokio::test]
#[ignore = "requires real public Relay; uses a fresh host and a temporary local Python file root"]
async fn public_relay_real_python_files() {
    crate::tls::initialize();
    let relay = RelayAddress::parse(&std::env::var("REMOTED_TEST_RELAY").unwrap()).unwrap();
    assert_eq!(
        relay,
        RelayAddress::parse("https://neko-relay.czyt.eu.org").unwrap()
    );
    let state = Arc::new(AppState::new_for_test(":memory:".into()));
    let mut saved = state.remoted.snapshot().unwrap();
    saved.owner = "isolated-test-owner".into();
    saved.enabled = true;
    saved.relay = Some(relay.clone());
    state.remoted.persist(saved).unwrap();
    let mut host =
        tokio_util::task::AbortOnDropHandle::new(crate::remoted::relay::start(state.clone()));
    timeout(Duration::from_secs(15), async {
        loop {
            tokio::select! {
                result = &mut host => panic!("host exited: {result:?}"),
                () = tokio::time::sleep(Duration::from_millis(20)) => {
                    let runtime = state.remoted.runtime.lock().unwrap();
                    if runtime.status == "online" && runtime.relay_rtt.is_some() { break; }
                }
            }
        }
    })
    .await
    .unwrap();
    let adapter =
        crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter::new(state.clone());
    timeout(
        Duration::from_secs(90),
        Box::pin(files::run(&state, &relay, adapter)),
    )
    .await
    .unwrap();
    host.abort();
    let _ = host.await;
}
