//! Identity-bound production onboarding seam. Hard-disabled by default.
//! Routes are mounted inside `api::app_router` and extract state from AppState.
//!
//! Provider principal comes from a server-held CredentialStore (config/auth session),
//! never from request JSON. This prevents authenticated clients from minting arbitrary
//! provider-principal bindings.
use crate::auth::AuthenticatedSubject;
use crate::error::ApiError;
use crate::main_lib::AppState;
use axum::{
    extract::{Extension, Path, State},
    Json,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use wealthfolio_core::household_policy::CandidateAccount;
use wealthfolio_storage_sqlite::onboarding::{
    CredentialContext, DeploymentContext, OnboardingStore, ProviderPrincipal,
};

/// Maps an authenticated subject identity to its verified provider credentials.
/// The map is constructed from server configuration (env / config file) and is
/// never influenced by client request payloads.
#[derive(Clone, Debug)]
pub struct CredentialStore {
    /// (subject) -> (provider, base_principal)
    entries: HashMap<String, (String, String)>,
    /// Tracks the current incarnation generation for each base principal.
    incarnation: std::sync::Arc<std::sync::Mutex<HashMap<String, u64>>>,
}

impl CredentialStore {
    /// Build from a static mapping. In production this would come from env or a config file.
    /// Format: subject=provider:principal
    pub fn from_env() -> Self {
        let mut entries = HashMap::new();
        // Allow multiple WF_CREDENTIAL_<N> vars: WF_CREDENTIAL_0, WF_CREDENTIAL_1, ...
        // Or a single WF_CREDENTIALS map
        if let Ok(raw) = std::env::var("WF_CREDENTIALS") {
            for pair in raw.split(',') {
                let parts: Vec<&str> = pair.splitn(3, '=').collect();
                if parts.len() == 3 {
                    entries.insert(
                        parts[0].trim().to_string(),
                        (parts[1].trim().to_string(), parts[2].trim().to_string()),
                    );
                }
            }
        }
        // Also check individual WF_CREDENTIAL_<subject> vars
        for (key, val) in std::env::vars() {
            if key.starts_with("WF_CREDENTIAL_") {
                let subject = key.trim_start_matches("WF_CREDENTIAL_").to_string();
                if let Some((provider, principal)) = val.split_once(':') {
                    entries.insert(subject, (provider.to_string(), principal.to_string()));
                }
            }
        }
        Self {
            entries,
            incarnation: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Look up the verified (provider, principal) for an authenticated subject.
    pub fn get(&self, subject: &str) -> Option<(&str, &str)> {
        self.entries
            .get(subject)
            .map(|(p, pr)| (p.as_str(), pr.as_str()))
    }

    /// Generate a fresh provider principal for a relink/reconnect, appending
    /// an incarnation counter so each reconnect creates a distinct source identity.
    pub fn fresh_principal(&self, subject: &str) -> Option<(String, String)> {
        let base = self.entries.get(subject)?;
        let key = format!("{}:{}", base.0, base.1);
        let mut guard = self.incarnation.lock().unwrap();
        let gen = guard.entry(key.clone()).or_insert(0);
        *gen += 1;
        Some((base.0.clone(), format!("{}-v{}", base.1, gen)))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Helper to build a CredentialContext from the store.
fn credential(
    state: &AppState,
    subject: &AuthenticatedSubject,
) -> Result<CredentialContext, ApiError> {
    let svc = state
        .onboarding_state
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("household feature not enabled".into()))?;
    let (provider, principal) = svc
        .credential_store
        .get(subject.as_str())
        .ok_or_else(|| ApiError::Forbidden("no credential configured for subject".into()))?;
    CredentialContext::authenticated(
        svc.deployment.clone(),
        ProviderPrincipal::verified(provider, principal)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        subject.as_str(),
    )
    .map_err(|e| ApiError::BadRequest(e.to_string()))
}

/// Server-configured onboarding state stored in AppState.
pub struct OnboardingState {
    pub store: OnboardingStore,
    pub deployment: DeploymentContext,
    pub credential_store: Arc<CredentialStore>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdResponse {
    id: String,
}

pub async fn discover(
    State(state): State<Arc<AppState>>,
    Extension(subject): Extension<AuthenticatedSubject>,
    Json(r): Json<serde_json::Value>,
) -> Result<Json<IdResponse>, ApiError> {
    let svc = state
        .onboarding_state
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("household feature not enabled".into()))?;
    let provider_account_id = r
        .get("providerAccountId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("missing providerAccountId".into()))?
        .to_string();
    let name = r
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("missing name".into()))?
        .to_string();
    let account_type = r
        .get("accountType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("missing accountType".into()))?
        .to_string();
    let currency = r
        .get("currency")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("missing currency".into()))?
        .to_string();
    let provider = r
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("missing provider".into()))?
        .to_string();

    let c = credential(&state, &subject)?;
    // Verify provider matches the credential store entry for this subject
    if let Some((cred_provider, _)) = svc.credential_store.get(subject.as_str()) {
        if provider != cred_provider {
            return Err(ApiError::Forbidden("provider mismatch".into()));
        }
    }
    svc.store
        .bind_credential_owner(&c)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let id = svc
        .store
        .discover_candidate(
            &c,
            CandidateAccount {
                provider,
                provider_account_id,
                name,
                account_type,
                currency,
            },
        )
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(IdResponse { id }))
}

pub async fn approve(
    State(state): State<Arc<AppState>>,
    Extension(subject): Extension<AuthenticatedSubject>,
    Path(id): Path<String>,
) -> Result<Json<IdResponse>, ApiError> {
    let svc = state
        .onboarding_state
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("household feature not enabled".into()))?;
    let c = credential(&state, &subject)?;
    let aid = svc
        .store
        .approve_candidate(&c, &id, None)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(IdResponse { id: aid }))
}

pub async fn relink(
    State(state): State<Arc<AppState>>,
    Extension(subject): Extension<AuthenticatedSubject>,
    Path(id): Path<String>,
) -> Result<Json<IdResponse>, ApiError> {
    let svc = state
        .onboarding_state
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("household feature not enabled".into()))?;
    // Generate a fresh provider principal for each relink so the new
    // connection cannot silently inherit the previous credential identity.
    let (provider, fresh_principal) = svc
        .credential_store
        .fresh_principal(subject.as_str())
        .ok_or_else(|| ApiError::Forbidden("no credential configured for subject".into()))?;

    let c = CredentialContext::authenticated(
        svc.deployment.clone(),
        ProviderPrincipal::verified(provider, fresh_principal)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
        subject.as_str(),
    )
    .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Ensure the credential owner binding exists for the fresh principal
    svc.store
        .bind_credential_owner(&c)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let new_id = svc
        .store
        .unlink_and_relink(&c, &id)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(IdResponse { id: new_id }))
}
