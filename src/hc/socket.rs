use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::state::AppState;

use super::model::{EventLines, program_records};
use super::protocol::{CLOSE, CREDIT, Client, DATA, FOCUS, Frame, RESIZE};
use super::session::Session;

const IO_TIMEOUT: Duration = Duration::from_secs(10);
static CONNECTIONS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(32)));

pub(crate) struct AttachRequest {
    pub session_id: String,
    pub pane_id: String,
    pub cols: u16,
    pub rows: u16,
    pub single_controller: bool,
    pub history_lines: u16,
    pub display: Value,
}

pub(crate) async fn upgrade(
    state: Arc<AppState>,
    request: AttachRequest,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(error) = crate::validation::validate_size(request.cols, request.rows) {
        return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
    }
    let session = match Session::load(state, &request.session_id).await {
        Ok(session) => session,
        Err(error) => return (StatusCode::FORBIDDEN, error.to_string()).into_response(),
    };
    let pane_matches = session.state.workspaces.read().is_ok_and(|workspaces| {
        workspaces
            .get(&session.record.selector)
            .is_some_and(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .flat_map(|tab| &tab.panes)
                    .any(|pane| pane.id == request.pane_id && pane.session_id == request.session_id)
            })
    });
    if !pane_matches {
        return (
            StatusCode::FORBIDDEN,
            "hc pane and session identity do not match",
        )
            .into_response();
    }
    let Ok(permit) = CONNECTIONS.clone().try_acquire_owned() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "hc connection capacity reached",
        )
            .into_response();
    };
    ws.on_upgrade(move |mut socket| async move {
        let _permit = permit;
        if let Err(error) = serve(&mut socket, &session, &request).await {
            let _ = send(
                &mut socket,
                json!({"type":"error","message":error.to_string(),
                    "fatal":error.is::<super::session::PermanentError>(),
                    "retryable":!error.is::<super::session::PermanentError>()}),
            )
            .await;
        }
    })
}

struct Control {
    state: Arc<AppState>,
    session: String,
    connection: Option<String>,
}

impl Control {
    fn new(session: &Session, single: bool) -> Result<Self> {
        let connection = if single {
            Some(
                session
                    .state
                    .terminal_control
                    .connect(&session.record.id)?
                    .connection_id,
            )
        } else {
            None
        };
        Ok(Self {
            state: Arc::clone(&session.state),
            session: session.record.id.clone(),
            connection,
        })
    }

    fn allows_input(&self) -> bool {
        self.connection
            .as_deref()
            .is_none_or(|id| self.state.terminal_control.is_controller(&self.session, id))
    }

    async fn report(&self, socket: &mut WebSocket) -> Result<()> {
        if let Some(id) = &self.connection
            && let Some(snapshot) = self.state.terminal_control.snapshot(&self.session, id)?
        {
            send(socket, json!({"type":"control-state","session_id":self.session,"connection_id":id,
                "controller_id":snapshot.controller_id,"controller":snapshot.is_controller,"connection_count":snapshot.connection_count})).await?;
        }
        Ok(())
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        if let Some(id) = &self.connection {
            let _ = self.state.terminal_control.disconnect(&self.session, id);
        }
    }
}

async fn serve(socket: &mut WebSocket, session: &Session, request: &AttachRequest) -> Result<()> {
    let control = Control::new(session, request.single_controller)?;
    let mut client = session.connect().await?;
    let terminal = session.resolve(&mut client, true).await?;
    if terminal["state"] == "finished" {
        replay_finished(socket, session, request, &mut client, &terminal).await?;
        client.close().await?;
        return Ok(());
    }
    send(socket, json!({"type":"replay-start","session_id":session.record.id,"pane_id":request.pane_id,
        "selector":session.record.selector,"replay_after":0,"replay_mode":"snapshot","replay_gap":terminal["screen"] == "inexact"})).await?;
    let attach = client
        .call(
            "terminal.attach",
            json!({"terminal":terminal["address"],
        "cols":request.cols,"rows":request.rows,"history":request.history_lines,
        "resumed":!control.allows_input(),"observe":false,"display":request.display}),
        )
        .await?;
    let channel = channel_id(&attach)?;
    let subscription = client
        .call("events.subscribe", json!({"terminal":terminal["address"]}))
        .await?;
    let events = channel_id(&subscription)?;
    send(socket, json!({"type":"ready","session_id":session.record.id,"cols":request.cols,"rows":request.rows})).await?;
    // hc has no first-paint terminator. Reset before delivering its ordered snapshot/live stream.
    send(
        socket,
        json!({"type":"replay-complete","session_id":session.record.id,"pane_id":request.pane_id,
            "selector":session.record.selector,"last_sequence":0}),
    )
    .await?;
    let snapshot = subscription["terminals"]
        .as_array()
        .and_then(|terminals| {
            terminals
                .iter()
                .find(|entry| entry["address"] == terminal["address"])
        })
        .context("hc subscription omitted terminal snapshot")?;
    let mut stream = Stream {
        session,
        terminal: channel,
        events,
        records: program_records(snapshot),
        lines: EventLines {
            sequence: subscription["seq"]
                .as_u64()
                .context("hc omitted event sequence")?,
            ..EventLines::default()
        },
        keep: snapshot["keep"] == true,
    };
    let records = &stream.records;
    send(socket, json!({"type":"program-status","records":records})).await?;
    session
        .state
        .sessions
        .mark_status(&session.record.id, "running");
    control.report(socket).await?;
    let mut updates = session.state.terminal_control.subscribe(&session.record.id);
    loop {
        tokio::select! {
            message = socket.recv() => {
                if !browser_message(message, socket, session, &control, &mut client, channel).await? { break; }
            }
            frame = client.next_frame() => {
                if let Ok(Some(frame)) = frame {
                    if !core_frame(socket, &mut client, frame, &mut stream).await? { break; }
                } else {
                    send(socket, json!({"type":"error","message":"hc connection lost; reconnecting does not create another terminal","fatal":false,"retryable":true})).await?;
                    break;
                }
            }
            () = control_changed(&mut updates) => { control.report(socket).await?; }
        }
    }
    // Dropping the viewer/connection is a detach; only an explicit action calls terminal.kill.
    let _ = client.send(channel, CLOSE, &[]).await;
    client.close().await?;
    Ok(())
}

async fn control_changed(updates: &mut Option<tokio::sync::broadcast::Receiver<()>>) {
    match updates {
        Some(receiver) => {
            if matches!(
                receiver.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            ) {
                *updates = None;
                std::future::pending::<()>().await;
            }
        }
        None => std::future::pending().await,
    }
}

fn channel_id(reply: &Value) -> Result<u32> {
    let id = reply["channel"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id != 0)
        .context("hc omitted stream channel")?;
    Ok(id)
}

struct Stream<'a> {
    session: &'a Session,
    terminal: u32,
    events: u32,
    records: Vec<Value>,
    lines: EventLines,
    keep: bool,
}

async fn retry(socket: &mut WebSocket, message: &str) -> Result<()> {
    send(
        socket,
        json!({"type":"error","message":message,"fatal":false,"retryable":true}),
    )
    .await
}

fn recoverable_loss(keep: bool, value: &Value) -> bool {
    keep && matches!(
        value["reason"].as_str(),
        Some("holder_lost" | "stopped_by_restart" | "server_stop" | "host_shutdown" | "signaled")
    )
}

async fn core_frame(
    socket: &mut WebSocket,
    client: &mut Client,
    frame: Frame,
    stream: &mut Stream<'_>,
) -> Result<bool> {
    if frame.channel == stream.terminal {
        if frame.kind == DATA {
            let length = u32::try_from(frame.payload.len())
                .context("hc output length exceeds credit range")?;
            timeout(
                IO_TIMEOUT,
                socket.send(Message::Binary(frame.payload.into())),
            )
            .await??;
            client
                .send(stream.terminal, CREDIT, &length.to_le_bytes())
                .await?;
        } else if frame.kind == CLOSE {
            let value: Value =
                serde_json::from_slice(&frame.payload).context("invalid hc close reason")?;
            if recoverable_loss(stream.keep, &value) {
                retry(
                    socket,
                    "hc terminal is recovering; reconnect to the same terminal",
                )
                .await?;
            } else {
                report_exit(socket, stream.session, &value).await?;
            }
            return Ok(false);
        }
    } else if frame.channel == stream.events {
        if frame.kind == CLOSE {
            retry(socket, "hc events ended; reconnect for a fresh snapshot").await?;
            return Ok(false);
        }
        if frame.kind != DATA {
            return Ok(true);
        }
        for value in stream.lines.push(&frame.payload)? {
            if value["method"] == "missed" {
                retry(
                    socket,
                    "hc events were missed; reconnect for a fresh snapshot",
                )
                .await?;
                return Ok(false);
            }
            if value["method"] != "event" {
                continue;
            }
            let params = &value["params"];
            let sequence = params["seq"]
                .as_u64()
                .context("hc event omitted sequence")?;
            if sequence <= stream.lines.sequence {
                continue;
            }
            stream.lines.sequence = sequence;
            if params["type"] == "status" {
                let next = program_records(&params["data"]);
                if stream.records != next {
                    stream.records = next;
                    send(
                        socket,
                        json!({"type":"program-status","records":stream.records}),
                    )
                    .await?;
                }
            } else if params["type"] == "keep" {
                stream.keep = params["data"]["keep"] == true;
            }
        }
        let length =
            u32::try_from(frame.payload.len()).context("hc event length exceeds credit range")?;
        client
            .send(stream.events, CREDIT, &length.to_le_bytes())
            .await?;
    }
    Ok(true)
}

async fn browser_message(
    message: Option<std::result::Result<Message, axum::Error>>,
    socket: &mut WebSocket,
    session: &Session,
    control: &Control,
    client: &mut Client,
    channel: u32,
) -> Result<bool> {
    match message {
        None | Some(Ok(Message::Close(_))) => return Ok(false),
        Some(Err(error)) => return Err(error.into()),
        Some(Ok(Message::Binary(data))) if control.allows_input() => {
            if data.len() > 64 * 1024 {
                bail!("hc input exceeds 64 KiB");
            }
            client.send(channel, DATA, &data).await?;
        }
        Some(Ok(Message::Text(text))) => {
            let value: Value = serde_json::from_str(&text)?;
            control_message(socket, session, control, client, channel, &value).await?;
        }
        Some(Ok(Message::Ping(data))) => {
            timeout(IO_TIMEOUT, socket.send(Message::Pong(data))).await??;
        }
        _ => {}
    }
    Ok(true)
}

async fn control_message(
    socket: &mut WebSocket,
    session: &Session,
    control: &Control,
    client: &mut Client,
    channel: u32,
    value: &Value,
) -> Result<()> {
    match value["type"].as_str() {
        Some("input") if control.allows_input() && value["generated"] != true => {
            let text = value["data"]
                .as_str()
                .context("hc input data must be text")?;
            if text.len() > 64 * 1024 {
                bail!("hc input exceeds 64 KiB");
            }
            client.send(channel, DATA, text.as_bytes()).await?;
        }
        Some("resize") if control.allows_input() => {
            let cols = dimension(&value["cols"])?;
            let rows = dimension(&value["rows"])?;
            crate::validation::validate_size(cols, rows)?;
            let mut payload = Vec::from(cols.to_le_bytes());
            payload.extend_from_slice(&rows.to_le_bytes());
            client.send(channel, RESIZE, &payload).await?;
        }
        Some("take-control" | "release-control") => {
            if let Some(id) = &control.connection {
                if value["type"] == "take-control" {
                    session
                        .state
                        .terminal_control
                        .take_control(&session.record.id, id)?;
                    client.send(channel, FOCUS, &[1]).await?;
                } else {
                    session
                        .state
                        .terminal_control
                        .release_control(&session.record.id, id)?;
                }
                control.report(socket).await?;
            }
        }
        Some("close") if control.allows_input() => {
            super::session::close(Arc::clone(&session.state), &session.record).await?;
        }
        Some("ping") => {
            send(socket, json!({"type":"pong"})).await?;
        }
        // Native restart/history options do not own hc's persistence and are not forwarded.
        _ => {}
    }
    Ok(())
}

fn dimension(value: &Value) -> Result<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .context("invalid hc terminal dimension")
}

async fn replay_finished(
    socket: &mut WebSocket,
    session: &Session,
    request: &AttachRequest,
    client: &mut Client,
    terminal: &Value,
) -> Result<()> {
    let snapshot = client
        .call(
            "terminal.read",
            json!({"terminal":terminal["address"],"lines":request.history_lines}),
        )
        .await?;
    send(socket, json!({"type":"replay-start","session_id":session.record.id,"pane_id":request.pane_id,
        "selector":session.record.selector,"replay_after":0,"replay_mode":"snapshot","replay_gap":snapshot["complete"] != true})).await?;
    send(socket, json!({"type":"ready","session_id":session.record.id,"cols":request.cols,"rows":request.rows})).await?;
    send(
        socket,
        json!({"type":"replay-complete","session_id":session.record.id,"pane_id":request.pane_id,
        "selector":session.record.selector,"last_sequence":0}),
    )
    .await?;
    // terminal.read is plain text. Normalize its line endings for a fresh terminal.
    let text = snapshot["text"]
        .as_str()
        .context("hc omitted final screen text")?
        .replace('\n', "\r\n");
    timeout(
        IO_TIMEOUT,
        socket.send(Message::Binary(text.into_bytes().into())),
    )
    .await??;
    report_exit(socket, session, terminal).await
}

async fn report_exit(socket: &mut WebSocket, session: &Session, terminal: &Value) -> Result<()> {
    session
        .state
        .sessions
        .mark_status(&session.record.id, "exited");
    send(socket, json!({"type":"program-status","records":[]})).await?;
    // A signal or missing status is final too; never manufacture a successful exit code.
    send(
        socket,
        json!({"type":"process-exit","exit_code":terminal["exit_code"],
        "signal":terminal["signal"],"reason":terminal["reason"]}),
    )
    .await
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<()> {
    timeout(
        IO_TIMEOUT,
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| anyhow!("hc browser send timed out"))??;
    Ok(())
}

#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
