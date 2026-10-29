//! Delegated `MicroVM` terminal bridge; Catflap owns sessions and target authorization.
use crate::{http_body::read_limited_body, lightos_admin};
use axum::{
    Json,
    extract::{
        OriginalUri, Path, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        Message as UpstreamMessage, client::IntoClientRequest, protocol::WebSocketConfig,
    },
};

const PROTOCOL: &str = "catflap.terminal.v1";
const DEADLINE: Duration = Duration::from_secs(10);
const MAX_FRAME: usize = 1024 * 1024;
type ApiError = (StatusCode, String);

pub(crate) fn origin(app: &str) -> Result<&'static str, ApiError> {
    match app {
        "community.lazycat.app.catflap" => Ok("http://app.community.lazycat.app.catflap.lzcx"),
        "community.lazycat.app.catflap.dev" => {
            Ok("http://app.community.lazycat.app.catflap.dev.lzcx")
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            "unsupported terminal provider".into(),
        )),
    }
}
fn uuid(value: &str) -> Result<(), ApiError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid terminal identity".into()))
}
fn delegated(headers: &HeaderMap) -> Result<HeaderMap, ApiError> {
    if !headers.contains_key("x-hc-user-ticket") {
        return Err((
            StatusCode::UNAUTHORIZED,
            "platform user delegation is required".into(),
        ));
    }
    lightos_admin::build_upstream_headers(headers, "").map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            "invalid platform user delegation".into(),
        )
    })
}
fn local_request(headers: &HeaderMap) -> Result<(), ApiError> {
    // Browser WebSockets carry ambient platform credentials. Reject cross-site upgrades.
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin terminal request rejected".into(),
        ));
    }
    if let Some(value) = headers.get("origin") {
        let url = value.to_str().ok().and_then(|v| url::Url::parse(v).ok());
        let host = headers.get("host").and_then(|v| v.to_str().ok());
        if url
            .as_ref()
            .and_then(|u| {
                u.host_str()
                    .map(|_| u[url::Position::BeforeHost..url::Position::AfterPort].to_owned())
            })
            .as_deref()
            != host
        {
            return Err((
                StatusCode::FORBIDDEN,
                "cross-origin terminal request rejected".into(),
            ));
        }
    }
    Ok(())
}
fn error(status: StatusCode) -> ApiError {
    let status = if status.is_client_error() || status.is_server_error() {
        status
    } else {
        StatusCode::BAD_GATEWAY
    };
    (
        status,
        format!("Catflap terminal request failed (HTTP {})", status.as_u16()),
    )
}
fn network_error(e: &reqwest::Error) -> ApiError {
    error(if e.is_timeout() {
        StatusCode::GATEWAY_TIMEOUT
    } else {
        StatusCode::BAD_GATEWAY
    })
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateSession {
    session_id: String,
    cols: u16,
    rows: u16,
}

// Upstream responses can gain fields independently of this provider. Keep the
// browser request strict while accepting additional response metadata.
#[derive(Deserialize)]
struct CreatedSession {
    session_id: String,
    cols: u16,
    rows: u16,
}
impl CreateSession {
    fn validate(&self) -> Result<(), ApiError> {
        uuid(&self.session_id)?;
        if !(1..=500).contains(&self.cols) || !(1..=500).contains(&self.rows) {
            return Err((
                StatusCode::BAD_REQUEST,
                "terminal size must be within 1..500".into(),
            ));
        }
        Ok(())
    }
}
async fn create_at(
    base: &str,
    vm: &str,
    headers: &HeaderMap,
    body: &CreateSession,
) -> Result<CreateSession, ApiError> {
    uuid(vm)?;
    body.validate()?;
    let mut upstream = delegated(headers)?;
    upstream.insert("x-catflap-request", HeaderValue::from_static("1"));
    let client =
        lightos_admin::admin_http_client(headers, DEADLINE).map_err(|e| (e.status, e.message))?;
    let response = client
        .post(format!(
            "{base}/api/smolvm/instances/{vm}/terminal/sessions"
        ))
        .headers(upstream)
        .json(body)
        .send()
        .await
        .map_err(|error| network_error(&error))?;
    if response.status() != StatusCode::OK {
        return Err(error(response.status()));
    }
    let bytes = read_limited_body(response, 16 * 1024, "terminal session")
        .await
        .map_err(|_| error(StatusCode::BAD_GATEWAY))?;
    let created: CreatedSession =
        serde_json::from_slice(&bytes).map_err(|_| error(StatusCode::BAD_GATEWAY))?;
    if created.session_id != body.session_id
        || created.cols != body.cols
        || created.rows != body.rows
    {
        return Err(error(StatusCode::BAD_GATEWAY));
    }
    Ok(body.clone())
}
pub(crate) async fn create(
    Path((app, vm)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<CreateSession>,
) -> Result<Json<CreateSession>, ApiError> {
    local_request(&headers)?;
    Ok(Json(create_at(origin(&app)?, &vm, &headers, &body).await?))
}
pub(crate) async fn close(
    Path((app, session)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    local_request(&headers)?;
    uuid(&session)?;
    let mut upstream = delegated(&headers)?;
    upstream.insert("x-catflap-request", HeaderValue::from_static("1"));
    let client =
        lightos_admin::admin_http_client(&headers, DEADLINE).map_err(|e| (e.status, e.message))?;
    let response = client
        .delete(format!(
            "{}/api/smolvm/terminal/sessions/{session}",
            origin(&app)?
        ))
        .headers(upstream)
        .send()
        .await
        .map_err(|error| network_error(&error))?;
    if response.status() != StatusCode::NO_CONTENT {
        return Err(error(response.status()));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn attach(
    Path((app, session)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    local_request(&headers)?;
    uuid(&session)?;
    if uri.query().is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "terminal query parameters are unsupported".into(),
        ));
    }
    let base = origin(&app)?;
    let headers = delegated(&headers)?;
    Ok(ws
        .protocols([PROTOCOL])
        .max_message_size(MAX_FRAME)
        .max_frame_size(MAX_FRAME)
        .on_upgrade(move |browser| relay(browser, base.to_owned(), session, headers))
        .into_response())
}
fn upstream_request(
    base: &str,
    session: &str,
    headers: &HeaderMap,
) -> Result<http::Request<()>, ApiError> {
    uuid(session)?;
    let url = format!(
        "{}/api/smolvm/terminal/sessions/{session}/ws",
        base.replacen("http://", "ws://", 1)
    );
    let mut request = url
        .into_client_request()
        .map_err(|_| error(StatusCode::BAD_GATEWAY))?;
    request
        .headers_mut()
        .insert("sec-websocket-protocol", HeaderValue::from_static(PROTOCOL));
    request.headers_mut().insert(
        "x-hc-user-ticket",
        headers
            .get("x-hc-user-ticket")
            .ok_or_else(|| error(StatusCode::UNAUTHORIZED))?
            .clone(),
    );
    Ok(request)
}
fn control(text: &str) -> Option<String> {
    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Resize {
        r#type: String,
        cols: u16,
        rows: u16,
    }
    if text.len() > 1024 {
        return None;
    }
    let value: Resize = serde_json::from_str(text).ok()?;
    if value.r#type != "resize"
        || !(1..=500).contains(&value.cols)
        || !(1..=500).contains(&value.rows)
    {
        return None;
    }
    serde_json::to_string(&value).ok()
}
async fn notify(browser: &mut WebSocket, retryable: bool) {
    let text = serde_json::json!({"type":"error","message":"Catflap terminal connection unavailable","fatal":!retryable,"retryable":retryable}).to_string();
    let _ = timeout(DEADLINE, browser.send(Message::Text(text.into()))).await;
}
async fn relay(mut browser: WebSocket, base: String, session: String, headers: HeaderMap) {
    let Ok(request) = upstream_request(&base, &session, &headers) else {
        notify(&mut browser, false).await;
        return;
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let connected = timeout(
        DEADLINE,
        connect_async_with_config(request, Some(config), false),
    )
    .await;
    let (mut upstream, response) = match connected {
        Ok(Ok(value)) => value,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
            notify(&mut browser, response.status().is_server_error()).await;
            return;
        }
        _ => {
            notify(&mut browser, true).await;
            return;
        }
    };
    if response
        .headers()
        .get("sec-websocket-protocol")
        .is_none_or(|v| v != PROTOCOL)
    {
        notify(&mut browser, false).await;
        let _ = timeout(DEADLINE, upstream.close(None)).await;
        return;
    }
    loop {
        tokio::select! {
            message = browser.recv() => {
                let message = match message {
                    Some(Ok(Message::Binary(bytes))) => UpstreamMessage::Binary(bytes),
                    Some(Ok(Message::Text(text))) => if let Some(text) = control(text.as_str()) { UpstreamMessage::Text(text.into()) } else { notify(&mut browser, false).await; break; },
                    Some(Ok(Message::Ping(_)|Message::Pong(_))) => continue,
                    _ => break,
                };
                if !matches!(timeout(DEADLINE, upstream.send(message)).await, Ok(Ok(()))) { notify(&mut browser, true).await; break; }
            }
            message = upstream.next() => {
                let message = match message {
                    Some(Ok(UpstreamMessage::Binary(bytes))) => Message::Binary(bytes),
                    Some(Ok(UpstreamMessage::Text(text))) if text.len() <= 16*1024 => Message::Text(text.as_str().into()),
                    Some(Ok(UpstreamMessage::Ping(bytes))) => { if !matches!(timeout(DEADLINE, upstream.send(UpstreamMessage::Pong(bytes))).await, Ok(Ok(()))) { break; } continue; },
                    Some(Ok(UpstreamMessage::Pong(_))) => continue,
                    Some(Ok(UpstreamMessage::Close(_))) | None => break,
                    _ => { notify(&mut browser, true).await; break; },
                };
                if !matches!(timeout(DEADLINE, browser.send(message)).await, Ok(Ok(()))) { break; }
            }
        }
    }
    // Disconnect only detaches. Never DELETE or create a session as a side effect.
    let _ = timeout(DEADLINE, upstream.close(None)).await;
    let _ = timeout(DEADLINE, browser.send(Message::Close(None))).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        routing::{get, post},
    };
    const SESSION: &str = "12345678-1234-4234-8234-123456789abc";
    const VM: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-hc-user-ticket", "fixture-user-ticket".parse().unwrap());
        headers.insert("cookie", "browser-secret".parse().unwrap());
        headers.insert("authorization", "browser-secret".parse().unwrap());
        headers.insert("x-hc-user-id", "forged".parse().unwrap());
        headers
    }
    fn check_headers(headers: &HeaderMap) {
        assert_eq!(headers["x-hc-user-ticket"], "fixture-user-ticket");
        for key in [
            "cookie",
            "authorization",
            "x-hc-user-id",
            "origin",
            "sec-fetch-site",
        ] {
            assert!(!headers.contains_key(key), "unexpected {key}");
        }
    }
    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, task)
    }
    #[test]
    fn restricts_providers_credentials_origins_and_controls() {
        assert!(origin("community.lazycat.app.catflap.dev").is_ok());
        assert!(origin("evil.example").is_err());
        assert!(delegated(&HeaderMap::new()).is_err());
        let upstream = delegated(&headers()).unwrap();
        check_headers(&upstream);
        assert!(upstream["x-hc-user-ticket"].is_sensitive());
        assert!(control(r#"{"type":"resize","cols":113,"rows":39}"#).is_some());
        for text in [
            r#"{"type":"resize","cols":0,"rows":24}"#,
            r#"{"type":"resize","cols":80,"rows":501}"#,
            r#"{"type":"input","data":"bad"}"#,
        ] {
            assert!(control(text).is_none());
        }
        let mut headers = headers();
        headers.insert("host", "neko.example".parse().unwrap());
        headers.insert("origin", "https://evil.example".parse().unwrap());
        assert!(local_request(&headers).is_err());
        headers.insert("origin", "https://neko.example".parse().unwrap());
        assert!(local_request(&headers).is_ok());
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(local_request(&headers).is_err());
        let request = upstream_request("http://localhost:1234", SESSION, &upstream).unwrap();
        assert_eq!(request.headers()["sec-websocket-protocol"], PROTOCOL);
        assert!(request.uri().query().is_none());
    }
    #[tokio::test]
    async fn create_preserves_client_identity_and_initial_dimensions_without_credentials() {
        let app =
            Router::new().route(
                "/api/smolvm/instances/{vm}/terminal/sessions",
                post(
                    |Path(vm): Path<String>,
                     headers: HeaderMap,
                     Json(body): Json<CreateSession>| async move {
                        assert_eq!(vm, VM);
                        check_headers(&headers);
                        assert_eq!(headers["x-catflap-request"], "1");
                        assert_eq!(body.session_id, SESSION);
                        assert_eq!((body.cols, body.rows), (91, 27));
                        Json(serde_json::json!({
                            "session_id": body.session_id,
                            "cols": body.cols,
                            "rows": body.rows,
                            "output_gap": true,
                        }))
                    },
                ),
            );
        let (base, task) = serve(app).await;
        let result = create_at(
            &base,
            VM,
            &headers(),
            &CreateSession {
                session_id: SESSION.into(),
                cols: 91,
                rows: 27,
            },
        )
        .await
        .unwrap();
        assert_eq!(result.session_id, SESSION);
        assert!(
            create_at(&base, "bad/id", &headers(), &result)
                .await
                .is_err()
        );
        task.abort();
    }
    #[tokio::test]
    async fn websocket_relays_raw_bytes_resize_and_ordered_real_exit() {
        let app = Router::new().route("/api/smolvm/terminal/sessions/{session}/ws", get(|Path(session): Path<String>, headers: HeaderMap, ws: WebSocketUpgrade| async move {
            assert_eq!(session,SESSION); check_headers(&headers); assert_eq!(headers["sec-websocket-protocol"],PROTOCOL);
            ws.protocols([PROTOCOL]).on_upgrade(|mut socket| async move {
                socket.send(Message::Text(serde_json::json!({"type":"ready","session_id":SESSION,"cols":91,"rows":27,"replay":false,"output_gap":true}).to_string().into())).await.unwrap();
                let Some(Ok(Message::Binary(bytes))) = socket.recv().await else { panic!("binary input expected") };
                assert_eq!(bytes.as_ref(), &[0,255,13,10]);
                let Some(Ok(Message::Text(text))) = socket.recv().await else { panic!("resize expected") };
                assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap(),serde_json::json!({"type":"resize","cols":113,"rows":39}));
                socket.send(Message::Binary(bytes)).await.unwrap();
                socket.send(Message::Text(r#"{"type":"process-exit","exit_code":7}"#.into())).await.unwrap();
                socket.send(Message::Close(None)).await.unwrap();
            })
        }));
        let (upstream, upstream_task) = serve(app).await;
        let relay_app = Router::new().route(
            "/ws",
            get(move |ws: WebSocketUpgrade| {
                let upstream = upstream.clone();
                async move {
                    ws.protocols([PROTOCOL]).on_upgrade(move |socket| {
                        relay(
                            socket,
                            upstream,
                            SESSION.into(),
                            delegated(&headers()).unwrap(),
                        )
                    })
                }
            }),
        );
        let (base, relay_task) = serve(relay_app).await;
        let mut request = format!("{}/ws", base.replace("http://", "ws://"))
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", PROTOCOL.parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let ready = timeout(DEADLINE, socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(ready.into_text().unwrap().contains("output_gap"));
        socket
            .send(UpstreamMessage::Binary(vec![0, 255, 13, 10].into()))
            .await
            .unwrap();
        socket
            .send(UpstreamMessage::Text(
                r#"{"type":"resize","cols":113,"rows":39}"#.into(),
            ))
            .await
            .unwrap();
        let output = timeout(DEADLINE, socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(output.into_data().as_ref(), &[0, 255, 13, 10]);
        let exit = timeout(DEADLINE, socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&exit.into_text().unwrap()).unwrap()["exit_code"],
            7
        );
        relay_task.abort();
        upstream_task.abort();
    }
}
