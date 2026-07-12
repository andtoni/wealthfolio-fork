use std::{net::SocketAddr, time::Duration};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;
use wealthfolio_server::{
    api::app_router,
    auth::{AuthConfig, CookieSecurePolicy},
    build_state,
    config::Config,
};

static TEST_DB_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn test_config(db_path: String, addons_root: String) -> Config {
    Config {
        listen_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        db_path,
        cors_allow: vec!["*".to_string()],
        request_timeout: Duration::from_secs(30),
        static_dir: "dist".to_string(),
        addons_root,
        raw_secret_key: vec![7; 32],
        secrets_encryption_key: [7; 32],
        auth: None,
        oidc: None,
        mcp_enabled: false,
        mcp_audit_enabled: true,
        mcp_allowed_hosts: None,
        household_enabled: false,
        deployment_id: "test".to_string(),
    }
}

fn test_config_with_auth(db_path: String, addons_root: String) -> Config {
    Config {
        listen_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        db_path,
        cors_allow: vec!["*".to_string()],
        request_timeout: Duration::from_secs(30),
        static_dir: "dist".to_string(),
        addons_root,
        raw_secret_key: vec![7; 32],
        secrets_encryption_key: [7; 32],
        auth: Some(AuthConfig {
            password_hash: None,
            password_subject: "urn:wealthfolio:password#bound-owner".to_owned(),
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

// --- Status list tests -------------------------------------------------------

#[tokio::test]
async fn list_returns_auth_required_when_auth_not_configured() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder().uri("/api/v1/provider-connections").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let providers: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(providers.len(), 4);
    for p in &providers {
        assert_eq!(p["status"], "auth_required");
    }
    let names: Vec<&str> = providers.iter().map(|p| p["provider"].as_str().unwrap()).collect();
    for name in &["investec", "mercury", "plaid", "snaptrade"] {
        assert!(names.contains(name));
    }
}

#[tokio::test]
async fn list_returns_provider_statuses_with_no_secrets() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let auth_mgr = state.auth.as_ref().expect("auth should be configured");
    let token = auth_mgr.issue_password_token().unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder().uri("/api/v1/provider-connections")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let providers: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(providers.len(), 4);
    let names: Vec<&str> = providers.iter().map(|p| p["provider"].as_str().unwrap()).collect();
    for name in &["investec", "mercury", "plaid", "snaptrade"] {
        assert!(names.contains(name));
    }
    for p in &providers {
        assert_eq!(p["status"], "not_configured");
        assert!(p.get("secret").is_none());
    }
}

#[tokio::test]
async fn unauthenticated_list_denied_when_auth_enabled() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder().uri("/api/v1/provider-connections").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authenticated_list_succeeds_with_no_secrets() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state.clone(), &config);
    let auth_mgr = state.auth.as_ref().expect("auth should be configured");
    let token = auth_mgr.issue_password_token().unwrap();

    let response = app.oneshot(
        Request::builder().uri("/api/v1/provider-connections")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let providers: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(providers.len(), 4);
    for p in &providers {
        assert!(p.get("secret").is_none());
    }
}

#[tokio::test]
async fn list_subject_injection_has_no_effect() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state.clone(), &config);
    let auth_mgr = state.auth.as_ref().expect("auth should be configured");
    let token = auth_mgr.issue_password_token().unwrap();

    let response = app.oneshot(
        Request::builder().uri("/api/v1/provider-connections")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("x-authenticated-subject", "https://id.example.test#jordan")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let providers: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(providers.len(), 4);
    for p in &providers {
        assert!(p.get("secret").is_none());
    }
}

// --- Start endpoint tests ----------------------------------------------------

#[tokio::test]
async fn start_fails_closed_when_auth_not_configured() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    for provider in &["investec", "mercury", "plaid", "snaptrade"] {
        let response = app.clone().oneshot(
            Request::builder().method("POST")
                .uri(&format!("/api/v1/provider-connections/{provider}/start"))
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        // Fail-closed: auth required means no start possible
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}

#[tokio::test]
async fn start_returns_truthful_response_with_subject_when_authenticated() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let auth_mgr = state.auth.as_ref().expect("auth should be configured");
    let token = auth_mgr.issue_password_token().unwrap();
    let app = app_router(state, &config);

    for provider in &["investec", "mercury", "plaid", "snaptrade"] {
        let response = app.clone().oneshot(
            Request::builder().method("POST")
                .uri(&format!("/api/v1/provider-connections/{provider}/start"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        // No longer 501 -- truthful 200 with subject-bound info
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let start_resp: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(start_resp["provider"], *provider);
        assert_eq!(start_resp["status"], "not_implemented");
        assert_eq!(
            start_resp["authenticatedSubject"],
            "urn:wealthfolio:password#bound-owner"
        );
        // No secret fields
        for forbidden in &["secret", "apiKey", "token", "accessToken", "url", "redirectUrl"] {
            assert!(start_resp.get(*forbidden).is_none(),
                "start response must not contain field: {forbidden}");
        }
    }
}

#[tokio::test]
async fn start_unknown_provider_rejected_as_unauthenticated() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder().method("POST")
            .uri("/api/v1/provider-connections/unknown/start")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unauthenticated_start_denied_when_auth_enabled() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder().method("POST")
            .uri("/api/v1/provider-connections/plaid/start")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// --- Review endpoint tests ---------------------------------------------------

#[tokio::test]
async fn review_fails_closed_when_auth_not_configured() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    for provider in &["investec", "mercury", "plaid", "snaptrade"] {
        let response = app.clone().oneshot(
            Request::builder()
                .uri(&format!("/api/v1/provider-connections/{provider}"))
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}

#[tokio::test]
async fn review_returns_truthful_status_with_subject_when_authenticated() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let auth_mgr = state.auth.as_ref().expect("auth should be configured");
    let token = auth_mgr.issue_password_token().unwrap();
    let app = app_router(state, &config);

    for provider in &["investec", "mercury", "plaid", "snaptrade"] {
        let response = app.clone().oneshot(
            Request::builder()
                .uri(&format!("/api/v1/provider-connections/{provider}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let review: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(review["provider"], *provider);
        assert_eq!(review["status"], "not_configured");
        assert_eq!(
            review["authenticatedSubject"],
            "urn:wealthfolio:password#bound-owner"
        );
        // No secret fields
        for forbidden in &["secret", "apiKey", "token", "accessToken", "url", "redirectUrl"] {
            assert!(review.get(*forbidden).is_none(),
                "review response must not contain field: {forbidden}");
        }
    }
}

#[tokio::test]
async fn review_unknown_provider_rejected_as_unauthenticated() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder()
            .uri("/api/v1/provider-connections/unknown")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unauthenticated_review_denied_when_auth_enabled() {
    let _db_guard = TEST_DB_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db").to_string_lossy().into_owned();
    let addons_root = temp_dir.path().join("addons").to_string_lossy().into_owned();
    let config = test_config_with_auth(db_path, addons_root);
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let response = app.oneshot(
        Request::builder()
            .uri("/api/v1/provider-connections/plaid")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
