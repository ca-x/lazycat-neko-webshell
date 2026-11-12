use super::*;

fn binding() -> Binding {
    Binding {
        relay: RelayAddress::parse("https://relay.example.com").unwrap(),
        route: protocol::encode(&[1; 32]),
        host_key: protocol::encode(&[2; 32]),
        device_id: "paired-device".into(),
    }
}

fn aliases(name: &str) -> storage::HostAliases {
    let binding = binding();
    serde_json::from_value(serde_json::json!([{
        "identity": { "relay": binding.relay, "route": binding.route, "host_key": binding.host_key },
        "name": name,
    }]))
    .unwrap()
}

fn actor() -> Actor {
    let mut actor = Actor::new(mpsc::channel(32).0);
    actor.model.bindings.push(binding());
    actor.model.active_binding = Some(binding());
    actor.model.aliases = aliases("原名称");
    actor.model.notice = "原连接提示".into();
    actor.scope.select(Some(binding()));
    actor
}

#[tokio::test]
async fn rename_success_preserves_selection_generation_and_existing_retry() {
    for phase in [Phase::Offline, Phase::Ready] {
        let mut actor = actor();
        actor.model.phase = phase.clone();
        actor.retry.arm();
        actor.retry.schedule(Instant::now());
        let retry_at = actor.retry.at;
        let attempts = actor.retry.attempts;
        actor
            .rename_with(&binding(), async { Ok(aliases("开发主机")) })
            .await;
        assert_eq!(actor.model.aliases.get(&binding()), Some("开发主机"));
        assert_eq!(actor.model.active_binding, Some(binding()));
        assert!(actor.scope.is_selected(&binding()));
        assert_eq!(actor.scope.current(), 0);
        assert_eq!(actor.model.generation, 0);
        assert_eq!(actor.model.phase, phase);
        assert_eq!(actor.retry.at, retry_at);
        assert_eq!(actor.retry.attempts, attempts);
        assert!(actor.retry.enabled);
        assert_eq!(actor.model.notice, "原连接提示");
    }
}

#[tokio::test]
async fn rename_storage_timeout_is_sanitized_and_never_enters_retry_handling() {
    for enabled in [false, true] {
        let mut actor = actor();
        actor.retry.enabled = enabled;
        let error: anyhow::Error = std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("{} {}", binding().route, binding().host_key),
        )
        .into();
        assert!(network::retryable(&error));
        actor.rename_with(&binding(), async { Err(error) }).await;
        actor.connection_result(Ok(())).await;
        assert_eq!(actor.model.aliases.get(&binding()), Some("原名称"));
        assert!(actor.model.error.contains("未能确认保存"));
        assert!(!actor.model.error.contains(&binding().host_key));
        assert!(!actor.model.error.contains(&binding().route));
        assert_eq!(actor.retry.enabled, enabled);
        assert!(actor.retry.at.is_none());
        assert_eq!(actor.retry.attempts, 0);
        assert_eq!(actor.model.reconnect_in_seconds, None);
        assert_eq!(actor.scope.current(), 0);
        assert_eq!(actor.model.notice, "原连接提示");
    }
}

#[tokio::test]
async fn rename_waits_for_persistence_confirmation() {
    let mut actor = actor();
    let (updates, mut receiver) = mpsc::channel(8);
    actor.updates = updates;
    let (confirm, confirmed) = tokio::sync::oneshot::channel();
    let target = binding();
    let mut rename = Box::pin(actor.rename_with(&target, async {
        confirmed.await?;
        Ok(aliases("确认之后"))
    }));
    assert!(futures_util::poll!(rename.as_mut()).is_pending());
    let Update::Model(model) = receiver.try_recv().unwrap() else {
        panic!("expected a model update while saving")
    };
    assert_eq!(model.alias_saving.as_ref(), Some(&target));
    assert_eq!(model.aliases.get(&target), Some("原名称"));
    assert_eq!(model.generation, 0);
    confirm.send(()).unwrap();
    rename.await;
    assert_eq!(actor.model.aliases.get(&target), Some("确认之后"));
    assert!(actor.model.alias_saving.is_none());
}

#[tokio::test]
async fn rename_rejects_stale_binding_without_polling_storage() {
    let mut actor = actor();
    let mut stale = binding();
    stale.host_key = protocol::encode(&[9; 32]);
    actor
        .rename_with(&stale, async { panic!("stale rename must not write") })
        .await;
    assert_eq!(actor.model.aliases.get(&binding()), Some("原名称"));
    assert!(actor.model.error.contains("配对已变化"));
    assert_eq!(actor.scope.current(), 0);
}

#[tokio::test]
async fn rename_command_validation_is_local_and_does_not_change_connections() {
    let mut actor = actor();
    actor
        .command(Command::RenameBinding {
            binding: binding(),
            alias: "主机\n".into(),
        })
        .await
        .unwrap();
    assert!(actor.model.error.contains("控制字符"));
    assert_eq!(actor.model.aliases.get(&binding()), Some("原名称"));
    assert_eq!(actor.scope.current(), 0);
    assert!(!actor.retry.enabled);
}

#[test]
fn rename_queued_from_ui_keeps_generation_and_rejects_old_views() {
    let actor = actor();
    let (sender, mut commands) = mpsc::channel(8);
    let client = Client {
        sender,
        scope: Some(actor.scope.clone()),
    };
    client
        .send_for(
            0,
            Command::RenameBinding {
                binding: binding(),
                alias: "开发主机".into(),
            },
        )
        .unwrap();
    assert_eq!(actor.scope.current(), 0);
    assert!(
        matches!(commands.try_recv().unwrap(), Command::Scoped { generation: 0, command } if matches!(*command, Command::RenameBinding { .. }))
    );
    actor.scope.advance().unwrap();
    assert!(
        client
            .send_for(
                0,
                Command::RenameBinding {
                    binding: binding(),
                    alias: "旧页面".into()
                }
            )
            .is_err()
    );
    assert!(commands.try_recv().is_err());
    assert_eq!(actor.scope.current(), 1);
}

#[test]
fn rename_aliases_restore_without_reenabling_a_manually_stopped_connection() {
    let mut actor = actor();
    assert!(
        actor
            .restore_preferences(storage::Preferences {
                bindings: vec![binding()],
                selected: Some(binding()),
                auto_connect: false,
                aliases: aliases("重启后"),
                host_names: storage::HostNames::default(),
            })
            .is_none()
    );
    assert_eq!(actor.model.aliases.get(&binding()), Some("重启后"));
    assert_eq!(actor.model.active_binding, Some(binding()));
    assert_eq!(actor.scope.current(), 0);
}

fn reported_names(name: &str) -> storage::HostNames {
    let target = binding();
    serde_json::from_value(serde_json::json!([{
        "identity": { "relay": target.relay, "route": target.route, "host_key": target.host_key },
        "name": name,
    }]))
    .unwrap()
}

#[tokio::test]
async fn host_info_uses_current_authenticated_binding_and_never_overrides_an_alias() {
    let mut actor = actor();
    actor.binding = Some(binding());
    actor.model.phase = Phase::Ready;
    actor.model.host_names = reported_names("server.local");
    actor
        .response(Response {
            id: 0,
            event: Event::HostInfo {
                name: "server.local".into(),
            },
        })
        .await
        .unwrap();
    assert_eq!(actor.model.remote_host_name, "server.local");
    assert_eq!(actor.model.host_names.get(&binding()), Some("server.local"));
    assert_eq!(actor.model.aliases.get(&binding()), Some("原名称"));
    assert_eq!(actor.model.generation, 0);
    assert_eq!(actor.scope.current(), 0);
    assert!(!actor.retry.enabled);
    assert!(actor.retry.at.is_none());
    actor.disconnected().await;
    assert!(actor.model.remote_host_name.is_empty());
    assert_eq!(actor.model.host_names.get(&binding()), Some("server.local"));
}

#[tokio::test]
async fn host_info_before_ready_from_local_or_stale_binding_is_ignored_without_storage() {
    for invalid in [
        "phase",
        "local",
        "unsaved",
        "binding",
        "generation",
        "malformed",
    ] {
        let mut actor = actor();
        actor.binding = Some(binding());
        actor.model.phase = Phase::Ready;
        let mut name = "server.local".to_owned();
        match invalid {
            "phase" => actor.model.phase = Phase::AwaitingReady,
            "local" => actor.model.local = true,
            "unsaved" => actor.model.bindings.clear(),
            "binding" => actor.binding.as_mut().unwrap().host_key = protocol::encode(&[9; 32]),
            "generation" => {
                actor.scope.advance().unwrap();
            }
            _ => name.push('\n'),
        }
        actor
            .response(Response {
                id: 0,
                event: Event::HostInfo { name },
            })
            .await
            .unwrap();
        assert!(actor.model.remote_host_name.is_empty(), "{invalid}");
        assert_eq!(actor.model.host_names.get(&binding()), None, "{invalid}");
        assert_eq!(actor.model.aliases.get(&binding()), Some("原名称"));
        assert!(actor.retry.at.is_none());
    }
}

#[tokio::test]
async fn hostname_cache_failure_keeps_live_name_and_does_not_enter_connection_retry() {
    let mut actor = actor();
    actor.model.phase = Phase::Ready;
    actor.binding = Some(binding());
    actor.model.host_names = reported_names("old-server");
    actor.retry.arm();
    actor.retry.schedule(Instant::now());
    let retry_at = actor.retry.at;
    let attempts = actor.retry.attempts;
    let error = std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("{} {}", binding().host_key, binding().route),
    );
    actor
        .reported_name_with(&binding(), "new-server", async { Err(error.into()) })
        .await;
    actor.connection_result(Ok(())).await;
    assert_eq!(actor.model.remote_host_name, "new-server");
    assert_eq!(actor.model.host_names.get(&binding()), Some("old-server"));
    assert_eq!(actor.model.aliases.get(&binding()), Some("原名称"));
    assert_eq!(actor.retry.at, retry_at);
    assert_eq!(actor.retry.attempts, attempts);
    assert!(actor.retry.enabled);
    assert_eq!(actor.model.phase, Phase::Ready);
    assert_eq!(actor.model.generation, 0);
    assert!(actor.model.notice.contains("未能确认保存"));
    assert!(!actor.model.notice.contains(&binding().host_key));
    assert!(!actor.model.notice.contains(&binding().route));
}

#[tokio::test]
async fn hostname_cache_changes_only_after_persistence_confirmation() {
    let mut actor = actor();
    actor.model.phase = Phase::Ready;
    actor.model.host_names = reported_names("old-server");
    let (confirm, confirmed) = tokio::sync::oneshot::channel();
    let target = binding();
    let mut operation = Box::pin(actor.reported_name_with(&target, "new-server", async {
        confirmed.await?;
        Ok(reported_names("new-server"))
    }));
    assert!(futures_util::poll!(operation.as_mut()).is_pending());
    confirm.send(()).unwrap();
    operation.await;
    assert_eq!(actor.model.remote_host_name, "new-server");
    assert_eq!(actor.model.host_names.get(&target), Some("new-server"));
    assert_eq!(actor.model.aliases.get(&target), Some("原名称"));
    assert_eq!(actor.model.generation, 0);
}
