//! Aggregated, local evidence derived from trustworthy historical manual labels.
//!
//! Evidence is advisory input to categorization. Import and rollback never mutate
//! activities, balances, manual assignments, or categorization overlays.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const AGREEMENT: &str = "agreement";
pub const CONFLICT: &str = "conflict";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCategorizationEvidence {
    pub merchant_key: String,
    pub merchant_key_version: i32,
    pub normalization_version: i32,
    /// Explicit mapping to a category in the current local taxonomy.
    pub taxonomy_id: String,
    pub category_id: String,
    pub direction: String,
    pub activity_type: String,
    pub provider_context: Option<String>,
    pub account_context: Option<String>,
    pub label_count: i32,
    /// One of [`AGREEMENT`] or [`CONFLICT`].
    pub agreement_state: String,
    pub first_label_date: String,
    pub last_label_date: String,
    pub provenance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorizationEvidence {
    pub id: String,
    pub import_run_id: String,
    #[serde(flatten)]
    pub evidence: NewCategorizationEvidence,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorizationEvidenceImport {
    pub import_run_id: String,
    #[serde(default)]
    pub dry_run: bool,
    pub records: Vec<NewCategorizationEvidence>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorizationEvidenceImportResult {
    pub dry_run: bool,
    pub validated: usize,
    /// Rows inserted, or rows that would be inserted for a dry run.
    pub inserted: usize,
    pub skipped_identical: usize,
}

#[async_trait]
pub trait CategorizationEvidenceRepositoryTrait: Send + Sync {
    async fn import(
        &self,
        request: CategorizationEvidenceImport,
    ) -> Result<CategorizationEvidenceImportResult>;
    async fn rollback_import(&self, import_run_id: &str) -> Result<usize>;
    async fn list_for_merchant(
        &self,
        merchant_key: &str,
        merchant_key_version: i32,
        normalization_version: i32,
    ) -> Result<Vec<CategorizationEvidence>>;
}

/// Narrow application seam for validated evidence imports and reads.
pub struct CategorizationEvidenceService {
    repository: Arc<dyn CategorizationEvidenceRepositoryTrait>,
}

impl CategorizationEvidenceService {
    pub fn new(repository: Arc<dyn CategorizationEvidenceRepositoryTrait>) -> Self {
        Self { repository }
    }

    pub async fn import(
        &self,
        request: CategorizationEvidenceImport,
    ) -> Result<CategorizationEvidenceImportResult> {
        self.repository.import(request).await
    }

    pub async fn rollback_import(&self, import_run_id: &str) -> Result<usize> {
        self.repository.rollback_import(import_run_id).await
    }

    pub async fn list_for_merchant(
        &self,
        merchant_key: &str,
        merchant_key_version: i32,
        normalization_version: i32,
    ) -> Result<Vec<CategorizationEvidence>> {
        self.repository
            .list_for_merchant(merchant_key, merchant_key_version, normalization_version)
            .await
    }
}
