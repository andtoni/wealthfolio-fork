use std::{net::SocketAddr, time::Duration};

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use tempfile::tempdir;
use tower::ServiceExt;
use wealthfolio_server::{api::app_router, build_state, config::Config};

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

async fn send(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn assigning_a_budget_period_is_stored_validated_and_leaves_the_date_unchanged() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("app.db");
    let addons_root = temp_dir.path().join("addons");
    let config = test_config(
        db_path.to_string_lossy().into_owned(),
        addons_root.to_string_lossy().into_owned(),
    );
    let state = build_state(&config).await.unwrap();
    let app = app_router(state, &config);

    let (status, account) = send(
        &app,
        "POST",
        "/api/v1/accounts",
        json!({
            "name": "Card",
            "accountType": "CASH",
            "currency": "USD",
            "isDefault": false,
            "isActive": true,
            "isArchived": false,
            "trackingMode": "TRANSACTIONS",
        }),
    )
    .await;
    assert!(status.is_success(), "account: {status} {account}");
    let account_id = account["id"].as_str().unwrap().to_string();
    let mut ids = Vec::new();
    for (date, amount) in [
        ("2026-06-23T12:00:00+00:00", "100"),
        ("2026-07-10T12:00:00+00:00", "30"),
    ] {
        let (status, activity) = send(
            &app,
            "POST",
            "/api/v1/activities",
            json!({
                "accountId": account_id,
                "activityType": "WITHDRAWAL",
                "activityDate": date,
                "amount": amount,
                "currency": "USD",
            }),
        )
        .await;
        assert!(status.is_success(), "activity: {status} {activity}");
        ids.push(activity["id"].as_str().unwrap().to_string());
    }
    let early_groceries = ids[0].clone();
    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/spending/settings",
        json!({ "enabled": true, "accountIds": [account_id] }),
    )
    .await;
    assert!(status.is_success());

    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/spending/cash-activities/budget-period",
        json!({ "activityIds": [early_groceries], "periodStart": "2026-06-25" }),
    )
    .await;
    assert!(status.is_success());

    let (status, search) = send(
        &app,
        "POST",
        "/api/v1/spending/cash-activities/search",
        json!({}),
    )
    .await;
    assert!(status.is_success());
    let moved = search["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == early_groceries.as_str())
        .unwrap();
    assert_eq!(moved["budgetPeriodStart"], "2026-06-25");
    assert!(moved["activityDate"]
        .as_str()
        .unwrap()
        .starts_with("2026-06-23"));

    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/spending/cash-activities/budget-period",
        json!({ "activityIds": [early_groceries], "periodStart": "2026-06-26" }),
    )
    .await;
    assert!(status.is_client_error());

    let (status, _) = send(
        &app,
        "PUT",
        "/api/v1/spending/cash-activities/budget-period",
        json!({ "activityIds": [early_groceries], "periodStart": null }),
    )
    .await;
    assert!(status.is_success());
    let (_, search) = send(
        &app,
        "POST",
        "/api/v1/spending/cash-activities/search",
        json!({}),
    )
    .await;
    assert!(search["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item.get("budgetPeriodStart").is_none()));
}
