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
        "Direct {provider} linking is not yet implemented. Authenticated subject is required (not available in this runtime)."
    ))
}
