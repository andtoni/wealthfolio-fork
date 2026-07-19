//! Authenticated, subject-bound proxy for the Finance Sync provider control API.
//! Durable provider credentials remain in Finance Sync and are never returned here.

use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Extension, Json, Router,
};
use futures::StreamExt;
use reqwest::{header, Client, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wealthfolio_core::accounts::{
    AccountServiceTrait, AccountUpdate, NewAccount as CoreNewAccount, TrackingMode,
};

use crate::{
    auth::AuthenticatedSubject,
    error::{ApiError, ApiResult},
    main_lib::AppState,
};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_PUBLIC_TOKEN_BYTES: usize = 8 * 1024;
const MAX_LINK_VALUE_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Investec,
    Mercury,
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

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "investec" => Ok(Self::Investec),
            "mercury" => Ok(Self::Mercury),
            "plaid" => Ok(Self::Plaid),
            "snaptrade" => Ok(Self::SnapTrade),
            _ => Err("Unknown provider".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Available,
    Configured,
    NotConfigured,
    Unavailable,
    AuthRequired,
    Pending,
    LinkReady,
    Connected,
    Completed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnectionStatus {
    pub provider: ProviderKind,
    pub status: ProviderStatus,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnectionStartResponse {
    pub provider: ProviderKind,
    pub status: ProviderStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaidCompleteRequest {
    public_token: String,
    continuation_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncPlaidCompleteRequest<'a> {
    public_token: &'a str,
    continuation_token: &'a str,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnectionCompleteResponse {
    pub provider: ProviderKind,
    pub status: ProviderStatus,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveredProviderAsset {
    pub provider_asset_id: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub instrument_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveredProviderAccount {
    pub connection_id: String,
    pub provider_account_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub currency: String,
    pub mask: Option<String>,
    pub provider_generation: i64,
    pub assets: Vec<DiscoveredProviderAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AdmitProviderAccountRequest {
    connection_id: String,
    provider_account_id: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncAssetMapping<'a> {
    provider_asset_id: &'a str,
    wealthfolio_asset_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncAdmission<'a> {
    connection_id: &'a str,
    provider_account_id: &'a str,
    wealthfolio_account_id: &'a str,
    authenticated_subject: &'a str,
    database_state_digest: &'a str,
    provider_generation: i64,
    receiver_access_generation: i64,
    baseline_kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    prior_provider_generation: Option<i64>,
    asset_mappings: Vec<FinanceSyncAssetMapping<'a>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinanceSyncAdmissionStatus {
    wealthfolio_account_id: String,
    provider_generation: i64,
    receiver_access_generation: i64,
    baseline_status: String,
    baseline_digest: Option<String>,
    asset_mapping_digest: String,
}

fn empty_asset_mapping_digest() -> String {
    format!("sha256:{:x}", Sha256::digest(b"[]"))
}

fn canonical_empty_account_state_digest(
    subject: &str,
    provider: ProviderKind,
    connection_id: &str,
    provider_account_id: &str,
    account_id: &str,
    generation: i64,
) -> String {
    let mut digest = Sha256::new();
    for field in [
        "wealthfolio-empty-account-attestation-v1",
        subject,
        &provider.to_string(),
        connection_id,
        provider_account_id,
        account_id,
        &generation.to_string(),
        "activities=0;snapshots=0;positions=0;valuations=0",
    ] {
        digest.update(field.len().to_be_bytes());
        digest.update(field.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

fn canonical_provider_relink_digest(
    subject: &str,
    provider: ProviderKind,
    connection_id: &str,
    provider_account_id: &str,
    account_id: &str,
    receiver_generation: i64,
    prior_provider_generation: i64,
    provider_generation: i64,
) -> String {
    let mut digest = Sha256::new();
    for field in [
        "wealthfolio-provider-relink-attestation-v1",
        subject,
        &provider.to_string(),
        connection_id,
        provider_account_id,
        account_id,
        &receiver_generation.to_string(),
        &prior_provider_generation.to_string(),
        &provider_generation.to_string(),
        "existing-provider-account-and-overlays-preserved",
    ] {
        digest.update(field.len().to_be_bytes());
        digest.update(field.as_bytes());
    }
    format!("sha256:{:x}", digest.finalize())
}

fn empty_account_state_attestation(
    db_path: &str,
    subject: &str,
    provider: ProviderKind,
    connection_id: &str,
    provider_account_id: &str,
    account_id: &str,
    deployment_id: &str,
) -> ApiResult<(String, i64)> {
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let owns_account = connection
        .query_row(
            "SELECT count(*) FROM account_access_epochs WHERE account_id=?1 AND person_subject_id=?2 AND end_generation IS NULL",
            [account_id, subject],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if owns_account != 1 {
        return Err(ApiError::Forbidden(
            "Provider admission account is not owned by the authenticated subject.".to_string(),
        ));
    }
    let counts = [
        "SELECT count(*) FROM activities WHERE account_id=?1",
        "SELECT count(*) FROM holdings_snapshots WHERE account_id=?1",
        "SELECT count(*) FROM snapshot_positions WHERE snapshot_id IN (SELECT id FROM holdings_snapshots WHERE account_id=?1)",
        "SELECT count(*) FROM daily_account_valuation WHERE account_id=?1",
    ]
    .into_iter()
    .map(|sql| connection.query_row(sql, [account_id], |row| row.get::<_, i64>(0)))
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|error| ApiError::Internal(error.to_string()))?;
    if counts.iter().any(|count| *count != 0) {
        return Err(ApiError::BadRequest(
            "Provider admission requires an account with no activities, snapshots, positions, or valuations."
                .to_string(),
        ));
    }
    let generation = connection
        .query_row(
            "SELECT access_generation FROM policy_deployments WHERE id=?1",
            [deployment_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok((
        canonical_empty_account_state_digest(
            subject,
            provider,
            connection_id,
            provider_account_id,
            account_id,
            generation,
        ),
        generation,
    ))
}

fn provider_relink_state_attestation(
    db_path: &str,
    subject: &str,
    provider: ProviderKind,
    connection_id: &str,
    provider_account_id: &str,
    account_id: &str,
    deployment_id: &str,
    prior_provider_generation: i64,
    provider_generation: i64,
) -> ApiResult<(String, i64)> {
    if prior_provider_generation < 1 || provider_generation <= prior_provider_generation {
        return Err(ApiError::BadRequest(
            "Provider relink generation is not monotonic.".to_string(),
        ));
    }
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let owns_account: i64 = connection
        .query_row(
            "SELECT count(*) FROM account_access_epochs WHERE deployment_id=?1 AND account_id=?2 AND person_subject_id=?3 AND end_generation IS NULL",
            rusqlite::params![deployment_id, account_id, subject],
            |row| row.get(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let bound_mapping: i64 = connection
        .query_row(
            "SELECT count(*) FROM provider_account_connections WHERE account_id=?1 AND provider=?2 AND connection_id=?3 AND provider_account_id=?4 AND provider_generation=?5",
            rusqlite::params![account_id, provider.to_string(), connection_id, provider_account_id, provider_generation],
            |row| row.get(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if owns_account != 1 || bound_mapping != 1 {
        return Err(ApiError::Forbidden(
            "Provider relink account is not bound to the authenticated subject.".to_string(),
        ));
    }
    let receiver_generation: i64 = connection
        .query_row(
            "SELECT access_generation FROM policy_deployments WHERE id=?1",
            [deployment_id],
            |row| row.get(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok((
        canonical_provider_relink_digest(
            subject,
            provider,
            connection_id,
            provider_account_id,
            account_id,
            receiver_generation,
            prior_provider_generation,
            provider_generation,
        ),
        receiver_generation,
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmittedProviderAccount {
    pub account_id: String,
}

struct FinanceSyncControl {
    base_url: Url,
    token: String,
    client: Client,
}

impl FinanceSyncControl {
    fn from_env() -> ApiResult<Self> {
        let raw_url =
            std::env::var("FINANCE_SYNC_CONTROL_URL").map_err(|_| service_unavailable())?;
        let base_url = validate_control_url(&raw_url).map_err(|_| service_unavailable())?;
        let token_path = std::env::var("FINANCE_SYNC_CONTROL_TOKEN_FILE")
            .map(PathBuf::from)
            .map_err(|_| service_unavailable())?;
        let token = std::fs::read_to_string(token_path).map_err(|_| service_unavailable())?;
        let token = token.trim().to_owned();
        if token.is_empty() || token.len() > 16 * 1024 || token.chars().any(char::is_control) {
            return Err(service_unavailable());
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| service_unavailable())?;
        Ok(Self {
            base_url,
            token,
            client,
        })
    }

    fn endpoint(&self, suffix: &str) -> ApiResult<Url> {
        let mut url = self.base_url.clone();
        let base_path = url.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}{suffix}"));
        Ok(url)
    }

    async fn get<T: DeserializeOwned>(&self, suffix: &str, subject: &str) -> ApiResult<T> {
        let response = self
            .client
            .get(self.endpoint(suffix)?)
            .bearer_auth(&self.token)
            .header("X-Authenticated-Subject", subject)
            .send()
            .await
            .map_err(|_| service_unavailable())?;
        decode_response(response).await
    }

    async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        suffix: &str,
        subject: &str,
        body: Option<&B>,
    ) -> ApiResult<T> {
        let request = self
            .client
            .post(self.endpoint(suffix)?)
            .bearer_auth(&self.token)
            .header("X-Authenticated-Subject", subject);
        let response = match body {
            Some(value) => request.json(value),
            None => request.header(header::CONTENT_LENGTH, "0"),
        }
        .send()
        .await
        .map_err(|_| service_unavailable())?;
        decode_response(response).await
    }

    async fn admission_status(
        &self,
        provider: ProviderKind,
        subject: &str,
        connection_id: &str,
        provider_account_id: &str,
    ) -> ApiResult<Option<FinanceSyncAdmissionStatus>> {
        let mut endpoint = self.endpoint(&format!(
            "/control/v1/provider-connections/{provider}/admissions"
        ))?;
        endpoint
            .path_segments_mut()
            .map_err(|_| service_unavailable())?
            .push(connection_id)
            .push(provider_account_id);
        let response = self
            .client
            .get(endpoint)
            .bearer_auth(&self.token)
            .header("X-Authenticated-Subject", subject)
            .send()
            .await
            .map_err(|_| service_unavailable())?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        decode_response(response).await.map(Some)
    }
}

fn validate_control_url(value: &str) -> Result<Url, ()> {
    let url = Url::parse(value).map_err(|_| ())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    Ok(url)
}

fn service_unavailable() -> ApiError {
    ApiError::Internal("Provider connection service is unavailable.".to_string())
}

fn subject(subject: Option<Extension<AuthenticatedSubject>>) -> ApiResult<AuthenticatedSubject> {
    subject
        .map(|Extension(value)| value)
        .ok_or_else(|| ApiError::Unauthorized("Authentication is required.".to_string()))
}

async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> ApiResult<T> {
    if response.status().is_redirection() || !response.status().is_success() {
        return Err(if response.status() == StatusCode::BAD_REQUEST {
            ApiError::BadRequest("Provider connection request was rejected.".to_string())
        } else {
            service_unavailable()
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(service_unavailable());
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| service_unavailable())?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(service_unavailable());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| service_unavailable())
}

fn validate_list(statuses: &[ProviderConnectionStatus]) -> ApiResult<()> {
    if statuses.len() > 4 {
        return Err(service_unavailable());
    }
    let unique: HashSet<_> = statuses.iter().map(|status| status.provider).collect();
    if unique.len() != statuses.len()
        || statuses
            .iter()
            .any(|status| status.message.len() > MAX_LINK_VALUE_BYTES)
    {
        return Err(service_unavailable());
    }
    Ok(())
}

fn validate_start(response: &ProviderConnectionStartResponse) -> ApiResult<()> {
    if response.message.len() > MAX_LINK_VALUE_BYTES {
        return Err(service_unavailable());
    }
    match response.provider {
        ProviderKind::Plaid => {
            let token = response
                .link_token
                .as_deref()
                .ok_or_else(service_unavailable)?;
            let continuation = response
                .continuation_token
                .as_deref()
                .ok_or_else(service_unavailable)?;
            if response.redirect_url.is_some()
                || token.is_empty()
                || token.len() > MAX_LINK_VALUE_BYTES
                || continuation.len() < 32
                || continuation.len() > MAX_LINK_VALUE_BYTES
            {
                return Err(service_unavailable());
            }
        }
        ProviderKind::SnapTrade => {
            let redirect = response
                .redirect_url
                .as_deref()
                .ok_or_else(service_unavailable)?;
            let url = Url::parse(redirect).map_err(|_| service_unavailable())?;
            if response.link_token.is_some()
                || response.continuation_token.is_some()
                || url.scheme() != "https"
                || url.host_str() != Some("app.snaptrade.com")
                || url.path() != "/snapTrade/redeemToken"
                || !url.username().is_empty()
                || url.password().is_some()
                || redirect.len() > MAX_LINK_VALUE_BYTES
            {
                return Err(service_unavailable());
            }
        }
        ProviderKind::Investec | ProviderKind::Mercury => {
            if response.link_token.is_some()
                || response.continuation_token.is_some()
                || response.redirect_url.is_some()
            {
                return Err(service_unavailable());
            }
        }
    }
    Ok(())
}

async fn list_provider_connections(
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
) -> ApiResult<Json<Vec<ProviderConnectionStatus>>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let statuses: Vec<ProviderConnectionStatus> = FinanceSyncControl::from_env()?
        .get("/control/v1/provider-connections", subject.as_str())
        .await?;
    validate_list(&statuses)?;
    Ok(Json(statuses))
}

async fn start_provider_connection(
    Path(provider): Path<ProviderKind>,
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
) -> ApiResult<Json<ProviderConnectionStartResponse>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let response: ProviderConnectionStartResponse = FinanceSyncControl::from_env()?
        .post::<_, ()>(
            &format!("/control/v1/provider-connections/{provider}/start"),
            subject.as_str(),
            None,
        )
        .await?;
    if response.provider != provider {
        return Err(service_unavailable());
    }
    validate_start(&response)?;
    Ok(Json(response))
}

async fn complete_plaid_connection(
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
    Json(request): Json<PlaidCompleteRequest>,
) -> ApiResult<Json<ProviderConnectionCompleteResponse>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let token = request.public_token.trim();
    let continuation = request.continuation_token.trim();
    if token.is_empty()
        || token.len() > MAX_PUBLIC_TOKEN_BYTES
        || token.chars().any(char::is_control)
        || continuation.len() < 32
        || continuation.len() > MAX_LINK_VALUE_BYTES
        || continuation.chars().any(char::is_control)
    {
        return Err(ApiError::BadRequest(
            "Invalid Plaid completion request.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let body = FinanceSyncPlaidCompleteRequest {
        public_token: token,
        continuation_token: continuation,
    };
    let response: ProviderConnectionCompleteResponse = FinanceSyncControl::from_env()?
        .post(
            "/control/v1/provider-connections/plaid/complete",
            subject.as_str(),
            Some(&body),
        )
        .await?;
    if response.provider != ProviderKind::Plaid || response.message.len() > MAX_LINK_VALUE_BYTES {
        return Err(service_unavailable());
    }
    Ok(Json(response))
}

fn validate_discovered_accounts(accounts: &[DiscoveredProviderAccount]) -> ApiResult<()> {
    if accounts.len() > 256
        || accounts.iter().any(|account| {
            account.connection_id.is_empty()
                || account.connection_id.len() > MAX_LINK_VALUE_BYTES
                || account.provider_account_id.is_empty()
                || account.provider_account_id.len() > MAX_LINK_VALUE_BYTES
                || account.name.is_empty()
                || account.name.len() > 512
                || account.account_type.is_empty()
                || account.account_type.len() > 128
                || account.currency.is_empty()
                || account.currency.len() > 16
                || account.mask.as_ref().is_some_and(|mask| mask.len() > 128)
                || account.provider_generation < 0
                || account.assets.len() > 4096
                || account.assets.iter().any(|asset| {
                    asset.provider_asset_id.is_empty()
                        || asset.provider_asset_id.len() > MAX_LINK_VALUE_BYTES
                        || asset.symbol.is_empty()
                        || asset.symbol.len() > 128
                        || asset.name.is_empty()
                        || asset.name.len() > 512
                        || asset.currency.is_empty()
                        || asset.currency.len() > 16
                        || asset.instrument_type.is_empty()
                        || asset.instrument_type.len() > 128
                })
        })
    {
        return Err(service_unavailable());
    }
    Ok(())
}

async fn list_discovered_accounts(
    Path(provider): Path<ProviderKind>,
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
) -> ApiResult<Json<Vec<DiscoveredProviderAccount>>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let accounts: Vec<DiscoveredProviderAccount> = FinanceSyncControl::from_env()?
        .get(
            &format!("/control/v1/provider-connections/{provider}/accounts"),
            subject.as_str(),
        )
        .await?;
    validate_discovered_accounts(&accounts)?;
    Ok(Json(accounts))
}

fn mapped_provider_account_id(
    db_path: &str,
    provider: ProviderKind,
    connection_id: &str,
    provider_account_id: &str,
) -> ApiResult<Option<(String, i64)>> {
    use rusqlite::OptionalExtension;
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    connection
        .query_row(
            "SELECT account_id,provider_generation FROM provider_account_connections WHERE provider=?1 AND connection_id=?2 AND provider_account_id=?3",
            rusqlite::params![provider.to_string(), connection_id, provider_account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| ApiError::Internal(error.to_string()))
}

fn ensure_provider_account_access(
    db_path: &str,
    deployment_id: &str,
    subject: &str,
    account_id: &str,
    provider: ProviderKind,
    connection_id: &str,
    provider_account_id: &str,
    provider_generation: i64,
) -> ApiResult<()> {
    let mut connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON")
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let conflicting_owner: i64 = transaction
        .query_row(
            "SELECT count(*) FROM account_access_epochs WHERE account_id=?1 AND person_subject_id<>?2 AND end_generation IS NULL",
            rusqlite::params![account_id, subject],
            |row| row.get(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if conflicting_owner != 0 {
        return Err(ApiError::Forbidden(
            "Provider account is already bound to another authenticated subject.".to_string(),
        ));
    }
    let already_bound: i64 = transaction
        .query_row(
            "SELECT count(*) FROM account_access_epochs WHERE deployment_id=?1 AND account_id=?2 AND person_subject_id=?3 AND end_generation IS NULL",
            rusqlite::params![deployment_id, account_id, subject],
            |row| row.get(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if already_bound > 1 {
        return Err(ApiError::Internal(
            "Provider account access state is ambiguous.".to_string(),
        ));
    }

    if already_bound == 0 {
        transaction
            .execute(
                "INSERT OR IGNORE INTO policy_deployments(id) VALUES(?1)",
                [deployment_id],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO person_subjects(deployment_id,id) VALUES(?1,?2)",
                rusqlite::params![deployment_id, subject],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        let household_id = format!("private:{subject}");
        transaction
            .execute(
                "INSERT OR IGNORE INTO households(deployment_id,id) VALUES(?1,?2)",
                rusqlite::params![deployment_id, household_id],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        transaction
            .execute(
                "UPDATE policy_deployments SET access_generation=access_generation+1 WHERE id=?1",
                [deployment_id],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        let generation: i64 = transaction
            .query_row(
                "SELECT access_generation FROM policy_deployments WHERE id=?1",
                [deployment_id],
                |row| row.get(0),
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO native_account_deployments(account_id,deployment_id) VALUES(?1,?2)",
                rusqlite::params![account_id, deployment_id],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        let arrangement_id = uuid::Uuid::new_v4().to_string();
        transaction
        .execute(
            "INSERT INTO account_ownership_arrangements(deployment_id,id,household_id,account_id,kind,start_epoch) VALUES(?1,?2,?3,?4,'provider',?5)",
            rusqlite::params![deployment_id, arrangement_id, household_id, account_id, generation],
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
        transaction
        .execute(
            "INSERT INTO account_ownership_participants(deployment_id,arrangement_id,person_subject_id) VALUES(?1,?2,?3)",
            rusqlite::params![deployment_id, arrangement_id, subject],
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
        transaction
        .execute(
            "INSERT INTO account_access_epochs(deployment_id,household_id,account_id,person_subject_id,start_generation) VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![deployment_id, household_id, account_id, subject, generation],
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO provider_account_connections(account_id,provider,connection_id,provider_account_id,provider_generation) VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![account_id, provider.to_string(), connection_id, provider_account_id, provider_generation],
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let bound: (String, i64) = transaction
        .query_row(
            "SELECT account_id,provider_generation FROM provider_account_connections WHERE provider=?1 AND connection_id=?2 AND provider_account_id=?3",
            rusqlite::params![provider.to_string(), connection_id, provider_account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| {
            ApiError::BadRequest(
                "Provider account identity conflicts with an existing connection mapping."
                    .to_string(),
            )
        })?;
    if bound.0 != account_id || bound.1 > provider_generation {
        return Err(ApiError::BadRequest(
            "Provider account identity conflicts with an existing connection mapping.".to_string(),
        ));
    }
    if bound.1 < provider_generation {
        let updated = transaction
            .execute(
                "UPDATE provider_account_connections SET provider_generation=?1 WHERE account_id=?2 AND provider=?3 AND connection_id=?4 AND provider_account_id=?5 AND provider_generation=?6",
                rusqlite::params![provider_generation, account_id, provider.to_string(), connection_id, provider_account_id, bound.1],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
        if updated != 1 {
            return Err(ApiError::BadRequest(
                "Provider account generation changed during admission.".to_string(),
            ));
        }
    }
    transaction
        .commit()
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(())
}

async fn admit_provider_account(
    Path(provider): Path<ProviderKind>,
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
    Json(request): Json<AdmitProviderAccountRequest>,
) -> ApiResult<Json<AdmittedProviderAccount>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let name = request.name.trim();
    if name.is_empty()
        || name.len() > 512
        || request.connection_id.is_empty()
        || request.connection_id.len() > MAX_LINK_VALUE_BYTES
        || request.provider_account_id.is_empty()
        || request.provider_account_id.len() > MAX_LINK_VALUE_BYTES
    {
        return Err(ApiError::BadRequest(
            "Invalid provider account selection.".to_string(),
        ));
    }

    let provider_name = provider.to_string();
    let accounts = state.account_service.get_all_accounts()?;
    let mapped_account_id = mapped_provider_account_id(
        &state.db_path,
        provider,
        &request.connection_id,
        &request.provider_account_id,
    )?;
    let prior_provider_generation = mapped_account_id
        .as_ref()
        .map(|(_, generation)| *generation);
    let mut provider_generation = prior_provider_generation;
    let existing =
        if let Some(mapped_account_id) = mapped_account_id.as_ref().map(|(id, _)| id.as_str()) {
            Some(
                accounts
                    .iter()
                    .find(|account| account.id == mapped_account_id)
                    .cloned()
                    .ok_or_else(|| {
                        ApiError::Internal(
                            "Provider account connection mapping references a missing account."
                                .to_string(),
                        )
                    })?,
            )
        } else {
            accounts.into_iter().find(|account| {
                account.provider.as_deref() == Some(provider_name.as_str())
                    && account.provider_account_id.as_deref()
                        == Some(request.provider_account_id.as_str())
            })
        };
    let mut snaptrade_asset_count = None;
    let mut reconcile_committed = false;

    let (account, created_new) = if let Some(account) = existing {
        if account.provider.as_deref() != Some(provider_name.as_str())
            || account.provider_account_id.as_deref() != Some(&request.provider_account_id)
        {
            return Err(ApiError::BadRequest(
                "Provider account identity conflicts with an existing account.".to_string(),
            ));
        }
        {
            let discovered: Vec<DiscoveredProviderAccount> = FinanceSyncControl::from_env()?
                .get(
                    &format!("/control/v1/provider-connections/{provider}/accounts"),
                    subject.as_str(),
                )
                .await?;
            validate_discovered_accounts(&discovered)?;
            let source = discovered.iter().find(|candidate| {
                candidate.connection_id == request.connection_id
                    && candidate.provider_account_id == request.provider_account_id
            });
            if let Some(source) = source {
                provider_generation = Some(source.provider_generation);
                if provider == ProviderKind::SnapTrade {
                    snaptrade_asset_count = Some(source.assets.len());
                }
            } else if !account.is_active && mapped_account_id.is_some() {
                reconcile_committed = true;
            } else {
                return Err(ApiError::BadRequest(
                    "Provider account is no longer available.".to_string(),
                ));
            }
        }
        (account, false)
    } else {
        let discovered: Vec<DiscoveredProviderAccount> = FinanceSyncControl::from_env()?
            .get(
                &format!("/control/v1/provider-connections/{provider}/accounts"),
                subject.as_str(),
            )
            .await?;
        validate_discovered_accounts(&discovered)?;
        let source = discovered
            .into_iter()
            .find(|account| {
                account.connection_id == request.connection_id
                    && account.provider_account_id == request.provider_account_id
            })
            .ok_or_else(|| {
                ApiError::BadRequest("Provider account is no longer available.".to_string())
            })?;
        provider_generation = Some(source.provider_generation);
        if provider == ProviderKind::SnapTrade {
            snaptrade_asset_count = Some(source.assets.len());
        }
        let tracking_mode = if provider == ProviderKind::SnapTrade {
            TrackingMode::Holdings
        } else {
            TrackingMode::Transactions
        };
        let new_account = CoreNewAccount {
            id: None,
            name: name.to_string(),
            account_type: source.account_type,
            group: None,
            currency: source.currency,
            is_default: false,
            // Provider accounts remain pending until remote admission is
            // unambiguously acknowledged and any provider assets are admitted.
            is_active: false,
            platform_id: None,
            account_number: source.mask,
            meta: None,
            provider: Some(provider_name),
            provider_account_id: Some(source.provider_account_id),
            is_archived: false,
            tracking_mode,
            include_in_net_worth: false,
            name_source: "user".to_string(),
        };
        (
            state.account_service.create_account(new_account).await?,
            true,
        )
    };

    let provider_generation = provider_generation.ok_or_else(service_unavailable)?;
    if let Err(error) = ensure_provider_account_access(
        &state.db_path,
        &state.deployment_id,
        subject.as_str(),
        &account.id,
        provider,
        &request.connection_id,
        &request.provider_account_id,
        provider_generation,
    ) {
        if created_new {
            state.account_service.delete_account(&account.id).await?;
        }
        return Err(error);
    }

    // Admission is fail-closed for deterministic-id reuse: the database state is
    // checked immediately before the server-to-server completion call and the
    // attestation binds the authenticated subject, both provider identities, the
    // Wealthfolio account, and the current policy generation. Finance must accept
    // this exact completion contract; a caller-supplied prefix is never trusted.
    let (attestation, baseline_kind, relink_prior_generation) = match prior_provider_generation {
        Some(prior) if prior < provider_generation => (
            provider_relink_state_attestation(
                &state.db_path,
                subject.as_str(),
                provider,
                &request.connection_id,
                &request.provider_account_id,
                &account.id,
                &state.deployment_id,
                prior,
                provider_generation,
            ),
            "relink",
            Some(prior),
        ),
        _ => (
            empty_account_state_attestation(
                &state.db_path,
                subject.as_str(),
                provider,
                &request.connection_id,
                &request.provider_account_id,
                &account.id,
                &state.deployment_id,
            ),
            "empty",
            None,
        ),
    };
    let (database_state_digest, receiver_access_generation) = match attestation {
        Ok(attestation) => attestation,
        Err(error) => {
            if created_new {
                state.account_service.delete_account(&account.id).await?;
            }
            return Err(error);
        }
    };
    if let Some(asset_count) = snaptrade_asset_count.filter(|count| *count != 0) {
        if created_new {
            state.account_service.delete_account(&account.id).await?;
        }
        return Err(ApiError::BadRequest(format!(
            "SnapTrade account is pending explicit provider-asset admission; {asset_count} discovered provider asset(s) require immutable asset mappings."
        )));
    }
    if reconcile_committed {
        let committed = FinanceSyncControl::from_env()?
            .admission_status(
                provider,
                subject.as_str(),
                &request.connection_id,
                &request.provider_account_id,
            )
            .await?
            .ok_or_else(|| {
                ApiError::BadRequest("Provider account is no longer available.".to_string())
            })?;
        if committed.wealthfolio_account_id != account.id
            || committed.provider_generation != provider_generation
            || committed.receiver_access_generation != receiver_access_generation
            || committed.baseline_status != "verified"
            || committed.baseline_digest.as_deref() != Some(database_state_digest.as_str())
            || committed.asset_mapping_digest != empty_asset_mapping_digest()
        {
            return Err(ApiError::BadRequest(
                "Committed provider admission conflicts with the pending account.".to_string(),
            ));
        }
    } else {
        let body = FinanceSyncAdmission {
            connection_id: &request.connection_id,
            provider_account_id: &request.provider_account_id,
            wealthfolio_account_id: &account.id,
            authenticated_subject: subject.as_str(),
            database_state_digest: &database_state_digest,
            provider_generation,
            receiver_access_generation,
            baseline_kind,
            prior_provider_generation: relink_prior_generation,
            // This slice admits only accounts whose discovery explicitly attests
            // that no provider assets exist. Never derive mappings from symbols.
            asset_mappings: Vec::new(),
        };
        let admitted = FinanceSyncControl::from_env()?
            .post::<FinanceSyncAdmissionStatus, _>(
                &format!("/control/v1/provider-connections/{provider}/admit"),
                subject.as_str(),
                Some(&body),
            )
            .await?;
        if admitted.wealthfolio_account_id != account.id
            || admitted.provider_generation != provider_generation
            || admitted.receiver_access_generation != receiver_access_generation
            || admitted.baseline_status != "verified"
            || admitted.baseline_digest.as_deref() != Some(database_state_digest.as_str())
            || admitted.asset_mapping_digest != empty_asset_mapping_digest()
        {
            return Err(service_unavailable());
        }
    }
    if !account.is_active {
        state
            .account_service
            .update_account(AccountUpdate {
                id: Some(account.id.clone()),
                name: account.name.clone(),
                account_type: account.account_type.clone(),
                group: account.group.clone(),
                is_default: account.is_default,
                is_active: true,
                platform_id: account.platform_id.clone(),
                account_number: account.account_number.clone(),
                meta: account.meta.clone(),
                provider: account.provider.clone(),
                provider_account_id: account.provider_account_id.clone(),
                is_archived: Some(account.is_archived),
                tracking_mode: Some(account.tracking_mode),
                include_in_net_worth: Some(account.include_in_net_worth),
                name_source: Some(account.name_source.clone()),
            })
            .await?;
    }
    Ok(Json(AdmittedProviderAccount {
        account_id: account.id,
    }))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/provider-connections", get(list_provider_connections))
        .route(
            "/provider-connections/{provider}/start",
            post(start_provider_connection),
        )
        .route(
            "/provider-connections/plaid/complete",
            post(complete_plaid_connection),
        )
        .route(
            "/provider-connections/{provider}/accounts",
            get(list_discovered_accounts),
        )
        .route(
            "/provider-connections/{provider}/admit",
            post(admit_provider_account),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_url_requires_plain_http_or_https_origin() {
        assert!(validate_control_url("https://finance-sync.internal").is_ok());
        assert!(validate_control_url("http://127.0.0.1:8080/base").is_ok());
        assert!(validate_control_url("file:///tmp/control").is_err());
        assert!(validate_control_url("https://user:secret@example.com").is_err());
        assert!(validate_control_url("https://example.com?token=secret").is_err());
    }

    #[test]
    fn strict_start_schema_rejects_durable_or_unknown_fields() {
        let json = r#"{"provider":"plaid","status":"link_ready","message":"ready","linkToken":"short-lived","accessToken":"durable"}"#;
        assert!(serde_json::from_str::<ProviderConnectionStartResponse>(json).is_err());
    }

    #[test]
    fn provider_specific_start_payload_is_enforced() {
        let valid = ProviderConnectionStartResponse {
            provider: ProviderKind::SnapTrade,
            status: ProviderStatus::LinkReady,
            message: "ready".to_string(),
            link_token: None,
            continuation_token: None,
            redirect_url: Some(
                "https://app.snaptrade.com/snapTrade/redeemToken?token=short-lived".to_string(),
            ),
        };
        assert!(validate_start(&valid).is_ok());

        let insecure = ProviderConnectionStartResponse {
            redirect_url: Some(
                "http://app.snaptrade.com/snapTrade/redeemToken?token=short-lived".to_string(),
            ),
            ..valid
        };
        assert!(validate_start(&insecure).is_err());
    }

    #[test]
    fn admission_contract_fixture_matches_finance_wire_and_attestation() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/provider-admission-v1.json"
        ))
        .unwrap();
        let request = &fixture["request"];
        let digest = canonical_empty_account_state_digest(
            request["authenticatedSubject"].as_str().unwrap(),
            ProviderKind::Plaid,
            request["connectionId"].as_str().unwrap(),
            request["providerAccountId"].as_str().unwrap(),
            request["wealthfolioAccountId"].as_str().unwrap(),
            request["receiverAccessGeneration"].as_i64().unwrap(),
        );
        assert_eq!(request["databaseStateDigest"], digest);
        let body = FinanceSyncAdmission {
            connection_id: request["connectionId"].as_str().unwrap(),
            provider_account_id: request["providerAccountId"].as_str().unwrap(),
            wealthfolio_account_id: request["wealthfolioAccountId"].as_str().unwrap(),
            authenticated_subject: request["authenticatedSubject"].as_str().unwrap(),
            database_state_digest: &digest,
            provider_generation: request["providerGeneration"].as_i64().unwrap(),
            receiver_access_generation: request["receiverAccessGeneration"].as_i64().unwrap(),
            baseline_kind: request["baselineKind"].as_str().unwrap(),
            prior_provider_generation: None,
            asset_mappings: Vec::new(),
        };
        assert_eq!(serde_json::to_value(body).unwrap(), *request);
        let response: ProviderConnectionCompleteResponse =
            serde_json::from_value(fixture["response"].clone()).unwrap();
        assert_eq!(response.provider, ProviderKind::Plaid);
        assert_eq!(response.status, ProviderStatus::Configured);
    }

    #[test]
    fn list_rejects_duplicate_providers() {
        let statuses = vec![
            ProviderConnectionStatus {
                provider: ProviderKind::Plaid,
                status: ProviderStatus::Configured,
                message: "configured".to_string(),
            },
            ProviderConnectionStatus {
                provider: ProviderKind::Plaid,
                status: ProviderStatus::Configured,
                message: "configured".to_string(),
            },
        ];
        assert!(validate_list(&statuses).is_err());
    }
}
