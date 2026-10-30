use anyhow::{Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

#[derive(Default)]
pub(super) struct EventLines {
    pub(super) partial: Vec<u8>,
    pub sequence: u64,
}

impl EventLines {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        let mut events = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.partial.len() + part.len() > 1024 * 1024 || events.len() >= 1024 {
                bail!("hc event stream exceeds its bounded line budget");
            }
            self.partial.extend_from_slice(part);
            if part.ends_with(b"\n") {
                let value = serde_json::from_slice(&self.partial)?;
                events.push(value);
                self.partial.clear();
            }
        }
        Ok(events)
    }
}

pub(super) fn terminal_name(session_id: &str) -> String {
    let digest = digest_hex(session_id.as_bytes());
    format!("n-{}", &digest[..30])
}

pub(super) fn digest_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}

pub(super) fn resolve_terminal(
    terminals: &[Value],
    provider: &str,
    session: &str,
    attempted: bool,
    expected: Option<&str>,
) -> Result<Option<Value>> {
    let mut matches = terminals.iter().filter(|terminal| {
        terminal["labels"]["neko_provider"] == provider
            && terminal["labels"]["neko_session"] == session
    });
    let Some(terminal) = matches.next() else {
        if attempted || expected.is_some() {
            bail!(
                "hc creation result is unknown or terminal was removed; creation was not retried"
            );
        }
        return Ok(None);
    };
    let address = terminal["address"].as_str().unwrap_or_default();
    if matches.next().is_some()
        || local_terminal_id(address).is_none()
        || expected.is_some_and(|expected| expected != address)
    {
        bail!("hc terminal identity does not match the saved session");
    }
    Ok(Some(terminal.clone()))
}

// Match upstream terminals::{address, parse_id}: first digit 1–9, then
// base-30 digits/consonants (no vowels or l), with a positive u32 counter.
fn local_terminal_id(address: &str) -> Option<u32> {
    const SYMBOLS: &[u8] = b"0123456789bcdfghjkmnpqrstvwxyz";
    let mut bytes = address.strip_prefix('t')?.bytes();
    let first = bytes.next()?;
    if !matches!(first, b'1'..=b'9') {
        return None;
    }
    let mut rest = u64::from(first - b'1');
    let (mut before, mut size) = (0_u64, 9_u64);
    for byte in bytes {
        let digit = u64::try_from(SYMBOLS.iter().position(|symbol| *symbol == byte)?).ok()?;
        rest = rest.checked_mul(30)?.checked_add(digit)?;
        before = before.checked_add(size)?;
        size = size.checked_mul(30)?;
    }
    u32::try_from(before.checked_add(rest)?.checked_add(1)?).ok()
}

pub(super) fn program_records(terminal: &Value) -> Vec<Value> {
    let mut records = Vec::new();
    if let Some(record) = status_record("", &terminal["status"]) {
        records.push(record);
    }
    if let Some(children) = terminal["records"].as_object() {
        for (id, status) in children.iter().take(63) {
            if let Some(record) = status_record(id, status) {
                records.push(record);
            }
        }
    }
    records
}

fn status_record(id: &str, status: &Value) -> Option<Value> {
    let state = status["state"].as_str()?;
    if !matches!(state, "idle" | "working" | "done" | "blocked" | "error") {
        return None;
    }
    let mut record = serde_json::Map::new();
    record.insert("id".into(), Value::String(id.into()));
    record.insert("state".into(), Value::String(state.into()));
    if !valid_id(id) {
        return None;
    }
    if let Some(app) = status["app"]
        .as_str()
        .filter(|app| !app.is_empty() && valid_segment(app))
    {
        record.insert("app".into(), Value::String(app.into()));
    }
    if let Some(kind) = status["kind"]
        .as_str()
        .filter(|kind| matches!(*kind, "permission" | "question" | "auth"))
    {
        record.insert("kind".into(), Value::String(kind.into()));
    }
    for (key, limit) in [("title", 192), ("msg", 2048)] {
        if let Some(value) = status[key].as_str() {
            let mut text = String::new();
            for ch in value.chars().filter(|ch| !ch.is_control()) {
                if text.len() + ch.len_utf8() > limit {
                    break;
                }
                text.push(ch);
            }
            record.insert(key.into(), Value::String(text));
        }
    }
    if let Some(value) = status["progress"].as_u64().filter(|n| *n <= 100) {
        record.insert("progress".into(), Value::from(value));
    }
    Some(Value::Object(record))
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte))
}

fn valid_id(id: &str) -> bool {
    id.is_empty()
        || (id.len() <= 128 && id.split('/').count() <= 8 && id.split('/').all(valid_segment))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn event_lines_survive_split_and_coalesced_frames() {
        let mut decoder = super::EventLines::default();
        assert!(decoder.push(b"{\"params\":{\"seq\":").unwrap().is_empty());
        let values = decoder.push(b"2}}\n{\"params\":{\"seq\":3}}\n").unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["params"]["seq"], 2);
        assert_eq!(values[1]["params"]["seq"], 3);
    }

    #[test]
    fn status_tables_fit_browser_contract_and_strip_control_data() {
        let terminal = json!({
            "status":{"state":"working","app":"bad app","title":"界".repeat(100),"msg":"a\u{1b}\n".repeat(1500),"kind":"unknown","progress":101},
            "records":{"valid/child":{"state":"blocked","kind":"permission","app":"claude"},
                "bad id":{"state":"working"},"nope":{"state":"not-a-state"}}
        });
        let records = super::program_records(&terminal);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["id"], "");
        assert_eq!(records[0]["title"].as_str().unwrap().len(), 192);
        assert!(records[0]["msg"].as_str().unwrap().len() <= 2048);
        assert!(
            !records[0]["msg"]
                .as_str()
                .unwrap()
                .chars()
                .any(char::is_control)
        );
        for key in ["app", "kind", "progress"] {
            assert!(records[0].get(key).is_none());
        }
        assert_eq!(records[1]["id"], "valid/child");
        assert_eq!(records[1]["kind"], "permission");
    }

    #[test]
    fn event_stream_rejects_unbounded_partial_lines() {
        let mut decoder = super::EventLines::default();
        assert!(decoder.push(&vec![b' '; 1024 * 1024]).unwrap().is_empty());
        assert!(decoder.push(b" ").is_err());
    }

    #[test]
    fn unknown_creation_is_not_repeated_after_missing_result() {
        assert!(super::resolve_terminal(&[], "provider", "session", true, None).is_err());
        assert!(
            super::resolve_terminal(&[], "provider", "session", false, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reconcile_requires_exact_owner_and_session_and_address() {
        let terminal = json!({"address":"t7","labels":{"neko_provider":"provider","neko_session":"session"},"state":"live"});
        assert_eq!(
            super::resolve_terminal(
                std::slice::from_ref(&terminal),
                "provider",
                "session",
                true,
                Some("t7")
            )
            .unwrap(),
            Some(terminal.clone())
        );
        assert!(
            super::resolve_terminal(
                std::slice::from_ref(&terminal),
                "provider",
                "session",
                true,
                Some("t8")
            )
            .is_err()
        );
        assert!(super::resolve_terminal(&[terminal], "other", "session", true, None).is_err());
    }

    #[test]
    fn finished_terminal_is_not_a_reason_to_start_another_program() {
        let terminal = json!({"address":"t7","labels":{"neko_provider":"provider","neko_session":"session"},"state":"finished","exit_code":7});
        let resolved = super::resolve_terminal(&[terminal], "provider", "session", true, None)
            .unwrap()
            .unwrap();
        assert_eq!(resolved["exit_code"], 7);
    }

    #[test]
    fn canonical_addresses_after_nine_reconcile_and_keep_exact_identity() {
        // Counter/address pairs from upstream's canonical address tests.
        for (counter, address) in [
            (1, "t1"),
            (9, "t9"),
            (10, "t10"),
            (19, "t19"),
            (20, "t1b"),
            (39, "t1z"),
            (40, "t20"),
            (279, "t9z"),
            (280, "t100"),
            (8379, "t9zz"),
            (8380, "t1000"),
            (148_377, "t65jk"),
            (u32::MAX, "t6kf3jk5"),
        ] {
            assert_eq!(
                super::local_terminal_id(address),
                Some(counter),
                "{address}"
            );
            let terminal = json!({"address":address,"labels":{"neko_provider":"provider","neko_session":"session"}});
            for expected in [None, Some(address)] {
                assert_eq!(
                    super::resolve_terminal(
                        std::slice::from_ref(&terminal),
                        "provider",
                        "session",
                        true,
                        expected
                    )
                    .unwrap(),
                    Some(terminal.clone())
                );
            }
            assert!(
                super::resolve_terminal(&[terminal], "provider", "session", true, Some("t8"))
                    .is_err(),
                "different saved address must fail: {address}"
            );
        }
    }

    #[test]
    fn noncanonical_or_remote_addresses_cannot_reconcile() {
        for address in [
            "",
            "t",
            "t0",
            "t01",
            "t001b",
            "tb",
            "t1l",
            "t1a",
            "t1e",
            "t1i",
            "t1o",
            "t1u",
            "T1",
            "t1B",
            "x1",
            "tmp",
            "t3po",
            " t1b",
            "t1b ",
            "t1b\n",
            "machine/t1b",
            "/t1b",
            "t1b/",
            "t1b/child",
            "t1-b",
            "t1_b",
            "t1.b",
            "t1é",
            "t6kf3jk6", // First counter above u32::MAX.
            "t9zzzzzzzzzz",
            "t9999999999999999999999999999999999999999999999",
        ] {
            let terminal = json!({"address":address,"labels":{"neko_provider":"provider","neko_session":"session"}});
            assert!(
                super::resolve_terminal(&[terminal], "provider", "session", true, Some(address))
                    .is_err(),
                "noncanonical address accepted: {address:?}"
            );
        }
    }

    #[test]
    fn duplicate_and_remote_addresses_fail_closed() {
        let terminal = json!({"address":"t7","labels":{"neko_provider":"provider","neko_session":"session"},"state":"live"});
        assert!(
            super::resolve_terminal(
                &[terminal.clone(), terminal],
                "provider",
                "session",
                true,
                None
            )
            .is_err()
        );
        let remote = json!({"address":"machine/t7","labels":{"neko_provider":"provider","neko_session":"session"},"state":"live"});
        assert!(super::resolve_terminal(&[remote], "provider", "session", true, None).is_err());
    }
}
