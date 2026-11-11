//! Runtime application ownership, not browser claims or process-global target visibility.
use super::store::Grant;
use crate::{
    ssh_backend,
    state::{AppState, METADATA_LOGIN_USER, SessionRecord},
};
use anyhow::{Context, Result, bail, ensure};
use axum::http::HeaderMap;
use lzc_sdk::{
    ApiGateway,
    proto::sys::{AppInfo, QueryApplicationRequest},
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::OnceCell, time::timeout};

static GATEWAY: OnceCell<ApiGateway> = OnceCell::const_new();
async fn apps(ids: Vec<String>) -> Result<Vec<AppInfo>> {
    let api = GATEWAY
        .get_or_try_init(|| async {
            timeout(Duration::from_secs(8), ApiGateway::connect())
                .await
                .context("platform connection timed out")?
                .context("platform ownership service unavailable")
        })
        .await?;
    let mut client = api.package_manager();
    let response = timeout(
        Duration::from_secs(8),
        client.query_application(QueryApplicationRequest {
            deploy_ids: ids,
            ..Default::default()
        }),
    )
    .await
    .context("platform ownership check timed out")??;
    Ok(response.into_inner().info_list)
}
pub async fn owner() -> Result<String> {
    ensure!(
        crate::tty_init::lightos_features_enabled(),
        "远程控制需要平台部署身份"
    );
    let primary = std::env::var("LAZYCAT_APP_DEPLOY_ID").ok();
    let alternate = std::env::var("LAZYCAT_DEPLOY_ID").ok();
    let app = std::env::var("LAZYCAT_APP_ID").ok();
    let deploy = deployment_candidate(primary.as_deref(), alternate.as_deref(), app.as_deref())?;
    owner_with(deploy, apps).await
}
fn deployment_candidate<'a>(
    primary: Option<&'a str>,
    alternate: Option<&'a str>,
    app: Option<&'a str>,
) -> Result<&'a str> {
    let primary = primary.filter(|value| !value.is_empty());
    let alternate = alternate.filter(|value| !value.is_empty());
    ensure!(
        primary
            .zip(alternate)
            .is_none_or(|(left, right)| left == right),
        "部署身份冲突，无法验证所有者"
    );
    primary
        .or(alternate)
        .or(app.filter(|value| !value.is_empty()))
        .context("缺少当前部署身份，无法验证所有者")
}
async fn owner_with<F, Fut>(deploy: &str, fetch: F) -> Result<String>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<AppInfo>>>,
{
    let records = fetch(vec![deploy.to_owned()]).await?;
    let mut matches = records.into_iter().filter(|app| app.deploy_id == deploy);
    let item = matches.next().context("无法验证当前部署的所有者")?;
    ensure!(matches.next().is_none(), "部署身份不唯一，拒绝猜测所有者");
    ensure!(!item.owner.is_empty(), "无法验证当前部署的所有者");
    Ok(item.owner)
}
pub async fn management(headers: &HeaderMap) -> Result<String> {
    ensure!(
        headers
            .get("x-neko-remoted-request")
            .is_some_and(|v| v == "1"),
        "缺少远程控制请求标记"
    );
    let account = headers
        .get("x-hc-user-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .context("需要平台用户认证")?;
    ensure!(
        headers.get("x-hc-user-ticket").is_some(),
        "需要当前平台用户委托"
    );
    let current = owner().await?;
    ensure!(account == current, "只有当前部署所有者可以管理远程设备");
    Ok(current)
}

pub async fn authorize(
    state: &Arc<AppState>,
    grant: &Grant,
    owner_id: &str,
) -> Result<SessionRecord> {
    ensure!(owner().await? == owner_id, "部署所有者已经变化");
    let session = state
        .sessions
        .read()
        .map_err(|_| anyhow::anyhow!("session store unavailable"))?
        .get(&grant.session)
        .cloned()
        .context("已授权会话不存在")?;
    ensure!(
        session.selector == grant.selector
            && session
                .metadata
                .get(METADATA_LOGIN_USER)
                .cloned()
                .unwrap_or_default()
                == grant.login
            && session
                .metadata
                .get("sessionBackend")
                .map_or("webshell", String::as_str)
                == grant.backend,
        "会话目标身份已变化，请重新授权"
    );
    ensure!(session.status != "closed", "会话已经关闭");
    if ssh_backend::is_ssh_selector(&grant.selector) {
        let profile = ssh_backend::load_enabled_profile(&state.database(), &grant.selector)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let (_, args) = ssh_backend::terminal_command_for_profile(&profile);
        ensure!(
            neko_remoted_protocol::fingerprint(serde_json::to_vec(&args)?.as_slice())
                == grant.target_signature,
            "SSH目标配置已变化"
        );
    } else {
        ensure!(
            !crate::lightos_admin::is_client_selector(&grant.selector),
            "该目标需要短期平台票据，不能持久委托"
        );
        let (_, deployment) = grant
            .selector
            .rsplit_once('@')
            .context("完整目标标识缺失")?;
        let targets = apps(vec![deployment.into()]).await?;
        ensure!(
            targets
                .iter()
                .any(|a| a.deploy_id == deployment && a.owner == owner_id),
            "无法验证目标归属"
        );
        let login = crate::lightos::login_user_for_selector(&grant.selector, true)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        ensure!(login == grant.login, "目标登录用户已变化");
    }
    if !matches!(
        grant.backend.as_str(),
        "webshell" | "ssh" | "herdr" | "hc" | "zellij"
    ) {
        bail!("该会话后端暂不支持远程控制");
    }
    Ok(session)
}
pub async fn grant(
    state: &Arc<AppState>,
    owner_id: &str,
    session_id: &str,
    root: Option<String>,
) -> Result<Grant> {
    let record = state
        .sessions
        .read()
        .map_err(|_| anyhow::anyhow!("session store unavailable"))?
        .get(session_id)
        .cloned()
        .context("会话不存在")?;
    let signature = if ssh_backend::is_ssh_selector(&record.selector) {
        let profile = ssh_backend::load_enabled_profile(&state.database(), &record.selector)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        neko_remoted_protocol::fingerprint(&serde_json::to_vec(
            &ssh_backend::terminal_command_for_profile(&profile).1,
        )?)
    } else {
        String::new()
    };
    let mut grant = Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: record.id.clone(),
        selector: record.selector.clone(),
        login: record
            .metadata
            .get(METADATA_LOGIN_USER)
            .cloned()
            .unwrap_or_default(),
        backend: record
            .metadata
            .get("sessionBackend")
            .cloned()
            .unwrap_or_else(|| "webshell".into()),
        name: record
            .metadata
            .get("label")
            .cloned()
            .unwrap_or_else(|| record.host.clone()),
        target_signature: signature,
        root,
        root_identity: None,
    };
    authorize(state, &grant, owner_id).await?;
    if grant.root.is_some() {
        let mut helper = super::files::Helper::start(state, &grant, true).await?;
        grant.root_identity = Some(helper.identity.clone());
        helper.close().await;
    }
    Ok(grant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_candidates_are_explicit_and_conflicts_fail_closed() {
        assert_eq!(
            deployment_candidate(Some("instance"), None, Some("app")).unwrap(),
            "instance"
        );
        assert_eq!(
            deployment_candidate(None, Some("instance"), Some("app")).unwrap(),
            "instance"
        );
        assert_eq!(
            deployment_candidate(None, None, Some("exact-deploy")).unwrap(),
            "exact-deploy"
        );
        assert!(deployment_candidate(Some("first"), Some("second"), None).is_err());
        assert!(deployment_candidate(Some(""), None, Some("")).is_err());
    }

    fn record(deploy: &str, owner: &str) -> AppInfo {
        AppInfo {
            deploy_id: deploy.into(),
            owner: owner.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn owner_uses_exact_sdk_deployment_and_returns_account_not_deploy() {
        let owner = owner_with("instance", |requested| async move {
            assert_eq!(requested, vec!["instance"]);
            Ok(vec![
                record("other", "someone"),
                record("instance", "account"),
            ])
        })
        .await
        .unwrap();
        assert_eq!(owner, "account");
    }

    #[tokio::test]
    async fn owner_never_uses_visible_other_instance_or_empty_owner() {
        for records in [
            vec![],
            vec![record("other-instance", "account")],
            vec![record("instance", "")],
            vec![record("instance", "account"), record("instance", "other")],
        ] {
            assert!(
                owner_with("instance", |_| async { Ok(records) })
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn app_id_candidate_requires_sdk_deploy_match_even_when_appid_matches() {
        let visible = AppInfo {
            appid: "app".into(),
            ..record("other-deploy", "account")
        };
        assert!(
            owner_with("app", |_| async { Ok(vec![visible]) })
                .await
                .is_err()
        );
        assert_eq!(
            owner_with("app", |_| async { Ok(vec![record("app", "account")]) })
                .await
                .unwrap(),
            "account"
        );
    }

    #[tokio::test]
    async fn sdk_failure_is_an_error() {
        assert!(
            owner_with("instance", |_| async { anyhow::bail!("SDK unavailable") })
                .await
                .is_err()
        );
    }
}
