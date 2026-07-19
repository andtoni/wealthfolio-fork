//! Transactional, subject-owned receiver for reviewed Finance historical imports.
//!
//! Core activity/evidence services expose separate async transactions, so this bounded
//! repository owns the only safe write path for this import. It validates the complete
//! batch first, then writes activities, provenance, evidence, ownership mappings, the
//! import run, and the durable receipt on the writer actor's single SQLite transaction.

use crate::{errors::StorageError, WriteHandle};
use chrono::{DateTime, NaiveDate, Utc};
use diesel::{
    connection::Connection,
    prelude::*,
    sql_query,
    sql_types::{BigInt, Integer, Nullable, Text},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalSource {
    Tarn,
    LunchMoney,
}
impl HistoricalSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tarn => "tarn",
            Self::LunchMoney => "lunch_money",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerDisposition {
    Selected,
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapReason {
    CrossSourceMatch,
    AtOrAfterCutover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionAction {
    ExcludeAll,
    KeepOne,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalOverlapResolution {
    pub candidate_id: String,
    pub reason: OverlapReason,
    pub candidate_ledger_identities: Vec<String>,
    pub action: ResolutionAction,
    pub selected_ledger_identity: Option<String>,
    pub excluded_ledger_identities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalManifestLedgerRow {
    pub identity: String,
    pub source: HistoricalSource,
    pub account_id: String,
    pub disposition: LedgerDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalReviewedManifest {
    pub ledger: Vec<HistoricalManifestLedgerRow>,
    /// Exact user-confirmed evidence set approved during review. Each digest is
    /// the canonical digest of the complete evidence row that may be staged.
    pub evidence: Vec<HistoricalManifestEvidenceRow>,
    pub overlap_resolutions: Vec<HistoricalOverlapResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalManifestEvidenceRow {
    pub identity: String,
    pub source: HistoricalSource,
    pub account_id: String,
    pub authority: EvidenceAuthority,
    pub payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalLedgerRow {
    pub idempotency_key: String,
    pub historical_identity: String,
    pub source: HistoricalSource,
    pub source_id: String,
    pub account_id: String,
    pub activity_type: String,
    pub effective_at: String,
    pub cutover_watermark: String,
    pub amount: String,
    pub currency: String,
    pub raw_description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAuthority {
    UserConfirmed,
    HistoricalHint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalEvidenceRow {
    pub idempotency_key: String,
    pub merchant_key: String,
    pub merchant_key_version: i32,
    pub normalization_version: i32,
    pub taxonomy_id: String,
    pub category_id: String,
    pub direction: String,
    pub activity_type: String,
    pub provider_context: Option<String>,
    pub account_id: String,
    pub source: HistoricalSource,
    pub source_category_ref: String,
    pub first_label_date: String,
    pub last_label_date: String,
    pub label_count: i32,
    pub agreement_state: String,
    pub authority: EvidenceAuthority,
    pub manifest_ledger_identity: String,
    pub ledger_disposition: LedgerDisposition,
    pub overlap_provenance: Vec<HistoricalOverlapResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum HistoricalOperation {
    StageLedger { rows: Vec<HistoricalLedgerRow> },
    StageLearningEvidence { rows: Vec<HistoricalEvidenceRow> },
    Finalize,
    Rollback,
}
impl HistoricalOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::StageLedger { .. } => "stage_ledger",
            Self::StageLearningEvidence { .. } => "stage_learning_evidence",
            Self::Finalize => "finalize",
            Self::Rollback => "rollback",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalImportBatch {
    pub schema_version: u8,
    pub subject: String,
    pub import_run_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub dry_run: bool,
    pub overlaps_resolved: bool,
    #[serde(default)]
    pub manifest_hash: Option<String>,
    #[serde(default)]
    pub manifest: Option<HistoricalReviewedManifest>,
    #[serde(flatten)]
    pub operation: HistoricalOperation,
}

impl<'de> Deserialize<'de> for HistoricalImportBatch {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Tag {
            operation: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct RowsWire<T> {
            schema_version: u8,
            subject: String,
            import_run_id: String,
            idempotency_key: String,
            #[serde(default)]
            dry_run: bool,
            overlaps_resolved: bool,
            #[serde(default)]
            manifest_hash: Option<String>,
            #[serde(default)]
            manifest: Option<HistoricalReviewedManifest>,
            #[serde(rename = "operation")]
            _operation: String,
            rows: Vec<T>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct NoRowsWire {
            schema_version: u8,
            subject: String,
            import_run_id: String,
            idempotency_key: String,
            #[serde(default)]
            dry_run: bool,
            overlaps_resolved: bool,
            #[serde(default)]
            manifest_hash: Option<String>,
            #[serde(default)]
            manifest: Option<HistoricalReviewedManifest>,
            operation: String,
        }
        fn from_rows<T>(
            wire: RowsWire<T>,
            operation: HistoricalOperation,
        ) -> HistoricalImportBatch {
            HistoricalImportBatch {
                schema_version: wire.schema_version,
                subject: wire.subject,
                import_run_id: wire.import_run_id,
                idempotency_key: wire.idempotency_key,
                dry_run: wire.dry_run,
                overlaps_resolved: wire.overlaps_resolved,
                manifest_hash: wire.manifest_hash,
                manifest: wire.manifest,
                operation,
            }
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        let tag: Tag = serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?;
        match tag.operation.as_str() {
            "stage_ledger" => {
                let wire: RowsWire<HistoricalLedgerRow> =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                let rows = wire.rows.clone();
                Ok(from_rows(wire, HistoricalOperation::StageLedger { rows }))
            }
            "stage_learning_evidence" => {
                let wire: RowsWire<HistoricalEvidenceRow> =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                let rows = wire.rows.clone();
                Ok(from_rows(
                    wire,
                    HistoricalOperation::StageLearningEvidence { rows },
                ))
            }
            "finalize" | "rollback" => {
                let wire: NoRowsWire =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                let operation = if wire.operation == "finalize" {
                    HistoricalOperation::Finalize
                } else if wire.operation == "rollback" {
                    HistoricalOperation::Rollback
                } else {
                    return Err(serde::de::Error::custom("operation mismatch"));
                };
                Ok(HistoricalImportBatch {
                    schema_version: wire.schema_version,
                    subject: wire.subject,
                    import_run_id: wire.import_run_id,
                    idempotency_key: wire.idempotency_key,
                    dry_run: wire.dry_run,
                    overlaps_resolved: wire.overlaps_resolved,
                    manifest_hash: wire.manifest_hash,
                    manifest: wire.manifest,
                    operation,
                })
            }
            _ => Err(serde::de::Error::custom("unsupported historical operation")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalBatchResult {
    pub dry_run: bool,
    pub import_run_id: String,
    pub validated: usize,
    pub written: usize,
    pub already_present: usize,
    pub ledger_deleted: usize,
    pub learning_evidence_deleted: usize,
    pub checksum: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoricalImportError {
    #[error("invalid historical import: {0}")]
    Invalid(String),
    #[error("historical import subject does not own every manifest account")]
    AccountAccessDenied,
    #[error("importRunId is owned by another subject or has different manifest bytes")]
    RunConflict,
    #[error("idempotency key was reused with a changed canonical payload")]
    IdempotencyConflict,
    #[error("historical identity collides with different durable data")]
    IdentityConflict,
    #[error("historical import storage: {0}")]
    Storage(String),
}
type Result<T> = std::result::Result<T, HistoricalImportError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalImportFault {
    None,
    AfterFirstWrite,
    AfterFirstRollbackDelete,
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).expect("string"),
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("key"),
                        canonical_json(&values[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}
fn canonical_digest<T: Serialize>(value: &T) -> String {
    let value = serde_json::to_value(value).expect("serializable historical contract");
    format!("{:x}", Sha256::digest(canonical_json(&value).as_bytes()))
}
pub fn historical_manifest_hash(manifest: &HistoricalReviewedManifest) -> String {
    canonical_digest(manifest)
}
pub fn historical_batch_payload_digest(batch: &HistoricalImportBatch) -> String {
    canonical_digest(batch)
}
pub fn historical_evidence_payload_digest(row: &HistoricalEvidenceRow) -> String {
    canonical_digest(row)
}
fn digest_parts(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_string());
        hasher.update(":");
        hasher.update(part);
    }
    format!("{:x}", hasher.finalize())
}
fn checksum<'a>(keys: impl IntoIterator<Item = &'a str>) -> String {
    let mut keys = keys.into_iter().map(str::to_string).collect::<Vec<_>>();
    keys.sort();
    digest_parts(&keys)
}
fn invalid(message: impl Into<String>) -> HistoricalImportError {
    HistoricalImportError::Invalid(message.into())
}

fn validate_manifest(
    manifest: &HistoricalReviewedManifest,
) -> Result<HashMap<String, HistoricalManifestLedgerRow>> {
    if manifest.ledger.is_empty()
        || manifest.ledger.len() > 100_000
        || manifest.evidence.len() > 100_000
        || manifest.overlap_resolutions.len() > 100_000
    {
        return Err(invalid("manifest size is invalid"));
    }
    let mut rows = HashMap::new();
    for row in &manifest.ledger {
        if row.identity.trim().is_empty()
            || row.account_id.trim().is_empty()
            || rows.insert(row.identity.clone(), row.clone()).is_some()
        {
            return Err(invalid(
                "manifest ledger identities and accounts must be unique and non-empty",
            ));
        }
    }
    let mut evidence_identities = HashSet::new();
    for row in &manifest.evidence {
        if row.identity.trim().is_empty()
            || row.account_id.trim().is_empty()
            || row.payload_digest.len() != 64
            || !row
                .payload_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || row.authority != EvidenceAuthority::UserConfirmed
            || !evidence_identities.insert(row.identity.clone())
        {
            return Err(invalid(
                "manifest evidence must be unique user-confirmed identities with canonical digests",
            ));
        }
    }
    let mut candidates = HashSet::new();
    let mut selected = HashSet::new();
    let mut excluded = HashSet::new();
    for resolution in &manifest.overlap_resolutions {
        if resolution.candidate_id.trim().is_empty()
            || !candidates.insert(resolution.candidate_id.clone())
        {
            return Err(invalid(
                "overlap candidate identities must be unique and non-empty",
            ));
        }
        let candidate = resolution
            .candidate_ledger_identities
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if candidate.is_empty()
            || candidate.len() != resolution.candidate_ledger_identities.len()
            || candidate.iter().any(|id| !rows.contains_key(id))
        {
            return Err(invalid(
                "overlap candidate ledger identities are incomplete",
            ));
        }
        let supplied_excluded = resolution
            .excluded_ledger_identities
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if supplied_excluded.len() != resolution.excluded_ledger_identities.len() {
            return Err(invalid("excluded overlap identities contain duplicates"));
        }
        match resolution.action {
            ResolutionAction::ExcludeAll => {
                if resolution.selected_ledger_identity.is_some() || supplied_excluded != candidate {
                    return Err(invalid("exclude_all must exclude the complete candidate"));
                }
                excluded.extend(candidate);
            }
            ResolutionAction::KeepOne => {
                let kept = resolution
                    .selected_ledger_identity
                    .as_ref()
                    .ok_or_else(|| invalid("keep_one requires selectedLedgerIdentity"))?;
                if !candidate.contains(kept) {
                    return Err(invalid("selected ledger identity is outside its candidate"));
                }
                let expected = candidate
                    .iter()
                    .filter(|id| *id != kept)
                    .cloned()
                    .collect::<HashSet<_>>();
                if supplied_excluded != expected {
                    return Err(invalid("keep_one excluded identities are incomplete"));
                }
                if excluded.contains(kept) {
                    return Err(invalid(
                        "overlap resolutions select and exclude the same ledger identity",
                    ));
                }
                selected.insert(kept.clone());
                excluded.extend(expected);
            }
        }
    }
    if selected.iter().any(|id| excluded.contains(id)) {
        return Err(invalid("overlap resolutions conflict"));
    }
    for row in rows.values() {
        let expected = if excluded.contains(&row.identity) {
            LedgerDisposition::Excluded
        } else {
            LedgerDisposition::Selected
        };
        if row.disposition != expected {
            return Err(invalid(format!(
                "manifest disposition is incomplete for {}",
                row.identity
            )));
        }
    }
    Ok(rows)
}

fn validate_batch(
    batch: &HistoricalImportBatch,
) -> Result<HashMap<String, HistoricalManifestLedgerRow>> {
    if batch.schema_version != 1
        || batch.subject.trim().is_empty()
        || batch.import_run_id.trim().is_empty()
        || batch.idempotency_key.trim().is_empty()
    {
        return Err(invalid(
            "schemaVersion, subject, importRunId, and idempotencyKey are required",
        ));
    }
    if !batch.overlaps_resolved {
        return Err(invalid("unresolved historical overlaps are refused"));
    }
    if matches!(batch.operation, HistoricalOperation::Rollback) {
        if batch.manifest.is_some() || batch.manifest_hash.is_some() {
            return Err(invalid("rollback must not replace the durable manifest"));
        }
        return Ok(HashMap::new());
    }
    let manifest = batch
        .manifest
        .as_ref()
        .ok_or_else(|| invalid("reviewed manifest is required"))?;
    let supplied = batch
        .manifest_hash
        .as_deref()
        .ok_or_else(|| invalid("manifestHash is required"))?;
    if supplied != historical_manifest_hash(manifest) {
        return Err(invalid(
            "manifestHash does not match canonical manifest bytes",
        ));
    }
    let manifest_rows = validate_manifest(manifest)?;
    let row_count = match &batch.operation {
        HistoricalOperation::StageLedger { rows } => rows.len(),
        HistoricalOperation::StageLearningEvidence { rows } => rows.len(),
        HistoricalOperation::Finalize | HistoricalOperation::Rollback => 0,
    };
    if row_count > 1_000 {
        return Err(invalid("batch exceeds 1000 rows"));
    }
    Ok(manifest_rows)
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    count: i64,
}
#[derive(QueryableByName)]
struct RunRow {
    #[diesel(sql_type=Text)]
    subject: String,
    #[diesel(sql_type=Text)]
    manifest_hash: String,
    #[diesel(sql_type=Text)]
    manifest_payload_digest: String,
    #[diesel(sql_type=Text)]
    status: String,
}
#[derive(QueryableByName)]
struct ReceiptRow {
    #[diesel(sql_type=Text)]
    payload_digest: String,
    #[diesel(sql_type=Text)]
    result_json: String,
}
#[derive(QueryableByName)]
struct ExistingActivity {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    activity_type: String,
    #[diesel(sql_type=Text)]
    activity_date: String,
    #[diesel(sql_type=Nullable<Text>)]
    amount: Option<String>,
    #[diesel(sql_type=Text)]
    currency: String,
    #[diesel(sql_type=Nullable<Text>)]
    notes: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    source_system: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    source_record_id: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    idempotency_key: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    import_run_id: Option<String>,
}
#[derive(QueryableByName)]
struct ExistingEvidence {
    #[diesel(sql_type=Text)]
    id: String,
    #[diesel(sql_type=Text)]
    merchant_key: String,
    #[diesel(sql_type=Integer)]
    merchant_key_version: i32,
    #[diesel(sql_type=Integer)]
    normalization_version: i32,
    #[diesel(sql_type=Text)]
    taxonomy_id: String,
    #[diesel(sql_type=Text)]
    category_id: String,
    #[diesel(sql_type=Text)]
    direction: String,
    #[diesel(sql_type=Text)]
    activity_type: String,
    #[diesel(sql_type=Nullable<Text>)]
    provider_context: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    account_context: Option<String>,
    #[diesel(sql_type=Integer)]
    label_count: i32,
    #[diesel(sql_type=Text)]
    agreement_state: String,
    #[diesel(sql_type=Text)]
    first_label_date: String,
    #[diesel(sql_type=Text)]
    last_label_date: String,
    #[diesel(sql_type=Text)]
    provenance: String,
    #[diesel(sql_type=Text)]
    import_run_id: String,
}

fn authorize_manifest_accounts(
    c: &mut SqliteConnection,
    subject: &str,
    manifest: &HistoricalReviewedManifest,
) -> Result<()> {
    for account in manifest
        .ledger
        .iter()
        .map(|row| row.account_id.as_str())
        .collect::<HashSet<_>>()
    {
        let count = sql_query("SELECT count(*) count FROM accounts account WHERE account.id=? AND EXISTS (SELECT 1 FROM account_access_epochs access WHERE access.account_id=account.id AND access.person_subject_id=? AND access.end_generation IS NULL)")
            .bind::<Text,_>(account).bind::<Text,_>(subject).get_result::<Count>(c).map_err(|e| HistoricalImportError::Storage(e.to_string()))?.count;
        if count != 1 {
            return Err(HistoricalImportError::AccountAccessDenied);
        }
    }
    Ok(())
}

fn canonical_label_date(value: &str) -> Result<String> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date.to_string());
    }
    DateTime::parse_from_rfc3339(value)
        .map(|instant| instant.date_naive().to_string())
        .map_err(|_| invalid("evidence label dates must be YYYY-MM-DD or RFC 3339"))
}

fn evidence_identity(row: &HistoricalEvidenceRow) -> String {
    let fields = [
        row.merchant_key.as_str(),
        &row.merchant_key_version.to_string(),
        &row.normalization_version.to_string(),
        row.direction.as_str(),
        row.activity_type.as_str(),
        row.provider_context.as_deref().unwrap_or(""),
        row.account_id.as_str(),
    ];
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update(field.len().to_be_bytes());
        hasher.update(field.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}
fn evidence_provenance(batch: &HistoricalImportBatch, row: &HistoricalEvidenceRow) -> String {
    canonical_json(
        &serde_json::json!({"contract":"finance-historical-evidence-v1","manifestHash":batch.manifest_hash,"manifestLedgerIdentity":row.manifest_ledger_identity,"ledgerDisposition":row.ledger_disposition,"overlapProvenance":row.overlap_provenance,"source":row.source,"sourceCategoryRef":row.source_category_ref,"idempotencyKey":row.idempotency_key}),
    )
}

fn validate_ledger_rows(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    manifest: &HashMap<String, HistoricalManifestLedgerRow>,
    rows: &[HistoricalLedgerRow],
) -> Result<Vec<bool>> {
    let mut keys = HashSet::new();
    let mut existing_flags = Vec::with_capacity(rows.len());
    for row in rows {
        let manifest_row = manifest
            .get(&row.historical_identity)
            .ok_or_else(|| invalid("ledger row is absent from reviewed manifest"))?;
        if manifest_row.disposition != LedgerDisposition::Selected
            || manifest_row.source != row.source
            || manifest_row.account_id != row.account_id
            || row.idempotency_key != row.historical_identity
            || !keys.insert(row.idempotency_key.clone())
            || row.activity_type.trim().is_empty()
            || row.source_id.trim().is_empty()
            || row.currency.len() != 3
            || row.currency != row.currency.to_uppercase()
        {
            return Err(invalid(
                "ledger row does not match its selected manifest identity",
            ));
        }
        let effective = DateTime::parse_from_rfc3339(&row.effective_at)
            .map_err(|_| invalid("invalid effectiveAt"))?
            .with_timezone(&Utc);
        let cutover = DateTime::parse_from_rfc3339(&row.cutover_watermark)
            .map_err(|_| invalid("invalid cutoverWatermark"))?
            .with_timezone(&Utc);
        if effective >= cutover || Decimal::from_str(&row.amount).is_err() {
            return Err(invalid(
                "ledger row is not valid pre-cutover canonical data",
            ));
        }
        let existing=sql_query("SELECT account_id,activity_type,activity_date,amount,currency,notes,source_system,source_record_id,idempotency_key,import_run_id FROM activities WHERE id=?").bind::<Text,_>(&row.historical_identity).get_result::<ExistingActivity>(c).optional().map_err(|e|HistoricalImportError::Storage(e.to_string()))?;
        if let Some(existing) = existing {
            if existing.account_id != row.account_id
                || existing.activity_type != row.activity_type
                || existing.activity_date != row.effective_at
                || existing.amount.as_deref() != Some(&row.amount)
                || existing.currency != row.currency
                || existing.notes.as_deref() != Some(&row.raw_description)
                || existing.source_system.as_deref() != Some(row.source.as_str())
                || existing.source_record_id.as_deref() != Some(&row.source_id)
                || existing.idempotency_key.as_deref() != Some(&row.idempotency_key)
                || existing.import_run_id.is_some()
            {
                return Err(HistoricalImportError::IdentityConflict);
            }
            let count=sql_query("SELECT count(*) count FROM provenance_records provenance JOIN historical_import_activities imported ON imported.activity_id=provenance.entity_id AND imported.import_run_id=provenance.import_run_id AND imported.subject=provenance.actor_id WHERE provenance.entity_type='activity' AND provenance.entity_id=? AND provenance.role='historical_context' AND provenance.import_run_id=? AND provenance.actor_id=?").bind::<Text,_>(&row.historical_identity).bind::<Text,_>(&batch.import_run_id).bind::<Text,_>(&batch.subject).get_result::<Count>(c).map_err(|e|HistoricalImportError::Storage(e.to_string()))?.count;
            if count != 1 {
                return Err(HistoricalImportError::IdentityConflict);
            }
            existing_flags.push(true);
        } else {
            existing_flags.push(false);
        }
    }
    Ok(existing_flags)
}

fn validate_evidence_rows(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    manifest: &HashMap<String, HistoricalManifestLedgerRow>,
    rows: &[HistoricalEvidenceRow],
) -> Result<Vec<(String, String, bool)>> {
    let full_manifest = batch.manifest.as_ref().expect("validated");
    let mut keys = HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let manifest_row = manifest
            .get(&row.manifest_ledger_identity)
            .ok_or_else(|| invalid("evidence manifest ledger identity is unknown"))?;
        let manifest_evidence = full_manifest
            .evidence
            .iter()
            .find(|item| item.identity == row.idempotency_key)
            .ok_or_else(|| invalid("evidence identity is absent from reviewed manifest"))?;
        let expected_provenance = full_manifest
            .overlap_resolutions
            .iter()
            .filter(|resolution| {
                resolution
                    .candidate_ledger_identities
                    .contains(&row.manifest_ledger_identity)
            })
            .cloned()
            .collect::<Vec<_>>();
        if row.source != HistoricalSource::Tarn
            || row.authority != EvidenceAuthority::UserConfirmed
            || manifest_evidence.source != row.source
            || manifest_evidence.account_id != row.account_id
            || manifest_evidence.authority != row.authority
            || manifest_evidence.payload_digest != historical_evidence_payload_digest(row)
            || manifest_row.source != row.source
            || row.ledger_disposition != manifest_row.disposition
            || row.account_id != manifest_row.account_id
            || row.overlap_provenance != expected_provenance
            || !keys.insert(row.idempotency_key.clone())
            || row.merchant_key.trim().is_empty()
            || row.taxonomy_id.trim().is_empty()
            || row.category_id.trim().is_empty()
            || row.source_category_ref.trim().is_empty()
            || row.merchant_key_version <= 0
            || row.normalization_version <= 0
            || row.label_count <= 0
            || !matches!(row.agreement_state.as_str(), "agreement" | "conflict")
        {
            return Err(invalid(
                "evidence is not reviewed-manifest-bound Tarn user-confirmed evidence",
            ));
        }
        let first = canonical_label_date(&row.first_label_date)?;
        let last = canonical_label_date(&row.last_label_date)?;
        if first > last {
            return Err(invalid("evidence date range is inverted"));
        }
        let category = sql_query(
            "SELECT count(*) count FROM taxonomy_categories WHERE taxonomy_id=? AND id=?",
        )
        .bind::<Text, _>(&row.taxonomy_id)
        .bind::<Text, _>(&row.category_id)
        .get_result::<Count>(c)
        .map_err(|e| HistoricalImportError::Storage(e.to_string()))?
        .count;
        if category != 1 {
            return Err(invalid("evidence taxonomy mapping does not exist"));
        }
        let identity = evidence_identity(row);
        let id = format!("categorization-evidence-{identity}");
        let provenance = evidence_provenance(batch, row);
        let existing=sql_query("SELECT id,merchant_key,merchant_key_version,normalization_version,taxonomy_id,category_id,direction,activity_type,provider_context,account_context,label_count,agreement_state,first_label_date,last_label_date,provenance,import_run_id FROM spending_categorization_evidence WHERE identity_key=?").bind::<Text,_>(&identity).get_result::<ExistingEvidence>(c).optional().map_err(|e|HistoricalImportError::Storage(e.to_string()))?;
        let identical = existing.as_ref().is_some_and(|e| {
            e.id == id
                && e.merchant_key == row.merchant_key
                && e.merchant_key_version == row.merchant_key_version
                && e.normalization_version == row.normalization_version
                && e.taxonomy_id == row.taxonomy_id
                && e.category_id == row.category_id
                && e.direction == row.direction
                && e.activity_type == row.activity_type
                && e.provider_context == row.provider_context
                && e.account_context == Some(row.account_id.clone())
                && e.label_count == row.label_count
                && e.agreement_state == row.agreement_state
                && e.first_label_date == first
                && e.last_label_date == last
                && e.provenance == provenance
                && e.import_run_id == batch.import_run_id
        });
        if existing.is_some() && !identical {
            return Err(HistoricalImportError::IdentityConflict);
        }
        if identical {
            let mapped = sql_query("SELECT count(*) count FROM historical_import_evidence WHERE subject=? AND import_run_id=? AND evidence_id=?")
                .bind::<Text, _>(&batch.subject)
                .bind::<Text, _>(&batch.import_run_id)
                .bind::<Text, _>(&id)
                .get_result::<Count>(c)
                .map_err(|error| HistoricalImportError::Storage(error.to_string()))?
                .count;
            if mapped != 1 {
                return Err(HistoricalImportError::IdentityConflict);
            }
        }
        out.push((id, identity, identical));
    }
    Ok(out)
}

fn existing_receipt(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    payload_digest: &str,
) -> Result<Option<HistoricalBatchResult>> {
    let row=sql_query("SELECT payload_digest,result_json FROM historical_import_batch_receipts WHERE subject=? AND idempotency_key=?").bind::<Text,_>(&batch.subject).bind::<Text,_>(&batch.idempotency_key).get_result::<ReceiptRow>(c).optional().map_err(|e|HistoricalImportError::Storage(e.to_string()))?;
    match row {
        Some(row) if row.payload_digest == payload_digest => serde_json::from_str(&row.result_json)
            .map(Some)
            .map_err(|e| HistoricalImportError::Storage(e.to_string())),
        Some(_) => Err(HistoricalImportError::IdempotencyConflict),
        None => Ok(None),
    }
}
fn validate_or_create_staged_run(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    write: bool,
) -> Result<()> {
    let manifest = batch.manifest.as_ref().expect("non-rollback manifest");
    let hash = batch.manifest_hash.as_deref().expect("validated");
    let manifest_digest = canonical_digest(manifest);
    let manifest_json = canonical_json(&serde_json::to_value(manifest).expect("manifest"));
    let run = sql_query("SELECT subject,manifest_hash,manifest_payload_digest,status FROM historical_import_runs WHERE import_run_id=?")
        .bind::<Text, _>(&batch.import_run_id)
        .get_result::<RunRow>(c)
        .optional()
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    match run {
        Some(run)
            if run.subject == batch.subject
                && run.manifest_hash == hash
                && run.manifest_payload_digest == manifest_digest
                && run.status == "staged" =>
        {
            Ok(())
        }
        Some(_) => Err(HistoricalImportError::RunConflict),
        None if write => {
            sql_query("INSERT INTO historical_import_runs(import_run_id,subject,manifest_hash,manifest_payload_digest,manifest_json,status) VALUES(?,?,?,?,?,'staged')")
                .bind::<Text, _>(&batch.import_run_id)
                .bind::<Text, _>(&batch.subject)
                .bind::<Text, _>(hash)
                .bind::<Text, _>(&manifest_digest)
                .bind::<Text, _>(&manifest_json)
                .execute(c)
                .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
            Ok(())
        }
        None => Ok(()),
    }
}

fn write_receipt(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    payload_digest: &str,
    result: &HistoricalBatchResult,
) -> Result<()> {
    let json = serde_json::to_string(result)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    sql_query("INSERT INTO historical_import_batch_receipts(subject,idempotency_key,import_run_id,operation,payload_digest,result_json) VALUES(?,?,?,?,?,?)")
        .bind::<Text, _>(&batch.subject)
        .bind::<Text, _>(&batch.idempotency_key)
        .bind::<Text, _>(&batch.import_run_id)
        .bind::<Text, _>(batch.operation.name())
        .bind::<Text, _>(payload_digest)
        .bind::<Text, _>(json)
        .execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    Ok(())
}

#[derive(QueryableByName)]
struct StagedRow {
    #[diesel(sql_type = Text)]
    identity: String,
    #[diesel(sql_type = Text)]
    payload_digest: String,
    #[diesel(sql_type = Text)]
    row_json: String,
}

fn stage_row<T: Serialize>(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    table: &str,
    identity: &str,
    row: &T,
) -> Result<bool> {
    let manifest_hash = batch.manifest_hash.as_deref().expect("validated");
    let row_json = canonical_json(&serde_json::to_value(row).expect("staged row"));
    let payload_digest = format!("{:x}", Sha256::digest(row_json.as_bytes()));
    let existing = sql_query(format!(
        "SELECT identity,payload_digest,row_json FROM {table} WHERE subject=? AND import_run_id=? AND manifest_hash=? AND identity=?"
    ))
    .bind::<Text, _>(&batch.subject)
    .bind::<Text, _>(&batch.import_run_id)
    .bind::<Text, _>(manifest_hash)
    .bind::<Text, _>(identity)
    .get_result::<StagedRow>(c)
    .optional()
    .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    if let Some(existing) = existing {
        if existing.identity == identity
            && existing.payload_digest == payload_digest
            && existing.row_json == row_json
        {
            return Ok(true);
        }
        return Err(HistoricalImportError::IdentityConflict);
    }
    sql_query(format!("INSERT INTO {table}(subject,import_run_id,manifest_hash,identity,payload_digest,row_json) VALUES(?,?,?,?,?,?)"))
        .bind::<Text, _>(&batch.subject)
        .bind::<Text, _>(&batch.import_run_id)
        .bind::<Text, _>(manifest_hash)
        .bind::<Text, _>(identity)
        .bind::<Text, _>(&payload_digest)
        .bind::<Text, _>(&row_json)
        .execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    Ok(false)
}

fn load_staged<T: for<'de> Deserialize<'de>>(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    table: &str,
) -> Result<Vec<(StagedRow, T)>> {
    let rows = sql_query(format!(
        "SELECT identity,payload_digest,row_json FROM {table} WHERE subject=? AND import_run_id=? AND manifest_hash=? ORDER BY identity"
    ))
    .bind::<Text, _>(&batch.subject)
    .bind::<Text, _>(&batch.import_run_id)
    .bind::<Text, _>(batch.manifest_hash.as_deref().expect("validated"))
    .load::<StagedRow>(c)
    .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let parsed = serde_json::from_str(&row.row_json)
                .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
            Ok((row, parsed))
        })
        .collect()
}

fn storage_error(error: HistoricalImportError) -> StorageError {
    StorageError::CoreError(wealthfolio_core::Error::Repository(error.to_string()))
}

#[derive(Clone)]
pub struct HistoricalImportRepository {
    writer: WriteHandle,
}

impl HistoricalImportRepository {
    pub fn new(writer: WriteHandle) -> Self {
        Self { writer }
    }

    pub async fn apply(&self, batch: HistoricalImportBatch) -> Result<HistoricalBatchResult> {
        self.apply_with_fault(batch, HistoricalImportFault::None)
            .await
    }

    pub async fn apply_with_fault(
        &self,
        batch: HistoricalImportBatch,
        fault: HistoricalImportFault,
    ) -> Result<HistoricalBatchResult> {
        let payload_digest = historical_batch_payload_digest(&batch);
        self.writer
            .exec(move |c| {
                let outcome =
                    c.transaction::<Result<HistoricalBatchResult>, StorageError, _>(|c| {
                        let manifest_rows = match validate_batch(&batch) {
                            Ok(value) => value,
                            Err(error) => return Ok(Err(error)),
                        };
                        match existing_receipt(c, &batch, &payload_digest) {
                            Ok(Some(receipt)) => return Ok(Ok(receipt)),
                            Ok(None) => {}
                            Err(error) => return Ok(Err(error)),
                        }
                        if matches!(batch.operation, HistoricalOperation::Rollback) {
                            return rollback(c, &batch, &payload_digest, fault)
                                .map(Ok)
                                .map_err(storage_error);
                        }
                        let manifest = batch.manifest.as_ref().expect("validated");
                        if let Err(error) = authorize_manifest_accounts(c, &batch.subject, manifest)
                        {
                            return Ok(Err(error));
                        }
                        if let Err(error) = validate_or_create_staged_run(c, &batch, false) {
                            return Ok(Err(error));
                        }
                        let mut result = HistoricalBatchResult {
                            dry_run: batch.dry_run,
                            import_run_id: batch.import_run_id.clone(),
                            validated: 0,
                            written: 0,
                            already_present: 0,
                            ledger_deleted: 0,
                            learning_evidence_deleted: 0,
                            checksum: String::new(),
                        };
                        match &batch.operation {
                            HistoricalOperation::StageLedger { rows } => {
                                let flags =
                                    match validate_ledger_rows(c, &batch, &manifest_rows, rows) {
                                        Ok(value) => value,
                                        Err(error) => return Ok(Err(error)),
                                    };
                                if flags.iter().any(|exists| *exists) {
                                    return Ok(Err(HistoricalImportError::IdentityConflict));
                                }
                                if let Err(error) =
                                    validate_or_create_staged_run(c, &batch, !batch.dry_run)
                                {
                                    return Ok(Err(error));
                                }
                                result.validated = rows.len();
                                result.checksum =
                                    checksum(rows.iter().map(|row| row.idempotency_key.as_str()));
                                if !batch.dry_run {
                                    for (index, row) in rows.iter().enumerate() {
                                        match stage_row(
                                            c,
                                            &batch,
                                            "historical_import_staged_ledger",
                                            &row.historical_identity,
                                            row,
                                        ) {
                                            Ok(true) => result.already_present += 1,
                                            Ok(false) => result.written += 1,
                                            Err(error) => return Err(storage_error(error)),
                                        }
                                        if index == 0
                                            && fault == HistoricalImportFault::AfterFirstWrite
                                        {
                                            return Err(StorageError::QueryFailed(
                                                diesel::result::Error::RollbackTransaction,
                                            ));
                                        }
                                    }
                                } else {
                                    result.written = rows.len();
                                }
                            }
                            HistoricalOperation::StageLearningEvidence { rows } => {
                                if let Err(error) =
                                    validate_evidence_rows(c, &batch, &manifest_rows, rows)
                                {
                                    return Ok(Err(error));
                                }
                                if let Err(error) =
                                    validate_or_create_staged_run(c, &batch, !batch.dry_run)
                                {
                                    return Ok(Err(error));
                                }
                                result.validated = rows.len();
                                result.checksum =
                                    checksum(rows.iter().map(|row| row.idempotency_key.as_str()));
                                if !batch.dry_run {
                                    for (index, row) in rows.iter().enumerate() {
                                        match stage_row(
                                            c,
                                            &batch,
                                            "historical_import_staged_evidence",
                                            &row.idempotency_key,
                                            row,
                                        ) {
                                            Ok(true) => result.already_present += 1,
                                            Ok(false) => result.written += 1,
                                            Err(error) => return Err(storage_error(error)),
                                        }
                                        if index == 0
                                            && fault == HistoricalImportFault::AfterFirstWrite
                                        {
                                            return Err(StorageError::QueryFailed(
                                                diesel::result::Error::RollbackTransaction,
                                            ));
                                        }
                                    }
                                } else {
                                    result.written = rows.len();
                                }
                            }
                            HistoricalOperation::Finalize => {
                                match finalize(c, &batch, &manifest_rows, fault) {
                                    Ok(value) => result = value,
                                    Err(error @ HistoricalImportError::Storage(_)) => {
                                        return Err(storage_error(error));
                                    }
                                    Err(error) => return Ok(Err(error)),
                                }
                            }
                            HistoricalOperation::Rollback => unreachable!(),
                        }
                        if !batch.dry_run {
                            write_receipt(c, &batch, &payload_digest, &result)
                                .map_err(storage_error)?;
                        }
                        Ok(Ok(result))
                    });
                outcome.map_err(wealthfolio_core::Error::from)
            })
            .await
            .map_err(|error| HistoricalImportError::Storage(error.to_string()))?
    }
}

fn finalize(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    manifest_rows: &HashMap<String, HistoricalManifestLedgerRow>,
    fault: HistoricalImportFault,
) -> Result<HistoricalBatchResult> {
    let staged_ledger =
        load_staged::<HistoricalLedgerRow>(c, batch, "historical_import_staged_ledger")?;
    let staged_evidence =
        load_staged::<HistoricalEvidenceRow>(c, batch, "historical_import_staged_evidence")?;
    let expected_ledger = manifest_rows
        .values()
        .filter(|row| row.disposition == LedgerDisposition::Selected)
        .map(|row| row.identity.clone())
        .collect::<HashSet<_>>();
    let actual_ledger = staged_ledger
        .iter()
        .map(|(row, _)| row.identity.clone())
        .collect::<HashSet<_>>();
    let manifest = batch.manifest.as_ref().expect("validated");
    let expected_evidence = manifest
        .evidence
        .iter()
        .map(|row| row.identity.clone())
        .collect::<HashSet<_>>();
    let actual_evidence = staged_evidence
        .iter()
        .map(|(row, _)| row.identity.clone())
        .collect::<HashSet<_>>();
    if expected_ledger != actual_ledger
        || expected_evidence != actual_evidence
        || expected_ledger.len() != staged_ledger.len()
        || expected_evidence.len() != staged_evidence.len()
    {
        return Err(invalid(
            "finalize requires the complete reviewed ledger and evidence identity sets",
        ));
    }
    for (staged, row) in &staged_evidence {
        let expected = manifest
            .evidence
            .iter()
            .find(|item| item.identity == staged.identity)
            .ok_or_else(|| invalid("staged evidence is absent from manifest"))?;
        if staged.payload_digest != expected.payload_digest
            || expected.payload_digest != historical_evidence_payload_digest(row)
        {
            return Err(invalid(
                "staged evidence digest differs from reviewed manifest",
            ));
        }
    }
    let ledger_rows = staged_ledger
        .iter()
        .map(|(_, row)| row.clone())
        .collect::<Vec<_>>();
    let evidence_rows = staged_evidence
        .iter()
        .map(|(_, row)| row.clone())
        .collect::<Vec<_>>();
    if validate_ledger_rows(c, batch, manifest_rows, &ledger_rows)?
        .iter()
        .any(|exists| *exists)
    {
        return Err(HistoricalImportError::IdentityConflict);
    }
    if validate_evidence_rows(c, batch, manifest_rows, &evidence_rows)?
        .iter()
        .any(|(_, _, exists)| *exists)
    {
        return Err(HistoricalImportError::IdentityConflict);
    }
    let identities = staged_ledger
        .iter()
        .map(|(row, _)| row.identity.as_str())
        .chain(staged_evidence.iter().map(|(row, _)| row.identity.as_str()));
    let result = HistoricalBatchResult {
        dry_run: batch.dry_run,
        import_run_id: batch.import_run_id.clone(),
        validated: staged_ledger.len() + staged_evidence.len(),
        written: staged_ledger.len() + staged_evidence.len(),
        already_present: 0,
        ledger_deleted: 0,
        learning_evidence_deleted: 0,
        checksum: checksum(identities),
    };
    if batch.dry_run {
        return Ok(result);
    }
    for (index, row) in ledger_rows.iter().enumerate() {
        sql_query("INSERT INTO activities(id,account_id,activity_type,status,activity_date,amount,currency,notes,source_system,source_record_id,idempotency_key,is_user_modified,needs_review,created_at,updated_at) VALUES(?,?,?,'POSTED',?,?,?,?,?,?,?,0,0,?,?)")
            .bind::<Text, _>(&row.historical_identity).bind::<Text, _>(&row.account_id)
            .bind::<Text, _>(&row.activity_type).bind::<Text, _>(&row.effective_at)
            .bind::<Text, _>(&row.amount).bind::<Text, _>(&row.currency)
            .bind::<Text, _>(&row.raw_description).bind::<Text, _>(row.source.as_str())
            .bind::<Text, _>(&row.source_id).bind::<Text, _>(&row.idempotency_key)
            .bind::<Text, _>(&row.effective_at).bind::<Text, _>(&row.effective_at)
            .execute(c).map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
        sql_query("INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,import_run_id,actor_id) VALUES('activity',?,'historical_context',?,?,?,?)")
            .bind::<Text, _>(&row.historical_identity).bind::<Text, _>(row.source.as_str())
            .bind::<Text, _>(&row.source_id).bind::<Text, _>(&batch.import_run_id)
            .bind::<Text, _>(&batch.subject).execute(c)
            .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
        sql_query("INSERT INTO historical_import_activities(subject,import_run_id,activity_id) VALUES(?,?,?)")
            .bind::<Text, _>(&batch.subject).bind::<Text, _>(&batch.import_run_id)
            .bind::<Text, _>(&row.historical_identity).execute(c)
            .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
        if index == 0 && fault == HistoricalImportFault::AfterFirstWrite {
            return Err(HistoricalImportError::Storage(
                "injected finalize fault".into(),
            ));
        }
    }
    for row in &evidence_rows {
        let id = format!("categorization-evidence-{}", evidence_identity(row));
        let identity = evidence_identity(row);
        let provenance = evidence_provenance(batch, row);
        sql_query("INSERT INTO spending_categorization_evidence(id,identity_key,merchant_key,merchant_key_version,normalization_version,taxonomy_id,category_id,direction,activity_type,provider_context,account_context,label_count,agreement_state,first_label_date,last_label_date,provenance,import_run_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind::<Text, _>(&id).bind::<Text, _>(&identity).bind::<Text, _>(&row.merchant_key)
            .bind::<Integer, _>(row.merchant_key_version).bind::<Integer, _>(row.normalization_version)
            .bind::<Text, _>(&row.taxonomy_id).bind::<Text, _>(&row.category_id)
            .bind::<Text, _>(&row.direction).bind::<Text, _>(&row.activity_type)
            .bind::<Nullable<Text>, _>(row.provider_context.as_deref())
            .bind::<Nullable<Text>, _>(Some(row.account_id.as_str()))
            .bind::<Integer, _>(row.label_count).bind::<Text, _>(&row.agreement_state)
            .bind::<Text, _>(canonical_label_date(&row.first_label_date)?)
            .bind::<Text, _>(canonical_label_date(&row.last_label_date)?)
            .bind::<Text, _>(provenance).bind::<Text, _>(&batch.import_run_id)
            .execute(c).map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
        sql_query("INSERT INTO historical_import_evidence(subject,import_run_id,evidence_id) VALUES(?,?,?)")
            .bind::<Text, _>(&batch.subject).bind::<Text, _>(&batch.import_run_id)
            .bind::<Text, _>(&id).execute(c)
            .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    }
    sql_query("DELETE FROM historical_import_staged_ledger WHERE subject=? AND import_run_id=?")
        .bind::<Text, _>(&batch.subject)
        .bind::<Text, _>(&batch.import_run_id)
        .execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    sql_query("DELETE FROM historical_import_staged_evidence WHERE subject=? AND import_run_id=?")
        .bind::<Text, _>(&batch.subject)
        .bind::<Text, _>(&batch.import_run_id)
        .execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    sql_query("UPDATE historical_import_runs SET status='active',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE import_run_id=? AND subject=? AND status='staged'")
        .bind::<Text, _>(&batch.import_run_id).bind::<Text, _>(&batch.subject).execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    Ok(result)
}

fn rollback(
    c: &mut SqliteConnection,
    batch: &HistoricalImportBatch,
    payload_digest: &str,
    fault: HistoricalImportFault,
) -> Result<HistoricalBatchResult> {
    let run = sql_query("SELECT subject,manifest_hash,manifest_payload_digest,status FROM historical_import_runs WHERE import_run_id=?")
        .bind::<Text, _>(&batch.import_run_id).get_result::<RunRow>(c).optional()
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?
        .ok_or(HistoricalImportError::RunConflict)?;
    if run.subject != batch.subject {
        return Err(HistoricalImportError::RunConflict);
    }
    let staged_ledger = count_for_run(c, "historical_import_staged_ledger", batch)?;
    let staged_evidence = count_for_run(c, "historical_import_staged_evidence", batch)?;
    let active_ledger = count_for_run(c, "historical_import_activities", batch)?;
    let active_evidence = count_for_run(c, "historical_import_evidence", batch)?;
    let mut result = HistoricalBatchResult {
        dry_run: batch.dry_run,
        import_run_id: batch.import_run_id.clone(),
        validated: 0,
        written: 0,
        already_present: 0,
        ledger_deleted: staged_ledger + active_ledger,
        learning_evidence_deleted: staged_evidence + active_evidence,
        checksum: digest_parts(&[batch.import_run_id.clone(), "rollback".into()]),
    };
    if batch.dry_run {
        return Ok(result);
    }
    if run.status == "rolled_back" {
        result.ledger_deleted = 0;
        result.learning_evidence_deleted = 0;
        write_receipt(c, batch, payload_digest, &result)?;
        return Ok(result);
    }
    sql_query("DELETE FROM provenance_records WHERE entity_type='activity' AND entity_id IN (SELECT activity_id FROM historical_import_activities WHERE subject=? AND import_run_id=?) AND actor_id=? AND import_run_id=?")
        .bind::<Text, _>(&batch.subject).bind::<Text, _>(&batch.import_run_id)
        .bind::<Text, _>(&batch.subject).bind::<Text, _>(&batch.import_run_id).execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    sql_query("DELETE FROM activities WHERE id IN (SELECT activity_id FROM historical_import_activities WHERE subject=? AND import_run_id=?)")
        .bind::<Text, _>(&batch.subject).bind::<Text, _>(&batch.import_run_id).execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    if fault == HistoricalImportFault::AfterFirstRollbackDelete {
        return Err(HistoricalImportError::Storage(
            "injected rollback fault".into(),
        ));
    }
    sql_query("DELETE FROM spending_categorization_evidence WHERE id IN (SELECT evidence_id FROM historical_import_evidence WHERE subject=? AND import_run_id=?)")
        .bind::<Text, _>(&batch.subject).bind::<Text, _>(&batch.import_run_id).execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    for table in [
        "historical_import_staged_ledger",
        "historical_import_staged_evidence",
        "historical_import_evidence",
    ] {
        sql_query(format!(
            "DELETE FROM {table} WHERE subject=? AND import_run_id=?"
        ))
        .bind::<Text, _>(&batch.subject)
        .bind::<Text, _>(&batch.import_run_id)
        .execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    }
    sql_query("UPDATE historical_import_runs SET status='rolled_back',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE import_run_id=? AND subject=?")
        .bind::<Text, _>(&batch.import_run_id).bind::<Text, _>(&batch.subject).execute(c)
        .map_err(|error| HistoricalImportError::Storage(error.to_string()))?;
    write_receipt(c, batch, payload_digest, &result)?;
    Ok(result)
}

fn count_for_run(
    c: &mut SqliteConnection,
    table: &str,
    batch: &HistoricalImportBatch,
) -> Result<usize> {
    sql_query(format!(
        "SELECT count(*) count FROM {table} WHERE subject=? AND import_run_id=?"
    ))
    .bind::<Text, _>(&batch.subject)
    .bind::<Text, _>(&batch.import_run_id)
    .get_result::<Count>(c)
    .map(|row| row.count as usize)
    .map_err(|error| HistoricalImportError::Storage(error.to_string()))
}
