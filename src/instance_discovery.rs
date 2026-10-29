//! User-delegated discovery. A source failure never discards another source's results.
use std::{collections::HashMap, time::Duration};

use crate::{
    http_body::read_limited_body,
    lightos_admin::{self, LightOsAdminError},
    proto::lazycat::webshell::v1::{Instance, InstanceKind},
};
use axum::http::{HeaderMap, StatusCode};

const CATFLAP_ORIGINS: [&str; 2] = [
    "http://app.community.lazycat.app.catflap.lzcx",
    "http://app.community.lazycat.app.catflap.dev.lzcx",
];

#[derive(Default)]
pub(crate) struct Discovery {
    pub instances: Vec<Instance>,
    pub warnings: Vec<String>,
}

pub(crate) async fn discover(headers: &HeaderMap) -> Result<Discovery, LightOsAdminError> {
    let account =
        lightos_admin::current_request_account_id(headers).ok_or_else(|| LightOsAdminError {
            status: StatusCode::UNAUTHORIZED,
            message: "account id is required".into(),
        })?;
    let base = lightos_admin::admin_base_url_for_request(headers).await?;
    let client = lightos_admin::admin_http_client(headers, Duration::from_secs(10))?;
    let upstream = lightos_admin::build_upstream_headers(headers, &account).map_err(|message| {
        LightOsAdminError {
            status: StatusCode::UNAUTHORIZED,
            message,
        }
    })?;
    let mut endpoints = vec![
        (base.as_str(), "/api/webshell/instances", "lightos", false),
        (base.as_str(), "/api/client-instances", "lightos", true),
    ];
    // Legacy credentials must never be forwarded to a different application.
    if headers.contains_key("x-hc-user-ticket") {
        for origin in CATFLAP_ORIGINS {
            endpoints.push((origin, "/api/webshell/targets", "catflap", false));
            endpoints.push((origin, "/api/client-instances", "catflap", true));
        }
    }
    discover_as(&client, &upstream, &endpoints, &account).await
}

type Endpoint<'a> = (&'a str, &'a str, &'a str, bool);

#[cfg(test)]
pub(crate) async fn discover_from(
    client: &reqwest::Client,
    headers: &HeaderMap,
    endpoints: &[Endpoint<'_>],
) -> Result<Discovery, LightOsAdminError> {
    let account = lightos_admin::current_request_account_id(headers).unwrap_or_default();
    discover_as(client, headers, endpoints, &account).await
}

async fn discover_as(
    client: &reqwest::Client,
    headers: &HeaderMap,
    endpoints: &[Endpoint<'_>],
    account: &str,
) -> Result<Discovery, LightOsAdminError> {
    let results = futures::future::join_all(endpoints.iter().map(
        |&(base, path, source, remote)| async move {
            (
                source,
                fetch(client, headers, base, path, remote, account).await,
            )
        },
    ))
    .await;
    let mut warnings = Vec::new();
    let results = results
        .into_iter()
        .map(|(source, result)| {
            (
                source,
                result.map(|batch| {
                    warnings.extend(batch.warnings);
                    batch.instances
                }),
            )
        })
        .collect();
    let mut discovery = combine(results)?;
    discovery.warnings.extend(warnings);
    Ok(discovery)
}

async fn fetch(
    client: &reqwest::Client,
    headers: &HeaderMap,
    base: &str,
    path: &str,
    remote: bool,
    account: &str,
) -> Result<Discovery, LightOsAdminError> {
    let error = |status, detail: &str| LightOsAdminError {
        status,
        message: format!("{base}{path}: {detail}"),
    };
    let url = lightos_admin::build_admin_url(base, path)
        .map_err(|_| error(StatusCode::BAD_GATEWAY, "invalid discovery endpoint"))?;
    let mut response = client
        .get(url)
        .headers(headers.clone())
        .send()
        .await
        .map_err(|e| {
            error(
                if e.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
                "discovery request failed",
            )
        })?;
    let mut typed = path == "/api/webshell/targets";
    // Only absence of the new route allows the legacy fallback, never auth or server errors.
    if typed && response.status() == StatusCode::NOT_FOUND {
        let url = lightos_admin::build_admin_url(base, "/api/webshell/instances")
            .map_err(|_| error(StatusCode::BAD_GATEWAY, "invalid legacy endpoint"))?;
        response = client
            .get(url)
            .headers(headers.clone())
            .send()
            .await
            .map_err(|e| {
                error(
                    if e.is_timeout() {
                        StatusCode::GATEWAY_TIMEOUT
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    },
                    "legacy discovery request failed",
                )
            })?;
        typed = false;
    }
    let status = response.status();
    if status != StatusCode::OK {
        // Do not echo upstream bodies: they may contain credentials or platform details.
        return Err(error(status, &format!("HTTP {status}")));
    }
    let body = read_limited_body(response, 10 * 1024 * 1024, "instance discovery")
        .await
        .map_err(|_| error(StatusCode::BAD_GATEWAY, "invalid or oversized response"))?;
    if typed {
        let mut discovery = crate::catflap_targets::parse(&body)
            .map_err(|_| error(StatusCode::BAD_GATEWAY, "invalid Catflap targets JSON"))?;
        let app = match base {
            "http://app.community.lazycat.app.catflap.lzcx" => "community.lazycat.app.catflap",
            "http://app.community.lazycat.app.catflap.dev.lzcx" => {
                "community.lazycat.app.catflap.dev"
            }
            _ => "",
        };
        for instance in &mut discovery.instances {
            instance.provider_app_id = Some(app.to_owned());
        }
        return Ok(discovery);
    }
    (if remote && crate::client_broker::ClientBroker::from_origin(base).is_some() {
        lightos_admin::parse_broker_client_instances(
            &body,
            crate::client_broker::ClientBroker::from_origin(base).unwrap(),
            account,
        )
    } else if remote {
        lightos_admin::parse_client_instances(&body)
    } else {
        lightos_admin::parse_visible_instances(&body)
    })
    .map(|instances| Discovery {
        instances,
        warnings: Vec::new(),
    })
    .map_err(|_| error(StatusCode::BAD_GATEWAY, "invalid instances JSON"))
}

fn combine(
    results: Vec<(&str, Result<Vec<Instance>, LightOsAdminError>)>,
) -> Result<Discovery, LightOsAdminError> {
    let mut output = Discovery::default();
    let mut positions = HashMap::new();
    let mut succeeded = false;
    let mut errors = Vec::new();
    for (source, result) in results {
        match result {
            Ok(instances) => {
                succeeded = true;
                for mut instance in instances {
                    let key = instance_identity(&instance);
                    if let Some(&position) = positions.get(&key) {
                        let existing: &mut Instance = &mut output.instances[position];
                        if instance.kind == Some(InstanceKind::INSTANCE_KIND_REMOTE_CLIENT.into())
                            && client_rank(&instance) > client_rank(existing)
                        {
                            let sources = std::mem::take(&mut existing.sources);
                            *existing = instance.clone();
                            existing.sources = sources;
                        }
                        if !existing.sources.iter().any(|value| value == source) {
                            existing.sources.push(source.to_owned());
                        }
                        // Catflap's owned-instance metadata is authoritative for its login account.
                        if source == "catflap"
                            && instance.kind == Some(InstanceKind::INSTANCE_KIND_LIGHTOS.into())
                            && instance.username.as_deref().is_some_and(|s| !s.is_empty())
                        {
                            existing.username = instance.username;
                        }
                        if source == "catflap"
                            && instance.kind == Some(InstanceKind::INSTANCE_KIND_LIGHTOS.into())
                            && instance.terminal_available.is_some()
                        {
                            existing.terminal_available = instance.terminal_available;
                        }
                    } else {
                        instance.sources = vec![source.to_owned()];
                        positions.insert(key, output.instances.len());
                        output.instances.push(instance);
                    }
                }
            }
            Err(error) => errors.push(error),
        }
    }
    if !succeeded {
        let status = errors
            .iter()
            .find(|e| e.status == StatusCode::UNAUTHORIZED || e.status == StatusCode::FORBIDDEN)
            .map_or(StatusCode::BAD_GATEWAY, |e| e.status);
        return Err(LightOsAdminError {
            status,
            message: errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; "),
        });
    }
    // A missing optional endpoint is normal; other failures remain visible alongside results.
    output.warnings = errors
        .into_iter()
        .filter(|e| e.status != StatusCode::NOT_FOUND)
        .map(|e| e.message)
        .collect();
    output
        .instances
        .sort_by_key(|i| i.status.as_deref() != Some("running"));
    Ok(output)
}

fn instance_identity(
    instance: &Instance,
) -> (
    Option<buffa::EnumValue<InstanceKind>>,
    String,
    Option<String>,
) {
    let physical = instance.kind == Some(InstanceKind::INSTANCE_KIND_REMOTE_CLIENT.into());
    if physical
        && instance
            .physical_device_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
        && instance
            .owner_user_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
    {
        return (
            instance.kind,
            format!("device:{}", instance.owner_user_id.as_deref().unwrap()),
            instance.physical_device_id.clone(),
        );
    }
    let scope = if instance.kind == Some(InstanceKind::INSTANCE_KIND_LIGHTOS.into()) {
        String::new()
    } else {
        instance.provider_app_id.clone().unwrap_or_default()
    };
    (
        instance.kind,
        scope,
        instance.selector.clone().or(instance.target_id.clone()),
    )
}

fn client_rank(instance: &Instance) -> (bool, bool, bool) {
    (
        instance
            .terminal_available
            .unwrap_or(instance.status.as_deref() == Some("running")),
        instance.status.as_deref() == Some("running"),
        instance
            .provider_app_id
            .as_deref()
            .is_none_or(str::is_empty),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::lazycat::webshell::v1::InstanceKind;
    use axum::{Json, Router, routing::get};

    fn item(selector: &str) -> Instance {
        Instance {
            selector: Some(selector.into()),
            status: Some("running".into()),
            kind: Some(InstanceKind::INSTANCE_KIND_LIGHTOS.into()),
            ..Default::default()
        }
    }
    fn failure(status: StatusCode) -> Result<Vec<Instance>, LightOsAdminError> {
        Err(LightOsAdminError {
            status,
            message: format!("source HTTP {status}"),
        })
    }

    #[test]
    fn either_application_can_be_installed_alone() {
        for source in ["lightos", "catflap"] {
            let result = combine(vec![
                (source, Ok(vec![item("work@owner")])),
                ("absent", failure(StatusCode::NOT_FOUND)),
            ])
            .unwrap();
            assert_eq!(result.instances.len(), 1);
            assert_eq!(result.instances[0].sources, [source]);
            assert!(result.warnings.is_empty());
        }
    }

    #[test]
    fn physical_identity_merges_only_known_device_and_owner_and_keeps_ready_broker() {
        let mut official = item("client:official");
        official.kind = Some(InstanceKind::INSTANCE_KIND_REMOTE_CLIENT.into());
        official.physical_device_id = Some("DEVICE-A".into());
        official.owner_user_id = Some("alice".into());
        official.terminal_available = Some(false);
        let mut catflap = official.clone();
        catflap.selector = Some("client:catflap:registration".into());
        catflap.provider_app_id = Some("community.lazycat.app.catflap".into());
        catflap.terminal_available = Some(true);
        let mut other_owner = catflap.clone();
        other_owner.owner_user_id = Some("bob".into());
        let mut other_device = catflap.clone();
        other_device.physical_device_id = Some("DEVICE-B".into());
        let result = combine(vec![
            ("lightos", Ok(vec![official])),
            (
                "catflap",
                Ok(vec![
                    catflap.clone(),
                    catflap.clone(),
                    other_owner,
                    other_device,
                ]),
            ),
        ])
        .unwrap();
        assert_eq!(result.instances.len(), 3);
        assert_eq!(result.instances[0].selector, catflap.selector);
        assert_eq!(result.instances[0].sources, ["lightos", "catflap"]);
    }

    #[test]
    fn unknown_device_identity_and_microvm_providers_never_merge_by_name() {
        for kind in [
            InstanceKind::INSTANCE_KIND_REMOTE_CLIENT,
            InstanceKind::INSTANCE_KIND_SMOLVM,
        ] {
            let mut first = item("same-id");
            first.kind = Some(kind.into());
            first.name = Some("My computer".into());
            first.provider_app_id = Some("community.lazycat.app.catflap".into());
            let mut second = first.clone();
            second.provider_app_id = Some("community.lazycat.app.catflap.dev".into());
            assert_eq!(
                combine(vec![("catflap", Ok(vec![first.clone(), first, second]))])
                    .unwrap()
                    .instances
                    .len(),
                2
            );
        }
    }

    #[test]
    fn merges_full_selector_and_kind_but_preserves_sources_and_fresh_username() {
        let mut catflap = item("work@a");
        catflap.username = Some("alice".into());
        let mut remote = item("work@a");
        remote.kind = Some(InstanceKind::INSTANCE_KIND_REMOTE_CLIENT.into());
        let result = combine(vec![
            ("lightos", Ok(vec![item("work@a"), item("work@b"), remote])),
            ("catflap", Ok(vec![catflap.clone(), catflap])),
        ])
        .unwrap();
        assert_eq!(result.instances.len(), 3);
        assert_eq!(result.instances[0].sources, ["lightos", "catflap"]);
        assert_eq!(result.instances[0].username.as_deref(), Some("alice"));
    }

    #[test]
    fn failures_remain_visible_without_discarding_success() {
        for status in [
            StatusCode::FORBIDDEN,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            let result = combine(vec![
                ("lightos", failure(status)),
                ("catflap", Ok(vec![item("work@a")])),
            ])
            .unwrap();
            assert_eq!(result.instances.len(), 1);
            assert_eq!(result.warnings.len(), 1);
        }
        assert!(
            combine(vec![
                ("lightos", failure(StatusCode::NOT_FOUND)),
                ("catflap", failure(StatusCode::FORBIDDEN))
            ])
            .is_err()
        );
        assert!(
            combine(vec![("catflap", Ok(vec![]))])
                .unwrap()
                .instances
                .is_empty()
        );
    }

    #[tokio::test]
    async fn independent_endpoints_use_current_ticket_and_never_browser_identity() {
        let app = Router::new().route("/api/webshell/instances", get(|headers: HeaderMap| async move {
            for key in ["cookie", "authorization", "x-hc-user-id", "x-hc-user-role"] {
                assert!(!headers.contains_key(key));
            }
            let owner = match headers.get("x-hc-user-ticket").unwrap().to_str().unwrap() {
                "fixture-alice" => "alice", "fixture-bob" => "bob", _ => panic!("unexpected fixture"),
            };
            Json(serde_json::json!([{"selector": format!("work@{owner}"), "username": owner, "status":"running", "init_progress":100, "future_field":true}]))
        })).route("/api/client-instances", get(|| async { StatusCode::SERVICE_UNAVAILABLE }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for owner in ["alice", "bob"] {
            let mut incoming = HeaderMap::new();
            incoming.insert(
                "x-hc-user-ticket",
                format!("fixture-{owner}").parse().unwrap(),
            );
            incoming.insert("cookie", "private-browser-cookie".parse().unwrap());
            incoming.insert(
                "authorization",
                "private-browser-authorization".parse().unwrap(),
            );
            incoming.insert("x-hc-user-id", "forged".parse().unwrap());
            let headers = lightos_admin::build_upstream_headers(&incoming, "forged").unwrap();
            let client =
                lightos_admin::admin_http_client(&incoming, Duration::from_secs(2)).unwrap();
            let result = discover_from(
                &client,
                &headers,
                &[
                    (&base, "/missing", "lightos", false),
                    (&base, "/api/webshell/instances", "catflap", false),
                    (&base, "/api/client-instances", "lightos", true),
                ],
            )
            .await
            .unwrap();
            assert_eq!(result.instances.len(), 1);
            assert_eq!(
                result.instances[0].selector.as_deref(),
                Some(format!("work@{owner}").as_str())
            );
            assert_eq!(result.instances[0].username.as_deref(), Some(owner));
            assert_eq!(result.warnings.len(), 1);
        }
        server.abort();
    }

    #[tokio::test]
    async fn malformed_json_and_redirects_are_not_empty_success() {
        let app = Router::new()
            .route("/bad", get(|| async { "{broken" }))
            .route(
                "/redirect",
                get(|| async { axum::response::Redirect::temporary("/target") }),
            )
            .route(
                "/target",
                get(|| async {
                    panic!("must not follow redirects");
                    #[allow(unreachable_code)]
                    "[]"
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut headers = HeaderMap::new();
        headers.insert("x-hc-user-ticket", "fixture".parse().unwrap());
        let client = lightos_admin::admin_http_client(&headers, Duration::from_secs(2)).unwrap();
        for path in ["/bad", "/redirect"] {
            assert!(
                discover_from(&client, &headers, &[(&base, path, "catflap", false)])
                    .await
                    .is_err()
            );
        }
        server.abort();
    }
    #[tokio::test]
    async fn installation_matrix_and_legacy_fallback_use_real_http_contracts() {
        let legacy = serde_json::json!([{"selector":"work@owner","name":"work","username":"alice","status":"running"}]);
        let targets = serde_json::json!({"version":1,"targets":[
            {"kind":"lightos","selector":"work@owner","name":"work","username":"alice","status":"running","capabilities":{"terminal":true,"desktop":false}},
            {"kind":"smolvm","id":"12345678-1234-4234-8234-123456789abc","name":"vm","status":"running","capabilities":{"terminal":false,"desktop":false}}
        ],"sources":[{"kind":"lightos","state":"ok","code":null},{"kind":"smolvm","state":"ok","code":null}]});
        let app = Router::new()
            .route(
                "/official/api/webshell/instances",
                get({
                    let legacy = legacy.clone();
                    move || async move { Json(legacy) }
                }),
            )
            .route(
                "/catflap/api/webshell/targets",
                get(move || async move { Json(targets) }),
            )
            .route(
                "/legacy/api/webshell/instances",
                get(move || async move { Json(legacy) }),
            )
            .route(
                "/denied/api/webshell/targets",
                get(|| async { StatusCode::FORBIDDEN }),
            )
            .route(
                "/denied/api/webshell/instances",
                get(|| async {
                    panic!("must not fallback after auth failure");
                    #[allow(unreachable_code)]
                    "[]"
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let official = format!("{base}/official");
        let catflap = format!("{base}/catflap");
        let missing = format!("{base}/missing");
        let legacy = format!("{base}/legacy");
        let denied = format!("{base}/denied");
        let mut headers = HeaderMap::new();
        headers.insert("x-hc-user-ticket", "fixture".parse().unwrap());
        let client = lightos_admin::admin_http_client(&headers, Duration::from_secs(2)).unwrap();
        for (lightos, cat, expected) in [
            (&official, &missing, 1),
            (&missing, &catflap, 2),
            (&official, &catflap, 2),
            (&missing, &legacy, 1),
        ] {
            let result = discover_from(
                &client,
                &headers,
                &[
                    (lightos, "/api/webshell/instances", "lightos", false),
                    (cat, "/api/webshell/targets", "catflap", false),
                ],
            )
            .await
            .unwrap();
            assert_eq!(result.instances.len(), expected);
            assert!(result.warnings.is_empty());
            if lightos == &official && cat == &catflap {
                assert_eq!(result.instances[0].sources, ["lightos", "catflap"]);
                assert!(result.instances[1].selector.is_none());
            }
        }
        let error = discover_from(
            &client,
            &headers,
            &[(&denied, "/api/webshell/targets", "catflap", false)],
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        server.abort();
    }
}
