//! Bridge policy Axum handlers. Routes are mounted in `api::app_router` and
//! extract state from AppState. The `router()` function is kept for backward
//! compatibility but routes are now inlined in api.rs.
use axum::{extract::State, Extension, Json};
use chrono::NaiveDate;
use std::sync::Arc;

use crate::auth::AuthenticatedSubject;
use crate::error::ApiError;
use crate::events::{ServerEvent, PORTFOLIO_UPDATE_COMPLETE, PORTFOLIO_UPDATE_START};
use crate::main_lib::AppState;
use wealthfolio_core::portfolio::valuation::{ValuationRecalcMode, ValuationServiceTrait};

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
fn changes_activity_queries(
    kind: wealthfolio_storage_sqlite::bridge_policy::DisclosureKind,
) -> bool {
    use wealthfolio_storage_sqlite::bridge_policy::DisclosureKind;
    matches!(
        kind,
        DisclosureKind::ActivityUpsert | DisclosureKind::ActivityDelete
    )
}

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
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[test]
    fn activity_bridge_commits_invalidate_live_queries() {
        use wealthfolio_storage_sqlite::bridge_policy::DisclosureKind;
        assert!(changes_activity_queries(DisclosureKind::ActivityUpsert));
        assert!(changes_activity_queries(DisclosureKind::ActivityDelete));
        assert!(!changes_activity_queries(
            DisclosureKind::CurrentStateUpsert
        ));
        assert!(!changes_activity_queries(DisclosureKind::ProjectionFence));
    }

    #[test]
    fn bridge_storage_failures_ask_the_producer_to_retry() {
        use wealthfolio_storage_sqlite::bridge_policy::BridgePolicyError;
        let unavailable = bridge_error(
            BridgePolicyError::Storage("The transaction manager is broken".into()),
            "commit",
        );
        assert!(matches!(unavailable, ApiError::ServiceUnavailable(_)));
        assert_eq!(
            unavailable.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        let policy = bridge_error(BridgePolicyError::AccessDenied, "commit");
        assert!(matches!(policy, ApiError::BadRequest(_)));
        assert_eq!(policy.into_response().status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn bridge_requires_an_authenticated_subject() {
        let err = require_own_subject(None, "subject").expect_err("missing auth must fail closed");
        assert!(matches!(err, ApiError::Unauthorized(_)));
    }

    #[test]
    fn merchant_context_wire_shape_is_strict() {
        let valid = serde_json::json!({
            "idempotency_key": "key", "subject": "subject", "access_generation": 1,
            "kind": "ActivityUpsert", "resource_id": "activity", "lifecycle_successor_id": null, "observation_revision": 7,
            "activity": {
                "account_id": "account", "activity_type": "WITHDRAWAL", "status": "POSTED",
                "activity_date": "2026-01-01T00:00:00Z", "amount": "1", "currency": "USD",
                "notes": null, "activity_type_override": null, "needs_review": null,
                "merchant_context": {
                    "merchant_key": "merchant:coffee", "merchant_key_version": 1,
                    "normalization_version": 1
                }
            }
        });
        serde_json::from_value::<wealthfolio_storage_sqlite::bridge_policy::DisclosureOperation>(
            valid.clone(),
        )
        .expect("strict v1 context");

        let mut category_bearing = valid;
        category_bearing["activity"]["merchant_context"]["category_id"] = serde_json::json!("food");
        assert!(serde_json::from_value::<
            wealthfolio_storage_sqlite::bridge_policy::DisclosureOperation,
        >(category_bearing)
        .is_err());
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
            binding_version: 3,
            receipt_binding: "v3;binding".into(),
            semantic_hash: None,
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
                "bindingVersion": 3,
                "receiptBinding": "v3;binding",
                "semanticHash": null
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
    semantic_hash: Option<String>,
}

/// Maps a bridge-policy failure to an HTTP status.
///
/// Policy failures are the caller's problem and stay 4xx. A storage failure
/// means this instance could not write (an unwritable or wedged database), which
/// is transient: answer 503 so the producer retries the same operation instead of
/// recording it as a terminal rejection.
fn bridge_error(
    error: wealthfolio_storage_sqlite::bridge_policy::BridgePolicyError,
    action: &str,
) -> ApiError {
    use wealthfolio_storage_sqlite::bridge_policy::BridgePolicyError;
    match error {
        BridgePolicyError::Storage(reason) => {
            tracing::warn!(%reason, action, "bridge storage failure; asking the producer to retry");
            ApiError::ServiceUnavailable(format!(
                "{action} failed: service temporarily unavailable"
            ))
        }
        other => ApiError::BadRequest(format!("{action} failed: {other}")),
    }
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
        semantic_hash: r.semantic_hash,
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

fn enqueue_current_state_valuation_refresh(
    state: Arc<AppState>,
    account_id: String,
    as_of_date: String,
) {
    let Ok(as_of_date) = NaiveDate::parse_from_str(&as_of_date, "%Y-%m-%d") else {
        tracing::warn!(%account_id, "Skipping current-state valuation refresh with invalid date");
        return;
    };

    tokio::spawn(async move {
        state
            .event_bus
            .publish(ServerEvent::new(PORTFOLIO_UPDATE_START));
        if let Err(error) = state
            .valuation_service
            .calculate_valuation_history(&account_id, ValuationRecalcMode::SinceDate(as_of_date))
            .await
        {
            tracing::warn!(%account_id, %as_of_date, %error, "Current-state valuation refresh failed");
        }
        state
            .event_bus
            .publish(ServerEvent::new(PORTFOLIO_UPDATE_COMPLETE));
    });
}

pub async fn handle_commit(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<AuthenticatedSubject>>,
    Json(op): Json<wealthfolio_storage_sqlite::bridge_policy::DisclosureOperation>,
) -> Result<Json<ReceiptResponse>, ApiError> {
    require_own_subject(auth, &op.subject)?;
    let store = get_bridge(&state)?;
    let invalidate_activity_queries = changes_activity_queries(op.kind);
    let current_state_refresh = (op.kind
        == wealthfolio_storage_sqlite::bridge_policy::DisclosureKind::CurrentStateUpsert)
        .then(|| {
            op.current_state
                .as_ref()
                .map(|state| (state.account_id.clone(), state.as_of_date.clone()))
        })
        .flatten();
    match store.commit(op).await {
        Ok(r) => {
            if invalidate_activity_queries {
                state
                    .event_bus
                    .publish(ServerEvent::new(PORTFOLIO_UPDATE_COMPLETE));
            }
            if matches!(
                &r.outcome,
                wealthfolio_storage_sqlite::bridge_policy::ReceiptOutcome::Accepted
            ) {
                if let Some((account_id, as_of_date)) = current_state_refresh {
                    enqueue_current_state_valuation_refresh(state.clone(), account_id, as_of_date);
                }
            }
            Ok(Json(receipt(r)))
        }
        Err(e) => Err(bridge_error(e, "commit")),
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
        Err(e) => Err(bridge_error(e, "revoke")),
    }
}
