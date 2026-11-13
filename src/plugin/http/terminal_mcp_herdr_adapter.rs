use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::Value;

use crate::herdr::{HerdrTerminalOperation, run_terminal_mcp_operation};
use crate::state::AppState;

use crate::plugins::terminal_mcp::types::{
    TerminalMcpError, TerminalOutputFrame, TerminalReadResult,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HerdrPaneDescriptor {
    pub pane_id: String,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub title: String,
    pub focused: bool,
}

#[derive(Clone)]
pub struct HerdrTerminalAdapter {
    state: Arc<AppState>,
    #[cfg(test)]
    test_rpc: Option<Arc<TestRpc>>,
}

#[cfg(test)]
type TestRpc = dyn Fn(String, HerdrTerminalOperation) -> futures::future::BoxFuture<'static, Result<Value, String>>
    + Send
    + Sync;

impl HerdrTerminalAdapter {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            #[cfg(test)]
            test_rpc: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_rpc(
        state: Arc<AppState>,
        rpc: impl Fn(
            String,
            HerdrTerminalOperation,
        ) -> futures::future::BoxFuture<'static, Result<Value, String>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            state,
            test_rpc: Some(Arc::new(rpc)),
        }
    }

    async fn request(
        &self,
        session_id: &str,
        operation: HerdrTerminalOperation,
    ) -> Result<Value, String> {
        #[cfg(test)]
        if let Some(rpc) = &self.test_rpc {
            return rpc(session_id.to_owned(), operation).await;
        }
        run_terminal_mcp_operation(&self.state, session_id, operation).await
    }

    pub async fn snapshot(&self, session_id: &str) -> Result<Value, TerminalMcpError> {
        self.request(session_id, HerdrTerminalOperation::Snapshot)
            .await
            .map_err(map_herdr_error)
    }

    pub async fn list_panes(
        &self,
        session_id: &str,
    ) -> Result<Vec<HerdrPaneDescriptor>, TerminalMcpError> {
        let response = self.snapshot(session_id).await?;
        Ok(collection(&response, "panes")
            .into_iter()
            .filter_map(|pane| {
                let pane_id = string_field(pane, "pane_id")?;
                Some(HerdrPaneDescriptor {
                    pane_id,
                    workspace_id: string_field(pane, "workspace_id"),
                    tab_id: string_field(pane, "tab_id"),
                    title: string_field(pane, "terminal_title_stripped")
                        .or_else(|| string_field(pane, "terminal_title"))
                        .or_else(|| string_field(pane, "title"))
                        .unwrap_or_else(|| "Herdr pane".to_owned()),
                    focused: bool_field(pane, "focused"),
                })
            })
            .collect())
    }

    pub async fn read(
        &self,
        session_id: &str,
        pane_id: &str,
        after_sequence: u64,
        wait_ms: u64,
        max_bytes: usize,
    ) -> Result<TerminalReadResult, TerminalMcpError> {
        let mut response = self
            .request(
                session_id,
                HerdrTerminalOperation::Read {
                    pane_id: pane_id.to_owned(),
                },
            )
            .await
            .map_err(map_herdr_error)?;
        let mut timed_out = false;
        if !response_has_new_output(&response, after_sequence) && wait_ms > 0 {
            let waited = self
                .request(
                    session_id,
                    HerdrTerminalOperation::WaitForOutput {
                        pane_id: pane_id.to_owned(),
                        after_sequence,
                        timeout_ms: wait_ms,
                    },
                )
                .await
                .map_err(map_herdr_error)?;
            timed_out = bool_field(result(&waited), "timed_out");
            response = self
                .request(
                    session_id,
                    HerdrTerminalOperation::Read {
                        pane_id: pane_id.to_owned(),
                    },
                )
                .await
                .map_err(map_herdr_error)?;
        }
        Ok(normalize_read(
            session_id,
            pane_id,
            after_sequence,
            max_bytes,
            timed_out,
            &response,
        ))
    }

    /// Read a complete styled viewport, including empty replacements, without delta/history gaps.
    pub async fn read_ansi(
        &self,
        session_id: &str,
        pane_id: &str,
        after_sequence: u64,
        max_bytes: usize,
    ) -> Result<TerminalReadResult, TerminalMcpError> {
        let response = self
            .request(
                session_id,
                HerdrTerminalOperation::ReadAnsi {
                    pane_id: pane_id.to_owned(),
                },
            )
            .await
            .map_err(map_herdr_error)?;
        let value = read_result(&response);
        let data = data_field(value).ok_or_else(|| {
            TerminalMcpError::new(
                "BACKEND_OPERATION_FAILED",
                "Herdr visible read missing text",
            )
        })?;
        if data.len() > max_bytes || bool_field(value, "truncated") {
            return Err(TerminalMcpError::new(
                "BACKEND_OPERATION_FAILED",
                "Herdr visible screen is incomplete or exceeds the snapshot size limit",
            ));
        }
        if value.get("format").is_some_and(|format| format != "ansi")
            || value
                .get("source")
                .is_some_and(|source| source != "visible")
        {
            return Err(TerminalMcpError::new(
                "BACKEND_OPERATION_FAILED",
                "Herdr did not return the requested visible ANSI screen",
            ));
        }
        // pane.read's revision is not an output-stream sequence. Every successful
        // read is a replacement; the terminal bridge deduplicates its full bytes.
        let sequence = after_sequence.checked_add(1).ok_or_else(|| {
            TerminalMcpError::new(
                "BACKEND_OPERATION_FAILED",
                "Herdr screen sequence exhausted",
            )
        })?;
        Ok(TerminalReadResult {
            session_id: session_id.to_owned(),
            pane_id: Some(pane_id.to_owned()),
            frames: vec![TerminalOutputFrame {
                sequence,
                data_base64: BASE64_STANDARD.encode(data),
            }],
            next_sequence: sequence,
            last_sequence: sequence,
            oldest_sequence: Some(sequence),
            timed_out: false,
            truncated: false,
            replay_gap: false,
            exited: bool_field(value, "exited"),
        })
    }

    pub async fn send_text(
        &self,
        session_id: &str,
        pane_id: &str,
        text: String,
    ) -> Result<(), TerminalMcpError> {
        self.request(
            session_id,
            HerdrTerminalOperation::SendText {
                pane_id: pane_id.to_owned(),
                text,
            },
        )
        .await
        .map(|_| ())
        .map_err(map_herdr_error)
    }

    pub async fn send_keys(
        &self,
        session_id: &str,
        pane_id: &str,
        keys: Vec<String>,
    ) -> Result<(), TerminalMcpError> {
        self.request(
            session_id,
            HerdrTerminalOperation::SendKeys {
                pane_id: pane_id.to_owned(),
                keys,
            },
        )
        .await
        .map(|_| ())
        .map_err(map_herdr_error)
    }

    pub async fn send_input(
        &self,
        session_id: &str,
        pane_id: &str,
        data: &[u8],
    ) -> Result<(), TerminalMcpError> {
        self.request(
            session_id,
            HerdrTerminalOperation::SendInput {
                pane_id: pane_id.to_owned(),
                data_base64: BASE64_STANDARD.encode(data),
            },
        )
        .await
        .map(|_| ())
        .map_err(map_herdr_error)
    }

    pub async fn resize(
        &self,
        session_id: &str,
        pane_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), TerminalMcpError> {
        self.request(
            session_id,
            HerdrTerminalOperation::Resize {
                pane_id: pane_id.to_owned(),
                cols,
                rows,
            },
        )
        .await
        .map(|_| ())
        .map_err(map_herdr_error)
    }
}

fn normalize_read(
    session_id: &str,
    pane_id: &str,
    after_sequence: u64,
    max_bytes: usize,
    timed_out: bool,
    response: &Value,
) -> TerminalReadResult {
    let value = read_result(response);
    let sequence = sequence_field(value).unwrap_or_else(|| after_sequence.saturating_add(1));
    let mut data = data_field(value).unwrap_or_default();
    let has_new_output = !data.is_empty() && sequence > after_sequence;
    let truncated = data.len() > max_bytes || bool_field(value, "truncated");
    if truncated {
        data.truncate(max_bytes);
    }
    let frames = has_new_output
        .then(|| TerminalOutputFrame {
            sequence,
            data_base64: BASE64_STANDARD.encode(data),
        })
        .into_iter()
        .collect::<Vec<_>>();
    let next_sequence = frames.last().map_or(after_sequence, |frame| frame.sequence);
    TerminalReadResult {
        session_id: session_id.to_owned(),
        pane_id: Some(pane_id.to_owned()),
        frames,
        next_sequence,
        last_sequence: sequence.max(after_sequence),
        oldest_sequence: has_new_output.then_some(sequence),
        timed_out: timed_out && !has_new_output,
        truncated,
        replay_gap: has_new_output && sequence > after_sequence.saturating_add(1),
        exited: bool_field(value, "exited"),
    }
}

fn response_has_new_output(response: &Value, after_sequence: u64) -> bool {
    let value = read_result(response);
    data_field(value).is_some_and(|data| !data.is_empty())
        && sequence_field(value).is_none_or(|sequence| sequence > after_sequence)
}

fn read_result(value: &Value) -> &Value {
    let value = result(value);
    value.get("read").unwrap_or(value)
}

fn result(value: &Value) -> &Value {
    value.get("result").unwrap_or(value)
}

fn collection<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    let value = result(value);
    value
        .get(key)
        .and_then(Value::as_array)
        .or_else(|| {
            value
                .get("session")
                .and_then(|session| session.get(key))
                .and_then(Value::as_array)
        })
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn data_field(value: &Value) -> Option<Vec<u8>> {
    if let Some(encoded) = string_field(value, "data_base64") {
        return BASE64_STANDARD.decode(encoded).ok();
    }
    ["text", "content", "output", "data"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            value.get("pane").and_then(|pane| {
                ["text", "content", "output", "data"]
                    .into_iter()
                    .find_map(|key| pane.get(key).and_then(Value::as_str).map(str::to_owned))
            })
        })
        .map(String::into_bytes)
}

fn sequence_field(value: &Value) -> Option<u64> {
    ["sequence", "output_sequence", "last_sequence"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_u64))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn map_herdr_error(message: String) -> TerminalMcpError {
    if message.to_ascii_lowercase().contains("pane")
        && message.to_ascii_lowercase().contains("not found")
    {
        TerminalMcpError::pane_not_found()
    } else {
        TerminalMcpError::new("BACKEND_OPERATION_FAILED", message)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn ansi_terminal_reads_preserve_full_styles_and_empty_resets_without_changing_plain_reads()
     {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let source = Arc::new(std::sync::Mutex::new(
            "\x1b[38;2;12;34;56mRED\x1b[0m".to_owned(),
        ));
        let screen = source.clone();
        let adapter = HerdrTerminalAdapter::with_test_rpc(state, move |_, operation| {
            let screen = screen.clone();
            Box::pin(async move {
                match operation {
                    HerdrTerminalOperation::ReadAnsi { .. } => Ok(
                        json!({"read":{"text":screen.lock().unwrap().clone(),"source":"visible","format":"ansi","sequence":999,"truncated":false}}),
                    ),
                    HerdrTerminalOperation::Read { .. } => {
                        Ok(json!({"read":{"text":"RED","source":"recent","format":"text"}}))
                    }
                    _ => Err("unexpected operation".into()),
                }
            })
        });
        let plain = adapter.read("session", "w1:p1", 0, 0, 1024).await.unwrap();
        assert_eq!(
            BASE64_STANDARD
                .decode(&plain.frames[0].data_base64)
                .unwrap(),
            b"RED"
        );
        let ansi = adapter
            .read_ansi("session", "w1:p1", 4, 1024)
            .await
            .unwrap();
        assert_eq!(ansi.next_sequence, 5);
        assert!(!ansi.replay_gap && !ansi.truncated);
        assert_eq!(
            BASE64_STANDARD.decode(&ansi.frames[0].data_base64).unwrap(),
            source.lock().unwrap().as_bytes()
        );
        source.lock().unwrap().clear();
        let empty = adapter
            .read_ansi("session", "w1:p1", 5, 1024)
            .await
            .unwrap();
        assert_eq!(empty.frames.len(), 1);
        assert!(empty.frames[0].data_base64.is_empty());
        assert!(!empty.replay_gap);
    }

    #[tokio::test]
    async fn ansi_terminal_reads_reject_partial_and_oversized_snapshots_instead_of_slicing() {
        let source = Arc::new(std::sync::Mutex::new(
            json!({"text":"partial","truncated":true}),
        ));
        let screen = source.clone();
        let adapter = HerdrTerminalAdapter::with_test_rpc(
            Arc::new(AppState::new_for_test(":memory:".into())),
            move |_, _| {
                let screen = screen.clone();
                Box::pin(async move { Ok(json!({"read":screen.lock().unwrap().clone()})) })
            },
        );
        for value in [
            json!({"text":"partial","truncated":true}),
            json!({"text":"too long"}),
            json!({"text":"ok","format":"text"}),
            json!({"text":"ok","source":"recent"}),
            json!({}),
        ] {
            *source.lock().unwrap() = value;
            assert!(adapter.read_ansi("session", "w1:p1", 0, 4).await.is_err());
        }
    }

    #[tokio::test]
    async fn default_adapter_still_rejects_unknown_sessions() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        assert!(
            HerdrTerminalAdapter::new(state)
                .snapshot("unknown-session")
                .await
                .is_err()
        );
    }

    #[test]
    fn reads_the_genuine_herdr_pane_read_envelope() {
        let response = json!({"id":"inspect","result":{"type":"pane_read","read":{
            "pane_id":"w1:p1","text":"real-pty-output\n","revision":0,"truncated":false
        }}});
        assert!(response_has_new_output(&response, 0));
        let read = normalize_read("session", "w1:p1", 0, 262_144, false, &response);
        assert_eq!(read.frames.len(), 1);
        assert_eq!(
            read.frames[0].data_base64,
            BASE64_STANDARD.encode("real-pty-output\n")
        );
    }

    #[test]
    fn normalizes_bounded_herdr_reads() {
        let read = normalize_read(
            "session-one",
            "pane-one",
            4,
            3,
            false,
            &json!({"result":{"sequence":6,"text":"hello"}}),
        );

        assert_eq!(read.next_sequence, 6);
        assert!(read.replay_gap);
        assert!(read.truncated);
        assert_eq!(read.frames[0].data_base64, BASE64_STANDARD.encode("hel"));
    }
}
