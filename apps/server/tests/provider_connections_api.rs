// These integration tests serialize process-global control-plane configuration across awaits.
#![allow(clippy::await_holding_lock)]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

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
            bridge_service_token: None,
            bridge_service_subject: None,
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
        if valid_control_headers(&headers) && body["publicToken"] == "conflicting-public-token" {
            return (StatusCode::CONFLICT, Json(json!({"error":"conflict"})));
        }
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
    async fn update(headers: HeaderMap) -> impl IntoResponse {
        if !valid_control_headers(&headers) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"unauthorized"})),
            );
        }
        (
            StatusCode::OK,
            Json(
                json!({"provider":"plaid","status":"link_ready","message":"Plaid Link is ready to add accounts.","linkToken":"update-link","continuationToken":"update-continuation-token-1234567890123456"}),
            ),
        )
    }
    async fn update_complete(headers: HeaderMap, Json(body): Json<Value>) -> impl IntoResponse {
        if !valid_control_headers(&headers)
            || body != json!({"continuationToken":"update-continuation-token-1234567890123456"})
        {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"request_failed"})),
            );
        }
        (
            StatusCode::OK,
            Json(
                json!({"provider":"plaid","status":"pending","message":"Choose the Plaid accounts to add."}),
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
        )
        .route(
            "/control/v1/provider-connections/plaid/update",
            post(update),
        )
        .route(
            "/control/v1/provider-connections/plaid/update/complete",
            post(update_complete),
        );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() })
}

async fn start_snaptrade_control_server(
    root: &TempDir,
    admissions: Arc<Mutex<Vec<Value>>>,
    simulate_lost_response: bool,
    fail_first_admission: bool,
    provider_generation: i64,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let token_path = root.path().join("snaptrade-control-token");
    std::fs::write(&token_path, CONTROL_TOKEN).unwrap();
    std::env::set_var("FINANCE_SYNC_CONTROL_URL", format!("http://{address}"));
    std::env::set_var("FINANCE_SYNC_CONTROL_TOKEN_FILE", token_path);
    let committed = Arc::new(Mutex::new(None::<Value>));
    let accounts = {
        let committed = Arc::clone(&committed);
        move |headers: HeaderMap| {
            let committed = Arc::clone(&committed);
            async move {
                if !valid_control_headers(&headers) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error":"unauthorized"})),
                    );
                }
                let rows = if committed.lock().unwrap().is_none() {
                    json!([{
                        "connectionId": "dummy-connection",
                        "providerAccountId": "dummy-cash-account",
                        "name": "Dummy cash account",
                        "type": "brokerage",
                        "currency": "USD",
                        "mask": "4242",
                        "providerGeneration": provider_generation,
                        "assets": []
                    }])
                } else {
                    json!([])
                };
                (StatusCode::OK, Json(rows))
            }
        }
    };
    let admit = {
        let admissions = Arc::clone(&admissions);
        let committed = Arc::clone(&committed);
        move |headers: HeaderMap, Json(body): Json<Value>| {
            let admissions = Arc::clone(&admissions);
            let committed = Arc::clone(&committed);
            async move {
                if !valid_control_headers(&headers) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error":"unauthorized"})),
                    );
                }
                let attempt = {
                    let mut captured = admissions.lock().unwrap();
                    captured.push(body.clone());
                    captured.len()
                };
                if fail_first_admission && attempt == 1 {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error":"temporarily_unavailable"})),
                    );
                }
                *committed.lock().unwrap() = Some(body.clone());
                let mut response = json!({
                    "wealthfolioAccountId": body["wealthfolioAccountId"],
                    "providerGeneration": body["providerGeneration"],
                    "receiverAccessGeneration": body["receiverAccessGeneration"],
                    "baselineStatus": "verified",
                    "baselineDigest": body["databaseStateDigest"],
                    "assetMappingDigest": "sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
                });
                if simulate_lost_response && attempt == 1 {
                    // Simulate a response lost after Finance Sync committed it.
                    // The strict wire rejects this extra field, so retry must
                    // recover through the canonical admission-status endpoint.
                    response["unexpected"] = json!(true);
                }
                (StatusCode::OK, Json(response))
            }
        }
    };
    let status = {
        let committed = Arc::clone(&committed);
        move |headers: HeaderMap| {
            let committed = Arc::clone(&committed);
            async move {
                if !valid_control_headers(&headers) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error":"unauthorized"})),
                    );
                }
                let captured = committed.lock().unwrap();
                let Some(admission) = captured.as_ref() else {
                    return (StatusCode::NOT_FOUND, Json(json!({"error":"not_found"})));
                };
                (
                    StatusCode::OK,
                    Json(json!({
                        "wealthfolioAccountId": admission["wealthfolioAccountId"],
                        "providerGeneration": admission["providerGeneration"],
                        "receiverAccessGeneration": admission["receiverAccessGeneration"],
                        "baselineStatus": "verified",
                        "baselineDigest": admission["databaseStateDigest"],
                        "assetMappingDigest": "sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
                    })),
                )
            }
        }
    };
    let router = Router::new()
        .route(
            "/control/v1/provider-connections/snaptrade/accounts",
            get(accounts),
        )
        .route(
            "/control/v1/provider-connections/snaptrade/admit",
            post(admit),
        )
        .route(
            "/control/v1/provider-connections/snaptrade/admissions/{connection}/{account}",
            get(status),
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
        ("POST", "/api/v1/provider-connections/plaid/update"),
        ("POST", "/api/v1/provider-connections/plaid/complete"),
        ("POST", "/api/v1/provider-connections/plaid/update/complete"),
    ] {
        let body = if uri.ends_with("update/complete") {
            "{\"continuationToken\":\"continuation-token-1234567890123456\"}"
        } else {
            "{\"publicToken\":\"x\",\"continuationToken\":\"continuation-token-1234567890123456\"}"
        };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
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

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/provider-connections/plaid/update")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(update.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(payload["linkToken"], "update-link");
    assert!(payload.get("accessToken").is_none());

    let update_complete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/provider-connections/plaid/update/complete")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    "{\"continuationToken\":\"update-continuation-token-1234567890123456\"}",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update_complete.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(update_complete.into_body(), 64 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload["status"], "pending");

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
async fn control_conflict_is_reported_as_conflict_not_as_retryable() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let control = start_control_server(&root).await;
    let (app, token) = authenticated_app(&root).await;

    let complete = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/provider-connections/plaid/complete")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"publicToken\":\"conflicting-public-token\",\"continuationToken\":\"continuation-token-1234567890123456\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::CONFLICT);
    control.abort();
}

#[tokio::test]
async fn snaptrade_cash_only_admission_is_subject_bound_exact_and_retry_safe() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let admissions = Arc::new(Mutex::new(Vec::new()));
    let control =
        start_snaptrade_control_server(&root, Arc::clone(&admissions), true, false, 7).await;
    let (app, token) = authenticated_app(&root).await;
    let request = |name: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/provider-connections/snaptrade/admit")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "connectionId": "dummy-connection",
                    "providerAccountId": "dummy-cash-account",
                    "name": name
                })
                .to_string(),
            ))
            .unwrap()
    };

    let first = app
        .clone()
        .oneshot(request("Chosen account name"))
        .await
        .unwrap();
    let first_status = first.status();
    let first_body = to_bytes(first.into_body(), 64 * 1024).await.unwrap();
    assert_eq!(
        first_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected first response: {}",
        String::from_utf8_lossy(&first_body)
    );

    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let (account_id, active, name, meta, access_count): (String, bool, String, Option<String>, i64) = db
        .query_row(
            "SELECT id, is_active, name, meta, (SELECT count(*) FROM account_access_epochs access WHERE access.account_id=accounts.id AND access.person_subject_id=?1 AND access.end_generation IS NULL) FROM accounts WHERE provider='snaptrade' AND provider_account_id='dummy-cash-account'",
            [SUBJECT],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert!(
        !active,
        "an inexact acknowledgement must leave the account pending"
    );
    assert_eq!(name, "Chosen account name");
    assert_eq!(
        access_count, 1,
        "pending admission must retain subject-bound account access"
    );
    let meta: Value = serde_json::from_str(meta.as_deref().expect("normal accounting metadata"))
        .expect("account metadata JSON");
    assert_eq!(
        meta.as_object().map(|object| object.len()),
        Some(1),
        "provider admission must not add provider-owned overlay fields"
    );
    assert!(meta.get("accounting").is_some());
    db.execute(
        "UPDATE accounts SET name='Locally renamed' WHERE id=?1",
        [&account_id],
    )
    .unwrap();
    drop(db);

    let second = app
        .clone()
        .oneshot(request("Remote overwrite attempt"))
        .await
        .unwrap();
    let second_status = second.status();
    let second_body = to_bytes(second.into_body(), 64 * 1024).await.unwrap();
    assert_eq!(
        second_status,
        StatusCode::OK,
        "unexpected retry response: {}",
        String::from_utf8_lossy(&second_body)
    );
    let payload: Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(payload["accountId"], account_id);

    let captured = admissions.lock().unwrap();
    assert_eq!(
        captured.len(),
        1,
        "lost-response retry must reconcile the committed admission without a duplicate POST"
    );
    assert_eq!(captured[0]["authenticatedSubject"], SUBJECT);
    assert_eq!(captured[0]["assetMappings"], json!([]));
    assert_eq!(captured[0]["wealthfolioAccountId"], account_id);
    assert!(captured[0]["databaseStateDigest"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    drop(captured);

    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let (active, name, account_count): (bool, String, i64) = db
        .query_row(
            "SELECT is_active, name, (SELECT count(*) FROM accounts WHERE provider='snaptrade' AND provider_account_id='dummy-cash-account') FROM accounts WHERE id=?1",
            [&account_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(active);
    assert_eq!(
        name, "Locally renamed",
        "retry must not overwrite the local name"
    );
    assert_eq!(account_count, 1);
    db.execute(
        "INSERT INTO holdings_snapshots(id,account_id,snapshot_date,currency,source) VALUES('relink-existing-snapshot',?1,'2026-07-18','USD','BROKER_IMPORTED')",
        [&account_id],
    )
    .unwrap();
    drop(db);
    control.abort();

    // A provider relink advances providerGeneration without changing immutable
    // provider identity or replacing any Wealthfolio-owned overlays.
    let relink_admissions = Arc::new(Mutex::new(Vec::new()));
    let relink_control =
        start_snaptrade_control_server(&root, Arc::clone(&relink_admissions), true, false, 8).await;
    let lost_response = app
        .clone()
        .oneshot(request("Relink overwrite attempt"))
        .await
        .unwrap();
    assert_eq!(lost_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let generation: i64 = db
        .query_row(
            "SELECT provider_generation FROM provider_account_connections WHERE account_id=?1",
            [&account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        generation, 7,
        "a lost acknowledgement must not advance the local generation"
    );
    drop(db);

    let relink = app
        .clone()
        .oneshot(request("Relink retry overwrite attempt"))
        .await
        .unwrap();
    let relink_status = relink.status();
    let relink_body = to_bytes(relink.into_body(), 64 * 1024).await.unwrap();
    assert_eq!(
        relink_status,
        StatusCode::OK,
        "unexpected relink retry response: {}",
        String::from_utf8_lossy(&relink_body)
    );
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let (generation, name, snapshot_count): (i64, String, i64) = db
        .query_row(
            "SELECT provider_generation, accounts.name, (SELECT count(*) FROM holdings_snapshots WHERE account_id=accounts.id) FROM provider_account_connections JOIN accounts ON accounts.id=provider_account_connections.account_id WHERE provider_account_connections.account_id=?1",
            [&account_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(generation, 8);
    assert_eq!(name, "Locally renamed");
    assert_eq!(
        snapshot_count, 1,
        "relink must preserve existing provider history"
    );
    let relink_captured = relink_admissions.lock().unwrap();
    assert_eq!(
        relink_captured.len(),
        1,
        "lost-response retry must reconcile status without duplicate admission"
    );
    assert_eq!(relink_captured[0]["baselineKind"], "relink");
    assert_eq!(relink_captured[0]["priorProviderGeneration"], 7);
    drop(relink_captured);
    relink_control.abort();

    let failed_admissions = Arc::new(Mutex::new(Vec::new()));
    let failed_control =
        start_snaptrade_control_server(&root, Arc::clone(&failed_admissions), false, true, 9).await;
    let failed = app
        .clone()
        .oneshot(request("Failed relink overwrite attempt"))
        .await
        .unwrap();
    assert_eq!(failed.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let generation: i64 = db
        .query_row(
            "SELECT provider_generation FROM provider_account_connections WHERE account_id=?1",
            [&account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        generation, 8,
        "remote rejection must not advance the local generation"
    );
    drop(db);

    let retried = app
        .clone()
        .oneshot(request("Failed relink retry overwrite attempt"))
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::OK);
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let (generation, name, snapshot_count): (i64, String, i64) = db
        .query_row(
            "SELECT provider_generation, accounts.name, (SELECT count(*) FROM holdings_snapshots WHERE account_id=accounts.id) FROM provider_account_connections JOIN accounts ON accounts.id=provider_account_connections.account_id WHERE provider_account_connections.account_id=?1",
            [&account_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(generation, 9);
    assert_eq!(name, "Locally renamed");
    assert_eq!(snapshot_count, 1);
    let failed_captured = failed_admissions.lock().unwrap();
    assert_eq!(failed_captured.len(), 2);
    assert_eq!(failed_captured[0], failed_captured[1]);
    assert_eq!(failed_captured[1]["priorProviderGeneration"], 8);
    drop(failed_captured);
    failed_control.abort();

    let stale_admissions = Arc::new(Mutex::new(Vec::new()));
    let stale_control =
        start_snaptrade_control_server(&root, stale_admissions, false, false, 8).await;
    let stale = app
        .oneshot(request("Stale relink overwrite attempt"))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let generation: i64 = db
        .query_row(
            "SELECT provider_generation FROM provider_account_connections WHERE account_id=?1",
            [&account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(generation, 9);
    stale_control.abort();
}

#[tokio::test]
async fn conflicting_legacy_owner_cannot_poison_provider_connection_mapping() {
    let _guard = TEST_DB_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempdir().unwrap();
    let admissions = Arc::new(Mutex::new(Vec::new()));
    let control = start_snaptrade_control_server(&root, admissions, false, false, 7).await;
    let (app, token) = authenticated_app(&root).await;
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    db.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode,provider,provider_account_id,include_in_net_worth,name_source) VALUES('legacy-account','Legacy','BROKERAGE','USD',0,0,0,'HOLDINGS','snaptrade','dummy-cash-account',0,'user')", []).unwrap();
    db.execute(
        "INSERT OR IGNORE INTO policy_deployments(id,access_generation) VALUES('test',1)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO person_subjects(deployment_id,id) VALUES('test','issuer#other')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO households(deployment_id,id) VALUES('test','private:issuer#other')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO native_account_deployments(account_id,deployment_id) VALUES('legacy-account','test')", []).unwrap();
    db.execute("INSERT INTO account_ownership_arrangements(deployment_id,id,household_id,account_id,kind,start_epoch) VALUES('test','legacy-arrangement','private:issuer#other','legacy-account','provider',1)", []).unwrap();
    db.execute("INSERT INTO account_ownership_participants(deployment_id,arrangement_id,person_subject_id) VALUES('test','legacy-arrangement','issuer#other')", []).unwrap();
    db.execute("INSERT INTO account_access_epochs(deployment_id,household_id,account_id,person_subject_id,start_generation) VALUES('test','private:issuer#other','legacy-account','issuer#other',1)", []).unwrap();
    drop(db);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/provider-connections/snaptrade/admit")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "connectionId": "dummy-connection",
                        "providerAccountId": "dummy-cash-account",
                        "name": "Attack"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let db = rusqlite::Connection::open(root.path().join("app.db")).unwrap();
    let mappings: i64 = db
        .query_row(
            "SELECT count(*) FROM provider_account_connections",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        mappings, 0,
        "ownership rejection must roll back the immutable sidecar insert"
    );
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
