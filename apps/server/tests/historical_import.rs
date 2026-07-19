use std::net::SocketAddr;

use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{rngs::OsRng, RngCore};
use reqwest::header;
use rusqlite::{params, Connection};
use tempfile::TempDir;
use wealthfolio_server::{api::app_router, build_state, config::Config};
use wealthfolio_storage_sqlite::historical_import::{
    historical_manifest_hash, HistoricalImportBatch, HistoricalLedgerRow,
    HistoricalManifestLedgerRow, HistoricalOperation, HistoricalReviewedManifest, HistoricalSource,
    LedgerDisposition,
};

const PASSWORD: &str = "history-test-password";
const SUBJECT: &str = "urn:wealthfolio:password#local";
const AUTHORITY: &str = "dedicated-importer-authority";
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestServer {
    base: String,
    client: reqwest::Client,
    _tmp: TempDir,
}

async fn spawn_server() -> TestServer {
    let tmp = tempfile::tempdir().unwrap();
    let token_path = tmp.path().join("importer.token");
    std::fs::write(&token_path, format!("{AUTHORITY}\n")).unwrap();
    let router = {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("WF_DB_PATH", tmp.path().join("test.db"));
        std::env::set_var("WF_SECRET_FILE", tmp.path().join("secrets.json"));
        std::env::set_var("WF_HISTORICAL_IMPORTER_TOKEN_FILE", &token_path);
        let salt = SaltString::generate(&mut OsRng);
        std::env::set_var(
            "WF_AUTH_PASSWORD_HASH",
            Argon2::default()
                .hash_password(PASSWORD.as_bytes(), &salt)
                .unwrap()
                .to_string(),
        );
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        std::env::set_var("WF_SECRET_KEY", BASE64.encode(secret));
        std::env::set_var("WF_CORS_ALLOW_ORIGINS", "http://localhost:3000");
        std::env::set_var("WF_HOUSEHOLD_ENABLED", "true");
        let config = Config::from_env();
        let state = build_state(&config).await.unwrap();
        let connection = Connection::open(tmp.path().join("test.db")).unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        connection
            .execute("INSERT INTO policy_deployments(id) VALUES('default')", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO households(deployment_id,id) VALUES('default','h')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO person_subjects(deployment_id,id) VALUES('default',?)",
                [SUBJECT],
            )
            .unwrap();
        connection.execute("INSERT INTO household_membership_epochs(deployment_id,household_id,person_subject_id,role,start_epoch) VALUES('default','h',?,'owner',1)", [SUBJECT]).unwrap();
        for account in ["allowed", "denied"] {
            connection.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode) VALUES(?,?,'CASH','USD',0,1,0,'TRANSACTIONS')", params![account, account]).unwrap();
            connection.execute("INSERT INTO native_account_deployments(account_id,deployment_id) VALUES(?,'default')", [account]).unwrap();
        }
        connection.execute("INSERT INTO account_access_epochs(deployment_id,household_id,account_id,person_subject_id,start_generation) VALUES('default','h','allowed',?,1)", [SUBJECT]).unwrap();
        app_router(state, &config)
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    TestServer {
        base: format!("http://{address}"),
        client: reqwest::Client::new(),
        _tmp: tmp,
    }
}

async fn login(server: &TestServer) -> String {
    let response = server
        .client
        .post(format!("{}/api/v1/auth/login", server.base))
        .json(&serde_json::json!({"password": PASSWORD}))
        .send()
        .await
        .unwrap();
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("wf_session=")
        .to_string()
}

fn batch(subject: &str, account: &str) -> HistoricalImportBatch {
    let manifest = HistoricalReviewedManifest {
        ledger: vec![HistoricalManifestLedgerRow {
            identity: "history-1".into(),
            source: HistoricalSource::Tarn,
            account_id: account.into(),
            disposition: LedgerDisposition::Selected,
        }],
        evidence: vec![],
        overlap_resolutions: vec![],
    };
    HistoricalImportBatch {
        schema_version: 1,
        subject: subject.into(),
        import_run_id: "http-run".into(),
        idempotency_key: "http-batch".into(),
        dry_run: false,
        overlaps_resolved: true,
        manifest_hash: Some(historical_manifest_hash(&manifest)),
        manifest: Some(manifest),
        operation: HistoricalOperation::StageLedger {
            rows: vec![HistoricalLedgerRow {
                idempotency_key: "history-1".into(),
                historical_identity: "history-1".into(),
                source: HistoricalSource::Tarn,
                source_id: "tarn-1".into(),
                account_id: account.into(),
                activity_type: "WITHDRAWAL".into(),
                effective_at: "2025-01-01T00:00:00Z".into(),
                cutover_watermark: "2026-01-01T00:00:00Z".into(),
                amount: "-1".into(),
                currency: "USD".into(),
                raw_description: "Cafe".into(),
            }],
        },
    }
}

async fn send(
    server: &TestServer,
    jwt: &str,
    authority: Option<&str>,
    batch: &HistoricalImportBatch,
) -> reqwest::Response {
    serde_json::from_value::<HistoricalImportBatch>(serde_json::to_value(batch).unwrap())
        .expect("historical wire roundtrip");
    let mut request = server
        .client
        .post(format!("{}/api/v1/internal/history", server.base))
        .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
        .header("idempotency-key", &batch.idempotency_key)
        .json(batch);
    if let Some(authority) = authority {
        request = request.header(
            "x-wealthfolio-importer-authority",
            format!("Bearer {authority}"),
        );
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn endpoint_requires_dedicated_authority_and_exact_authenticated_subject() {
    let server = spawn_server().await;
    let jwt = login(&server).await;
    assert_eq!(
        send(&server, &jwt, None, &batch(SUBJECT, "allowed"))
            .await
            .status(),
        403
    );
    assert_eq!(
        send(&server, &jwt, Some("wrong"), &batch(SUBJECT, "allowed"))
            .await
            .status(),
        403
    );
    assert_eq!(
        send(
            &server,
            &jwt,
            Some(AUTHORITY),
            &batch("urn:other#subject", "allowed")
        )
        .await
        .status(),
        403
    );
}

#[tokio::test]
async fn endpoint_authorizes_accounts_and_enforces_receipt_payload_replay() {
    let server = spawn_server().await;
    let jwt = login(&server).await;
    assert_eq!(
        send(&server, &jwt, Some(AUTHORITY), &batch(SUBJECT, "denied"))
            .await
            .status(),
        403
    );
    let accepted = batch(SUBJECT, "allowed");
    let first = send(&server, &jwt, Some(AUTHORITY), &accepted).await;
    assert_eq!(first.status(), 200);
    let first_json = first.json::<serde_json::Value>().await.unwrap();
    let replay = send(&server, &jwt, Some(AUTHORITY), &accepted).await;
    assert_eq!(replay.status(), 200);
    assert_eq!(
        replay.json::<serde_json::Value>().await.unwrap(),
        first_json
    );

    let finalize = HistoricalImportBatch {
        schema_version: 1,
        subject: SUBJECT.into(),
        import_run_id: accepted.import_run_id.clone(),
        idempotency_key: "http-finalize".into(),
        dry_run: false,
        overlaps_resolved: true,
        manifest_hash: accepted.manifest_hash.clone(),
        manifest: accepted.manifest.clone(),
        operation: HistoricalOperation::Finalize,
    };
    let finalized = send(&server, &jwt, Some(AUTHORITY), &finalize).await;
    assert_eq!(finalized.status(), 200);
    let finalized_json = finalized.json::<serde_json::Value>().await.unwrap();
    let finalized_replay = send(&server, &jwt, Some(AUTHORITY), &finalize).await;
    assert_eq!(finalized_replay.status(), 200);
    assert_eq!(
        finalized_replay.json::<serde_json::Value>().await.unwrap(),
        finalized_json
    );

    let mut changed = accepted;
    let HistoricalOperation::StageLedger { rows } = &mut changed.operation else {
        unreachable!()
    };
    rows[0].amount = "-2".into();
    assert_eq!(
        send(&server, &jwt, Some(AUTHORITY), &changed)
            .await
            .status(),
        400
    );
}
