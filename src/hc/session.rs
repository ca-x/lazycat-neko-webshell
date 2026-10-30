use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::state::{AppState, METADATA_LOGIN_USER, SessionRecord};

use super::{model, protocol::Client, target};

const CREATED: &str = "hcCreationAttempted";
const ADDRESS: &str = "hcAddress";
const CORE: &str = "hcCoreId";
const CLOSING: &str = "hcClosing";
const JOURNAL_NAMESPACE: &str = "hc-session";
const LIFECYCLE_KEYS: [&str; 4] = [CREATED, ADDRESS, CORE, CLOSING];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    selector: String,
    login_user: String,
    lifecycle: HashMap<String, String>,
}
static PROVIDER_LOCK: Mutex<()> = Mutex::new(());
static SESSION_LOCKS: LazyLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(Mutex::default);

/// A session identity or lifecycle failure that reconnecting cannot repair.
#[derive(Debug)]
pub(super) struct PermanentError(anyhow::Error);

impl fmt::Display for PermanentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::error::Error for PermanentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

pub(super) struct Session {
    pub state: Arc<AppState>,
    pub record: SessionRecord,
    pub provider: String,
    pub scope: String,
    pub login_user: String,
}

impl Session {
    pub async fn load(state: Arc<AppState>, id: &str) -> Result<Self> {
        let record = state
            .sessions
            .read()
            .map_err(|_| anyhow!("session store lock poisoned"))?
            .get(id)
            .cloned()
            .context(PermanentError(anyhow!("unknown hc session")))?;
        if record.metadata.get("sessionBackend").map(String::as_str) != Some("hc")
            || record.status == "closed"
        {
            return Err(PermanentError(anyhow!("session is not an active hc session")).into());
        }
        if !crate::tty_init::lightos_features_enabled()
            || crate::ssh_backend::is_ssh_selector(&record.selector)
            || crate::lightos_admin::is_client_selector(&record.selector)
        {
            return Err(PermanentError(anyhow!("hc requires a LightOS target")).into());
        }
        let login_user = crate::lightos::login_user_for_selector(&record.selector, true)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        if record
            .metadata
            .get(METADATA_LOGIN_USER)
            .is_some_and(|saved| saved != &login_user)
        {
            return Err(PermanentError(anyhow!("hc target login identity changed")).into());
        }
        let provider = provider_id(&state)?;
        let scope = scope(&provider, &record.selector, &login_user);
        let mut session = Self {
            state,
            record,
            provider,
            scope,
            login_user,
        };
        session.record = session.refresh()?;
        Ok(session)
    }

    pub async fn connect(&self) -> Result<Client> {
        Client::start(target::command(
            &self.record.selector,
            &self.login_user,
            &self.scope,
        )?)
        .await
    }

    pub async fn lock(&self) -> OwnedMutexGuard<()> {
        let key = format!("{}:{}", self.provider, self.record.id);
        let lock = {
            let mut locks = SESSION_LOCKS
                .lock()
                .expect("hc session lock registry poisoned");
            locks.retain(|_, lock| lock.strong_count() > 0);
            let lock = locks
                .get(&key)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| Arc::new(AsyncMutex::new(())));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        };
        lock.lock_owned().await
    }

    pub async fn resolve(&self, client: &mut Client, allow_create: bool) -> Result<Value> {
        let _guard = self.lock().await;
        self.resolve_locked(client, allow_create).await
    }

    async fn resolve_locked(&self, client: &mut Client, allow_create: bool) -> Result<Value> {
        let record = self.refresh()?;
        if record.metadata.contains_key(CLOSING) {
            return Err(PermanentError(anyhow!("hc session is closing")).into());
        }
        let core = client.hello()["core"]["id"]
            .as_str()
            .context(PermanentError(anyhow!("hc omitted core identity")))?
            .to_owned();
        if record
            .metadata
            .get(CORE)
            .is_some_and(|saved| saved != &core)
        {
            return Err(PermanentError(anyhow!(
                "hc core identity changed; existing terminal was not recreated"
            ))
            .into());
        }
        let labels = json!({"neko_provider":self.provider,"neko_session":record.id});
        let list = client
            .call("terminal.list", json!({"all":true,"labels":labels}))
            .await?;
        let terminals = list["terminals"]
            .as_array()
            .context(PermanentError(anyhow!("hc omitted terminal list")))?;
        let terminal = model::resolve_terminal(
            terminals,
            &self.provider,
            &record.id,
            record.metadata.contains_key(CREATED),
            record.metadata.get(ADDRESS).map(String::as_str),
        )
        .map_err(PermanentError)?;
        let terminal = if let Some(terminal) = terminal {
            terminal
        } else {
            if !allow_create {
                return Err(PermanentError(anyhow!("hc terminal has not been created")).into());
            }
            // Persist before the non-idempotent request. A lost reply never causes another run.
            self.update(&[(CREATED, "true"), (CORE, &core)])?;
            client.call("terminal.run", json!({
                "name":model::terminal_name(&record.id),"labels":labels,
                "no_parent":true,"no_notify":true,"outlive_parent":true,
                "keep":record.metadata.get("restartable").is_some_and(|v| v == "true"),
                "env":{"TERM":"xterm-256color","TERM_PROGRAM":"Neko WebShell","COLORTERM":"truecolor"}
            })).await?
        };
        let address = terminal["address"]
            .as_str()
            .context(PermanentError(anyhow!("hc omitted terminal address")))?;
        // Verify returned identity even when it is the initial run response.
        model::resolve_terminal(
            std::slice::from_ref(&terminal),
            &self.provider,
            &record.id,
            true,
            Some(address),
        )
        .map_err(PermanentError)?;
        self.update(&[(ADDRESS, address), (CORE, &core), (CREATED, "true")])?;
        Ok(terminal)
    }

    fn refresh(&self) -> Result<SessionRecord> {
        self.synchronize(&[])
    }

    fn update(&self, values: &[(&str, &str)]) -> Result<()> {
        self.synchronize(values).map(|_| ())
    }

    fn synchronize(&self, values: &[(&str, &str)]) -> Result<SessionRecord> {
        // Serialize journal read/modify/write and publication with every HC reader.
        let mut records = self
            .state
            .sessions
            .write()
            .map_err(|_| anyhow!("session store lock poisoned"))?;
        let record = records
            .get_mut(&self.record.id)
            .context(PermanentError(anyhow!("hc session was closed")))?;
        if record.selector != self.record.selector
            || record.metadata.get("sessionBackend").map(String::as_str) != Some("hc")
            || record
                .metadata
                .get(METADATA_LOGIN_USER)
                .is_some_and(|saved| saved != &self.login_user)
        {
            return Err(PermanentError(anyhow!("hc session target identity changed")).into());
        }
        let database = self.state.database();
        let bytes = database.load_kv(JOURNAL_NAMESPACE, &record.id)?;
        let mut journal: Journal = match bytes.as_deref() {
            Some(bytes) => serde_json::from_slice(bytes).map_err(|error| {
                PermanentError(anyhow!(error).context("invalid hc session journal"))
            })?,
            // Import existing sessions once, before treating their snapshots as replaceable.
            None => Journal {
                selector: record.selector.clone(),
                login_user: self.login_user.clone(),
                lifecycle: record
                    .metadata
                    .iter()
                    .filter(|(key, _)| LIFECYCLE_KEYS.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            },
        };
        if journal.selector != record.selector || journal.login_user != self.login_user {
            return Err(PermanentError(anyhow!("hc journal target identity changed")).into());
        }
        if journal
            .lifecycle
            .keys()
            .any(|key| !LIFECYCLE_KEYS.contains(&key.as_str()))
        {
            return Err(PermanentError(anyhow!("invalid hc journal lifecycle key")).into());
        }
        for &(key, value) in values {
            anyhow::ensure!(
                LIFECYCLE_KEYS.contains(&key),
                "invalid hc journal update key"
            );
            journal.lifecycle.insert(key.to_owned(), value.to_owned());
        }
        if bytes.is_none() || !values.is_empty() {
            database.store_kv(
                JOURNAL_NAMESPACE,
                &record.id,
                &serde_json::to_vec(&journal)?,
            )?;
        }
        // The journal wins even when a stale snapshot introduced a key it doesn't contain.
        for key in LIFECYCLE_KEYS {
            record.metadata.remove(key);
        }
        record.metadata.extend(journal.lifecycle);
        Ok(record.clone())
    }
}

pub(crate) async fn close(state: Arc<AppState>, record: &SessionRecord) -> Result<()> {
    let session = Session::load(state, &record.id).await?;
    let _guard = session.lock().await;
    // Refresh also republishes a durable closing tombstone for workspace deletion guards.
    let saved = session.refresh()?;
    if saved.metadata.contains_key(CLOSING) {
        return Ok(());
    }
    let attempted = saved.metadata.contains_key(CREATED);
    if !attempted {
        session.update(&[(CLOSING, "true")])?;
        return Ok(());
    }
    let mut client = session.connect().await?;
    let terminal = session.resolve_locked(&mut client, false).await?;
    if terminal["state"] != "finished" {
        let result = client
            .call("terminal.kill", json!({"terminal":terminal["address"]}))
            .await?;
        if result["state"] != "finished" {
            bail!("hc did not confirm terminal termination");
        }
    }
    session.update(&[(CLOSING, "true")])?;
    client.close().await?;
    Ok(())
}

fn provider_id(state: &AppState) -> Result<String> {
    let _guard = PROVIDER_LOCK
        .lock()
        .map_err(|_| anyhow!("hc provider identity lock poisoned"))?;
    let database = state.database();
    if let Some(bytes) = database.load_kv("hc", "provider-id")? {
        let id = String::from_utf8(bytes)
            .context(PermanentError(anyhow!("invalid hc provider identity")))?;
        Uuid::parse_str(&id).context(PermanentError(anyhow!("invalid hc provider identity")))?;
        return Ok(id);
    }
    let id = Uuid::new_v4().to_string();
    database.store_kv("hc", "provider-id", id.as_bytes())?;
    Ok(id)
}

fn scope(provider: &str, selector: &str, login: &str) -> String {
    model::digest_hex(format!("{provider}\0{selector}\0{login}").as_bytes())[..32].to_owned()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context as _, anyhow};
    use serde_json::json;

    use super::{PermanentError, model};

    #[test]
    fn permanent_error_survives_context_and_preserves_its_source() {
        let error = anyhow::Error::new(PermanentError(anyhow!("hc session is closing")))
            .context("attaching hc session");
        assert!(error.downcast_ref::<PermanentError>().is_some());
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string() == "hc session is closing")
        );
        let missing = None::<()>
            .context(PermanentError(anyhow!("hc session was closed")))
            .unwrap_err();
        assert!(missing.downcast_ref::<PermanentError>().is_some());
    }

    #[test]
    fn unknown_creation_and_changed_terminal_identity_are_permanent() {
        let terminal =
            json!({"address":"t7","labels":{"neko_provider":"provider","neko_session":"session"}});
        for result in [
            model::resolve_terminal(&[], "provider", "session", true, None),
            model::resolve_terminal(&[terminal], "provider", "session", true, Some("t8")),
        ] {
            let error = result
                .map_err(PermanentError)
                .map_err(anyhow::Error::new)
                .unwrap_err();
            assert!(error.downcast_ref::<PermanentError>().is_some());
        }
        let timeout = anyhow!("hc RPC timed out").context("resolving terminal");
        assert!(timeout.downcast_ref::<PermanentError>().is_none());
    }

    #[test]
    fn failed_intent_save_does_not_poison_live_session_metadata() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::state::{AppState, SessionRecord};

        let directory =
            std::env::temp_dir().join(format!("neko-hc-session-{}", uuid::Uuid::new_v4()));
        let path = directory.join("state.db");
        let state = Arc::new(AppState::new_for_test(path.clone()));
        let record = SessionRecord {
            id: "session".into(),
            host: "box".into(),
            selector: "box@owner".into(),
            status: "starting".into(),
            cols: 80,
            rows: 24,
            command: "hc".into(),
            args: Vec::new(),
            metadata: HashMap::from([("sessionBackend".into(), "hc".into())]),
        };
        state
            .sessions
            .write()
            .unwrap()
            .insert(record.id.clone(), record.clone());
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON app_kv WHEN NEW.namespace = 'hc-session' BEGIN SELECT RAISE(FAIL, 'intent save failed'); END;").unwrap();
        let session = super::Session {
            state: Arc::clone(&state),
            record: record.clone(),
            provider: "provider".into(),
            scope: super::scope("provider", &record.selector, "alice"),
            login_user: "alice".into(),
        };
        assert!(
            session
                .update(&[(super::CREATED, "true"), (super::CORE, "core")])
                .is_err()
        );
        assert_eq!(
            state.sessions.read().unwrap()["session"].metadata,
            record.metadata
        );
        connection
            .execute_batch("DROP TRIGGER fail_intent;")
            .unwrap();
        session.update(&[(super::CREATED, "true")]).unwrap();
        assert!(
            state.sessions.read().unwrap()["session"]
                .metadata
                .contains_key(super::CREATED)
        );
        drop(connection);
        drop(session);
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_snapshot_reload_preserves_journal_and_republishes_closing_tombstone() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::database::AppDatabase;
        use crate::session_manager::SessionManager;
        use crate::state::{AppState, SessionRecord, SessionStore};

        let directory =
            std::env::temp_dir().join(format!("neko-hc-journal-{}", uuid::Uuid::new_v4()));
        let path = directory.join("state.db");
        let state = Arc::new(AppState::new_for_test(path.clone()));
        let record = SessionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            host: "box".into(),
            selector: "box@owner".into(),
            status: "starting".into(),
            cols: 80,
            rows: 24,
            command: "hc".into(),
            args: Vec::new(),
            metadata: HashMap::from([
                ("sessionBackend".into(), "hc".into()),
                ("loginUser".into(), "alice".into()),
            ]),
        };
        state
            .sessions
            .write()
            .unwrap()
            .insert(record.id.clone(), record.clone());
        let old_snapshot = state.sessions.read().unwrap().clone();
        state.persist_sessions_snapshot(&old_snapshot).unwrap();
        let session = super::Session {
            state: Arc::clone(&state),
            record: record.clone(),
            provider: "provider".into(),
            scope: super::scope("provider", &record.selector, "alice"),
            login_user: "alice".into(),
        };
        let expected = [
            (super::CREATED, "true"),
            (super::ADDRESS, "t7"),
            (super::CORE, "core"),
            (super::CLOSING, "true"),
        ];
        session.update(&expected).unwrap();
        // A generic writer captured this before the intent was saved, then committed last.
        state.persist_sessions_snapshot(&old_snapshot).unwrap();
        // Rebuild AppState's session manager from a fresh SQLite connection, as startup does.
        // Cloning the remaining services avoids changing process-wide environment for AppState::new.
        let database = Arc::new(AppDatabase::open(path).unwrap());
        let store = Arc::new(SessionStore::new(Arc::clone(&database)));
        let restored = store.load().unwrap();
        assert!(!restored[&record.id].metadata.contains_key(super::CREATED));
        let mut reloaded = (*state).clone();
        reloaded.sessions = Arc::new(SessionManager::new(restored, store, database));
        let mut resumed = super::Session {
            state: Arc::new(reloaded),
            ..session
        };
        resumed.record = resumed.refresh().unwrap();
        for (key, value) in expected {
            assert_eq!(
                resumed.record.metadata.get(key).map(String::as_str),
                Some(value)
            );
        }
        // Exercise the same refresh used by close's early-return path after stale memory too.
        resumed
            .state
            .sessions
            .write()
            .unwrap()
            .get_mut(&record.id)
            .unwrap()
            .metadata
            .remove(super::CLOSING);
        assert!(
            resumed
                .refresh()
                .unwrap()
                .metadata
                .contains_key(super::CLOSING)
        );
        crate::workspace::close_workspace_session(&resumed.state, &record.id).unwrap();
        assert!(
            resumed
                .state
                .database()
                .load_kv(super::JOURNAL_NAMESPACE, &record.id)
                .unwrap()
                .is_some()
        );
        drop(resumed);
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn core_scope_retains_owner_login_and_provider_boundaries() {
        let scope = super::scope("provider", "box@owner", "alice");
        assert_eq!(scope.len(), 32);
        for other in [
            super::scope("other", "box@owner", "alice"),
            super::scope("provider", "box@other", "alice"),
            super::scope("provider", "box@owner", "bob"),
        ] {
            assert_ne!(scope, other);
        }
    }
}
