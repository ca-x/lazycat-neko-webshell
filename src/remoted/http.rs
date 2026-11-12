use super::{
    authority,
    store::{Device, Grant, Remoted, Saved},
};
use crate::state::AppState;
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use neko_remoted_protocol::{RelayAddress, code::DeviceCode, messages::Permission};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    Configure {
        enabled: bool,
        relay: String,
        #[serde(default, rename = "registrationKey")]
        registration_key: Option<String>,
    },
    Invite {},
    Approve {
        pending: String,
        permission: Permission,
        resources: Vec<ResourceChoice>,
    },
    Reject {
        pending: String,
    },
    Permission {
        id: String,
        permission: Permission,
    },
    Disconnect {
        id: String,
    },
    Revoke {
        id: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceChoice {
    session: String,
    root: Option<String>,
}
fn error(e: &anyhow::Error) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
        .into_response()
}
fn authorization_error(error: &anyhow::Error) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error":error.to_string(),"code":"AUTHORIZATION_FAILED"})),
    )
        .into_response()
}
pub async fn get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Response> {
    authority::management(&headers)
        .await
        .map_err(|failure| authorization_error(&failure))?;
    view(&state.remoted)
        .map(Json)
        .map_err(|failure| error(&failure))
}
pub async fn post(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(action): Json<Action>,
) -> Result<Json<Value>, Response> {
    let owner = authority::management(&headers)
        .await
        .map_err(|failure| authorization_error(&failure))?;
    let result = match action {
        Action::Approve {
            pending,
            permission,
            resources,
        } => {
            approve_prepared(
                &state.remoted,
                &owner,
                pending,
                permission,
                prepare_grants(&state, &headers, &owner, resources),
            )
            .await
        }
        action => {
            let _guard = state.remoted.mutation.lock().await;
            apply(&state, &owner, action)
        }
    };
    result.map(Json).map_err(|failure| error(&failure))
}

fn apply(state: &Arc<AppState>, owner: &str, action: Action) -> Result<Value> {
    let manager = &state.remoted;
    let mut saved = manager.snapshot()?;
    ensure!(
        saved.owner.is_empty() || saved.owner == owner,
        "部署所有者已经变化，现有授权不可沿用"
    );
    match action {
        Action::Configure {
            enabled,
            relay,
            registration_key,
        } => configure(manager, saved, owner, enabled, &relay, registration_key),
        Action::Invite {} => {
            ensure!(saved.enabled, "请先开启远程控制");
            let relay = saved.relay.as_ref().context("请先连接 Relay")?;
            let mut rt = manager
                .runtime
                .lock()
                .map_err(|_| anyhow::anyhow!("remote state unavailable"))?;
            ensure!(rt.status == "online", "请等待 Relay 连接成功后再配对");
            rt.pending
                .retain(|_, p| p.created.elapsed() < Duration::from_mins(2));
            let code = DeviceCode::generate(&saved.route()?, relay)?;
            let text = code.display();
            rt.invitation = Some((code, Instant::now()));
            Ok(json!({"code":text,"expiresInSeconds":300}))
        }
        Action::Approve { .. } => {
            anyhow::bail!("approval resources must be verified before mutation")
        }
        Action::Reject { pending } => {
            manager
                .runtime
                .lock()
                .map_err(|_| anyhow::anyhow!("remote state unavailable"))?
                .pending
                .remove(&pending);
            view(manager)
        }
        Action::Permission { id, permission } => {
            let d = saved
                .devices
                .iter_mut()
                .find(|d| d.id == id)
                .context("设备不存在")?;
            d.permission = permission;
            d.generation = d
                .generation
                .checked_add(1)
                .context("authorization generation exhausted")?;
            manager.persist(saved)?;
            manager.cancel_device(&id);
            view(manager)
        }
        Action::Disconnect { id } => {
            ensure!(saved.devices.iter().any(|d| d.id == id), "设备不存在");
            let device = saved
                .devices
                .iter_mut()
                .find(|device| device.id == id)
                .context("设备不存在")?;
            device.generation = device
                .generation
                .checked_add(1)
                .context("authorization generation exhausted")?;
            manager.persist(saved)?;
            manager.cancel_device(&id);
            view(manager)
        }
        Action::Revoke { id } => {
            ensure!(saved.devices.iter().any(|d| d.id == id), "设备不存在");
            saved.devices.retain(|d| d.id != id);
            manager.persist(saved)?;
            manager.cancel_device(&id);
            view(manager)
        }
    }
}

fn configure(
    manager: &Remoted,
    mut saved: Saved,
    owner: &str,
    enabled: bool,
    relay: &str,
    registration_key: Option<String>,
) -> Result<Value> {
    if let Some(key) = registration_key {
        validate_registration_key(&key)?;
        saved.registration_key = (!key.is_empty()).then_some(key);
    }
    let address = if relay.trim().is_empty() {
        None
    } else {
        Some(RelayAddress::parse(relay)?)
    };
    ensure!(!enabled || address.is_some(), "请先填写 Relay 地址");
    // Binding identities cannot migrate silently to another relay.
    if saved.relay.is_some() && saved.relay != address {
        ensure!(saved.devices.is_empty(), "更换 Relay 前请先移除已绑定设备");
    }
    saved.owner = owner.into();
    saved.enabled = enabled;
    saved.relay = address;
    manager.persist(saved)?;
    if let Ok(mut rt) = manager.runtime.lock() {
        rt.invitation = None;
        rt.pending.clear();
        for p in rt.peers.values() {
            p.cancel.cancel();
        }
    }
    manager.restart();
    view(manager)
}

async fn prepare_grants(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    owner: &str,
    resources: Vec<ResourceChoice>,
) -> Result<Vec<Grant>> {
    ensure!(state.remoted.snapshot()?.enabled, "远程控制已关闭");
    ensure!(
        !resources.is_empty() && resources.len() <= 16,
        "请选择1到16个会话"
    );
    let discovery = crate::instance_discovery::discover(headers)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let mut grants = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for choice in resources {
        ensure!(seen.insert(choice.session.clone()), "会话不能重复授权");
        let grant = authority::grant(state, owner, &choice.session, choice.root).await?;
        ensure!(
            crate::ssh_backend::is_ssh_selector(&grant.selector)
                || discovery
                    .instances
                    .iter()
                    .any(|i| i.selector.as_deref() == Some(&grant.selector)),
            "目标不在当前用户可见实例中"
        );
        grants.push(grant);
    }

    Ok(grants)
}
async fn approve_prepared(
    manager: &Remoted,
    owner: &str,
    pending: String,
    permission: Permission,
    preparation: impl std::future::Future<Output = Result<Vec<Grant>>>,
) -> Result<Value> {
    let grants = preparation.await?;
    let _guard = manager.mutation.lock().await;
    finalize_approval(manager, owner, &pending, permission, grants)
}
fn finalize_approval(
    manager: &Remoted,
    owner: &str,
    pending: &str,
    permission: Permission,
    grants: Vec<Grant>,
) -> Result<Value> {
    let mut saved = manager.snapshot()?;
    ensure!(
        saved.owner.is_empty() || saved.owner == owner,
        "部署所有者已经变化，现有授权不可沿用"
    );
    ensure!(saved.enabled, "远程控制已关闭");
    let mut rt = manager
        .runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("remote state unavailable"))?;
    let request = rt
        .pending
        .remove(pending)
        .context("配对申请不存在或已经过期")?;
    ensure!(
        request.created.elapsed() < Duration::from_mins(2),
        "配对申请已经过期"
    );
    ensure!(!request.decision.is_closed(), "配对设备已离线");
    ensure!(
        saved.devices.len() < neko_remoted_protocol::MAX_DEVICES
            || saved.devices.iter().any(|d| d.public_key == request.key),
        "已绑定设备达到上限"
    );
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: request.name,
        public_key: request.key,
        permission,
        grants,
        generation: 1,
    };
    let replaced: Vec<_> = saved
        .devices
        .iter()
        .filter(|d| d.public_key == device.public_key)
        .map(|d| d.id.clone())
        .collect();
    saved.devices.retain(|d| d.public_key != device.public_key);
    saved.devices.push(device.clone());
    manager.persist(saved)?;
    drop(rt);
    for old in replaced {
        manager.cancel_device(&old);
    }
    request
        .decision
        .send(device)
        .map_err(|_| anyhow::anyhow!("设备已离线，绑定已保存，可使用设备身份重新连接"))?;
    view(manager)
}

fn fresh(sample: Option<(u64, Instant)>) -> Option<u64> {
    sample
        .filter(|(_, at)| at.elapsed() < Duration::from_secs(30))
        .map(|(n, _)| n)
}
fn validate_registration_key(key: &str) -> Result<()> {
    ensure!(
        key.len() <= 256 && key.is_ascii() && !key.bytes().any(|byte| byte.is_ascii_control()),
        "注册密钥必须是最多256字节的ASCII文本且不能含控制字符"
    );
    Ok(())
}

fn view(manager: &Remoted) -> Result<Value> {
    let operations = manager.operation_status()?;
    let saved = manager.snapshot()?;
    let host_fingerprint =
        neko_remoted_protocol::fingerprint(&neko_remoted_protocol::decode32(&saved.public_key)?);
    let rt = manager
        .runtime
        .lock()
        .map_err(|_| anyhow::anyhow!("remote state unavailable"))?;
    let devices:Vec<_>=saved.devices.iter().map(|d|{
  let peers:Vec<_>=rt.peers.values().filter(|p|p.device.as_deref()==Some(&d.id)&&!p.cancel.is_cancelled()).collect();
  let rtt=peers.iter().filter_map(|p|fresh(p.rtt)).max();
  let connection_types: Vec<_> = peers.iter().map(|p| p.path.as_ref().map_or(neko_remoted_protocol::messages::ConnectionKind::Relay, neko_remoted_transport::Sender::kind)).collect();
  json!({"id":d.id,"name":d.name,"fingerprint":neko_remoted_protocol::decode32(&d.public_key).map(|k|neko_remoted_protocol::fingerprint(&k)).unwrap_or_default(),"permission":d.permission,"resources":d.grants.iter().map(super::store::Grant::resource).collect::<Vec<_>>(),"online":!peers.is_empty(),"connections":peers.len(),"rttMs":rtt,"connectionTypes":connection_types})
 }).collect();
    let pending:Vec<_>=rt.pending.values().filter(|p|p.created.elapsed()<Duration::from_mins(2)).map(|p|json!({"id":p.id,"name":p.name,"fingerprint":neko_remoted_protocol::decode32(&p.key).map(|k|neko_remoted_protocol::fingerprint(&k)).unwrap_or_default(),"expiresInSeconds":120-p.created.elapsed().as_secs()})).collect();
    Ok(
        json!({"enabled":saved.enabled,"hostFingerprint":host_fingerprint,"relay":saved.relay.map(|a|a.origin().to_owned()).unwrap_or_default(),"has_registration_key":saved.registration_key.is_some(),"status":if saved.enabled{rt.status.as_str()}else{"disabled"},"error":rt.error,"relayRttMs":if saved.enabled&&rt.status=="online"{fresh(rt.relay_rtt)}else{None},"devices":devices,"pending":pending,"operations":operations}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_connection_settings_survive_database_reopen() {
        let path = std::env::temp_dir().join(format!(
            "neko-remoted-settings-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        {
            let database = Arc::new(crate::database::AppDatabase::open(path.clone()).unwrap());
            let manager = Remoted::load(database).unwrap();
            let reply = configure(
                &manager,
                manager.snapshot().unwrap(),
                "owner",
                true,
                "https://neko-relay.czyt.eu.org",
                Some("test-registration-key".into()),
            )
            .unwrap();
            assert_eq!(reply["enabled"], true);
            assert_eq!(reply["relay"], "wss://neko-relay.czyt.eu.org");
        }
        {
            let database = Arc::new(crate::database::AppDatabase::open(path.clone()).unwrap());
            let manager = Remoted::load(database).unwrap();
            let reply = view(&manager).unwrap();
            assert_eq!(reply["enabled"], true);
            assert_eq!(reply["relay"], "wss://neko-relay.czyt.eu.org");
            assert_eq!(reply["has_registration_key"], true);
            assert_eq!(manager.snapshot().unwrap().owner, "owner");
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn slow_approval_preparation_does_not_block_disable_or_revive_stale_pending() {
        let state = Arc::new(AppState::new_for_test(":memory:".into()));
        let mut saved = state.remoted.snapshot().unwrap();
        saved.owner = "owner".into();
        saved.enabled = true;
        saved.relay = Some(RelayAddress::parse("https://relay.example").unwrap());
        state.remoted.persist(saved).unwrap();
        let (decision, _receiver) = tokio::sync::oneshot::channel();
        state.remoted.runtime.lock().unwrap().pending.insert(
            "pending".into(),
            super::super::store::Pending {
                id: "pending".into(),
                name: "device".into(),
                key: "key".into(),
                created: Instant::now(),
                decision,
            },
        );
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let manager = state.remoted.clone();
        let signal = started.clone();
        let resume = release.clone();
        let preparation = tokio::spawn(async move {
            approve_prepared(
                &manager,
                "owner",
                "pending".into(),
                Permission::Operate,
                async {
                    signal.notify_one();
                    resume.notified().await;
                    Ok(vec![])
                },
            )
            .await
        });
        started.notified().await;
        let guard = state
            .remoted
            .mutation
            .try_lock()
            .expect("remote approval must not hold mutation lock");
        apply(
            &state,
            "owner",
            Action::Configure {
                enabled: false,
                relay: "https://relay.example".into(),
                registration_key: None,
            },
        )
        .unwrap();
        apply(
            &state,
            "owner",
            Action::Configure {
                enabled: true,
                relay: "https://relay.example".into(),
                registration_key: None,
            },
        )
        .unwrap();
        drop(guard);
        release.notify_one();
        assert!(preparation.await.unwrap().is_err());
        assert!(state.remoted.snapshot().unwrap().devices.is_empty());
    }

    #[test]
    fn authenticated_view_has_host_fingerprint_without_private_identity_material() {
        let manager = Remoted::load(Arc::new(
            crate::database::AppDatabase::open(":memory:".into()).unwrap(),
        ))
        .unwrap();
        let saved = manager.snapshot().unwrap();
        let reply = view(&manager).unwrap();
        let expected = neko_remoted_protocol::fingerprint(
            &neko_remoted_protocol::decode32(&saved.public_key).unwrap(),
        );
        assert_eq!(reply["hostFingerprint"], expected);
        let encoded = serde_json::to_string(&reply).unwrap();
        assert!(!encoded.contains(&saved.private_key));
        assert!(!encoded.contains(&saved.routing_secret));
    }

    #[tokio::test]
    async fn management_interruptions_report_inflight_unknown_without_waiting_for_target() {
        for action in ["revoke", "permission", "disconnect", "disable"] {
            let state = Arc::new(AppState::new_for_test(":memory:".into()));
            let device = Device {
                id: "device".into(),
                name: "device".into(),
                public_key: String::new(),
                permission: Permission::Operate,
                grants: vec![],
                generation: 1,
            };
            let mut saved = state.remoted.snapshot().unwrap();
            saved.owner = "owner".into();
            saved.enabled = true;
            saved.relay = Some(RelayAddress::parse("https://relay.example").unwrap());
            saved.devices.push(device.clone());
            state.remoted.persist(saved).unwrap();
            let mut admission = state
                .remoted
                .admit(
                    &device,
                    "peer",
                    17,
                    tokio_util::sync::CancellationToken::new(),
                )
                .unwrap();
            admission.describe("file_commit").unwrap();
            admission.submit(true).unwrap();
            let action = match action {
                "revoke" => Action::Revoke {
                    id: device.id.clone(),
                },
                "permission" => Action::Permission {
                    id: device.id.clone(),
                    permission: Permission::ReadOnly,
                },
                "disconnect" => Action::Disconnect {
                    id: device.id.clone(),
                },
                _ => Action::Configure {
                    enabled: false,
                    relay: "https://relay.example".into(),
                    registration_key: None,
                },
            };
            let reply = tokio::time::timeout(Duration::from_secs(1), async {
                apply(&state, "owner", action)
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(reply["operations"]["inFlightUnknown"], 1);
            assert_eq!(reply["operations"]["active"][0]["requestId"], 17);
            assert_eq!(reply["operations"]["active"][0]["operation"], "file_commit");
            assert_eq!(reply["operations"]["active"][0]["outcomeUnknown"], true);
            assert!(admission.submit(true).is_err());
            admission.finish(false, true);
            let reply = view(&state.remoted).unwrap();
            assert_eq!(reply["operations"]["inFlightUnknown"], 0);
            assert_eq!(reply["operations"]["recent"][0]["outcomeUnknown"], true);
        }
    }

    #[test]
    fn registration_key_is_optional_bounded_ascii_without_control_characters() {
        for key in ["", "secret", &"x".repeat(256)] {
            assert!(validate_registration_key(key).is_ok());
        }
        for key in ["a\rb", "a\nb", "a\0b", "a\tb", "猫", &"x".repeat(257)] {
            assert!(validate_registration_key(key).is_err());
        }
        let action: Action =
            serde_json::from_value(json!({"action":"configure", "enabled":false,"relay":""}))
                .unwrap();
        assert!(matches!(
            action,
            Action::Configure {
                registration_key: None,
                ..
            }
        ));
        let action: Action = serde_json::from_value(
            json!({"action":"configure", "enabled":false,"relay":"", "registrationKey":"secret"}),
        )
        .unwrap();
        assert!(
            matches!(action, Action::Configure { registration_key: Some(key), .. } if key == "secret")
        );
    }
}
