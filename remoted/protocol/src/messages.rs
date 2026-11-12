use crate::crypto::Mode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Relay-supplied peer IDs are connection IDs, never authenticated device identities.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RelayFrame {
    Connected {
        peer: String,
    },
    Opened {
        peer: String,
    },
    Hello {
        peer: String,
        mode: Mode,
    },
    Data {
        peer: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<u64>,
    },
    Acknowledge {
        peer: String,
        sequence: u64,
    },
    Delivered {
        peer: String,
        count: u8,
    },
    Closed {
        peer: String,
    },
    Ping {
        nonce: String,
    },
    Pong {
        nonce: String,
    },
    Error {
        code: String,
    },
}

/// Backend chosen when creating a session. Existing resources keep their own backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionBackend {
    #[default]
    Native,
    Hc,
    Herdr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadOnly,
    Operate,
}
impl Permission {
    pub fn can_write(self) -> bool {
        self == Self::Operate
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// Host-issued opaque grant ID. Does not encode a selector.
    pub id: String,
    pub name: String,
    pub backend: String,
    pub terminal: bool,
    pub herdr: bool,
    pub files: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub operation: Operation,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Identify {
        name: String,
    },
    AcknowledgePairing {},
    Ping {
        nonce: String,
    },
    Pong {
        nonce: String,
    },
    Resources {},
    TerminalAttach {
        grant: String,
        pane: Option<String>,
    },
    TerminalControl {
        stream: String,
        take: bool,
    },
    TerminalInput {
        stream: String,
        data: String,
    },
    TerminalResize {
        stream: String,
        cols: u16,
        rows: u16,
    },
    TerminalDetach {
        stream: String,
    },
    Herdr {
        grant: String,
        method: String,
        params: Value,
    },
    FileList {
        grant: String,
        path: String,
    },
    FileDownload {
        grant: String,
        path: String,
    },
    FileUpload {
        grant: String,
        path: String,
        size: u64,
        sha256: String,
        overwrite: bool,
    },
    FileChunk {
        transfer: String,
        offset: u64,
        data: String,
    },
    FileCommit {
        transfer: String,
    },
    TransferAcknowledge {
        transfer: String,
        offset: u64,
    },
    Cancel {
        transfer: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub id: u64,
    pub event: Event,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    Unknown,
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedKind {
    Permission,
    Question,
    Auth,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramRecord {
    pub id: String,
    pub state: ProgramState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<BlockedKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Pending {
        fingerprint: String,
    },
    Approved {
        host_key: String,
        device_id: String,
        permission: Permission,
    },
    Ready {
        device_id: String,
        permission: Permission,
        resources: Vec<Resource>,
    },
    Pong {
        nonce: String,
    },
    Ping {
        nonce: String,
    },
    Resources {
        resources: Vec<Resource>,
    },
    TerminalAttached {
        stream: String,
        name: String,
        read_only: bool,
    },
    TerminalOutput {
        stream: String,
        sequence: u64,
        data: String,
        replay: bool,
        reset: bool,
        gap: bool,
    },
    TerminalExit {
        stream: String,
        code: i32,
    },
    /// This attachment stopped without a confirmed numeric process exit.
    /// The underlying persistent terminal may still be running.
    TerminalUnavailable {
        stream: String,
        message: String,
    },
    TerminalProgramStatus {
        stream: String,
        records: Vec<ProgramRecord>,
    },
    TerminalControl {
        stream: String,
        controller: bool,
    },
    Herdr {
        data: Value,
    },
    FileList {
        path: String,
        entries: Vec<FileEntry>,
        truncated: bool,
    },
    TransferStarted {
        transfer: String,
        size: u64,
    },
    FileChunk {
        transfer: String,
        offset: u64,
        data: String,
    },
    TransferProgress {
        transfer: String,
        offset: u64,
    },
    TransferComplete {
        transfer: String,
        size: u64,
        sha256: String,
    },
    Cancelled {
        transfer: String,
    },
    Ok,
    Error {
        code: String,
        message: String,
        outcome_unknown: bool,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub name: String,
    pub directory: bool,
    pub size: u64,
}

/// Explicit remote subset of the host's compatibility allowlist. Unknown means denied.
pub fn herdr_allowed(permission: Permission, method: &str) -> bool {
    matches!(
        method,
        "session.snapshot"
            | "workspace.list"
            | "workspace.get"
            | "tab.list"
            | "tab.get"
            | "pane.list"
            | "pane.get"
            | "pane.read"
            | "pane.process_info"
            | "agent.list"
            | "agent.get"
            | "agent.read"
    ) || (permission.can_write()
        && matches!(
            method,
            "pane.send_text" | "pane.send_keys" | "pane.send_input" | "pane.resize"
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_program_status_roundtrips_a_typed_replacement_table() {
        let wire = serde_json::json!({
            "type": "terminal_program_status", "stream": "terminal-1",
            "records": [{"id": "agent", "state": "unknown", "app": "codex"}]
        });
        let event: Event = serde_json::from_value(wire.clone()).unwrap();
        let Event::TerminalProgramStatus { stream, records } = &event else {
            panic!("expected program status");
        };
        assert_eq!(stream, "terminal-1");
        assert_eq!(records[0].state, ProgramState::Unknown);
        assert_eq!(records[0].app.as_deref(), Some("codex"));
        assert_eq!(serde_json::to_value(event).unwrap(), wire);
        let empty: Event = serde_json::from_value(serde_json::json!({
            "type": "terminal_program_status", "stream": "terminal-1", "records": []
        }))
        .unwrap();
        assert!(
            matches!(empty, Event::TerminalProgramStatus { records, .. } if records.is_empty())
        );
    }

    #[test]
    fn terminal_program_status_rejects_missing_states_and_unknown_fields() {
        for record in [
            serde_json::json!({"id": "agent"}),
            serde_json::json!({"id": "agent", "state": "future_state"}),
            serde_json::json!({"id": "agent", "state": "working", "activity": "idle"}),
            serde_json::json!({"id": "agent", "state": "blocked", "kind": "future_kind"}),
        ] {
            assert!(
                serde_json::from_value::<Event>(serde_json::json!({
                    "type": "terminal_program_status", "stream": "s", "records": [record]
                }))
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<Event>(serde_json::json!({
                "type": "terminal_output", "stream": "s", "sequence": 1, "data": "",
                "replay": false, "reset": false, "gap": false, "records": []
            }))
            .is_err()
        );
    }

    #[test]
    fn older_event_decoders_reject_program_status_instead_of_discarding_it() {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum OlderEvent {
            Ok,
        }
        let event = Event::TerminalProgramStatus {
            stream: "s".into(),
            records: Vec::new(),
        };
        assert!(
            serde_json::from_value::<OlderEvent>(serde_json::to_value(event).unwrap()).is_err()
        );
    }

    #[test]
    fn readonly_is_default_deny_not_prefix_matching() {
        for method in [
            "server.stop",
            "server.live_handoff",
            "pane.close",
            "plugin.action.invoke",
            "pane.read.extra",
            "integration.install",
            "events.subscribe",
            "agent.prompt",
            "",
        ] {
            assert!(!herdr_allowed(Permission::ReadOnly, method));
            assert!(!herdr_allowed(Permission::Operate, method));
        }
        for method in [
            "pane.send_text",
            "pane.send_keys",
            "pane.send_input",
            "pane.resize",
        ] {
            assert!(!herdr_allowed(Permission::ReadOnly, method));
            assert!(herdr_allowed(Permission::Operate, method));
        }
        assert!(herdr_allowed(Permission::ReadOnly, "pane.read"));
    }
    #[test]
    fn rejects_identity_injection_and_unknown_operations() {
        for json in [
            r#"{"id":1,"operation":{"type":"resources","selector":"other@owner"}}"#,
            r#"{"id":1,"user_id":"admin","operation":{"type":"resources"}}"#,
            r#"{"id":1,"operation":{"type":"exec","command":"id"}}"#,
        ] {
            assert!(crate::unpack::<Request>(json.as_bytes()).is_err());
        }
    }
}
