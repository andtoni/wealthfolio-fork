//! SQLite sidecar repository for aggregated categorization evidence.

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

use crate::db::{get_connection, DbPool, WriteHandle};
use crate::errors::StorageError;
use crate::schema::{spending_categorization_evidence, taxonomy_categories};
use wealthfolio_core::errors::Error;
use wealthfolio_spending::categorization_evidence::{
    CategorizationEvidence, CategorizationEvidenceImport, CategorizationEvidenceImportResult,
    CategorizationEvidenceRepositoryTrait, NewCategorizationEvidence, AGREEMENT, CONFLICT,
};

#[derive(Queryable, Identifiable, Selectable, Insertable, Debug, Clone, Serialize, Deserialize)]
#[diesel(table_name = spending_categorization_evidence)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
struct CategorizationEvidenceDB {
    id: String,
    identity_key: String,
    merchant_key: String,
    merchant_key_version: i32,
    normalization_version: i32,
    taxonomy_id: String,
    category_id: String,
    direction: String,
    activity_type: String,
    provider_context: Option<String>,
    account_context: Option<String>,
    label_count: i32,
    agreement_state: String,
    first_label_date: String,
    last_label_date: String,
    provenance: String,
    import_run_id: String,
    created_at: String,
}

impl From<CategorizationEvidenceDB> for CategorizationEvidence {
    fn from(row: CategorizationEvidenceDB) -> Self {
        Self {
            id: row.id,
            import_run_id: row.import_run_id,
            evidence: NewCategorizationEvidence {
                merchant_key: row.merchant_key,
                merchant_key_version: row.merchant_key_version,
                normalization_version: row.normalization_version,
                taxonomy_id: row.taxonomy_id,
                category_id: row.category_id,
                direction: row.direction,
                activity_type: row.activity_type,
                provider_context: row.provider_context,
                account_context: row.account_context,
                label_count: row.label_count,
                agreement_state: row.agreement_state,
                first_label_date: row.first_label_date,
                last_label_date: row.last_label_date,
                provenance: row.provenance,
            },
            created_at: row.created_at,
        }
    }
}

pub struct CategorizationEvidenceRepository {
    pool: Arc<DbPool>,
    writer: WriteHandle,
}

impl CategorizationEvidenceRepository {
    pub fn new(pool: Arc<DbPool>, writer: WriteHandle) -> Self {
        Self { pool, writer }
    }
}

fn repository_error(message: impl Into<String>) -> Error {
    Error::Repository(message.into())
}

fn identity_key(record: &NewCategorizationEvidence) -> String {
    let merchant_key_version = record.merchant_key_version.to_string();
    let normalization_version = record.normalization_version.to_string();
    let fields = [
        record.merchant_key.as_str(),
        merchant_key_version.as_str(),
        normalization_version.as_str(),
        record.direction.as_str(),
        record.activity_type.as_str(),
        record.provider_context.as_deref().unwrap_or(""),
        record.account_context.as_deref().unwrap_or(""),
    ];
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update(field.len().to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn same_payload(row: &CategorizationEvidenceDB, record: &NewCategorizationEvidence) -> bool {
    row.merchant_key == record.merchant_key
        && row.merchant_key_version == record.merchant_key_version
        && row.normalization_version == record.normalization_version
        && row.taxonomy_id == record.taxonomy_id
        && row.category_id == record.category_id
        && row.direction == record.direction
        && row.activity_type == record.activity_type
        && row.provider_context == record.provider_context
        && row.account_context == record.account_context
        && row.label_count == record.label_count
        && row.agreement_state == record.agreement_state
        && row.first_label_date == record.first_label_date
        && row.last_label_date == record.last_label_date
        && row.provenance == record.provenance
}

fn validate_record(record: &NewCategorizationEvidence) -> std::result::Result<(), Error> {
    for (name, value) in [
        ("merchantKey", record.merchant_key.as_str()),
        ("taxonomyId", record.taxonomy_id.as_str()),
        ("categoryId", record.category_id.as_str()),
        ("direction", record.direction.as_str()),
        ("activityType", record.activity_type.as_str()),
        ("provenance", record.provenance.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(repository_error(format!(
                "categorization evidence {name} must not be empty"
            )));
        }
    }
    if record.merchant_key_version <= 0 || record.normalization_version <= 0 {
        return Err(repository_error(
            "categorization evidence versions must be positive",
        ));
    }
    if record.label_count <= 0 {
        return Err(repository_error(
            "categorization evidence labelCount must be positive",
        ));
    }
    if record.agreement_state != AGREEMENT && record.agreement_state != CONFLICT {
        return Err(repository_error(
            "categorization evidence agreementState must be agreement or conflict",
        ));
    }
    let first = NaiveDate::parse_from_str(&record.first_label_date, "%Y-%m-%d").map_err(|_| {
        repository_error("categorization evidence firstLabelDate must be YYYY-MM-DD")
    })?;
    let last = NaiveDate::parse_from_str(&record.last_label_date, "%Y-%m-%d").map_err(|_| {
        repository_error("categorization evidence lastLabelDate must be YYYY-MM-DD")
    })?;
    if first > last {
        return Err(repository_error(
            "categorization evidence firstLabelDate is after lastLabelDate",
        ));
    }
    for (name, value) in [
        ("providerContext", record.provider_context.as_deref()),
        ("accountContext", record.account_context.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(repository_error(format!(
                "categorization evidence {name} must be omitted or non-empty"
            )));
        }
    }
    Ok(())
}

#[async_trait]
impl CategorizationEvidenceRepositoryTrait for CategorizationEvidenceRepository {
    async fn import(
        &self,
        request: CategorizationEvidenceImport,
    ) -> Result<CategorizationEvidenceImportResult> {
        if request.import_run_id.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "categorization evidence importRunId must not be empty"
            ));
        }
        let result = self.writer.exec(move |conn| {
            let mut incoming: HashMap<String, NewCategorizationEvidence> = HashMap::new();
            let mut identical_in_request = 0usize;
            for record in &request.records {
                validate_record(record)?;
                let key = identity_key(record);
                match incoming.get(&key) {
                    Some(existing) if existing == record => identical_in_request += 1,
                    Some(_) => return Err(repository_error(format!(
                        "categorization evidence identity collision for merchantKey {}",
                        record.merchant_key
                    ))),
                    None => { incoming.insert(key, record.clone()); }
                }
            }

            let now = chrono::Utc::now().to_rfc3339();
            let mut rows = Vec::new();
            let mut skipped = identical_in_request;
            for (key, record) in incoming {
                let mapping_exists = taxonomy_categories::table
                    .filter(taxonomy_categories::taxonomy_id.eq(&record.taxonomy_id))
                    .filter(taxonomy_categories::id.eq(&record.category_id))
                    .select(taxonomy_categories::id)
                    .first::<String>(conn)
                    .optional()
                    .map_err(StorageError::from)?
                    .is_some();
                if !mapping_exists {
                    return Err(repository_error(format!(
                        "categorization evidence has no explicit current taxonomy mapping for {}/{}",
                        record.taxonomy_id, record.category_id
                    )));
                }

                if let Some(existing) = spending_categorization_evidence::table
                    .filter(spending_categorization_evidence::identity_key.eq(&key))
                    .first::<CategorizationEvidenceDB>(conn)
                    .optional()
                    .map_err(StorageError::from)?
                {
                    if same_payload(&existing, &record) {
                        skipped += 1;
                        continue;
                    }
                    return Err(repository_error(format!(
                        "categorization evidence identity collision for merchantKey {}",
                        record.merchant_key
                    )));
                }

                rows.push(CategorizationEvidenceDB {
                    id: format!("categorization-evidence-{key}"),
                    identity_key: key,
                    merchant_key: record.merchant_key,
                    merchant_key_version: record.merchant_key_version,
                    normalization_version: record.normalization_version,
                    taxonomy_id: record.taxonomy_id,
                    category_id: record.category_id,
                    direction: record.direction,
                    activity_type: record.activity_type,
                    provider_context: record.provider_context,
                    account_context: record.account_context,
                    label_count: record.label_count,
                    agreement_state: record.agreement_state,
                    first_label_date: record.first_label_date,
                    last_label_date: record.last_label_date,
                    provenance: record.provenance,
                    import_run_id: request.import_run_id.clone(),
                    created_at: now.clone(),
                });
            }
            let inserted = rows.len();
            if !request.dry_run && !rows.is_empty() {
                diesel::insert_into(spending_categorization_evidence::table)
                    .values(&rows)
                    .execute(conn)
                    .map_err(StorageError::from)?;
            }
            Ok(CategorizationEvidenceImportResult {
                dry_run: request.dry_run,
                validated: request.records.len(),
                inserted,
                skipped_identical: skipped,
            })
        }).await?;
        Ok(result)
    }

    async fn rollback_import(&self, import_run_id: &str) -> Result<usize> {
        if import_run_id.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "categorization evidence importRunId must not be empty"
            ));
        }
        let import_run_id = import_run_id.to_string();
        Ok(self
            .writer
            .exec(move |conn| {
                diesel::delete(
                    spending_categorization_evidence::table
                        .filter(spending_categorization_evidence::import_run_id.eq(import_run_id)),
                )
                .execute(conn)
                .map_err(StorageError::from)
                .map_err(Error::from)
            })
            .await?)
    }

    async fn list_for_merchant(
        &self,
        merchant_key: &str,
        merchant_key_version: i32,
        normalization_version: i32,
    ) -> Result<Vec<CategorizationEvidence>> {
        let mut conn = get_connection(&self.pool).map_err(|error| anyhow::anyhow!(error))?;
        let rows = spending_categorization_evidence::table
            .filter(spending_categorization_evidence::merchant_key.eq(merchant_key))
            .filter(spending_categorization_evidence::merchant_key_version.eq(merchant_key_version))
            .filter(
                spending_categorization_evidence::normalization_version.eq(normalization_version),
            )
            .order((
                spending_categorization_evidence::label_count.desc(),
                spending_categorization_evidence::id.asc(),
            ))
            .load::<CategorizationEvidenceDB>(&mut conn)
            .map_err(StorageError::from)
            .map_err(|error| anyhow::anyhow!(error))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
