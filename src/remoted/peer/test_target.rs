//! Test-only target RPC/ownership seam, scoped to one retained in-memory `AppState`.
use super::*;
use crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter;

#[derive(Clone)]
struct Target {
    grant: Grant,
    owner: String,
    adapter: HerdrTerminalAdapter,
    local_files: bool,
}
static TARGETS: LazyLock<Mutex<HashMap<usize, Target>>> = LazyLock::new(Mutex::default);

pub(in crate::remoted) struct Scope {
    state: Arc<AppState>,
}
impl Scope {
    pub(in crate::remoted) fn new(
        state: Arc<AppState>,
        grant: Grant,
        adapter: HerdrTerminalAdapter,
    ) -> Self {
        let owner = state.remoted.snapshot().unwrap().owner;
        assert!(
            TARGETS
                .lock()
                .unwrap()
                .insert(
                    Arc::as_ptr(&state) as usize,
                    Target {
                        grant,
                        owner,
                        adapter,
                        local_files: false,
                    }
                )
                .is_none()
        );
        Self { state }
    }
    pub(super) fn enable_local_files(&self) {
        let mut targets = TARGETS.lock().unwrap();
        let target = targets
            .get_mut(&(Arc::as_ptr(&self.state) as usize))
            .unwrap();
        let root =
            std::path::Path::new(target.grant.root.as_ref().expect("temporary root required"));
        assert!(root.starts_with(std::env::temp_dir()));
        assert!(target.grant.root_identity.is_some());
        target.local_files = true;
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        TARGETS
            .lock()
            .unwrap()
            .remove(&(Arc::as_ptr(&self.state) as usize));
    }
}

pub(in crate::remoted) fn adapter(state: &Arc<AppState>) -> Option<HerdrTerminalAdapter> {
    TARGETS
        .lock()
        .unwrap()
        .get(&(Arc::as_ptr(state) as usize))
        .map(|target| target.adapter.clone())
}

pub(super) fn authorize(state: &Arc<AppState>, grant: &Grant, owner: &str) -> Option<Result<()>> {
    let target = TARGETS
        .lock()
        .unwrap()
        .get(&(Arc::as_ptr(state) as usize))
        .cloned()?;
    Some((|| {
        ensure!(owner == target.owner, "test target owner changed");
        ensure!(
            serde_json::to_value(grant)? == serde_json::to_value(&target.grant)?,
            "test target grant was not authorized"
        );
        Ok(())
    })())
}

pub(in crate::remoted) fn file_command(
    state: &Arc<AppState>,
    grant: &Grant,
    read_only: bool,
) -> Option<tokio::process::Command> {
    let targets = TARGETS.lock().unwrap();
    let target = targets.get(&(Arc::as_ptr(state) as usize))?;
    if !target.local_files
        || serde_json::to_value(grant).unwrap() != serde_json::to_value(&target.grant).unwrap()
    {
        return None;
    }
    let mut command = tokio::process::Command::new("python3");
    command.args([
        "-u",
        "-c",
        include_str!("../files_helper.py"),
        grant.root.as_deref().unwrap(),
        if read_only { "read-only" } else { "operate" },
    ]);
    Some(command)
}
