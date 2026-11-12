use super::store::Grant;
use crate::{
    plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter,
    state::AppState,
    terminal_manager::{ManagedTerminal, OutputSnapshot, TerminalEvent},
};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use neko_remoted_protocol::messages::Event;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

const REPLAY_BYTES: usize = 256 * 1024;
const REPLAY_FRAMES: usize = 128;

#[cfg(all(test, unix))]
#[path = "terminals/herdr_bridge_tests.rs"]
pub(in crate::remoted) mod herdr_bridge_tests;

#[cfg(test)]
#[path = "terminals/program_status_tests.rs"]
mod program_status_tests;

fn program_record(
    record: neko_program_status::ProgramRecord,
) -> neko_remoted_protocol::messages::ProgramRecord {
    use neko_program_status::{BlockedKind, ProgramState};
    use neko_remoted_protocol::messages as protocol;
    protocol::ProgramRecord {
        id: record.id,
        state: match record.state {
            ProgramState::Idle => protocol::ProgramState::Idle,
            ProgramState::Working => protocol::ProgramState::Working,
            ProgramState::Done => protocol::ProgramState::Done,
            ProgramState::Blocked => protocol::ProgramState::Blocked,
            ProgramState::Error => protocol::ProgramState::Error,
        },
        kind: record.kind.map(|kind| match kind {
            BlockedKind::Permission => protocol::BlockedKind::Permission,
            BlockedKind::Question => protocol::BlockedKind::Question,
            BlockedKind::Auth => protocol::BlockedKind::Auth,
        }),
        progress: record.progress,
        app: record.app,
        title: record.title,
        msg: record.msg,
    }
}

fn replay_tail(mut snapshot: OutputSnapshot) -> OutputSnapshot {
    let mut bytes = 0;
    let mut frames = Vec::new();
    for mut frame in snapshot.frames.into_iter().rev() {
        if bytes >= REPLAY_BYTES || frames.len() >= REPLAY_FRAMES {
            snapshot.truncated = true;
            break;
        }
        let remaining = REPLAY_BYTES - bytes;
        if frame.data.len() > remaining {
            frame.data = frame.data.slice(frame.data.len() - remaining..);
            snapshot.truncated = true;
        }
        bytes += frame.data.len();
        frames.push(frame);
    }
    frames.reverse();
    snapshot.replay_gap |=
        snapshot.truncated || frames.first().is_some_and(|frame| frame.sequence > 1);
    snapshot.frames = frames;
    snapshot
}

struct OutputStream {
    stream: String,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
    sequence: u64,
    first: bool,
}
impl OutputStream {
    fn new(stream: String, events: mpsc::Sender<Event>, cancel: CancellationToken) -> Self {
        Self {
            stream,
            events,
            cancel,
            sequence: 0,
            first: true,
        }
    }
    async fn write(&mut self, bytes: &[u8], replay: bool, reset: bool, gap: bool) -> Result<()> {
        if bytes.is_empty() {
            return self.chunk(bytes, replay, reset, gap).await;
        }
        for (index, chunk) in bytes.chunks(neko_remoted_protocol::FILE_CHUNK).enumerate() {
            self.chunk(chunk, replay, reset && index == 0, gap && index == 0)
                .await?;
        }
        Ok(())
    }
    async fn replay(&mut self, snapshot: OutputSnapshot) -> Result<()> {
        self.write(&[], true, true, snapshot.replay_gap || snapshot.truncated)
            .await?;
        for frame in snapshot.frames {
            self.write(&frame.data, true, false, false).await?;
        }
        Ok(())
    }
    async fn current_programs(
        &self,
        terminal: &ManagedTerminal,
        revision: &mut Option<u64>,
    ) -> Result<()> {
        self.program_status(terminal.program_status_since(*revision), revision)
            .await
    }
    async fn program_status(
        &self,
        snapshot: Option<(u64, Vec<neko_program_status::ProgramRecord>)>,
        revision: &mut Option<u64>,
    ) -> Result<()> {
        if let Some((next, records)) = snapshot {
            let event = Event::TerminalProgramStatus {
                stream: self.stream.clone(),
                records: records.into_iter().map(program_record).collect(),
            };
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => anyhow::bail!("terminal detached"),
                result = self.events.send(event) => result?,
            }
            *revision = Some(next);
        }
        Ok(())
    }
    async fn chunk(&mut self, bytes: &[u8], replay: bool, reset: bool, gap: bool) -> Result<()> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .context("terminal sequence exhausted")?;
        let event = Event::TerminalOutput {
            stream: self.stream.clone(),
            sequence: self.sequence,
            data: neko_remoted_protocol::encode(bytes),
            replay,
            reset: reset || self.first,
            gap,
        };
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => anyhow::bail!("terminal detached"),
            result = self.events.send(event) => result?,
        }
        self.first = false;
        Ok(())
    }
}

pub enum Backend {
    Managed(Arc<ManagedTerminal>),
    Herdr(String, HerdrTerminalAdapter),
    Hc(mpsc::Sender<crate::hc::remoted::Command>),
}
pub struct Terminal {
    pub grant: Grant,
    pub backend: Backend,
    pub controller: Option<String>,
    pub cancel: CancellationToken,
    state: Arc<AppState>,
    control_cancel: Option<CancellationToken>,
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(id) = &self.controller {
            let _ = self
                .state
                .terminal_control
                .disconnect(&self.grant.session, id);
        }
    }
}

pub(super) async fn require_herdr_pane(
    state: &Arc<AppState>,
    session: &str,
    pane: &str,
) -> Result<()> {
    require_herdr_pane_with_adapter(&herdr_adapter(state), session, pane).await
}

fn herdr_adapter(state: &Arc<AppState>) -> HerdrTerminalAdapter {
    #[cfg(test)]
    if let Some(adapter) = super::peer::test_target::adapter(state) {
        return adapter;
    }
    HerdrTerminalAdapter::new(state.clone())
}

async fn require_herdr_pane_with_adapter(
    adapter: &HerdrTerminalAdapter,
    session: &str,
    pane: &str,
) -> Result<()> {
    let response = adapter
        .snapshot(session)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    require_herdr_pane_in_snapshot(&response, pane)
}

fn require_herdr_pane_in_snapshot(response: &serde_json::Value, pane: &str) -> Result<()> {
    let result = response.get("result").unwrap_or(response);
    // session.snapshot wraps its authoritative collections inside result.snapshot.
    // Never fall back to a different collection when that envelope is present.
    let panes = if let Some(snapshot) = result.get("snapshot") {
        snapshot.get("panes").and_then(serde_json::Value::as_array)
    } else {
        result
            .get("panes")
            .and_then(serde_json::Value::as_array)
            .or_else(|| result.get("session")?.get("panes")?.as_array())
    };
    ensure!(
        !pane.is_empty()
            && panes.is_some_and(|panes| panes
                .iter()
                .any(|item| item.get("pane_id").and_then(serde_json::Value::as_str) == Some(pane))),
        "Herdr pane不属于授权会话"
    );
    Ok(())
}

async fn attach_herdr(
    adapter: HerdrTerminalAdapter,
    grant: &Grant,
    pane: Option<String>,
    stream: String,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) -> Result<Backend> {
    let pane = pane.context("请选择Herdr pane")?;
    require_herdr_pane_with_adapter(&adapter, &grant.session, &pane).await?;
    let id = pane.clone();
    let session = grant.session.clone();
    let token = cancel.clone();
    let reader = adapter.clone();
    tokio::spawn(async move {
        let mut sequence = 0;
        let mut previous_snapshot: Option<Vec<u8>> = None;
        let mut output = OutputStream::new(stream.clone(), events.clone(), token.clone());
        loop {
            tokio::select! {()=token.cancelled()=>return,()=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}}
            let result = tokio::select! {()=token.cancelled()=>return,r=reader.read(&session,&id,sequence,1000,usize::MAX)=>r};
            match result {
                Ok(read) => {
                    for frame in read.frames {
                        sequence = frame.sequence;
                        if let Ok(bytes) =
                            base64::engine::general_purpose::STANDARD.decode(frame.data_base64)
                        {
                            let trimmed = bytes.len() > REPLAY_BYTES;
                            let bytes = &bytes[bytes.len().saturating_sub(REPLAY_BYTES)..];
                            // Herdr pane.read is a replacement screen and may omit a sequence.
                            // Repeated unchanged screens must not saturate the Relay business lane.
                            if previous_snapshot.as_deref() == Some(bytes) {
                                continue;
                            }
                            if output
                                .write(
                                    bytes,
                                    true,
                                    true,
                                    trimmed || read.truncated || read.replay_gap,
                                )
                                .await
                                .is_err()
                            {
                                return;
                            }
                            previous_snapshot = Some(bytes.to_vec());
                        }
                    }
                    if read.exited {
                        let _ = events
                            .send(Event::Error {
                                code: "HERDR_STREAM_ENDED".into(),
                                message: "Herdr pane已结束，请刷新状态确认退出原因".into(),
                                outcome_unknown: false,
                            })
                            .await;
                        return;
                    }
                }
                Err(error) => {
                    let _ = events
                        .send(Event::Error {
                            code: "HERDR_STREAM_ERROR".into(),
                            message: error.to_string(),
                            outcome_unknown: false,
                        })
                        .await;
                    return;
                }
            }
        }
    });
    Ok(Backend::Herdr(pane, adapter))
}
async fn attach_hc(
    state: &Arc<AppState>,
    grant: &Grant,
    read_only: bool,
    stream: String,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) -> Result<Backend> {
    let mut remote =
        crate::hc::remoted::attach(state.clone(), &grant.session, read_only, cancel.clone())
            .await?;
    let commands = remote.commands;
    let token = cancel.clone();
    tokio::spawn(async move {
        let mut output = OutputStream::new(stream.clone(), events.clone(), token.clone());
        loop {
            let message = tokio::select! {()=token.cancelled()=>return,m=remote.output.recv()=>m};
            match message {
                Some(Ok(bytes)) => {
                    if output.write(&bytes, false, false, false).await.is_err() {
                        return;
                    }
                }
                Some(Err(e)) => {
                    let _ = events
                        .send(Event::Error {
                            code: "HC_STREAM_ERROR".into(),
                            message: e.to_string(),
                            outcome_unknown: false,
                        })
                        .await;
                    return;
                }
                None => return,
            }
        }
    });
    Ok(Backend::Hc(commands))
}
fn managed_replay(terminal: &ManagedTerminal) -> OutputSnapshot {
    let buffer = terminal.output_buffer();
    let cursor = buffer.snapshot_after_bounded(u64::MAX, 1, 1).last_sequence;
    replay_tail(buffer.snapshot_after_bounded(
        cursor.saturating_sub(REPLAY_FRAMES as u64),
        usize::MAX,
        usize::MAX,
    ))
}

fn attach_managed(
    state: &Arc<AppState>,
    grant: &Grant,
    stream: String,
    events: mpsc::Sender<Event>,
    cancel: &CancellationToken,
) -> Result<Backend> {
    let terminal = state
        .sessions
        .terminal(&grant.session)?
        .context("终端尚未运行，请先在小橘中打开该会话")?;
    let mut source = terminal.subscribe();
    let snapshot = managed_replay(&terminal);
    let token = cancel.clone();
    let status_terminal = terminal.clone();
    tokio::spawn(async move {
        let mut sequence = snapshot.last_sequence;
        let mut output = OutputStream::new(stream.clone(), events.clone(), token.clone());
        if output.replay(snapshot).await.is_err() {
            return;
        }
        let mut revision = None;
        if output
            .current_programs(&status_terminal, &mut revision)
            .await
            .is_err()
        {
            return;
        }
        let mut status_poll = tokio::time::interval(std::time::Duration::from_millis(500));
        status_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let message = tokio::select! {
                () = token.cancelled() => return,
                _ = status_poll.tick() => {
                    if output.current_programs(&status_terminal, &mut revision).await.is_err() {
                        return;
                    }
                    continue;
                }
                event = source.recv() => event,
            };
            match message {
                Ok(TerminalEvent::Output(frame)) if frame.sequence > sequence => {
                    sequence = frame.sequence;
                    if output
                        .write(&frame.data, false, false, false)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if output
                        .current_programs(&status_terminal, &mut revision)
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(TerminalEvent::Exit(exit)) => {
                    if output
                        .current_programs(&status_terminal, &mut revision)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let _ = events
                        .send(Event::TerminalExit {
                            stream: stream.clone(),
                            code: exit.exit_code,
                        })
                        .await;
                    return;
                }
                Ok(TerminalEvent::Error(message)) => {
                    let _ = events
                        .send(Event::Error {
                            code: "TERMINAL_ERROR".into(),
                            message,
                            outcome_unknown: false,
                        })
                        .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = events
                        .send(Event::Error {
                            code: "OUTPUT_GAP".into(),
                            message: "输出接收落后，请重新附着获取当前画面".into(),
                            outcome_unknown: false,
                        })
                        .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                _ => {}
            }
        }
    });
    Ok(Backend::Managed(terminal))
}

impl Terminal {
    pub async fn attach(
        state: Arc<AppState>,
        grant: Grant,
        pane: Option<String>,
        read_only: bool,
        stream: String,
        events: mpsc::Sender<Event>,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let backend = match grant.backend.as_str() {
            "herdr" => {
                attach_herdr(
                    herdr_adapter(&state),
                    &grant,
                    pane,
                    stream,
                    events,
                    cancel.clone(),
                )
                .await?
            }
            "hc" => attach_hc(&state, &grant, read_only, stream, events, cancel.clone()).await?,
            _ => attach_managed(&state, &grant, stream, events, &cancel)?,
        };
        Ok(Self {
            grant,
            backend,
            controller: None,
            cancel,
            state,
            control_cancel: None,
        })
    }
    pub fn control(&mut self, take: bool) -> Result<bool> {
        if take {
            if self.controller.is_none() {
                self.controller = Some(
                    self.state
                        .terminal_control
                        .connect(&self.grant.session)?
                        .connection_id,
                );
                let state = self.state.clone();
                let session = self.grant.session.clone();
                let connection = self.controller.clone().context("controller missing")?;
                let cancel = self.cancel.child_token();
                self.control_cancel = Some(cancel.clone());
                tokio::spawn(async move {
                    cancel.cancelled().await;
                    let _ = state.terminal_control.disconnect(&session, &connection);
                });
            }
            self.state.terminal_control.take_control(
                &self.grant.session,
                self.controller.as_deref().context("controller missing")?,
            )?;
        } else if let Some(id) = self.controller.take() {
            if let Some(cancel) = self.control_cancel.take() {
                cancel.cancel();
            }
            self.state
                .terminal_control
                .disconnect(&self.grant.session, &id)?;
        }
        Ok(self.controls())
    }
    pub fn controls(&self) -> bool {
        self.controller.as_ref().is_some_and(|id| {
            self.state
                .terminal_control
                .is_controller(&self.grant.session, id)
        })
    }
    pub async fn input(&mut self, data: Vec<u8>) -> Result<()> {
        ensure!(self.controls(), "请先取得终端控制权");
        match &self.backend {
            Backend::Managed(t) => t.write_input(data),
            Backend::Herdr(pane, adapter) => adapter
                .send_input(&self.grant.session, pane, &data)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string())),
            Backend::Hc(tx) => {
                let (done, reply) = oneshot::channel();
                tx.try_send(crate::hc::remoted::Command::Input(data, done))?;
                reply.await.context("hc input worker stopped")?
            }
        }
    }
    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        ensure!(self.controls(), "请先取得终端控制权");
        crate::validation::validate_size(cols, rows).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        match &self.backend {
            Backend::Managed(t) => t.resize(cols, rows),
            Backend::Herdr(pane, adapter) => adapter
                .resize(&self.grant.session, pane, cols, rows)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string())),
            Backend::Hc(tx) => {
                let (done, reply) = oneshot::channel();
                tx.try_send(crate::hc::remoted::Command::Resize(cols, rows, done))?;
                reply.await.context("hc resize worker stopped")?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_manager::OutputFrame;

    #[tokio::test]
    async fn herdr_replacement_screens_emit_only_when_the_screen_changes() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let screen = Arc::new(std::sync::Mutex::new("initial screen".to_owned()));
        let source = screen.clone();
        let adapter = HerdrTerminalAdapter::with_test_rpc(state, move |_, operation| {
            let screen = source.clone();
            Box::pin(async move {
                match operation {
                    crate::herdr::HerdrTerminalOperation::Snapshot => Ok(
                        serde_json::json!({"result":{"snapshot":{"panes":[{"pane_id":"w1:p1"}]}}}),
                    ),
                    crate::herdr::HerdrTerminalOperation::Read { .. } => Ok(
                        serde_json::json!({"result":{"type":"pane_read","read":{"text":screen.lock().unwrap().clone()}}}),
                    ),
                    _ => Err("unexpected operation".into()),
                }
            })
        });
        let grant = Grant {
            id: "grant".into(),
            session: "session".into(),
            selector: "test@owner".into(),
            login: "user".into(),
            backend: "herdr".into(),
            name: "Herdr".into(),
            target_signature: String::new(),
            root: None,
            root_identity: None,
        };
        let cancel = CancellationToken::new();
        let (events, mut incoming) = mpsc::channel(8);
        let _backend = attach_herdr(
            adapter,
            &grant,
            Some("w1:p1".into()),
            "stream".into(),
            events,
            cancel.clone(),
        )
        .await
        .unwrap();
        let initial = tokio::time::timeout(std::time::Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(initial, Event::TerminalOutput { data, .. } if data == neko_remoted_protocol::encode(b"initial screen"))
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(350), incoming.recv())
                .await
                .is_err(),
            "unchanged sequence-less Herdr screens must not flood the Relay"
        );
        *screen.lock().unwrap() = "changed screen".into();
        let changed = tokio::time::timeout(std::time::Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(changed, Event::TerminalOutput { data, reset: true, .. } if data == neko_remoted_protocol::encode(b"changed screen"))
        );
        cancel.cancel();
    }

    #[test]
    fn herdr_pane_membership_accepts_the_session_snapshot_contract() {
        // Same envelope as herdr::tests::parses_workspace_and_tab_lists_from_herdr_snapshot_response.
        let response = serde_json::json!({
            "result": {
                "type": "session_snapshot",
                "snapshot": {
                    "focused_workspace_id": "w1",
                    "panes": [
                        {"pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1"},
                        {"pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1"}
                    ]
                }
            }
        });
        require_herdr_pane_in_snapshot(&response, "w1:p1").unwrap();
        require_herdr_pane_in_snapshot(&response, "w2:p1").unwrap();
        assert!(require_herdr_pane_in_snapshot(&response, "w1:p2").is_err());
    }

    #[test]
    fn herdr_pane_membership_keeps_supported_flat_and_session_envelopes() {
        for response in [
            serde_json::json!({"panes": [{"pane_id": "w1:p1"}]}),
            serde_json::json!({"result": {"panes": [{"pane_id": "w1:p1"}]}}),
            serde_json::json!({"result": {"session": {"panes": [{"pane_id": "w1:p1"}]}}}),
        ] {
            require_herdr_pane_in_snapshot(&response, "w1:p1").unwrap();
            assert!(require_herdr_pane_in_snapshot(&response, "w2:p1").is_err());
        }
    }

    #[test]
    fn herdr_pane_membership_never_uses_other_collections_or_partial_ids() {
        let response = serde_json::json!({"result": {
            "type": "session_snapshot",
            "snapshot": {"panes": [{"pane_id": "w1:p1"}]},
            "panes": [{"pane_id": "w2:p1"}],
            "session": {"panes": [{"pane_id": "w3:p1"}]}
        }});
        for pane in ["w2:p1", "w3:p1", "p1", "w1", "w1:p", " w1:p1", "w1:p1 ", ""] {
            assert!(require_herdr_pane_in_snapshot(&response, pane).is_err());
        }
        let other_session = serde_json::json!({"result": {
            "snapshot": {"panes": [{"pane_id": "w4:p1"}]}
        }});
        require_herdr_pane_in_snapshot(&other_session, "w4:p1").unwrap();
        assert!(require_herdr_pane_in_snapshot(&response, "w4:p1").is_err());
    }

    #[test]
    fn herdr_pane_membership_fails_closed_for_empty_or_malformed_snapshots() {
        for snapshot in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"panes": []}),
            serde_json::json!({"panes": {"pane_id": "w1:p1"}}),
            serde_json::json!({"panes": [{"id": "w1:p1"}, {"pane_id": null}, {"pane_id": ""}]}),
        ] {
            let response = serde_json::json!({"result": {
                "snapshot": snapshot,
                "panes": [{"pane_id": "w1:p1"}]
            }});
            assert!(require_herdr_pane_in_snapshot(&response, "w1:p1").is_err());
        }
    }

    #[tokio::test]
    async fn cancellation_releases_control_while_terminal_is_still_owned_by_target_worker() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let cancel = CancellationToken::new();
        let mut terminal = Terminal {
            grant: Grant {
                id: "grant".into(),
                session: "session".into(),
                selector: "target".into(),
                login: "user".into(),
                backend: "herdr".into(),
                name: "terminal".into(),
                target_signature: String::new(),
                root: None,
                root_identity: None,
            },
            backend: Backend::Herdr("pane".into(), HerdrTerminalAdapter::new(state.clone())),
            controller: None,
            cancel: cancel.clone(),
            state,
            control_cancel: None,
        };
        assert!(terminal.control(true).unwrap());
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while terminal.controls() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!terminal.controls());
        assert!(terminal.controller.is_some());
    }

    #[tokio::test]
    async fn split_frames_get_unique_sequences_and_only_initial_chunk_resets() {
        let (events, mut incoming) = mpsc::channel(8);
        let mut output = OutputStream::new("stream".into(), events, CancellationToken::new());
        let data = vec![42; neko_remoted_protocol::FILE_CHUNK * 2 + 1];
        output.write(&data, true, true, true).await.unwrap();
        output.write(b"live", false, false, false).await.unwrap();
        for expected in 1..=4 {
            let Event::TerminalOutput {
                sequence,
                reset,
                gap,
                ..
            } = incoming.recv().await.unwrap()
            else {
                panic!("expected output")
            };
            assert_eq!(sequence, expected);
            assert_eq!(reset, expected == 1);
            assert_eq!(gap, expected == 1);
        }
    }

    #[test]
    fn bounded_replay_keeps_newest_bytes_and_source_cursor() {
        let snapshot = OutputSnapshot {
            frames: (1..=150)
                .map(|sequence| OutputFrame {
                    sequence,
                    data: vec![u8::try_from(sequence).unwrap(); 4096].into(),
                })
                .collect(),
            oldest_sequence: Some(1),
            last_sequence: 150,
            truncated: false,
            replay_gap: false,
        };
        let snapshot = replay_tail(snapshot);
        assert_eq!(snapshot.last_sequence, 150);
        assert_eq!(snapshot.frames.last().unwrap().sequence, 150);
        assert_eq!(snapshot.frames.first().unwrap().sequence, 87);
        assert_eq!(
            snapshot
                .frames
                .iter()
                .map(|frame| frame.data.len())
                .sum::<usize>(),
            REPLAY_BYTES
        );
        assert!(snapshot.truncated && snapshot.replay_gap);
    }
}
