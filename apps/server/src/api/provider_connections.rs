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
    pub redirect_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaidCompleteRequest {
    public_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinanceSyncPlaidCompleteRequest<'a> {
    public_token: &'a str,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnectionCompleteResponse {
    pub provider: ProviderKind,
    pub status: ProviderStatus,
    pub message: String,
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
            if response.redirect_url.is_some()
                || token.is_empty()
                || token.len() > MAX_LINK_VALUE_BYTES
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
                || url.scheme() != "https"
                || !url.has_host()
                || !url.username().is_empty()
                || url.password().is_some()
                || redirect.len() > MAX_LINK_VALUE_BYTES
            {
                return Err(service_unavailable());
            }
        }
        ProviderKind::Investec | ProviderKind::Mercury => {
            if response.link_token.is_some() || response.redirect_url.is_some() {
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
    if token.is_empty()
        || token.len() > MAX_PUBLIC_TOKEN_BYTES
        || token.chars().any(char::is_control)
    {
        return Err(ApiError::BadRequest(
            "Invalid Plaid completion request.".to_string(),
        ));
    }
    let subject = subject(auth_subject)?;
    let body = FinanceSyncPlaidCompleteRequest {
        public_token: token,
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
            redirect_url: Some("https://connect.snaptrade.com/redirect".to_string()),
        };
        assert!(validate_start(&valid).is_ok());

        let insecure = ProviderConnectionStartResponse {
            redirect_url: Some("http://connect.snaptrade.com/redirect".to_string()),
            ..valid
        };
        assert!(validate_start(&insecure).is_err());
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
