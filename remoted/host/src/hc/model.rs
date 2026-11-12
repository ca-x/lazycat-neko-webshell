//! Persisted creation intents and the small, app-owned part of core's records.
use anyhow::{Context, Result, ensure};
use neko_remoted_protocol::messages::{BlockedKind, ProgramRecord, ProgramState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedHc {
    pub namespace: String,
    pub core_id: Option<String>,
    pub terminals: Vec<Record>,
}
impl Default for SavedHc {
    fn default() -> Self {
        Self {
            namespace: uuid::Uuid::new_v4().to_string(),
            core_id: None,
            terminals: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    // This UUID is both the opaque grant and the durable, never-replayed run intent.
    pub grant: String,
    pub name: String,
    pub address: Option<String>,
}
impl SavedHc {
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.namespace).context("invalid saved hc namespace")?;
        ensure!(self.terminals.len() <= 32, "too many saved hc terminals");
        if let Some(id) = &self.core_id {
            ensure!(
                !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
                "invalid saved core identity"
            );
        }
        ensure!(
            self.terminals.is_empty() || self.core_id.is_some(),
            "hc intents require a saved core identity"
        );
        let mut grants = HashSet::new();
        let mut addresses = HashSet::new();
        for record in &self.terminals {
            uuid::Uuid::parse_str(&record.grant)?;
            crate::state::check_name(&record.name)?;
            ensure!(grants.insert(&record.grant), "duplicate hc grant");
            if let Some(address) = &record.address {
                ensure!(
                    local_address(address) && addresses.insert(address),
                    "invalid or duplicate hc address"
                );
            }
        }
        Ok(())
    }
    pub fn labels(&self, record: &Record) -> Value {
        json!({"neoko_host":self.namespace,"neoko_session":record.grant})
    }
    pub fn verify<'a>(&self, record: &Record, terminal: &'a Value) -> Result<&'a str> {
        let address = terminal["address"]
            .as_str()
            .context("hc omitted terminal address")?;
        ensure!(
            local_address(address)
                && record
                    .address
                    .as_deref()
                    .is_none_or(|saved| saved == address)
                && terminal["labels"]["neoko_host"] == self.namespace
                && terminal["labels"]["neoko_session"] == record.grant,
            "hc terminal identity differs from its saved grant"
        );
        Ok(address)
    }
    pub fn resolve(&self, record: &Record, list: &Value) -> Result<Option<Value>> {
        let terminals = list["terminals"]
            .as_array()
            .context("hc omitted terminal list")?;
        let mut matching = terminals.iter().filter(|t| {
            t["labels"]["neoko_host"] == self.namespace
                && t["labels"]["neoko_session"] == record.grant
        });
        let Some(terminal) = matching.next() else {
            ensure!(
                record.address.is_none(),
                "saved hc terminal is missing; it was not recreated"
            );
            return Ok(None);
        };
        ensure!(matching.next().is_none(), "ambiguous hc terminal labels");
        self.verify(record, terminal)?;
        Ok(Some(terminal.clone()))
    }
}
// Local core addresses only; remote machine names are never accepted from peers.
fn local_address(address: &str) -> bool {
    let Some(id) = address.strip_prefix('t') else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 16
        && matches!(id.as_bytes()[0], b'1'..=b'9')
        && id
            .bytes()
            .all(|b| b"0123456789bcdfghjkmnpqrstvwxyz".contains(&b))
}

fn text(value: &Value, limit: usize) -> Option<String> {
    let value = value.as_str()?;
    let mut result = String::new();
    for c in value.chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > limit {
            break;
        }
        result.push(c);
    }
    Some(result)
}
pub(super) fn programs(terminal: &Value) -> Vec<ProgramRecord> {
    let mut records = Vec::new();
    let mut add = |id: &str, status: &Value| {
        if id.len() > 128 || id.chars().any(char::is_control) {
            return;
        }
        let state = match status["state"].as_str() {
            Some("idle") => ProgramState::Idle,
            Some("working") => ProgramState::Working,
            Some("done") => ProgramState::Done,
            Some("blocked") => ProgramState::Blocked,
            Some("error") => ProgramState::Error,
            _ => return,
        };
        records.push(ProgramRecord {
            id: id.into(),
            state,
            kind: match status["kind"].as_str() {
                Some("permission") => Some(BlockedKind::Permission),
                Some("question") => Some(BlockedKind::Question),
                Some("auth") => Some(BlockedKind::Auth),
                _ => None,
            },
            progress: status["progress"]
                .as_u64()
                .filter(|n| *n <= 100)
                .map(|n| n as u8),
            app: text(&status["app"], 96),
            title: text(&status["title"], 192),
            msg: text(&status["msg"], 2048),
        });
    };
    add("", &terminal["status"]);
    if let Some(children) = terminal["records"].as_object() {
        for (id, status) in children.iter().take(63) {
            add(id, status);
        }
    }
    records
}
