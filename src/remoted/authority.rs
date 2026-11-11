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

fn deployed_owner() -> Result<String> {
    std::env::var("LAZYCAT_APP_DEPLOY_UID")
        .ok()
        .filter(|value| !value.is_empty())
        .context("缺少平台部署用户，无法验证所有者")
}

fn application_request(
    ids: Vec<String>,
    deployed_owner: &str,
) -> Result<tonic::Request<QueryApplicationRequest>> {
    ensure!(!deployed_owner.is_empty(), "缺少平台部署用户");
    let mut request = tonic::Request::new(QueryApplicationRequest {
        deploy_ids: ids,
        ignore_pending_pkg: Some(true),
        ..Default::default()
    });
    // This compatibility context scopes SDK results; returned ownership is still verified.
    lzc_sdk::with_real_uid(&mut request, deployed_owner).context("平台部署用户上下文无效")?;
    Ok(request)
}

async fn apps(ids: Vec<String>) -> Result<Vec<AppInfo>> {
    let request = application_request(ids, &deployed_owner()?)?;
    let api = GATEWAY
        .get_or_try_init(|| async {
            timeout(Duration::from_secs(8), ApiGateway::connect())
                .await
                .context("platform connection timed out")?
                .context("platform ownership service unavailable")
        })
        .await?;
    let response = timeout(
        Duration::from_secs(8),
        api.package_manager().query_application(request),
    )
    .await
    .context("platform ownership check timed out")??;
    Ok(response.into_inner().info_list)
}

#[derive(Debug)]
struct DeploymentIdentity<'a> {
    appid: &'a str,
    owner: &'a str,
    domain: &'a str,
    deployment: Option<&'a str>,
}

impl<'a> DeploymentIdentity<'a> {
    fn new(
        appid: &'a str,
        owner: &'a str,
        domain: &'a str,
        primary: Option<&'a str>,
        alternate: Option<&'a str>,
    ) -> Result<Self> {
        ensure!(
            [appid, owner, domain]
                .iter()
                .all(|value| !value.is_empty() && value.trim() == *value),
            "缺少平台应用、部署用户或域名，无法验证部署身份"
        );
        let primary = primary.filter(|value| !value.is_empty());
        let alternate = alternate.filter(|value| !value.is_empty());
        ensure!(
            primary
                .zip(alternate)
                .is_none_or(|(left, right)| left == right),
            "部署身份冲突，无法验证所有者"
        );
        Ok(Self {
            appid,
            owner,
            domain,
            deployment: primary.or(alternate),
        })
    }
}

pub async fn owner() -> Result<String> {
    ensure!(
        crate::tty_init::lightos_features_enabled(),
        "远程控制需要平台部署身份"
    );
    let appid = std::env::var("LAZYCAT_APP_ID").unwrap_or_default();
    let user = deployed_owner()?;
    let domain = std::env::var("LAZYCAT_APP_DOMAIN").unwrap_or_default();
    let primary = std::env::var("LAZYCAT_APP_DEPLOY_ID").ok();
    let alternate = std::env::var("LAZYCAT_DEPLOY_ID").ok();
    let identity = DeploymentIdentity::new(
        &appid,
        &user,
        &domain,
        primary.as_deref(),
        alternate.as_deref(),
    )?;
    owner_with(&identity, apps).await
}

async fn owner_with<F, Fut>(identity: &DeploymentIdentity<'_>, fetch: F) -> Result<String>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<AppInfo>>>,
{
    let records = fetch(identity.deployment.into_iter().map(str::to_owned).collect()).await?;
    // APP_ID is not a deployment ID. Bind the SDK's actual instance to all injected identity fields.
    let mut matches = records.into_iter().filter(|app| {
        app.appid == identity.appid
            && app.owner == identity.owner
            && app.domain.as_deref() == Some(identity.domain)
            && identity
                .deployment
                .is_none_or(|deployment| app.deploy_id == deployment)
    });
    let item = matches.next().context("无法验证当前部署的所有者")?;
    ensure!(matches.next().is_none(), "部署身份不唯一，拒绝猜测所有者");
    ensure!(
        !item.deploy_id.is_empty()
            && !item.resource_only
            && item.status == lzc_sdk::proto::sys::AppStatus::Installed as i32,
        "当前部署不是有效的已安装应用实例"
    );
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
    fn application_lookup_carries_deployment_user_context() {
        let request = application_request(vec!["instance".into()], "account").unwrap();
        assert_eq!(
            request
                .metadata()
                .get("x-hc-user-id")
                .map(|value| value.to_str().unwrap()),
            Some("account"),
            "the platform rejects unscoped application queries with uid is empty"
        );
        assert_eq!(request.get_ref().deploy_ids, vec!["instance"]);
        assert!(request.get_ref().other_uid.is_none());
    }

    fn identity(deployment: Option<&str>) -> DeploymentIdentity<'_> {
        DeploymentIdentity::new("app", "account", "app.example", deployment, None).unwrap()
    }

    #[test]
    fn application_lookup_rejects_missing_or_invalid_context() {
        assert!(application_request(vec![], "").is_err());
        assert!(application_request(vec![], "account\nforged").is_err());
    }

    #[test]
    fn deployment_identity_requires_platform_fields_and_consistent_ids() {
        for (app, owner, domain) in [
            ("", "account", "app.example"),
            ("app", "", "app.example"),
            ("app", "account", ""),
        ] {
            assert!(DeploymentIdentity::new(app, owner, domain, None, None).is_err());
        }
        assert!(
            DeploymentIdentity::new("app", "account", "app.example", Some("one"), Some("two"))
                .is_err()
        );
        assert_eq!(
            DeploymentIdentity::new("app", "account", "app.example", None, Some("one"))
                .unwrap()
                .deployment,
            Some("one")
        );
        assert!(identity(None).deployment.is_none());
    }

    fn record(deploy: &str, owner: &str) -> AppInfo {
        AppInfo {
            appid: "app".into(),
            deploy_id: deploy.into(),
            owner: owner.into(),
            domain: Some("app.example".into()),
            status: lzc_sdk::proto::sys::AppStatus::Installed as i32,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn owner_uses_exact_sdk_deployment_and_returns_account_not_deploy() {
        let owner = owner_with(&identity(Some("instance")), |requested| async move {
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
    async fn missing_deploy_id_uses_sdk_instance_bound_to_app_owner_and_domain() {
        let current = record("sdk-instance-uuid", "account");
        let other_app = AppInfo {
            appid: "other".into(),
            ..current.clone()
        };
        let other_domain = AppInfo {
            domain: Some("other.example".into()),
            ..current.clone()
        };
        let result = owner_with(&identity(None), |requested| async move {
            assert!(
                requested.is_empty(),
                "do not mistake the application ID for a deployment ID"
            );
            Ok(vec![
                other_app,
                other_domain,
                record("other-owner", "someone"),
                current,
            ])
        })
        .await
        .unwrap();
        assert_eq!(result, "account");
    }

    #[tokio::test]
    async fn owner_never_uses_visible_other_instance_or_invalid_owner() {
        for records in [
            vec![],
            vec![record("other-instance", "account")],
            vec![record("instance", "")],
            vec![record("instance", "account"), record("instance", "account")],
            vec![AppInfo {
                resource_only: true,
                ..record("instance", "account")
            }],
            vec![AppInfo {
                status: 0,
                ..record("instance", "account")
            }],
        ] {
            assert!(
                owner_with(&identity(Some("instance")), |_| async { Ok(records) })
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn absent_deploy_id_still_rejects_ambiguous_or_empty_sdk_identity() {
        for records in [
            vec![record("one", "account"), record("two", "account")],
            vec![record("", "account")],
            vec![AppInfo {
                appid: "other".into(),
                ..record("one", "account")
            }],
            vec![AppInfo {
                domain: None,
                ..record("one", "account")
            }],
        ] {
            assert!(
                owner_with(&identity(None), |_| async { Ok(records) })
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn sdk_failure_is_an_error() {
        assert!(
            owner_with(&identity(None), |_| async {
                anyhow::bail!("SDK unavailable")
            })
            .await
            .is_err()
        );
    }
}
