// These integration tests serialize process-global control-plane configuration across awaits.
#![allow(clippy::await_holding_lock)]

use std::{net::SocketAddr, time::Duration};

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;
use wealthfolio_server::{
    api::app_router,
    auth::{AuthConfig, CookieSecurePolicy},
    build_state,
    config::Config,
};

static TEST_DB_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const SUBJECT: &str = "urn:wealthfolio:password#bound-owner";
const CONTROL_TOKEN: &str = "dedicated-control-token";

fn config(root: &TempDir, authenticated: bool) -> Config {
    Config {
        listen_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        db_path: root.path().join("app.db").to_string_lossy().into_owned(),
        cors_allow: vec!["*".to_string()],
        request_timeout: Duration::from_secs(30),
        static_dir: "dist".to_string(),
        addons_root: root.path().join("addons").to_string_lossy().into_owned(),
        raw_secret_key: vec![7; 32],
        secrets_encryption_key: [7; 32],
        auth: authenticated.then(|| AuthConfig {
            password_hash: None,
            password_subject: SUBJECT.to_owned(),
            jwt_secret: vec![7; 32],
            access_token_ttl: Duration::from_secs(3600),
            cookie_secure: CookieSecurePolicy::Never,
        }),
        oidc: None,
        mcp_enabled: false,
        mcp_audit_enabled: true,
        mcp_allowed_hosts: None,
        household_enabled: false,
        deployment_id: "test".to_string(),
    }
}

fn valid_control_headers(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some("Bearer dedicated-control-token")
        && headers
            .get("x-authenticated-subject")
            .and_then(|v| v.to_str().ok())
            == Some(SUBJECT)
}

async fn start_control_server(root: &TempDir) -> tokio::task::JoinHandle<()> {
    async fn list(headers: HeaderMap) -> impl IntoResponse {
        if !valid_control_headers(&headers) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"unauthorized"})),
            );
        }
        (
            StatusCode::OK,
            Json(json!([
                {"provider":"investec","status":"configured","message":"Provider is configured."},
                {"provider":"mercury","status":"configured","message":"Provider is configured."},
                {"provider":"plaid","status":"configured","message":"Provider is configured."},
                {"provider":"snaptrade","status":"configured","message":"Provider is configured."}
            ])),
        )
    }
    async fn start(headers: HeaderMap) -> impl IntoResponse {
        if !valid_control_headers(&headers) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"unauthorized"})),
            );
        }
        (
            StatusCode::OK,
            Json(
                json!({"provider":"plaid","status":"link_ready","message":"Plaid Link is ready.","linkToken":"short-lived-link","continuationToken":"continuation-token-1234567890123456"}),
            ),
        )
    }
    async fn complete(headers: HeaderMap, Json(body): Json<Value>) -> impl IntoResponse {
        if !valid_control_headers(&headers)
            || body
                != json!({"publicToken":"public-token","continuationToken":"continuation-token-1234567890123456"})
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"request_failed"})),
            );
        }
        (
            StatusCode::OK,
            Json(
                json!({"provider":"plaid","status":"configured","message":"Plaid connection was stored."}),
            ),
        )
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let token_path = root.path().join("control-token");
    std::fs::write(&token_path, CONTROL_TOKEN).unwrap();
    std::env::set_var("FINANCE_SYNC_CONTROL_URL", format!("http://{address}"));
    std::env::set_var("FINANCE_SYNC_CONTROL_TOKEN_FILE", token_path);
    let router = Router::new()
        .route("/control/v1/provider-connections", get(list))
        .route("/control/v1/provider-connections/plaid/start", post(start))
        .route(
            "/control/v1/provider-connections/plaid/complete",
            post(complete),
        );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() })
}

async fn authenticated_app(root: &TempDir) -> (Router, String) {
    let config = config(root, true);
    let state = build_state(&config).await.unwrap();
    let token = state
        .auth
        .as_ref()
        .expect("auth configured")
        .issue_password_token()
        .unwrap();
    (app_router(state, &config), token)
}

#[tokio::test]
async fn routes_fail_closed_without_application_auth() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let cfg = config(&root, false);
    let state = build_state(&cfg).await.unwrap();
    let app = app_router(state, &cfg);
    for (method, uri) in [
        ("GET", "/api/v1/provider-connections"),
        ("POST", "/api/v1/provider-connections/plaid/start"),
        ("POST", "/api/v1/provider-connections/plaid/complete"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{\"publicToken\":\"x\",\"continuationToken\":\"continuation-token-1234567890123456\"}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn authenticated_routes_fail_closed_without_control_configuration() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    std::env::remove_var("FINANCE_SYNC_CONTROL_URL");
    std::env::remove_var("FINANCE_SYNC_CONTROL_TOKEN_FILE");
    let root = tempdir().unwrap();
    let (app, token) = authenticated_app(&root).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/provider-connections")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn list_proxies_only_typed_status_and_overrides_injected_subject() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let control = start_control_server(&root).await;
    let (app, token) = authenticated_app(&root).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/provider-connections")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("x-authenticated-subject", "attacker#subject")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let providers: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(providers.len(), 4);
    assert!(providers
        .iter()
        .all(|provider| provider.get("secret").is_none() && provider.get("token").is_none()));
    control.abort();
}

#[tokio::test]
async fn plaid_start_and_completion_proxy_only_short_lived_browser_values() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let control = start_control_server(&root).await;
    let (app, token) = authenticated_app(&root).await;

    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/provider-connections/plaid/start")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(start.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(payload["linkToken"], "short-lived-link");
    assert!(payload.get("accessToken").is_none());

    let complete = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/provider-connections/plaid/complete")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"publicToken\":\"public-token\",\"continuationToken\":\"continuation-token-1234567890123456\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(complete.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(payload["status"], "configured");
    assert!(payload.get("accessToken").is_none());
    control.abort();
}

#[tokio::test]
async fn unauthenticated_and_unknown_provider_requests_are_rejected_before_proxying() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let cfg = config(&root, true);
    let state = build_state(&cfg).await.unwrap();
    let app = app_router(state, &cfg);
    for uri in [
        "/api/v1/provider-connections",
        "/api/v1/provider-connections/unknown/start",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(if uri.ends_with("start") {
                        "POST"
                    } else {
                        "GET"
                    })
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
