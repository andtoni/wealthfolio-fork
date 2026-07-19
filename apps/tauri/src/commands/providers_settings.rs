use tauri::State;
use wealthfolio_core::quotes::service::ProviderInfo;

use crate::context::ServiceContext;
use std::sync::Arc;

use super::error::CommandResult;

#[tauri::command]
pub async fn get_market_data_providers_settings(
    context: State<'_, Arc<ServiceContext>>,
) -> CommandResult<Vec<ProviderInfo>> {
    Ok(context.quote_service.get_providers_info().await?)
}

#[tauri::command]
pub async fn update_market_data_provider_settings(
    context: State<'_, Arc<ServiceContext>>,
    provider_id: String,
    priority: i32,
    enabled: bool,
) -> CommandResult<()> {
    context
        .quote_service
        .update_provider_settings(&provider_id, priority, enabled)
        .await?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Provider Connections — contract-safe local status (same as server HTTP endpoint)
// ──────────────────────────────────────────────────────────────────────────────

/// Canonical provider identifiers (must match server-side ProviderKind enum).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionStatus {
    pub provider: String,
    pub status: String,
    pub message: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveredProviderAsset {
    pub provider_asset_id: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub instrument_type: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
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

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderAssetMapping {
    pub provider_asset_id: String,
    pub wealthfolio_asset_id: String,
}

const STATUS_NOT_CONFIGURED: &str =
    "Direct provider linking is not yet available. Connect manually via CSV import or add an account.";

/// Returns typed provider connection statuses. Every entry reports
/// `not_configured` — no real network calls, credentials, or secrets.
#[tauri::command]
pub async fn list_provider_connections() -> Result<Vec<ProviderConnectionStatus>, String> {
    Ok(vec![
        ProviderConnectionStatus {
            provider: "investec".to_string(),
            status: "not_configured".to_string(),
            message: STATUS_NOT_CONFIGURED.to_string(),
        },
        ProviderConnectionStatus {
            provider: "mercury".to_string(),
            status: "not_configured".to_string(),
            message: STATUS_NOT_CONFIGURED.to_string(),
        },
        ProviderConnectionStatus {
            provider: "plaid".to_string(),
            status: "not_configured".to_string(),
            message: STATUS_NOT_CONFIGURED.to_string(),
        },
        ProviderConnectionStatus {
            provider: "snaptrade".to_string(),
            status: "not_configured".to_string(),
            message: STATUS_NOT_CONFIGURED.to_string(),
        },
    ])
}

#[tauri::command]
pub async fn start_provider_connection(provider: String) -> Result<(), String> {
    Err(format!(
        "Direct {provider} linking is not available in this runtime. Use the authenticated Wealthfolio web application."
    ))
}

/// Provider account discovery is web-only; desktop keeps the shared adapter
/// contract registered while exposing no accounts.
#[tauri::command]
pub async fn list_discovered_provider_accounts(
    _provider: String,
) -> Result<Vec<serde_json::Value>, String> {
    Ok(Vec::new())
}

/// Provider account admission requires the authenticated web runtime.
#[tauri::command]
pub async fn admit_provider_account(
    provider: String,
    _connection_id: String,
    _provider_account_id: String,
    _name: String,
) -> Result<(), String> {
    Err(format!(
        "Direct {provider} account admission is not available in this runtime. Use the authenticated Wealthfolio web application."
    ))
}

/// Tauri has no server-authenticated subject to bind to Finance Sync, so Plaid
/// completion must fail closed without inspecting, logging, or persisting the token.
#[tauri::command]
pub async fn complete_plaid_connection(
    _public_token: String,
    _continuation_token: String,
) -> Result<(), String> {
    Err(
        "Plaid completion is not available in this runtime. Use the authenticated Wealthfolio web application."
            .to_string(),
    )
}
