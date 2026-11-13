use super::*;
use crate::remoted::store::Remoted;
use neko_remoted_protocol::messages::Permission;
use tokio_util::sync::CancellationToken;

fn parent() -> Grant {
    Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: uuid::Uuid::new_v4().to_string(),
        selector: "work@owner-deployment".into(),
        login: "alice".into(),
        backend: "herdr".into(),
        name: "approved".into(),
        target_signature: String::new(),
        root: Some("/private".into()),
        root_identity: None,
    }
}

fn fixture() -> (Arc<AppState>, Device, Grant, Grant) {
    let state = Arc::new(AppState::new_for_test(":memory:".into()));
    let parent = parent();
    let defaults = WorkspaceTerminalDefaults::new(
        &state,
        &parent.selector,
        DEFAULT_COLS,
        DEFAULT_ROWS,
        DEFAULT_OUTPUT_FRAME_LIMIT,
        false,
        &parent.login,
        SessionBackend::Webshell,
    )
    .unwrap();
    let record =
        workspace::create_workspace_session(&state, &parent.selector, &defaults, HashMap::new())
            .unwrap()
            .session;
    let child = child_grant(&parent, &record).unwrap();
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: "device".into(),
        public_key: String::new(),
        permission: Permission::Operate,
        grants: vec![parent.clone()],
        generation: 1,
    };
    let mut saved = state.remoted.snapshot().unwrap();
    saved.enabled = true;
    saved.owner = "account".into();
    saved.devices.push(device.clone());
    let mut other = device.clone();
    other.id = "other-device".into();
    saved.devices.push(other);
    state.remoted.persist(saved).unwrap();
    (state, device, parent, child)
}

#[test]
fn creation_uses_native_default_for_exact_parent_and_never_delegates_files() {
    let (state, _, parent, child) = fixture();
    assert_eq!(child.selector, "work@owner-deployment");
    assert_eq!(child.login, "alice");
    assert_eq!(child.backend, "webshell");
    assert!(child.root.is_none() && child.root_identity.is_none());
    assert_ne!(child.session, parent.session);
    let records = state.sessions.read().unwrap();
    let record = records.get(&child.session).unwrap();
    let trusted = crate::state::default_session_command_for_user(&parent.selector, &parent.login);
    assert_eq!((&record.command, &record.args), (&trusted.0, &trusted.1));
    assert!(!record.metadata.contains_key("root"));
    assert!(!record.metadata.contains_key("remoted"));
}

#[test]
fn persistent_child_is_scoped_to_creating_device_and_survives_reload_without_creation() {
    let (state, device, parent, child) = fixture();
    state
        .remoted
        .persist_created_grant(&device, &parent, child.clone())
        .unwrap();
    let reloaded = Remoted::load(state.database()).unwrap();
    let current = reloaded
        .current_device(&device.id, device.generation)
        .unwrap();
    assert!(current.grants.iter().any(|grant| grant.id == child.id));
    assert_eq!(current.grants.len(), 2);
    assert_eq!(reloaded.snapshot().unwrap().devices[1].grants.len(), 1);
    assert_eq!(
        state.sessions.read().unwrap().len(),
        1,
        "reload must never create a second session"
    );
    assert!(
        state
            .remoted
            .persist_created_grant(&device, &parent, child)
            .is_err(),
        "publishing twice cannot duplicate authority"
    );
}

#[test]
fn wrong_target_login_file_scope_readonly_revoke_and_generation_are_rejected() {
    for denial in [
        "selector",
        "owner",
        "login",
        "files",
        "backend",
        "signature",
        "parent",
        "readonly",
        "revoke",
        "generation",
        "disabled",
    ] {
        let (state, device, parent, mut child) = fixture();
        let mut saved = state.remoted.snapshot().unwrap();
        match denial {
            "selector" => child.selector = "different@owner-deployment".into(),
            "owner" => child.selector = "work@different-deployment".into(),
            "login" => child.login = "root".into(),
            "files" => child.root = Some("/".into()),
            "backend" => child.backend = "herdr".into(),
            "signature" => child.target_signature = "changed".into(),
            "parent" => saved.devices[0].grants.clear(),
            "readonly" => saved.devices[0].permission = Permission::ReadOnly,
            "revoke" => {
                saved.devices.remove(0);
            }
            "generation" => saved.devices[0].generation += 1,
            "disabled" => saved.enabled = false,
            _ => unreachable!(),
        }
        state.remoted.persist(saved).unwrap();
        assert!(
            state
                .remoted
                .persist_created_grant(&device, &parent, child)
                .is_err(),
            "{denial} escaped parent authority"
        );
        assert!(
            state
                .remoted
                .snapshot()
                .unwrap()
                .devices
                .iter()
                .all(|device| device.grants.len() <= 1)
        );
    }
}

#[test]
fn ticket_dependent_and_microvm_targets_are_not_physical_create_selectors() {
    for selector in [
        "client:registered",
        "12345678-1234-4234-8234-123456789abc",
        "smolvm:12345678-1234-4234-8234-123456789abc",
    ] {
        let mut parent = parent();
        parent.selector = selector.into();
        assert!(
            backend(&parent).is_err(),
            "{selector} requires its separate broker"
        );
    }
    let mut parent = parent();
    parent.backend = "hc".into();
    assert!(backend(&parent).is_err());
}

#[test]
fn creation_admission_distinguishes_denials_from_submitted_unknown_and_blocks_reconnect_retry() {
    for denial in ["readonly", "revoke", "generation", "parent", "cancel"] {
        let (state, device, parent, _) = fixture();
        let cancel = CancellationToken::new();
        let admission = state
            .remoted
            .admit(&device, "first", 1, cancel.clone())
            .unwrap();
        admission.describe("terminal_create").unwrap();
        let mut saved = state.remoted.snapshot().unwrap();
        match denial {
            "readonly" => saved.devices[0].permission = Permission::ReadOnly,
            "revoke" => {
                saved.devices.remove(0);
            }
            "generation" => saved.devices[0].generation += 1,
            "parent" => saved.devices[0].grants.clear(),
            "cancel" => cancel.cancel(),
            _ => unreachable!(),
        }
        state.remoted.persist(saved).unwrap();
        assert!(admission.submit_creation(&parent).is_err(), "{denial}");
        assert!(!admission.submitted());
        assert_eq!(
            state.remoted.operation_status().unwrap().in_flight_unknown,
            0
        );
    }
    let (state, device, parent, _) = fixture();
    let cancel = CancellationToken::new();
    let mut admission = state
        .remoted
        .admit(&device, "first", 1, cancel.clone())
        .unwrap();
    admission.describe("terminal_create").unwrap();
    admission.submit_creation(&parent).unwrap();
    cancel.cancel();
    assert!(
        state
            .remoted
            .admit(&device, "reconnect", 1, CancellationToken::new())
            .is_err()
    );
    assert_eq!(
        state.remoted.operation_status().unwrap().in_flight_unknown,
        1
    );
    admission.finish(false, true);
    let status = state.remoted.operation_status().unwrap();
    assert!(status.recent[0].outcome.outcome_unknown);
    assert_eq!(status.recent[0].operation, "terminal_create");
}

#[tokio::test]
async fn pending_worker_device_snapshot_can_authorize_child_from_current_saved_grants() {
    let (state, device, parent, child) = fixture();
    state
        .remoted
        .persist_created_grant(&device, &parent, child.clone())
        .unwrap();
    assert!(!device.grants.iter().any(|grant| grant.id == child.id));
    let _scope = crate::remoted::peer::test_target::Scope::new(
        state.clone(),
        child.clone(),
        crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter::new(state.clone()),
    );
    let authorized = peer::grant(&state, &device, &child.id).await.unwrap();
    assert!(
        authorized.same_authority(&child),
        "next attach must use the newly saved grant"
    );
}
