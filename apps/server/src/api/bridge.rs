//! Bridge policy Axum handlers. Routes are mounted in `api::app_router` and
//! extract state from AppState. The `router()` function is kept for backward
//! compatibility but routes are now inlined in api.rs.
use axum::{extract::State, Extension, Json};
use std::sync::Arc;

use crate::auth::AuthenticatedSubject;
use crate::error::ApiError;
use crate::main_lib::AppState;

fn get_bridge(
    state: &AppState,
) -> Result<&wealthfolio_storage_sqlite::bridge_policy::BridgePolicyStore, ApiError> {
    state
        .bridge_store
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("household feature not enabled".into()))
}

/// Reject the request when the authenticated subject does not match the
/// subject the caller supplied in a path parameter or request body.
///
/// Bridge operations always require an authenticated subject. Household
/// mode must never treat an absent identity as authorization, even when the
/// global UI auth requirement is explicitly disabled.
fn require_own_subject(
    auth: Option<Extension<AuthenticatedSubject>>,
    request_subject: &str,
) -> Result<(), ApiError> {
    let Some(Extension(subject)) = auth else {
        return Err(ApiError::Unauthorized(
            "bridge authentication is required".into(),
        ));
    };
    if subject.as_str() != request_subject {
        return Err(ApiError::Forbidden(
            "authenticated subject cannot act on behalf of another subject".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_requires_an_authenticated_subject() {
        let err = require_own_subject(None, "subject").expect_err("missing auth must fail closed");
        assert!(matches!(err, ApiError::Unauthorized(_)));
    }

    #[test]
    fn receipt_response_serializes_complete_binding_identity() {
        use wealthfolio_storage_sqlite::bridge_policy::{DisclosureReceipt, ReceiptOutcome};
        let value = serde_json::to_value(receipt(DisclosureReceipt {
            deployment_id: "deployment-1".into(),
            account_id: "account-1".into(),
            subject: "issuer#subject".into(),
            idempotency_key: "key-1".into(),
            access_generation: 2,
            operation: "activity_upsert".into(),
            resource_id: "activity-1".into(),
            lifecycle_successor_id: Some("activity-2".into()),
            outcome: ReceiptOutcome::Accepted,
            binding_version: 1,
            receipt_binding: "v1;binding".into(),
        }))
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "deploymentId": "deployment-1",
                "accountId": "account-1",
                "subject": "issuer#subject",
                "idempotencyKey": "key-1",
                "accessGeneration": 2,
                "operation": "activity_upsert",
                "resourceId": "activity-1",
                "lifecycleSuccessorId": "activity-2",
                "outcome": "Accepted",
                "bindingVersion": 1,
                "receiptBinding": "v1;binding"
            })
        );
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptResponse {
    deployment_id: String,
    account_id: String,
    subject: String,
    idempotency_key: String,
    access_generation: i64,
    operation: String,
    resource_id: String,
    lifecycle_successor_id: Option<String>,
    outcome: String,
    binding_version: i64,
    receipt_binding: String,
}

fn receipt(r: wealthfolio_storage_sqlite::bridge_policy::DisclosureReceipt) -> ReceiptResponse {
    ReceiptResponse {
        deployment_id: r.deployment_id,
        account_id: r.account_id,
        subject: r.subject,
        idempotency_key: r.idempotency_key,
        access_generation: r.access_generation,
        operation: r.operation,
        resource_id: r.resource_id,
        lifecycle_successor_id: r.lifecycle_successor_id,
        outcome: format!("{:?}", r.outcome),
        binding_version: r.binding_version,
        receipt_binding: r.receipt_binding,
    }
}

pub async fn handle_grant(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<AuthenticatedSubject>>,
    axum::extract::Path((subject, account)): axum::extract::Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_own_subject(auth, &subject)?;
    let store = get_bridge(&state)?;
    match store.grant(&subject, &account).await {
        Ok(g) => Ok(Json(serde_json::json!({"accessGeneration": g}))),
        Err(e) => Err(ApiError::BadRequest(format!("grant failed: {e}"))),
    }
}

pub async fn handle_receipt(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<AuthenticatedSubject>>,
    axum::extract::Path((subject, key)): axum::extract::Path<(String, String)>,
) -> Result<Json<ReceiptResponse>, ApiError> {
    require_own_subject(auth, &subject)?;
    let store = get_bridge(&state)?;
    match store.get_receipt(&subject, &key) {
        Ok(Some(value)) => Ok(Json(receipt(value))),
        Ok(None) => Err(ApiError::NotFound),
        Err(_) => Err(ApiError::Internal("receipt lookup failed".into())),
    }
}

pub async fn handle_commit(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<AuthenticatedSubject>>,
    Json(op): Json<wealthfolio_storage_sqlite::bridge_policy::DisclosureOperation>,
) -> Result<Json<ReceiptResponse>, ApiError> {
    require_own_subject(auth, &op.subject)?;
    let store = get_bridge(&state)?;
    match store.commit(op).await {
        Ok(r) => Ok(Json(receipt(r))),
        Err(e) => Err(ApiError::BadRequest(format!("commit failed: {e}"))),
    }
}

pub async fn handle_revoke(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<AuthenticatedSubject>>,
    axum::extract::Path((subject, account, key)): axum::extract::Path<(String, String, String)>,
) -> Result<Json<ReceiptResponse>, ApiError> {
    require_own_subject(auth, &subject)?;
    let store = get_bridge(&state)?;
    match store.revoke(&subject, &account, &key).await {
        Ok(r) => Ok(Json(receipt(r))),
        Err(e) => Err(ApiError::BadRequest(format!("revoke failed: {e}"))),
    }
}
