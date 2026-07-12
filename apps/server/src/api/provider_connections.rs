//! Contract-safe subject-bound provider Connect control plane.
//!
//! This module implements the typed provider allowlist (Investec, Mercury, Plaid,
//! SnapTrade) and exposes authenticated, subject-bound control endpoints. No real
//! provider network calls, credentials, secrets, or synthetic URLs/tokens are ever
//! produced or exposed to the browser.
//!
//! ## Security contract
//!
//! - All routes require authentication enforced by the `require_jwt` middleware
//!   on the `protected_api` group. When authentication is disabled (development
//!   mode), every provider returns `unavailable` status because safe provider
//!   linking requires an authenticated subject bound to the request.
//! - The `start` endpoint extracts the `AuthenticatedSubject` from request
//!   extensions (inserted by `require_jwt`). Cross-subject injection is
//!   defeated because the middleware overrides the extension after JWT
//!   validation — a client-supplied `x-authenticated-subject` header is
//!   ignored.
//! - No secret-adjacent field (`secret`, `apiKey`, `token`, `accessToken`,
//!   `url`, `redirectUrl`) is ever present in responses. This is enforced by
//!   the type definition itself — there is no conditional skip logic.
//! - Fail-closed: when runtime auth config is absent, the provider allowlist
//!   returns `auth_required` status for every provider with a clear human-readable
//!   explanation. The `start` and `review` endpoints also fail closed.
//!
//! ## Routes (mounted under /api/v1/provider-connections)
//!
//! - `GET /` — returns typed `Vec<ProviderConnectionStatus>` with truthful
//!   status per provider, never secrets.
//! - `GET /{provider}` — returns a detailed `ProviderConnectionReview` for one
//!   provider, including the authenticated subject bound to the request.
//! - `POST /{provider}/start` — subject-bound start request. Returns a truthful
//!   `ProviderConnectionStartResponse` confirming the authenticated subject and
//!   explaining that real provider linking is not yet implemented.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::AuthenticatedSubject,
    error::{ApiError, ApiResult},
    main_lib::AppState,
};

// Types

/// Typed provider allowlist. Every variant corresponds to a real financial
/// data provider. No other stringly-typed provider identifier is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Investec,
    Mercury,
    #[serde(rename = "plaid")]
    Plaid,
    SnapTrade,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Investec => write!(f, "investec"),
            Self::Mercury => write!(f, "mercury"),
            Self::Plaid => write!(f, "plaid"),
            Self::SnapTrade => write!(f, "snaptrade"),
        }
    }
}

impl std::str::FromStr for ProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "investec" => Ok(Self::Investec),
            "mercury" => Ok(Self::Mercury),
            "plaid" => Ok(Self::Plaid),
            "snaptrade" => Ok(Self::SnapTrade),
            other => Err(format!(
                "Unknown provider '{other}'. Allowed providers: investec, mercury, plaid, snaptrade"
            )),
        }
    }
}

/// Truthful connection status for one provider.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionStatus {
    pub provider: ProviderKind,
    pub status: &'static str,
    pub message: &'static str,
}

/// Subject-bound provider connection start response.
///
/// Returned when an authenticated user initiates a provider connection.
/// Never contains secrets, URLs, tokens, or credentials.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionStartResponse {
    pub provider: ProviderKind,
    pub status: &'static str,
    pub message: String,
    pub authenticated_subject: String,
}

/// Detailed review status for a single provider connection.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionReview {
    pub provider: ProviderKind,
    pub status: &'static str,
    pub message: &'static str,
    pub authenticated_subject: String,
}

const MSG_NOT_CONFIGURED: &str =
    "Direct provider linking is not yet available. Connect manually via CSV import or add an account.";
const MSG_AUTH_REQUIRED: &str =
    "Provider linking requires authentication to be configured. Set WF_AUTH_PASSWORD_HASH and restart.";

const PROVIDER_STATUSES_NOT_CONFIGURED: &[ProviderConnectionStatus] = &[
    ProviderConnectionStatus { provider: ProviderKind::Investec, status: "not_configured", message: MSG_NOT_CONFIGURED },
    ProviderConnectionStatus { provider: ProviderKind::Mercury, status: "not_configured", message: MSG_NOT_CONFIGURED },
    ProviderConnectionStatus { provider: ProviderKind::Plaid, status: "not_configured", message: MSG_NOT_CONFIGURED },
    ProviderConnectionStatus { provider: ProviderKind::SnapTrade, status: "not_configured", message: MSG_NOT_CONFIGURED },
];

const PROVIDER_STATUSES_AUTH_REQUIRED: &[ProviderConnectionStatus] = &[
    ProviderConnectionStatus { provider: ProviderKind::Investec, status: "auth_required", message: MSG_AUTH_REQUIRED },
    ProviderConnectionStatus { provider: ProviderKind::Mercury, status: "auth_required", message: MSG_AUTH_REQUIRED },
    ProviderConnectionStatus { provider: ProviderKind::Plaid, status: "auth_required", message: MSG_AUTH_REQUIRED },
    ProviderConnectionStatus { provider: ProviderKind::SnapTrade, status: "auth_required", message: MSG_AUTH_REQUIRED },
];

// Handlers

async fn list_provider_connections(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<&'static [ProviderConnectionStatus]>> {
    if state.auth.is_none() {
        return Ok(Json(PROVIDER_STATUSES_AUTH_REQUIRED));
    }
    Ok(Json(PROVIDER_STATUSES_NOT_CONFIGURED))
}

async fn review_provider_connection(
    Path(provider): Path<ProviderKind>,
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<AuthenticatedSubject>>,
) -> ApiResult<Json<ProviderConnectionReview>> {
    if state.auth.is_none() {
        return Err(ApiError::NotImplemented(format!(
            "Provider linking requires authentication to be configured. Cannot review {provider} connection."
        )));
    }
    let subject = subject
        .map(|Extension(s)| s)
        .ok_or_else(|| ApiError::Internal(
            "Authenticated subject not found in request extensions. This indicates the require_jwt middleware did not run.".to_string(),
        ))?;

    Ok(Json(ProviderConnectionReview {
        provider,
        status: "not_configured",
        message: MSG_NOT_CONFIGURED,
        authenticated_subject: subject.as_str().to_owned(),
    }))
}

async fn start_provider_connection(
    Path(provider): Path<ProviderKind>,
    State(state): State<Arc<AppState>>,
    subject: Option<Extension<AuthenticatedSubject>>,
) -> ApiResult<Json<ProviderConnectionStartResponse>> {
    if state.auth.is_none() {
        return Err(ApiError::NotImplemented(format!(
            "Provider linking requires authentication to be configured. Cannot initiate {provider} connection."
        )));
    }
    let subject = subject
        .map(|Extension(s)| s)
        .ok_or_else(|| ApiError::Internal(
            "Authenticated subject not found in request extensions. This indicates the require_jwt middleware did not run.".to_string(),
        ))?;

    Ok(Json(ProviderConnectionStartResponse {
        provider,
        status: "not_implemented",
        message: format!(
            "Direct provider linking for {provider} is not yet implemented. No URL, token, or credentials were generated."
        ),
        authenticated_subject: subject.as_str().to_owned(),
    }))
}

// Router

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/provider-connections", get(list_provider_connections))
        .route("/provider-connections/{provider}", get(review_provider_connection))
        .route("/provider-connections/{provider}/start", post(start_provider_connection))
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_display_matches_serde() {
        assert_eq!(ProviderKind::Investec.to_string(), "investec");
        assert_eq!(ProviderKind::Mercury.to_string(), "mercury");
        assert_eq!(ProviderKind::Plaid.to_string(), "plaid");
        assert_eq!(ProviderKind::SnapTrade.to_string(), "snaptrade");
    }

    #[test]
    fn provider_kind_from_str_roundtrips() {
        for kind in &[ProviderKind::Investec, ProviderKind::Mercury, ProviderKind::Plaid, ProviderKind::SnapTrade] {
            let s = kind.to_string();
            let parsed: ProviderKind = s.parse().unwrap();
            assert_eq!(parsed, *kind);
        }
    }

    #[test]
    fn provider_kind_from_str_rejects_unknown() {
        let result: Result<ProviderKind, String> = "unknown_bank".parse();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown_bank"));
        assert!(err.contains("investec"));
    }

    #[test]
    fn provider_statuses_not_configured_has_all_four() {
        assert_eq!(PROVIDER_STATUSES_NOT_CONFIGURED.len(), 4);
        let names: Vec<ProviderKind> = PROVIDER_STATUSES_NOT_CONFIGURED.iter().map(|s| s.provider).collect();
        assert!(names.contains(&ProviderKind::Investec));
        assert!(names.contains(&ProviderKind::Mercury));
        assert!(names.contains(&ProviderKind::Plaid));
        assert!(names.contains(&ProviderKind::SnapTrade));
    }

    #[test]
    fn provider_statuses_auth_required_has_all_four() {
        assert_eq!(PROVIDER_STATUSES_AUTH_REQUIRED.len(), 4);
        for status in PROVIDER_STATUSES_AUTH_REQUIRED {
            assert_eq!(status.status, "auth_required");
        }
    }

    #[test]
    fn start_response_has_no_secret_fields() {
        let response = ProviderConnectionStartResponse {
            provider: ProviderKind::Plaid,
            status: "not_implemented",
            message: "test".to_string(),
            authenticated_subject: "urn:test:subject".to_string(),
        };
        let json = serde_json::to_value(&response).unwrap();
        let obj = json.as_object().unwrap();
        for forbidden in &["secret", "apiKey", "token", "accessToken", "url", "redirectUrl"] {
            assert!(!obj.contains_key(*forbidden), "response must not contain field: {forbidden}");
        }
    }

    #[test]
    fn review_response_has_no_secret_fields() {
        let response = ProviderConnectionReview {
            provider: ProviderKind::Mercury,
            status: "not_configured",
            message: MSG_NOT_CONFIGURED,
            authenticated_subject: "urn:test:subject".to_string(),
        };
        let json = serde_json::to_value(&response).unwrap();
        let obj = json.as_object().unwrap();
        for forbidden in &["secret", "apiKey", "token", "accessToken", "url", "redirectUrl"] {
            assert!(!obj.contains_key(*forbidden), "response must not contain field: {forbidden}");
        }
    }

    #[test]
    fn start_response_contains_authenticated_subject() {
        let response = ProviderConnectionStartResponse {
            provider: ProviderKind::SnapTrade,
            status: "not_implemented",
            message: "test".to_string(),
            authenticated_subject: "urn:wealthfolio:password#test-user".to_string(),
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["authenticatedSubject"], "urn:wealthfolio:password#test-user");
        assert_eq!(json["provider"], "snaptrade");
        assert_eq!(json["status"], "not_implemented");
    }
}
