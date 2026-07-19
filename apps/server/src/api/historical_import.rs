//! Authenticated, importer-authority-gated Finance historical migration sink.
//! The storage repository validates then commits each batch or rollback atomically.

use axum::{extract::State, http::HeaderMap, Extension, Json};
use std::{path::PathBuf, sync::Arc};
use subtle::ConstantTimeEq;
use wealthfolio_storage_sqlite::historical_import::{
    HistoricalBatchResult, HistoricalImportBatch, HistoricalImportError,
};

use crate::{auth::AuthenticatedSubject, error::ApiError, main_lib::AppState};

const IMPORTER_AUTHORITY_HEADER: &str = "x-wealthfolio-importer-authority";
const MAX_AUTHORITY_BYTES: usize = 4096;

fn validate_importer_authority(headers: &HeaderMap) -> Result<(), ApiError> {
    let configured_path = std::env::var("WF_HISTORICAL_IMPORTER_TOKEN_FILE")
        .map(PathBuf::from)
        .map_err(|_| ApiError::Forbidden("historical importer authority is disabled".into()))?;
    let metadata = std::fs::symlink_metadata(&configured_path)
        .map_err(|_| ApiError::Forbidden("historical importer authority is unavailable".into()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() as usize > MAX_AUTHORITY_BYTES
    {
        return Err(ApiError::Forbidden(
            "historical importer authority is unavailable".into(),
        ));
    }
    let configured = std::fs::read(&configured_path)
        .map_err(|_| ApiError::Forbidden("historical importer authority is unavailable".into()))?;
    let configured = configured.strip_suffix(b"\n").unwrap_or(&configured);
    let presented = headers
        .get(IMPORTER_AUTHORITY_HEADER)
        .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "))
        .ok_or_else(|| {
            ApiError::Forbidden("dedicated historical importer authority is required".into())
        })?;
    if configured.is_empty()
        || configured.len() != presented.len()
        || configured.ct_eq(presented).unwrap_u8() != 1
    {
        return Err(ApiError::Forbidden(
            "dedicated historical importer authority is required".into(),
        ));
    }
    Ok(())
}

fn repository_error(error: HistoricalImportError) -> ApiError {
    match error {
        HistoricalImportError::AccountAccessDenied => ApiError::Forbidden(error.to_string()),
        HistoricalImportError::Storage(_) => ApiError::Internal(error.to_string()),
        HistoricalImportError::Invalid(_)
        | HistoricalImportError::RunConflict
        | HistoricalImportError::IdempotencyConflict
        | HistoricalImportError::IdentityConflict => ApiError::BadRequest(error.to_string()),
    }
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<AuthenticatedSubject>>,
    headers: HeaderMap,
    Json(request): Json<HistoricalImportBatch>,
) -> Result<Json<HistoricalBatchResult>, ApiError> {
    let Some(Extension(auth)) = auth else {
        return Err(ApiError::Unauthorized(
            "historical import subject authentication is required".into(),
        ));
    };
    if auth.as_str() != request.subject {
        return Err(ApiError::Forbidden(
            "authenticated subject cannot import for another subject".into(),
        ));
    }
    validate_importer_authority(&headers)?;
    let header_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::BadRequest("idempotency-key header is required".into()))?;
    if header_key != request.idempotency_key {
        return Err(ApiError::BadRequest(
            "idempotency-key header does not match canonical request".into(),
        ));
    }
    state
        .historical_import_repository
        .apply(request)
        .await
        .map(Json)
        .map_err(repository_error)
}
