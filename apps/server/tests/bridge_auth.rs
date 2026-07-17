//! Integration tests for bridge subject authorization.
//!
//! An authenticated user must NOT be able to specify an arbitrary subject
//! in a bridge path parameter or request body. These tests assert 403
//! rejection for cross-subject operations.

use std::net::SocketAddr;

use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{rngs::OsRng, RngCore};
use reqwest::header;
use rusqlite::Connection;
use tempfile::TempDir;
use wealthfolio_server::{api::app_router, build_state, config::Config};

const PASSWORD: &str = "super-secret";

/// Env vars are process-global; serialize config/state construction.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestServer {
    base: String,
    client: reqwest::Client,
    /// Keeps the database directory alive for the server's lifetime.
    _tmp: TempDir,
}

async fn spawn_server() -> TestServer {
    let tmp = tempfile::tempdir().unwrap();
    let router = {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("WF_DB_PATH", tmp.path().join("test.db"));
        std::env::set_var("WF_SECRET_FILE", tmp.path().join("secrets.json"));
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(PASSWORD.as_bytes(), &salt)
            .unwrap()
            .to_string();
        std::env::set_var("WF_AUTH_PASSWORD_HASH", password_hash);
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        std::env::set_var("WF_SECRET_KEY", BASE64.encode(secret_bytes));
        std::env::set_var("WF_CORS_ALLOW_ORIGINS", "http://localhost:3000");
        std::env::set_var("WF_HOUSEHOLD_ENABLED", "true");

        let config = Config::from_env();
        let state = build_state(&config).await.unwrap();
        let connection = Connection::open(tmp.path().join("test.db")).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        connection
            .execute("INSERT INTO policy_deployments(id) VALUES(?)", ["default"])
            .unwrap();
        connection
            .execute(
                "INSERT INTO households(deployment_id,id) VALUES(?,?)",
                rusqlite::params!["default", "test-household"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO person_subjects(deployment_id,id) VALUES(?,?)",
                rusqlite::params!["default", "urn:wealthfolio:password#local"],
            )
            .unwrap();
        connection.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode) VALUES(?,?, 'CASH','USD',0,1,0,'TRANSACTIONS')", rusqlite::params!["bridge-account", "bridge-account"]).unwrap();
        connection
            .execute(
                "INSERT INTO native_account_deployments(account_id,deployment_id) VALUES(?,?)",
                rusqlite::params!["bridge-account", "default"],
            )
            .unwrap();
        app_router(state, &config)
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    TestServer {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        _tmp: tmp,
    }
}

/// Logs in and returns the `wf_session` cookie value (the JWT).
async fn login(server: &TestServer) -> String {
    let response = server
        .client
        .post(format!("{}/api/v1/auth/login", server.base))
        .json(&serde_json::json!({ "password": PASSWORD }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("login should set a cookie")
        .to_str()
        .unwrap();
    set_cookie
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("wf_session=")
        .to_string()
}

// ── RED phase: these assertions will fail until the subject-binding
//    check is implemented in the bridge handlers ──

#[tokio::test]
async fn household_mode_without_auth_fails_closed_even_when_auth_required_is_false() {
    let _guard = ENV_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("WF_DB_PATH", tmp.path().join("test.db"));
    std::env::set_var("WF_SECRET_KEY", BASE64.encode([7u8; 32]));
    std::env::set_var("WF_LISTEN_ADDR", "127.0.0.1:8088");
    std::env::set_var("WF_HOUSEHOLD_ENABLED", "true");
    std::env::set_var("WF_AUTH_REQUIRED", "false");
    std::env::remove_var("WF_AUTH_PASSWORD_HASH");
    std::env::remove_var("WF_OIDC_ISSUER_URL");
    std::env::remove_var("WF_OIDC_CLIENT_ID");

    let result = std::panic::catch_unwind(Config::from_env);
    assert!(
        result.is_err(),
        "household mode must refuse startup without an authenticator"
    );
}

#[tokio::test]
async fn cross_subject_grant_is_forbidden() {
    let server = spawn_server().await;
    let cookie = login(&server).await;

    // The authenticated subject is "urn:wealthfolio:password#local".
    // Trying to grant for a different subject must be rejected.
    // Route: POST /bridge/grant/{subject}/{account}
    let intruder = urlencoding::encode("urn:evil#intruder");
    let response = server
        .client
        .post(format!(
            "{}/api/v1/bridge/grant/{}/victim-account",
            server.base, intruder
        ))
        .header(header::COOKIE, format!("wf_session={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        403,
        "cross-subject grant should be forbidden"
    );
}

#[tokio::test]
async fn bearer_bridge_commit_returns_accepted_receipt() {
    let server = spawn_server().await;
    let jwt = login(&server).await;
    let subject = "urn:wealthfolio:password#local";
    let grant = server
        .client
        .post(format!(
            "{}/api/v1/bridge/grant/{}/bridge-account",
            server.base,
            urlencoding::encode(subject)
        ))
        .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
        .send()
        .await
        .unwrap();
    assert_eq!(grant.status(), 200);
    let access_generation = grant.json::<serde_json::Value>().await.unwrap()["accessGeneration"]
        .as_i64()
        .unwrap();
    let idempotency_key = "bridge-positive-key";
    let commit = server.client.post(format!("{}/api/v1/bridge/commit", server.base)).header(header::AUTHORIZATION, format!("Bearer {jwt}"))
        .json(&serde_json::json!({"idempotency_key": idempotency_key, "subject": subject, "access_generation": access_generation, "kind": "ActivityUpsert", "resource_id": "bridge-activity", "activity": {"account_id": "bridge-account", "activity_type": "DEPOSIT", "status": "POSTED", "activity_date": "2026-01-01", "amount": "1", "currency": "USD", "notes": null, "activity_type_override": null, "needs_review": false}})).send().await.unwrap();
    assert_eq!(commit.status(), 200);
    assert_eq!(
        commit.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"subject": subject, "idempotencyKey": idempotency_key, "accessGeneration": access_generation, "operation": "activity_upsert", "resourceId": "bridge-activity", "outcome": "Accepted"})
    );
}

#[tokio::test]
async fn cross_subject_commit_is_forbidden() {
    let server = spawn_server().await;
    let cookie = login(&server).await;

    // Route: POST /bridge/commit — subject lives in the JSON body.
    // Note: DisclosureOperation deserializes from snake_case (no rename_all).
    let body = serde_json::json!({
        "idempotency_key": "test-key",
        "subject": "urn:evil#intruder",
        "access_generation": 1,
        "kind": "ActivityUpsert",
        "resource_id": "x1"
    });

    let response = server
        .client
        .post(format!("{}/api/v1/bridge/commit", server.base))
        .header(header::COOKIE, format!("wf_session={cookie}"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        403,
        "cross-subject commit should be forbidden"
    );
}

#[tokio::test]
async fn cross_subject_revoke_is_forbidden() {
    let server = spawn_server().await;
    let cookie = login(&server).await;

    // Route: POST /bridge/revoke/{subject}/{account}/{key}
    let intruder = urlencoding::encode("urn:evil#intruder");
    let response = server
        .client
        .post(format!(
            "{}/api/v1/bridge/revoke/{}/victim-account/some-key",
            server.base, intruder
        ))
        .header(header::COOKIE, format!("wf_session={cookie}"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        403,
        "cross-subject revoke should be forbidden"
    );
}
