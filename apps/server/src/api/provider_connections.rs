//! Authenticated, subject-bound proxy for the Finance Sync provider control API.
//! Durable provider credentials remain in Finance Sync and are never returned here.

use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    body::Bytes,
    extract::{Path, State},
    routing::{get, post},
    Extension, Json, Router,
};
use futures::StreamExt;
use reqwest::{header, Client, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wealthfolio_core::accounts::{
    account_types, AccountServiceTrait, AccountUpdate, NewAccount as CoreNewAccount, TrackingMode,
};
use wealthfolio_core::activities::{Activity, ActivityStatus};

use crate::{
    auth::AuthenticatedSubject,
    error::{ApiError, ApiResult},
    main_lib::AppState,
};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_PUBLIC_TOKEN_BYTES: usize = 8 * 1024;
const MAX_LINK_VALUE_BYTES: usize = 8 * 1024;
const MAX_PLAID_CONNECTION_ID_BYTES: usize = 512;
const MAX_PLAID_CONNECTIONS: usize = 64;
const MAX_CONNECTION_LABEL_BYTES: usize = 1024;
const MAX_CONNECTION_REASON_BYTES: usize = 64;

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

fn native_account_type(provider: ProviderKind, provider_type: &str) -> &'static str {
    if provider == ProviderKind::SnapTrade {
        return account_types::SECURITIES;
    }
    if provider != ProviderKind::Plaid {
        // Investec's private-bank products are transactional accounts that may
        // legitimately carry negative cash balances; Mercury is cash-ledger too.
        return account_types::CASH;
    }

    match provider_type.trim().to_ascii_lowercase().as_str() {
        "credit card" | "line of credit" | "loan" | "mortgage" | "student loan" => {
            account_types::CREDIT_CARD
        }
        "401a" | "401k" | "403b" | "457b" | "brokerage" | "hsa" | "ira" | "keogh" | "lira"
        | "pension" | "roth" | "rrif" | "rrsp" | "sep ira" | "simple ira" | "sipp"
        | "stock plan" | "tfsa" | "trust" | "ugma" | "utma" => account_types::SECURITIES,
        _ => account_types::CASH,
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

/// One connected Plaid Item. `connection_id` is the exact selection the Plaid
/// update route accepts; the label holds account names and masks only. `reason`
/// is a fixed code from Finance Sync's allowlist, never provider text.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnectionSummary {
    pub connection_id: String,
    pub label: String,
    pub needs_reauth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnectionStatus {
    pub provider: ProviderKind,
    pub status: ProviderStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ProviderConnectionSummary>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaidUpdateCompleteRequest {
    continuation_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaidUpdateRequest {
    connection_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncPlaidUpdateRequest<'a> {
    connection_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncPlaidCompleteRequest<'a> {
    public_token: &'a str,
    continuation_token: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncPlaidUpdateCompleteRequest<'a> {
    continuation_token: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvestecCardLifecycleLinkRequest {
    pending_activity_id: String,
    posted_activity_id: String,
    reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncCardLifecycleLinkRequest<'a> {
    source_resource_incarnation_id: &'a str,
    pending_transaction_id: &'a str,
    posted_transaction_id: &'a str,
    reason: &'a str,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvestecCardLifecycleLinkResponse {
    override_id: String,
    pending_resource_id: String,
    posted_resource_id: String,
    inserted: bool,
    delivered: u64,
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

#[derive(Clone, Debug)]
struct ProviderRelinkHandoff {
    provider_generation: i64,
    prior_provider_generation: i64,
    receiver_access_generation: i64,
    database_state_digest: String,
    acknowledged: bool,
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
            rusqlite::params![account_id, provider.to_string(), connection_id, provider_account_id, prior_provider_generation],
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
        decode_response(self.send_post(suffix, subject, body).await?).await
    }

    async fn send_post<B: Serialize + ?Sized>(
        &self,
        suffix: &str,
        subject: &str,
        body: Option<&B>,
    ) -> ApiResult<reqwest::Response> {
        let request = self
            .client
            .post(self.endpoint(suffix)?)
            .bearer_auth(&self.token)
            .header("X-Authenticated-Subject", subject);
        match body {
            Some(value) => request.json(value),
            None => request.header(header::CONTENT_LENGTH, "0"),
        }
        .send()
        .await
        .map_err(|_| service_unavailable())
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
        return Err(match response.status() {
            StatusCode::BAD_REQUEST => ApiError::BadRequest("Provider connection request was rejected.".to_string()),
            StatusCode::CONFLICT => ApiError::Conflict("A provider connection operation is already in progress or conflicts with current state.".to_string()),
            _ => service_unavailable(),
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

/// The same bound Finance Sync applies to a Plaid Item selection: an opaque,
/// non-empty provider identifier without control characters.
fn valid_plaid_connection_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PLAID_CONNECTION_ID_BYTES
        && !value.chars().any(char::is_control)
}

/// A reason is a fixed upper-case code such as `ITEM_LOGIN_REQUIRED`; anything
/// shaped like free text is rejected.
fn valid_connection_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONNECTION_REASON_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
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
    for status in statuses {
        // Only Plaid Items can be re-authenticated in place.
        if status.connections.len() > MAX_PLAID_CONNECTIONS
            || (status.provider != ProviderKind::Plaid && !status.connections.is_empty())
        {
            return Err(service_unavailable());
        }
        let ids: HashSet<_> = status
            .connections
            .iter()
            .map(|connection| connection.connection_id.as_str())
            .collect();
        if ids.len() != status.connections.len()
            || status.connections.iter().any(|connection| {
                !valid_plaid_connection_id(&connection.connection_id)
                    || connection.label.is_empty()
                    || connection.label.len() > MAX_CONNECTION_LABEL_BYTES
                    || connection.label.chars().any(char::is_control)
                    || connection
                        .reason
                        .as_deref()
                        .is_some_and(|reason| !valid_connection_reason(reason))
            })
        {
            return Err(service_unavailable());
        }
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

fn provider_activity_identity(
    deployment_id: &str,
    activity: &Activity,
) -> ApiResult<(String, String)> {
    if activity.source_system.as_deref() != Some("FINANCE")
        || activity.source_record_id.as_deref().is_none()
    {
        return Err(ApiError::BadRequest(
            "Both activities must be Finance Sync provider activities.".to_string(),
        ));
    }
    let prefix = format!("{deployment_id}:");
    let remainder = activity.id.strip_prefix(&prefix).ok_or_else(|| {
        ApiError::BadRequest("Activity is not a Finance Sync provider resource.".to_string())
    })?;
    let (source, transaction_id) = remainder.split_once(':').ok_or_else(|| {
        ApiError::BadRequest("Activity provider identity is incomplete.".to_string())
    })?;
    if uuid::Uuid::parse_str(source).is_err()
        || transaction_id.is_empty()
        || activity.source_record_id.as_deref() != Some(transaction_id)
    {
        return Err(ApiError::BadRequest(
            "Activity provider identity is invalid.".to_string(),
        ));
    }
    Ok((source.to_string(), transaction_id.to_string()))
}

fn validate_lifecycle_link_response(response: &InvestecCardLifecycleLinkResponse) -> ApiResult<()> {
    if response.override_id.is_empty()
        || response.override_id.len() > MAX_LINK_VALUE_BYTES
        || response.pending_resource_id.is_empty()
        || response.pending_resource_id.len() > MAX_LINK_VALUE_BYTES
        || response.posted_resource_id.is_empty()
        || response.posted_resource_id.len() > MAX_LINK_VALUE_BYTES
    {
        return Err(service_unavailable());
    }
    Ok(())
}

async fn link_investec_card_lifecycle(
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
    Json(request): Json<InvestecCardLifecycleLinkRequest>,
) -> ApiResult<Json<InvestecCardLifecycleLinkResponse>> {
    if !crate::features::investec_card_lifecycle_link_enabled() {
        return Err(ApiError::Forbidden(
            "Investec card lifecycle linking is disabled.".to_string(),
        ));
    }
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let reason = request.reason.trim();
    if request.pending_activity_id.is_empty()
        || request.pending_activity_id.len() > MAX_LINK_VALUE_BYTES
        || request.posted_activity_id.is_empty()
        || request.posted_activity_id.len() > MAX_LINK_VALUE_BYTES
        || request.pending_activity_id == request.posted_activity_id
        || reason.is_empty()
        || reason.len() > 2048
        || reason
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\r')
    {
        return Err(ApiError::BadRequest(
            "Invalid Investec card lifecycle link request.".to_string(),
        ));
    }
    let pending = state
        .activity_service
        .get_activity(&request.pending_activity_id)?;
    let posted = state
        .activity_service
        .get_activity(&request.posted_activity_id)?;
    if pending.status != ActivityStatus::Pending || posted.status != ActivityStatus::Posted {
        return Err(ApiError::BadRequest(
            "Select one pending and one posted activity.".to_string(),
        ));
    }
    if pending.account_id != posted.account_id {
        return Err(ApiError::BadRequest(
            "The activities must use the same account.".to_string(),
        ));
    }
    let account = state
        .account_service
        .get_all_accounts()?
        .into_iter()
        .find(|account| account.id == pending.account_id)
        .ok_or_else(|| ApiError::BadRequest("The activity account was not found.".to_string()))?;
    if !account
        .provider
        .as_deref()
        .is_some_and(|provider| provider.eq_ignore_ascii_case("investec"))
    {
        return Err(ApiError::BadRequest(
            "Only Investec card activities support manual lifecycle links.".to_string(),
        ));
    }
    let (pending_source, pending_transaction_id) =
        provider_activity_identity(&state.deployment_id, &pending)?;
    let (posted_source, posted_transaction_id) =
        provider_activity_identity(&state.deployment_id, &posted)?;
    if pending_source != posted_source || !pending_transaction_id.starts_with("investec-card:") {
        return Err(ApiError::BadRequest(
            "The activities are not a compatible Investec card lifecycle pair.".to_string(),
        ));
    }
    let body = FinanceSyncCardLifecycleLinkRequest {
        source_resource_incarnation_id: &pending_source,
        pending_transaction_id: &pending_transaction_id,
        posted_transaction_id: &posted_transaction_id,
        reason,
    };
    let response: InvestecCardLifecycleLinkResponse = FinanceSyncControl::from_env()?
        .post(
            "/control/v1/investec/card-lifecycle-links",
            subject.as_str(),
            Some(&body),
        )
        .await?;
    validate_lifecycle_link_response(&response)?;
    state.health_service.clear_cache().await;
    Ok(Json(response))
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

async fn start_plaid_update(
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
    body: Bytes,
) -> ApiResult<Json<ProviderConnectionStartResponse>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    // The body is optional: without one the request stays unqualified, which
    // Finance Sync accepts only for an install with a single Plaid Item.
    let invalid = || ApiError::BadRequest("Invalid Plaid update request.".to_string());
    let connection_id = if body.is_empty() {
        None
    } else {
        if body.len() > MAX_LINK_VALUE_BYTES {
            return Err(invalid());
        }
        // serde would also read a positional array into the struct; only an object is a selection.
        let request: serde_json::Value = serde_json::from_slice(&body).map_err(|_| invalid())?;
        if !request.is_object() {
            return Err(invalid());
        }
        serde_json::from_value::<PlaidUpdateRequest>(request)
            .map_err(|_| invalid())?
            .connection_id
    };
    if connection_id
        .as_deref()
        .is_some_and(|connection_id| !valid_plaid_connection_id(connection_id))
    {
        return Err(invalid());
    }
    let subject = subject(auth_subject)?;
    let control = FinanceSyncControl::from_env()?;
    let suffix = "/control/v1/provider-connections/plaid/update";
    let response: ProviderConnectionStartResponse = match connection_id.as_deref() {
        Some(connection_id) => {
            let response = control
                .send_post(
                    suffix,
                    subject.as_str(),
                    Some(&FinanceSyncPlaidUpdateRequest { connection_id }),
                )
                .await?;
            // The selected Item is no longer connected for this subject.
            if response.status() == StatusCode::NOT_FOUND {
                return Err(ApiError::NotFound);
            }
            decode_response(response).await?
        }
        None => {
            control
                .post::<_, ()>(suffix, subject.as_str(), None)
                .await?
        }
    };
    if response.provider != ProviderKind::Plaid {
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

async fn complete_plaid_update(
    State(state): State<Arc<AppState>>,
    auth_subject: Option<Extension<AuthenticatedSubject>>,
    Json(request): Json<PlaidUpdateCompleteRequest>,
) -> ApiResult<Json<ProviderConnectionCompleteResponse>> {
    if state.auth.is_none() && state.oidc.is_none() {
        return Err(ApiError::Unauthorized(
            "Authentication is required.".to_string(),
        ));
    }
    let continuation = request.continuation_token.trim();
    if continuation.len() < 32
        || continuation.len() > MAX_LINK_VALUE_BYTES
        || continuation.chars().any(char::is_control)
    {
        return Err(ApiError::BadRequest(
            "Invalid Plaid update completion request.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let body = FinanceSyncPlaidUpdateCompleteRequest {
        continuation_token: continuation,
    };
    let response: ProviderConnectionCompleteResponse = FinanceSyncControl::from_env()?
        .post(
            "/control/v1/provider-connections/plaid/update/complete",
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

fn provider_relink_handoff(
    db_path: &str,
    account_id: &str,
    state: &str,
    provider_generation: Option<i64>,
) -> ApiResult<Option<ProviderRelinkHandoff>> {
    use rusqlite::OptionalExtension;
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    connection
        .query_row(
            "SELECT provider_generation,prior_provider_generation,receiver_access_generation,database_state_digest,state='acknowledged' FROM provider_account_relink_handoffs WHERE account_id=?1 AND state=?2 AND (?3 IS NULL OR provider_generation=?3) ORDER BY provider_generation DESC LIMIT 1",
            rusqlite::params![account_id, state, provider_generation],
            |row| {
                Ok(ProviderRelinkHandoff {
                    provider_generation: row.get(0)?,
                    prior_provider_generation: row.get(1)?,
                    receiver_access_generation: row.get(2)?,
                    database_state_digest: row.get(3)?,
                    acknowledged: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| ApiError::Internal(error.to_string()))
}

fn record_provider_relink_handoff(
    db_path: &str,
    account_id: &str,
    handoff: &ProviderRelinkHandoff,
) -> ApiResult<()> {
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    connection
        .execute(
            "INSERT OR IGNORE INTO provider_account_relink_handoffs(account_id,provider_generation,prior_provider_generation,receiver_access_generation,database_state_digest) SELECT ?1,?2,?3,?4,?5 WHERE EXISTS(SELECT 1 FROM provider_account_connections WHERE account_id=?1 AND provider_generation=?3)",
            rusqlite::params![account_id, handoff.provider_generation, handoff.prior_provider_generation, handoff.receiver_access_generation, handoff.database_state_digest],
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let stored =
        provider_relink_handoff(db_path, account_id, "pending", None)?.ok_or_else(|| {
            ApiError::BadRequest("Provider relink generation changed during admission.".to_string())
        })?;
    if stored.provider_generation != handoff.provider_generation
        || stored.prior_provider_generation != handoff.prior_provider_generation
        || stored.receiver_access_generation != handoff.receiver_access_generation
        || stored.database_state_digest != handoff.database_state_digest
    {
        return Err(ApiError::BadRequest(
            "Provider relink generation changed during admission.".to_string(),
        ));
    }
    Ok(())
}

fn acknowledge_provider_relink_handoff(
    db_path: &str,
    account_id: &str,
    handoff: &ProviderRelinkHandoff,
) -> ApiResult<()> {
    let mut connection = rusqlite::Connection::open(db_path)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let generation: i64 = transaction
        .query_row(
            "SELECT provider_generation FROM provider_account_connections WHERE account_id=?1",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if generation == handoff.prior_provider_generation {
        transaction
            .execute(
                "UPDATE provider_account_connections SET provider_generation=?1 WHERE account_id=?2 AND provider_generation=?3",
                rusqlite::params![handoff.provider_generation, account_id, handoff.prior_provider_generation],
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?;
    } else if generation != handoff.provider_generation {
        return Err(ApiError::BadRequest(
            "Provider relink generation changed during admission.".to_string(),
        ));
    }
    let updated = transaction
        .execute(
            "UPDATE provider_account_relink_handoffs SET state='acknowledged',acknowledged_at=CURRENT_TIMESTAMP WHERE account_id=?1 AND provider_generation=?2 AND prior_provider_generation=?3 AND receiver_access_generation=?4 AND database_state_digest=?5 AND state='pending'",
            rusqlite::params![account_id, handoff.provider_generation, handoff.prior_provider_generation, handoff.receiver_access_generation, handoff.database_state_digest],
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if updated != 1 && !handoff.acknowledged {
        return Err(ApiError::BadRequest(
            "Provider relink generation changed during admission.".to_string(),
        ));
    }
    transaction
        .commit()
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(())
}

fn admission_matches_handoff(
    admitted: &FinanceSyncAdmissionStatus,
    account_id: &str,
    handoff: &ProviderRelinkHandoff,
) -> bool {
    admitted.wealthfolio_account_id == account_id
        && admitted.provider_generation == handoff.provider_generation
        && admitted.receiver_access_generation == handoff.receiver_access_generation
        && admitted.baseline_status == "verified"
        && admitted.baseline_digest.as_deref() == Some(handoff.database_state_digest.as_str())
        && admitted.asset_mapping_digest == empty_asset_mapping_digest()
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
    let mut relink_handoff = match mapped_account_id.as_ref() {
        Some((account_id, _)) => {
            provider_relink_handoff(&state.db_path, account_id, "pending", None)?
        }
        None => None,
    };
    let mut provider_generation = relink_handoff
        .as_ref()
        .map(|handoff| handoff.provider_generation)
        .or(prior_provider_generation);
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
        if relink_handoff.is_none() {
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
                if let Some(prior) = prior_provider_generation {
                    if source.provider_generation < prior {
                        return Err(ApiError::BadRequest(
                            "Provider relink generation is stale.".to_string(),
                        ));
                    }
                    if source.provider_generation == prior {
                        relink_handoff = provider_relink_handoff(
                            &state.db_path,
                            &account.id,
                            "acknowledged",
                            Some(prior),
                        )?;
                        if relink_handoff.is_none() {
                            return Err(ApiError::BadRequest(
                                "Provider relink generation is not monotonic.".to_string(),
                            ));
                        }
                    }
                }
                provider_generation = Some(source.provider_generation);
                if provider == ProviderKind::SnapTrade {
                    snaptrade_asset_count = Some(source.assets.len());
                }
            } else if let Some(prior) = prior_provider_generation {
                relink_handoff = provider_relink_handoff(
                    &state.db_path,
                    &account.id,
                    "acknowledged",
                    Some(prior),
                )?;
                if relink_handoff.is_none() {
                    if !account.is_active {
                        reconcile_committed = true;
                    } else {
                        return Err(ApiError::BadRequest(
                            "Provider account is no longer available.".to_string(),
                        ));
                    }
                }
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
            account_type: native_account_type(provider, &source.account_type).to_string(),
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
    let resumed_relink = relink_handoff.is_some();
    let (database_state_digest, receiver_access_generation, baseline_kind, relink_prior_generation) =
        if let Some(handoff) = relink_handoff.as_ref() {
            (
                handoff.database_state_digest.clone(),
                handoff.receiver_access_generation,
                "relink",
                Some(handoff.prior_provider_generation),
            )
        } else {
            match prior_provider_generation {
                Some(prior) if prior < provider_generation => {
                    let (digest, receiver_generation) = provider_relink_state_attestation(
                        &state.db_path,
                        subject.as_str(),
                        provider,
                        &request.connection_id,
                        &request.provider_account_id,
                        &account.id,
                        &state.deployment_id,
                        prior,
                        provider_generation,
                    )?;
                    relink_handoff = Some(ProviderRelinkHandoff {
                        provider_generation,
                        prior_provider_generation: prior,
                        receiver_access_generation: receiver_generation,
                        database_state_digest: digest.clone(),
                        acknowledged: false,
                    });
                    (digest, receiver_generation, "relink", Some(prior))
                }
                _ => {
                    let (digest, receiver_generation) = empty_account_state_attestation(
                        &state.db_path,
                        subject.as_str(),
                        provider,
                        &request.connection_id,
                        &request.provider_account_id,
                        &account.id,
                        &state.deployment_id,
                    )?;
                    (digest, receiver_generation, "empty", None)
                }
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
    if !resumed_relink {
        if let Some(handoff) = relink_handoff.as_ref() {
            record_provider_relink_handoff(&state.db_path, &account.id, handoff)?;
        }
    }
    let mut remotely_acknowledged = false;
    if reconcile_committed || resumed_relink {
        if let Some(committed) = FinanceSyncControl::from_env()?
            .admission_status(
                provider,
                subject.as_str(),
                &request.connection_id,
                &request.provider_account_id,
            )
            .await?
        {
            let matches = if let Some(handoff) = relink_handoff.as_ref() {
                admission_matches_handoff(&committed, &account.id, handoff)
            } else {
                committed.wealthfolio_account_id == account.id
                    && committed.provider_generation == provider_generation
                    && committed.receiver_access_generation == receiver_access_generation
                    && committed.baseline_status == "verified"
                    && committed.baseline_digest.as_deref() == Some(database_state_digest.as_str())
                    && committed.asset_mapping_digest == empty_asset_mapping_digest()
            };
            if !matches {
                return Err(ApiError::BadRequest(
                    "Committed provider admission conflicts with the pending account.".to_string(),
                ));
            }
            remotely_acknowledged = true;
        } else if reconcile_committed
            || relink_handoff
                .as_ref()
                .is_some_and(|handoff| handoff.acknowledged)
        {
            return Err(ApiError::BadRequest(
                "Provider account is no longer available.".to_string(),
            ));
        } else {
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
            if source.is_none_or(|source| {
                source.provider_generation != provider_generation || !source.assets.is_empty()
            }) {
                return Err(ApiError::BadRequest(
                    "Provider relink generation changed during admission.".to_string(),
                ));
            }
        }
    }
    if !remotely_acknowledged {
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
        let matches = if let Some(handoff) = relink_handoff.as_ref() {
            admission_matches_handoff(&admitted, &account.id, handoff)
        } else {
            admitted.wealthfolio_account_id == account.id
                && admitted.provider_generation == provider_generation
                && admitted.receiver_access_generation == receiver_access_generation
                && admitted.baseline_status == "verified"
                && admitted.baseline_digest.as_deref() == Some(database_state_digest.as_str())
                && admitted.asset_mapping_digest == empty_asset_mapping_digest()
        };
        if !matches {
            return Err(service_unavailable());
        }
    }
    if let Some(handoff) = relink_handoff.as_ref() {
        acknowledge_provider_relink_handoff(&state.db_path, &account.id, handoff)?;
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
    let router = Router::new().route("/provider-connections", get(list_provider_connections));
    let router = if crate::features::investec_card_lifecycle_link_enabled() {
        router.route(
            "/provider-connections/investec/card-lifecycle-links",
            post(link_investec_card_lifecycle),
        )
    } else {
        router
    };
    router
        .route(
            "/provider-connections/{provider}/start",
            post(start_provider_connection),
        )
        .route(
            "/provider-connections/plaid/complete",
            post(complete_plaid_connection),
        )
        .route(
            "/provider-connections/plaid/update",
            post(start_plaid_update),
        )
        .route(
            "/provider-connections/plaid/update/complete",
            post(complete_plaid_update),
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
    fn provider_accounts_use_native_account_types() {
        assert_eq!(
            native_account_type(ProviderKind::Investec, "Private Bank Account"),
            account_types::CASH
        );
        assert_eq!(
            native_account_type(ProviderKind::Investec, "PrimeSaver"),
            account_types::CASH
        );
        assert_eq!(
            native_account_type(ProviderKind::Mercury, "mercury"),
            account_types::CASH
        );
        assert_eq!(
            native_account_type(ProviderKind::Plaid, "savings"),
            account_types::CASH
        );
        assert_eq!(
            native_account_type(ProviderKind::Plaid, "credit card"),
            account_types::CREDIT_CARD
        );
        assert_eq!(
            native_account_type(ProviderKind::Plaid, "loan"),
            account_types::CREDIT_CARD
        );
        assert_eq!(
            native_account_type(ProviderKind::Plaid, "brokerage"),
            account_types::SECURITIES
        );
        assert_eq!(
            native_account_type(ProviderKind::SnapTrade, "unknown"),
            account_types::SECURITIES
        );
    }

    #[test]
    fn list_rejects_duplicate_providers() {
        let statuses = vec![
            ProviderConnectionStatus {
                provider: ProviderKind::Plaid,
                status: ProviderStatus::Configured,
                message: "configured".to_string(),
                connections: Vec::new(),
            },
            ProviderConnectionStatus {
                provider: ProviderKind::Plaid,
                status: ProviderStatus::Configured,
                message: "configured".to_string(),
                connections: Vec::new(),
            },
        ];
        assert!(validate_list(&statuses).is_err());
    }

    fn status_with_connections(
        provider: ProviderKind,
        connections: &str,
    ) -> ProviderConnectionStatus {
        serde_json::from_str(&format!(
            r#"{{"provider":"{provider}","status":"configured","message":"configured","connections":{connections}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn list_accepts_only_bounded_plaid_connection_summaries() {
        let valid =
            r#"[{"connectionId":"item-1","label":"Checking •••• 1234","needsReauth":true}]"#;
        assert!(validate_list(&[status_with_connections(ProviderKind::Plaid, valid)]).is_ok());
        assert!(validate_list(&[status_with_connections(ProviderKind::Mercury, valid)]).is_err());
        let with_reason = r#"[{"connectionId":"item-1","label":"Checking","needsReauth":true,"reason":"ACCOUNT_NOT_RETURNED"}]"#;
        assert!(
            validate_list(&[status_with_connections(ProviderKind::Plaid, with_reason)]).is_ok()
        );

        for invalid in [
            r#"[{"connectionId":"","label":"Checking","needsReauth":false}]"#.to_string(),
            r#"[{"connectionId":"item\u0000","label":"Checking","needsReauth":false}]"#.to_string(),
            r#"[{"connectionId":"item-1","label":"","needsReauth":false}]"#.to_string(),
            r#"[{"connectionId":"item-1","label":"line\nbreak","needsReauth":false}]"#.to_string(),
            format!(
                r#"[{{"connectionId":"{}","label":"Checking","needsReauth":false}}]"#,
                "i".repeat(MAX_PLAID_CONNECTION_ID_BYTES + 1)
            ),
            format!(
                r#"[{{"connectionId":"item-1","label":"{}","needsReauth":false}}]"#,
                "l".repeat(MAX_CONNECTION_LABEL_BYTES + 1)
            ),
            r#"[{"connectionId":"item-1","label":"A","needsReauth":false},{"connectionId":"item-1","label":"B","needsReauth":false}]"#.to_string(),
            r#"[{"connectionId":"item-1","label":"A","needsReauth":true,"reason":""}]"#.to_string(),
            r#"[{"connectionId":"item-1","label":"A","needsReauth":true,"reason":"the login at Example Bank expired"}]"#.to_string(),
            r#"[{"connectionId":"item-1","label":"A","needsReauth":true,"reason":"item_login_required"}]"#.to_string(),
            format!(
                r#"[{{"connectionId":"item-1","label":"A","needsReauth":true,"reason":"{}"}}]"#,
                "R".repeat(MAX_CONNECTION_REASON_BYTES + 1)
            ),
        ] {
            assert!(
                validate_list(&[status_with_connections(ProviderKind::Plaid, &invalid)]).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn strict_list_schema_rejects_durable_or_unknown_connection_fields() {
        for json in [
            r#"{"provider":"plaid","status":"configured","message":"ok","connections":[{"connectionId":"item-1","label":"Checking","needsReauth":false,"accessToken":"durable"}]}"#,
            r#"{"provider":"plaid","status":"configured","message":"ok","connections":[{"connectionId":"item-1","label":"Checking"}]}"#,
            r#"{"provider":"plaid","status":"configured","message":"ok","accessToken":"durable"}"#,
        ] {
            assert!(serde_json::from_str::<ProviderConnectionStatus>(json).is_err());
        }
    }
}
