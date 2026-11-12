//! Bounded JSON-RPC connection to an `hc connect` child, without terminal ownership.

use std::{collections::VecDeque, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};

/// Stream bytes, or JSON-RPC on channel zero.
pub const DATA: u8 = 0;
/// Consumed byte count as a little-endian u32; callers acknowledge after delivery.
pub const CREDIT: u8 = 1;
/// Detach a channel, or receive its final reason.
pub const CLOSE: u8 = 2;
/// Little-endian u16 columns and rows.
pub const RESIZE: u8 = 3;
/// One byte: gained (1) or lost (0) focus.
pub const FOCUS: u8 = 4;
/// JSON display facts.
pub const FACTS: u8 = 5;

const MAX_PAYLOAD: usize = 16 << 20;
const MAX_CONTROL: usize = 1 << 20;
// Independent byte and item limits also bound zero-length frames and early paints.
const QUEUE_BYTES: usize = 32 << 20;
const QUEUE_FRAMES: usize = 64;
const DEADLINE: Duration = Duration::from_secs(10);
const METHODS: &[&str] = &[
    "server.status",
    "terminal.run",
    "terminal.list",
    "terminal.get",
    "terminal.read",
    "terminal.kill",
    "terminal.attach",
    "events.subscribe",
];

/// An owned frame from the documented nine-byte hc wire header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
struct Received {
    frame: Frame,
    _budget: OwnedSemaphorePermit,
}

#[derive(Debug)]
struct Write {
    bytes: Vec<u8>,
    done: oneshot::Sender<Result<()>>,
}

/// One connection child; dropping it detaches streams without killing hc terminals.
#[derive(Debug)]
pub struct Client {
    child: Child,
    hello: Value,
    incoming: mpsc::Receiver<Received>,
    reader: JoinHandle<Result<()>>,
    outgoing: mpsc::Sender<Write>,
    writer: JoinHandle<()>,
    early: VecDeque<Frame>,
    early_bytes: usize,
    next_id: u64,
    pending: Option<u64>,
    write_pending: bool,
    closed: bool,
    read_failed: bool,
}

impl Client {
    /// Spawn a non-PTY connection child and negotiate generation one within ten seconds.
    ///
    /// # Errors
    /// Fails on spawn, timeout, malformed hello or missing required capabilities.
    pub async fn start(mut command: Command) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().context("spawn hc connection")?;
        let mut stdin = child.stdin.take().context("hc stdin missing")?;
        let mut stdout = BufReader::new(child.stdout.take().context("hc stdout missing")?);
        let hello = timeout(DEADLINE, async {
            let request = json!({"jsonrpc":"2.0","id":0,"method":"hello",
                "params":{"generation":1,"framed":true}});
            stdin.write_all(format!("{request}\n").as_bytes()).await?;
            stdin.flush().await?;
            let mut line = Vec::new();
            loop {
                let bytes = stdout.fill_buf().await?;
                ensure!(!bytes.is_empty(), "hc EOF during hello");
                let end = bytes.iter().position(|b| *b == b'\n');
                let count = end.map_or(bytes.len(), |n| n + 1);
                ensure!(line.len() + count <= MAX_CONTROL, "hc hello exceeds 1 MiB");
                line.extend_from_slice(&bytes[..count]);
                stdout.consume(count);
                if end.is_some() {
                    break;
                }
            }
            let reply: Value = serde_json::from_slice(&line).context("decode hc hello")?;
            let hello = response(&reply, 0)??;
            ensure!(
                hello["generation"].as_u64() == Some(1),
                "unsupported hc generation"
            );
            ensure!(hello["framed"] == true, "hc refused framed mode");
            for feature in ["attach", "framed", "status"] {
                ensure!(
                    advertises(&hello, "features", feature),
                    "hc missing feature {feature}"
                );
            }
            for method in METHODS {
                ensure!(
                    advertises(&hello, "methods", method),
                    "hc missing method {method}"
                );
            }
            Ok::<_, anyhow::Error>(hello)
        })
        .await
        .context("hc hello timed out")??;
        let (tx, incoming) = mpsc::channel(QUEUE_FRAMES);
        let reader = tokio::spawn(read_frames(stdout, tx));
        let (outgoing, rx) = mpsc::channel(1);
        let writer = tokio::spawn(write_frames(stdin, rx));
        Ok(Self {
            child,
            hello,
            incoming,
            reader,
            outgoing,
            writer,
            early: VecDeque::new(),
            early_bytes: 0,
            next_id: 1,
            pending: None,
            write_pending: false,
            closed: false,
            read_failed: false,
        })
    }

    /// Negotiated hello result, including the core's process identity.
    #[must_use]
    pub fn hello(&self) -> &Value {
        &self.hello
    }

    /// Perform one allowlisted call, preserving stream frames that precede its reply.
    ///
    /// # Errors
    /// Fails on RPC errors, mismatched IDs, queue overflow or a ten-second deadline.
    /// Cancelled or ambiguous calls make the connection unusable for further calls;
    /// mutations are never retried automatically.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.check_writable()?;
        ensure!(
            self.pending.is_none(),
            "hc call cancelled or still pending; reconnect required"
        );
        ensure!(
            METHODS.contains(&method),
            "hc method not allowlisted: {method}"
        );
        ensure!(
            advertises(&self.hello, "methods", method),
            "hc method not advertised: {method}"
        );
        let id = self.next_id;
        self.next_id = id.checked_add(1).context("hc request ID exhausted")?;
        let request =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        ensure!(request.len() <= MAX_CONTROL, "hc request exceeds 1 MiB");
        self.pending = Some(id);
        timeout(DEADLINE, async {
            self.send(0, DATA, &request).await?;
            loop {
                let frame = self.receive().await?;
                if frame.channel == 0 {
                    let message: Value =
                        serde_json::from_slice(&frame.payload).context("decode hc response")?;
                    if message.get("id").is_some() {
                        let result = response(&message, id)?;
                        self.pending = None;
                        return result;
                    }
                    ensure!(
                        message["jsonrpc"] == "2.0" && message["method"].is_string(),
                        "invalid hc notification"
                    );
                }
                ensure!(
                    self.early.len() < QUEUE_FRAMES
                        && self.early_bytes + frame.payload.len() <= QUEUE_BYTES,
                    "hc early frame queue overflow"
                );
                self.early_bytes += frame.payload.len();
                self.early.push_back(frame);
            }
        })
        .await
        .context("hc call timed out; outcome unknown")?
    }

    /// Receive buffered early paints first; cancellation cannot interrupt frame decoding.
    ///
    /// # Errors
    /// Unexpected EOF (including between frames), truncation and malformed frames fail.
    /// Returns `None` only after explicit local close.
    pub async fn next_frame(&mut self) -> Result<Option<Frame>> {
        if let Some(frame) = self.early.pop_front() {
            self.early_bytes -= frame.payload.len();
            return Ok(Some(frame));
        }
        if self.closed {
            return Ok(None);
        }
        let frame = self.receive().await?;
        if frame.channel == 0 {
            let message: Value = serde_json::from_slice(&frame.payload)?;
            ensure!(
                message.get("id").is_none()
                    && message["jsonrpc"] == "2.0"
                    && message["method"].is_string(),
                "unexpected hc response outside call"
            );
        }
        Ok(Some(frame))
    }

    /// Write a frame with a bounded deadline; no credit is granted implicitly.
    ///
    /// # Errors
    /// Fails for oversized or invalid frames, cancelled previous writes or broken pipes.
    pub async fn send(&mut self, channel: u32, kind: u8, payload: &[u8]) -> Result<()> {
        self.check_writable()?;
        validate(channel, kind, payload.len())?;
        let len = u32::try_from(payload.len())?;
        let mut bytes = Vec::with_capacity(9 + payload.len());
        bytes.extend_from_slice(&channel.to_le_bytes());
        bytes.push(kind);
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(payload);
        let (done, reply) = oneshot::channel();
        self.write_pending = true;
        timeout(DEADLINE, async {
            self.outgoing
                .send(Write { bytes, done })
                .await
                .context("hc writer stopped")?;
            reply.await.context("hc writer stopped")?
        })
        .await
        .context("hc send timed out")??;
        self.write_pending = false;
        Ok(())
    }

    /// Stop only the connection child and tasks, leaving the core and its PTYs alive.
    ///
    /// # Errors
    /// Fails if the child cannot be killed/reaped within ten seconds.
    pub async fn close(&mut self) -> Result<()> {
        self.closed = true;
        self.reader.abort();
        self.writer.abort();
        self.incoming.close();
        self.early.clear();
        self.early_bytes = 0;
        self.child
            .start_kill()
            .context("kill hc connection child")?;
        timeout(DEADLINE, self.child.wait())
            .await
            .context("hc child cleanup timed out")??;
        Ok(())
    }

    fn check_writable(&self) -> Result<()> {
        ensure!(!self.closed, "hc connection closed");
        ensure!(!self.read_failed, "hc reader failed; reconnect required");
        ensure!(
            !self.write_pending,
            "hc write cancelled or failed; reconnect required"
        );
        Ok(())
    }

    async fn receive(&mut self) -> Result<Frame> {
        ensure!(!self.read_failed, "hc reader failed; reconnect required");
        if let Some(received) = self.incoming.recv().await {
            return Ok(received.frame);
        }
        // A completed JoinHandle must not be polled twice after reporting failure.
        self.read_failed = true;
        match (&mut self.reader).await {
            Ok(result) => result?,
            Err(error) => bail!("hc reader stopped: {error}"),
        }
        bail!("hc frame reader stopped")
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
        let _ = self.child.start_kill();
    }
}

fn advertises(hello: &Value, key: &str, value: &str) -> bool {
    hello[key]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(value)))
}

// The outer error invalidates the connection; an inner RPC error is a valid reply.
fn response(message: &Value, id: u64) -> Result<Result<Value>> {
    ensure!(
        message["jsonrpc"] == "2.0" && message["id"].as_u64() == Some(id),
        "hc response ID/version mismatch"
    );
    ensure!(
        message.get("result").is_some() != message.get("error").is_some(),
        "invalid hc response envelope"
    );
    if let Some(error) = message.get("error") {
        ensure!(
            error["code"].as_i64().is_some() && error["message"].is_string(),
            "invalid hc RPC error"
        );
        return Ok(Err(anyhow::anyhow!("hc RPC error: {error}")));
    }
    Ok(Ok(message["result"].clone()))
}

fn validate(channel: u32, kind: u8, len: usize) -> Result<()> {
    ensure!(len <= MAX_PAYLOAD, "hc frame exceeds 16 MiB");
    ensure!(kind <= FACTS, "unknown hc frame kind {kind}");
    if channel == 0 {
        ensure!(
            kind == DATA && len <= MAX_CONTROL,
            "invalid or oversized hc control frame"
        );
    } else {
        ensure!(
            match kind {
                CREDIT | RESIZE => len == 4,
                FOCUS => len == 1,
                _ => true,
            },
            "invalid hc frame length"
        );
    }
    Ok(())
}

async fn read_frames(mut stdout: BufReader<ChildStdout>, tx: mpsc::Sender<Received>) -> Result<()> {
    let budget = Arc::new(Semaphore::new(QUEUE_BYTES));
    loop {
        let mut header = [0; 9];
        stdout
            .read_exact(&mut header)
            .await
            .context("hc EOF or truncated frame header")?;
        let channel = u32::from_le_bytes(header[..4].try_into()?);
        let kind = header[4];
        let len = u32::from_le_bytes(header[5..].try_into()?) as usize;
        validate(channel, kind, len)?;
        let permit = budget
            .clone()
            .try_acquire_many_owned(u32::try_from(len)?)
            .context("hc receive byte queue overflow")?;
        let mut payload = vec![0; len];
        stdout
            .read_exact(&mut payload)
            .await
            .context("hc truncated frame payload")?;
        // Never block the decoder behind a stalled stream and hide control replies.
        // Overflow ends this connection explicitly; no data or status is silently lost.
        tx.try_send(Received {
            frame: Frame {
                channel,
                kind,
                payload,
            },
            _budget: permit,
        })
        .context("hc receive frame queue overflow")?;
    }
}

async fn write_frames(mut stdin: ChildStdin, mut rx: mpsc::Receiver<Write>) {
    while let Some(write) = rx.recv().await {
        let result = timeout(DEADLINE, async {
            stdin.write_all(&write.bytes).await?;
            stdin.flush().await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("hc pipe write timed out")
        .and_then(|result| result);
        let failed = result.is_err();
        let _ = write.done.send(result);
        if failed {
            break;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn hello_attach_and_early_output() {
        let mut command = tokio::process::Command::new("python3");
        command.args(["-u", "-c", PYTHON, "early"]);
        let mut client = Client::start(command).await.unwrap();
        assert_eq!(client.hello()["core"]["id"], "fake-core");
        assert_eq!(
            client.call("terminal.attach", json!({})).await.unwrap()["channel"],
            7
        );
        let frame = client.next_frame().await.unwrap().unwrap();
        assert_eq!(
            (frame.channel, frame.kind, frame.payload.as_slice()),
            (7, DATA, b"paint".as_slice())
        );
        client.send(7, CREDIT, &5_u32.to_le_bytes()).await.unwrap();
        client.close().await.unwrap();
    }

    fn helper(mode: &str) -> Command {
        let mut command = Command::new("python3");
        command.args(["-u", "-c", PYTHON, mode]);
        command
    }

    #[tokio::test]
    async fn rejects_incompatible_hello() {
        for mode in [
            "generation",
            "feature",
            "method",
            "read_method",
            "not_framed",
            "hello_id",
            "hello_big",
            "hello_eof",
        ] {
            assert!(Client::start(helper(mode)).await.is_err(), "mode {mode}");
        }
    }

    #[tokio::test]
    async fn preserves_frames_buffered_with_hello() {
        let mut client = Client::start(helper("hello_frame")).await.unwrap();
        assert_eq!(
            client.next_frame().await.unwrap().unwrap().payload,
            b"ahead"
        );
        client.close().await.unwrap();
        assert!(client.next_frame().await.unwrap().is_none());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_receive_preserves_header_and_payload() {
        let mut client = Client::start(helper("fragment")).await.unwrap();
        for _ in 0..9 {
            assert!(
                timeout(Duration::from_millis(20), client.next_frame())
                    .await
                    .is_err()
            );
        }
        let frame = client.next_frame().await.unwrap().unwrap();
        assert_eq!(
            (frame.channel, frame.payload.as_slice()),
            (7, b"fragmented".as_slice())
        );
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn rejects_eof_truncation_and_oversized_headers() {
        for mode in [
            "eof",
            "header",
            "payload",
            "large",
            "control_large",
            "kind",
            "credit_length",
        ] {
            let mut client = Client::start(helper(mode)).await.unwrap();
            assert!(client.next_frame().await.is_err(), "mode {mode}");
            assert!(
                client.next_frame().await.is_err(),
                "EOF must remain an error: {mode}"
            );
            client.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn rejects_uncorrelated_and_malformed_replies() {
        for mode in ["wrong_id", "bad_rpc", "both_result_error"] {
            let mut client = Client::start(helper(mode)).await.unwrap();
            assert!(
                client.call("terminal.get", json!({})).await.is_err(),
                "mode {mode}"
            );
            assert!(client.call("terminal.get", json!({})).await.is_err());
            client.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn rpc_error_does_not_retry_or_poison_connection() {
        let mut client = Client::start(helper("rpc_error")).await.unwrap();
        let error = client.call("terminal.run", json!({})).await.unwrap_err();
        assert!(error.to_string().contains("name_taken"));
        assert_eq!(
            client.call("terminal.get", json!({})).await.unwrap()["channel"],
            7
        );
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn deny_unknown_and_server_mutation_methods() {
        let mut client = Client::start(helper("early")).await.unwrap();
        for method in [
            "server.stop",
            "server.restart",
            "server.update",
            "machine.exec",
            "new.method",
        ] {
            assert!(
                client.call(method, json!({})).await.is_err(),
                "method {method}"
            );
        }
        assert!(client.call("terminal.get", json!({})).await.is_ok());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn rejects_early_frame_overflow() {
        let mut client = Client::start(helper("overflow")).await.unwrap();
        assert!(client.call("terminal.attach", json!({})).await.is_err());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn receive_byte_budget_is_bounded() {
        let mut client = Client::start(helper("byte_overflow")).await.unwrap();
        timeout(Duration::from_secs(2), async {
            while !client.reader.is_finished() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        for _ in 0..2 {
            assert_eq!(
                client.next_frame().await.unwrap().unwrap().payload.len(),
                MAX_PAYLOAD
            );
        }
        assert!(
            client
                .next_frame()
                .await
                .unwrap_err()
                .to_string()
                .contains("byte queue overflow")
        );
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn accepts_maximum_payload() {
        let mut client = Client::start(helper("max_payload")).await.unwrap();
        assert_eq!(
            client.next_frame().await.unwrap().unwrap().payload.len(),
            MAX_PAYLOAD
        );
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_call_is_not_retried() {
        let mut client = Client::start(helper("delayed_reply")).await.unwrap();
        assert!(
            timeout(
                Duration::from_millis(30),
                client.call("terminal.run", json!({}))
            )
            .await
            .is_err()
        );
        assert!(client.call("terminal.run", json!({})).await.is_err());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn stderr_cannot_block_handshake_and_close_reaps_child() {
        let mut client = Client::start(helper("stderr")).await.unwrap();
        client.close().await.unwrap();
        assert!(client.child.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn handshake_deadline_kills_stalled_child() {
        assert!(
            Client::start(helper("hello_delay"))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }

    #[tokio::test]
    async fn call_deadline_does_not_retry_mutation() {
        let mut client = Client::start(helper("no_reply")).await.unwrap();
        assert!(
            client
                .call("terminal.run", json!({}))
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(client.call("terminal.run", json!({})).await.is_err());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn blocked_write_is_bounded_and_not_reused() {
        let mut client = Client::start(helper("no_read")).await.unwrap();
        assert!(client.send(7, DATA, &vec![0; MAX_PAYLOAD]).await.is_err());
        assert!(client.send(7, DATA, b"retry").await.is_err());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn send_round_trips_frame_and_validates_before_writing() {
        let mut client = Client::start(helper("echo_frame")).await.unwrap();
        assert!(client.send(7, CREDIT, b"bad").await.is_err());
        assert!(client.send(7, FOCUS, b"bad").await.is_err());
        assert!(
            client
                .send(7, DATA, &vec![0; MAX_PAYLOAD + 1])
                .await
                .is_err()
        );
        for (kind, payload) in [
            (DATA, b"input".as_slice()),
            (CREDIT, &9_u32.to_le_bytes()),
            (RESIZE, &[80, 0, 24, 0]),
            (FOCUS, &[1]),
            (FACTS, b"{}"),
            (CLOSE, b""),
        ] {
            client.send(7, kind, payload).await.unwrap();
            assert_eq!(
                client.next_frame().await.unwrap().unwrap(),
                Frame {
                    channel: 7,
                    kind,
                    payload: payload.to_vec()
                }
            );
        }
        client.close().await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn drop_kills_connection_child_without_waiting() {
        let client = Client::start(helper("no_read")).await.unwrap();
        let path = format!("/proc/{}", client.child.id().unwrap());
        drop(client);
        timeout(Duration::from_secs(2), async {
            while std::path::Path::new(&path).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    mod real_smoke {
        use super::*;
        use std::{ffi::OsString, os::unix::fs::DirBuilderExt, path::PathBuf};

        struct Scratch {
            binary: PathBuf,
            root: PathBuf,
            env: Vec<(&'static str, OsString)>,
            stopped: bool,
        }
        impl Scratch {
            fn command(&self, args: &[&str]) -> Command {
                let mut command = Command::new(&self.binary);
                command
                    .args(args)
                    .env_clear()
                    .envs(self.env.iter().cloned())
                    .current_dir(&self.root)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                command
            }
        }
        impl Drop for Scratch {
            fn drop(&mut self) {
                if self.stopped {
                    return;
                }
                // Independent bounded fallback survives a failed test's runtime teardown.
                // Do not delete evidence if stopping the isolated core fails.
                let child = std::process::Command::new("/usr/bin/timeout")
                    .args(["--signal=KILL", "5s"])
                    .arg(&self.binary)
                    .args(["server", "stop"])
                    .env_clear()
                    .envs(self.env.iter().cloned())
                    .current_dir(&self.root)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                if let Ok(mut child) = child {
                    let root = self.root.clone();
                    std::thread::spawn(move || {
                        if child.wait().is_ok_and(|status| status.success()) {
                            let _ = std::fs::remove_dir_all(root);
                        }
                    });
                }
            }
        }
        async fn paint(client: &mut Client, channel: u32) -> Result<()> {
            timeout(DEADLINE, async {
                let mut bytes = Vec::new();
                loop {
                    let frame = client
                        .next_frame()
                        .await?
                        .context("viewer ended before paint")?;
                    if frame.kind == DATA && frame.channel != 0 {
                        client
                            .send(
                                frame.channel,
                                CREDIT,
                                &u32::try_from(frame.payload.len())?.to_le_bytes(),
                            )
                            .await?;
                    }
                    if frame.channel == channel && frame.kind == DATA {
                        ensure!(
                            bytes.len() + frame.payload.len() <= MAX_CONTROL,
                            "smoke paint exceeds bound"
                        );
                        bytes.extend_from_slice(&frame.payload);
                        if String::from_utf8_lossy(&bytes).contains("NEKO_HC_READY") {
                            return Ok(());
                        }
                    }
                }
            })
            .await
            .context("real hc first paint timed out")?
        }
        async fn attach(client: &mut Client, terminal: &str) -> Result<u32> {
            let reply = client
                .call(
                    "terminal.attach",
                    json!({"terminal":terminal,"cols":101,"rows":37}),
                )
                .await?;
            u32::try_from(
                reply["channel"]
                    .as_u64()
                    .context("attach channel missing")?,
            )
            .context("invalid channel")
        }

        fn scratch() -> Result<Scratch> {
            let binary = std::env::var_os("NEKO_HC_TEST_BINARY")
                .context("set NEKO_HC_TEST_BINARY to a trusted hc executable")?;
            let binary = std::fs::canonicalize(binary)?;
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let root = PathBuf::from(format!("/var/tmp/nhc-{}-{nonce:x}", std::process::id()));
            std::fs::DirBuilder::new().mode(0o700).create(&root)?;
            let mut env = Vec::new();
            for (key, directory) in [
                ("HOME", "home"),
                ("XDG_CONFIG_HOME", "config"),
                ("XDG_STATE_HOME", "state"),
                ("XDG_RUNTIME_DIR", "run"),
                ("XDG_DATA_HOME", "data"),
                ("XDG_CACHE_HOME", "cache"),
            ] {
                let path = root.join(directory);
                std::fs::DirBuilder::new().mode(0o700).create(&path)?;
                env.push((key, path.into_os_string()));
            }
            env.extend([
                ("HC_SESSION", OsString::from("neko-smoke")),
                ("HC_SOCKET", root.join("s").into_os_string()),
                ("PATH", OsString::from("/usr/bin:/bin")),
                ("SHELL", OsString::from("/bin/sh")),
                ("TERM", OsString::from("xterm-256color")),
                ("LANG", OsString::from("C.UTF-8")),
            ]);
            Ok(Scratch {
                binary,
                root,
                env,
                stopped: false,
            })
        }

        async fn start_and_detach(scratch: &Scratch) -> Result<(String, u64, Value)> {
            let mut client = Client::start(scratch.command(&["connect"])).await?;
            let core = client.call("server.status", json!({})).await?;
            ensure!(
                core["core"] == client.hello()["core"],
                "hello/status core identity mismatch"
            );
            let run = client.call("terminal.run", json!({
            "argv":["/bin/sh","-c",r#"printf '\033]7501;state=working\033\\'; printf 'NEKO_HC_READY\n'; IFS= read -r line; printf 'NEKO_HC_INPUT:%s\n' "$line"; exit 7"#],
            "name":"neko-smoke", "no_parent":true, "no_notify":true, "keep":false
        })).await?;
            let terminal = run["address"]
                .as_str()
                .context("run address missing")?
                .to_owned();
            let pid = run["pid"].as_u64().context("run PID missing")?;
            let reply = client
                .call(
                    "terminal.attach",
                    json!({"terminal":terminal,"cols":80,"rows":24}),
                )
                .await?;
            let channel = u32::try_from(
                reply["channel"]
                    .as_u64()
                    .context("attach channel missing")?,
            )?;
            paint(&mut client, channel).await?;
            resize_and_detach(&mut client, &terminal, channel, pid).await?;
            client.close().await?;
            Ok((terminal, pid, core))
        }

        async fn resize_and_detach(
            client: &mut Client,
            terminal: &str,
            channel: u32,
            pid: u64,
        ) -> Result<()> {
            client.send(channel, RESIZE, &[101, 0, 37, 0]).await?;
            timeout(DEADLINE, async {
                loop {
                    let record = client
                        .call("terminal.get", json!({"terminal":terminal}))
                        .await?;
                    if record["cols"] == 101
                        && record["rows"] == 37
                        && record["status"]["state"] == "working"
                    {
                        ensure!(record["pid"] == pid, "resize changed terminal PID");
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .context("resize/status snapshot timed out")??;
            let snapshot = client
                .call("events.subscribe", json!({"terminal":terminal}))
                .await?;
            ensure!(
                snapshot["seq"].is_u64() && snapshot["channel"].is_u64(),
                "invalid event subscription: {snapshot}"
            );
            ensure!(
                snapshot["terminals"][0]["address"] == terminal
                    && snapshot["terminals"][0]["status"]["state"] == "working",
                "subscription lost status snapshot: {snapshot}"
            );
            client.send(channel, CLOSE, &[]).await?;
            timeout(DEADLINE, async {
                loop {
                    let frame = client
                        .next_frame()
                        .await?
                        .context("EOF before detach acknowledgement")?;
                    if frame.channel == channel && frame.kind == CLOSE {
                        ensure!(
                            frame.payload.is_empty(),
                            "detach returned unexpected reason"
                        );
                        return Ok::<_, anyhow::Error>(());
                    }
                }
            })
            .await
            .context("detach timed out")??;
            Ok(())
        }

        async fn reconnect(
            scratch: &Scratch,
            terminal: &str,
            pid: u64,
            core: &Value,
        ) -> Result<(Client, u32)> {
            let mut client = Client::start(scratch.command(&["connect"])).await?;
            let status = client.call("server.status", json!({})).await?;
            ensure!(
                status["core"] == core["core"] && status["pid"] == core["pid"],
                "reconnect replaced core"
            );
            let record = client
                .call("terminal.get", json!({"terminal":terminal}))
                .await?;
            ensure!(
                record["address"] == terminal && record["pid"] == pid && record["state"] == "live",
                "viewer detach replaced terminal: {record}"
            );
            let channel = attach(&mut client, terminal).await?;
            paint(&mut client, channel).await?;
            Ok((client, channel))
        }

        async fn exit_seven(client: &mut Client, terminal: &str, channel: u32) -> Result<()> {
            let subscription = client
                .call("events.subscribe", json!({"terminal":terminal}))
                .await?;
            let event_channel = u32::try_from(
                subscription["channel"]
                    .as_u64()
                    .context("event channel missing")?,
            )?;
            client.send(channel, DATA, b"smoke-input\n").await?;
            drain_exit(client, channel, event_channel).await?;
            let record = client
                .call("terminal.get", json!({"terminal":terminal}))
                .await?;
            ensure!(
                record["state"] == "finished" && record["exit_code"] == 7,
                "final snapshot lost exit: {record}"
            );
            let read = client
                .call("terminal.read", json!({"terminal":terminal,"lines":1000}))
                .await?;
            ensure!(
                read["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("NEKO_HC_INPUT:smoke-input")
                        && !text.contains('\u{1b}')),
                "finished read is not plain final text: {read}"
            );
            ensure!(
                read["complete"].is_boolean()
                    && read["how"] == "history"
                    && read.get("why").is_none(),
                "unexpected finished read envelope: {read}"
            );
            eprintln!("hc finished terminal.read: {read}");
            client.close().await?;
            Ok(())
        }

        async fn drain_exit(client: &mut Client, channel: u32, event_channel: u32) -> Result<()> {
            timeout(DEADLINE, async {
                let mut output = Vec::new();
                let mut events = Vec::new();
                let mut exited = false;
                let mut finished = false;
                loop {
                    let frame = client
                        .next_frame()
                        .await?
                        .context("EOF before terminal exit")?;
                    if frame.kind == DATA && frame.channel != 0 {
                        client
                            .send(
                                frame.channel,
                                CREDIT,
                                &u32::try_from(frame.payload.len())?.to_le_bytes(),
                            )
                            .await?;
                        let bytes = if frame.channel == channel {
                            &mut output
                        } else {
                            &mut events
                        };
                        ensure!(
                            bytes.len() + frame.payload.len() <= MAX_CONTROL,
                            "smoke output exceeds bound"
                        );
                        bytes.extend_from_slice(&frame.payload);
                        if frame.channel == event_channel {
                            while let Some(end) = events.iter().position(|byte| *byte == b'\n') {
                                let message: Value = serde_json::from_slice(
                                    &events.drain(..=end).collect::<Vec<_>>(),
                                )?;
                                if message["params"]["type"] == "finished" {
                                    ensure!(
                                        message["params"]["data"]["exit_code"] == 7,
                                        "event lost real exit: {message}"
                                    );
                                    eprintln!("hc finished event: {message}");
                                    finished = true;
                                }
                            }
                        }
                    } else if frame.channel == channel && frame.kind == CLOSE {
                        let reason: Value = serde_json::from_slice(&frame.payload)?;
                        ensure!(
                            reason["reason"] == "exited"
                                && reason["exit_code"] == 7
                                && reason["signal"].is_null(),
                            "stream lost real exit: {reason}"
                        );
                        ensure!(
                            reason["finished_ms"].is_u64() && reason.get("keep").is_none(),
                            "unexpected CLOSE shape: {reason}"
                        );
                        eprintln!("hc terminal CLOSE: {reason}");
                        exited = true;
                    }
                    if exited && finished {
                        ensure!(
                            String::from_utf8_lossy(&output).contains("NEKO_HC_INPUT:smoke-input"),
                            "PTY input/output missing"
                        );
                        return Ok::<_, anyhow::Error>(());
                    }
                }
            })
            .await
            .context("real hc exit timed out")??;
            Ok(())
        }

        /// Exercises the real wire contract only inside private scratch paths.
        #[tokio::test]
        #[ignore = "requires NEKO_HC_TEST_BINARY pointing to a trusted Linux hc executable"]
        async fn real_hc_isolated_session_smoke() -> Result<()> {
            let mut scratch = scratch()?;
            let (terminal, pid, core) = start_and_detach(&scratch).await?;
            let (viewer, _) = reconnect(&scratch, &terminal, pid, &core).await?;
            drop(viewer);
            let (mut viewer, channel) = reconnect(&scratch, &terminal, pid, &core).await?;
            exit_seven(&mut viewer, &terminal, channel).await?;
            let stopped = timeout(DEADLINE, scratch.command(&["server", "stop"]).status())
                .await
                .context("isolated core stop timed out")??;
            ensure!(stopped.success(), "isolated core stop failed: {stopped}");
            scratch.stopped = true;
            std::fs::remove_dir_all(&scratch.root)?;
            Ok(())
        }
    }

    const PYTHON: &str = r"
import sys,json,struct,time
r=sys.stdin.buffer; w=sys.stdout.buffer; mode=sys.argv[1]
hello=json.loads(r.readline())
assert hello['params']=={'generation':1,'framed':True}
if mode=='hello_delay': time.sleep(60)
result={'generation':1,'framed':True,'core':{'id':'fake-core'},'features':['attach','framed','status'],'methods':['server.status','terminal.run','terminal.list','terminal.get','terminal.kill','terminal.attach','events.subscribe','terminal.read']}
if mode=='generation': result['generation']=2
if mode=='feature': result['features'].remove('status')
if mode=='method': result['methods'].remove('terminal.attach')
if mode=='read_method': result['methods'].remove('terminal.read')
if mode=='not_framed': result['framed']=False
if mode=='hello_eof': sys.exit(0)
if mode=='hello_big': w.write(b'x'*(1024*1024+1)); w.flush(); sys.exit(0)
if mode=='stderr': sys.stderr.write('x'*(1024*1024)); sys.stderr.flush()
hello_reply=(json.dumps({'jsonrpc':'2.0','id':999 if mode=='hello_id' else hello['id'],'result':result})+'\n').encode()
if mode=='hello_frame': hello_reply+=struct.pack('<IBI',7,0,5)+b'ahead'
w.write(hello_reply); w.flush()
if mode=='no_read': time.sleep(60)
def exact(n):
    b=b''
    while len(b)<n:
        v=r.read(n-len(b))
        if not v: sys.exit(0)
        b+=v
    return b
def frame(ch,k,p):
    w.write(struct.pack('<IBI',ch,k,len(p))+p); w.flush()
if mode=='fragment':
    b=struct.pack('<IBI',7,0,10)+b'fragmented'
    w.write(b[:3]); w.flush(); time.sleep(.12)
    w.write(b[3:12]); w.flush(); time.sleep(.12)
    w.write(b[12:]); w.flush()
if mode=='max_payload': frame(7,0,b'x'*(16*1024*1024))
if mode=='byte_overflow':
    frame(7,0,b'x'*(16*1024*1024))
    frame(7,0,b'x'*(16*1024*1024))
    w.write(struct.pack('<IBI',7,0,16*1024*1024)); w.flush()
if mode=='eof': sys.exit(0)
if mode=='header': w.write(b'abc'); w.flush(); sys.exit(0)
if mode=='payload': w.write(struct.pack('<IBI',7,0,9)+b'abc'); w.flush(); sys.exit(0)
if mode=='large': w.write(struct.pack('<IBI',7,0,16*1024*1024+1)); w.flush()
if mode=='control_large': w.write(struct.pack('<IBI',0,0,1024*1024+1)); w.flush()
if mode=='kind': frame(7,6,b'')
if mode=='credit_length': frame(7,1,b'xx')
count=0
while True:
    ch,k,n=struct.unpack('<IBI',exact(9)); p=exact(n)
    if ch:
        if mode=='echo_frame': frame(ch,k,p)
        continue
    req=json.loads(p); count+=1
    if mode=='delayed_reply': time.sleep(1)
    if mode=='no_reply': time.sleep(60)
    if mode=='overflow':
        for i in range(70): frame(7,0,b'')
    if mode=='rpc_error' and count==1:
        frame(0,0,json.dumps({'jsonrpc':'2.0','id':req['id'],'error':{'code':-32000,'message':'name_taken'}}).encode()); continue
    frame(7,0,b'paint')
    reply={'jsonrpc':'2.0','id':req['id']+1 if mode=='wrong_id' else req['id'],'result':{'channel':7}}
    if mode=='bad_rpc': reply['jsonrpc']='1.0'
    if mode=='both_result_error': reply['error']={'code':-1,'message':'error'}
    frame(0,0,json.dumps(reply).encode())
";
}
