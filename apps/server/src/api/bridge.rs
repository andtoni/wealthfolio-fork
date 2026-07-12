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
/// When auth is not configured (`Extension` is `None`) the check is
/// skipped — there is no authenticated identity to compare against.
fn require_own_subject(
    auth: Option<Extension<AuthenticatedSubject>>,
    request_subject: &str,
) -> Result<(), ApiError> {
    if let Some(Extension(subject)) = auth {
        if subject.as_str() != request_subject {
            return Err(ApiError::Forbidden(
                "authenticated subject cannot act on behalf of another subject".into(),
            ));
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptResponse {
    subject: String,
    idempotency_key: String,
    access_generation: i64,
    operation: String,
    resource_id: String,
    outcome: String,
}

fn receipt(r: wealthfolio_storage_sqlite::bridge_policy::DisclosureReceipt) -> ReceiptResponse {
    ReceiptResponse {
        subject: r.subject,
        idempotency_key: r.idempotency_key,
        access_generation: r.access_generation,
        operation: r.operation,
        resource_id: r.resource_id,
        outcome: format!("{:?}", r.outcome),
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
