use crate::{DbPool, WriteHandle};
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{BigInt, Integer, Nullable, Text},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisclosureKind {
    ActivityUpsert,
    ActivityDelete,
    ProjectionFence,
    CurrentStateUpsert,
}
impl DisclosureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ActivityUpsert => "activity_upsert",
            Self::ActivityDelete => "activity_delete",
            Self::ProjectionFence => "projection_fence",
            Self::CurrentStateUpsert => "current_state_upsert",
        }
    }
}

/// Strict Finance merchant identity context. This is advisory provider data only;
/// category, rule, confidence, and other classification fields are deliberately
/// not representable on this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MerchantContextV1 {
    pub merchant_key: String,
    pub merchant_key_version: i32,
    pub normalization_version: i32,
}

/// Payload for creating or updating an activity row via the bridge.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityPayload {
    pub account_id: String,
    pub activity_type: String,
    pub status: String,
    pub activity_date: String,
    #[serde(default)]
    pub settlement_date: Option<String>,
    pub amount: String,
    pub currency: String,
    #[serde(default)]
    pub fee: Option<String>,
    #[serde(default)]
    pub tax: Option<String>,
    #[serde(default)]
    pub fx_rate: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub source_record_id: Option<String>,
    #[serde(default)]
    pub source_group_id: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub activity_type_override: Option<String>,
    #[serde(default)]
    pub needs_review: Option<bool>,
    /// Provider-owned JSON evidence. User-owned metadata is never accepted from this boundary.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Original/card amount and currency when `amount`/`currency` represent settlement.
    #[serde(default)]
    pub original_amount: Option<String>,
    #[serde(default)]
    pub original_currency: Option<String>,
    /// Account/pocket settlement amount and currency when they differ from the activity.
    #[serde(default)]
    pub settlement_amount: Option<String>,
    #[serde(default)]
    pub settlement_currency: Option<String>,
    #[serde(default)]
    pub merchant_context: Option<MerchantContextV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentStatePosition {
    pub symbol: String,
    pub quantity: String,
    pub price: String,
    pub market_value: String,
    pub currency: String,
    pub cost_basis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentStatePayload {
    pub account_id: String,
    pub provider_connection_id: String,
    pub provider_incarnation_id: String,
    pub provider_generation: i64,
    pub observed_at: String,
    pub revision: i64,
    pub as_of_date: String,
    pub currency: String,
    pub cash_balance: String,
    /// Positive debt magnitude.
    pub liability_balance: String,
    pub positions: Vec<CurrentStatePosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisclosureOperation {
    pub idempotency_key: String,
    pub subject: String,
    pub access_generation: i64,
    pub kind: DisclosureKind,
    pub resource_id: String,
    /// When present, the commit also upserts/deletes the real activities row.
    pub activity: Option<ActivityPayload>,
    /// When present, signals a pending->posted lifecycle migration from
    /// resource_id (pending) to lifecycle_successor_id (posted).
    pub lifecycle_successor_id: Option<String>,
    /// Provider identity corrections that must be merged into resource_id before
    /// the provider-owned upsert is exposed. The receiver migrates user overlays
    /// and child rows atomically, then removes each superseded local resource.
    #[serde(default)]
    pub superseded_resource_ids: Vec<String>,
    #[serde(default)]
    pub semantic_hash: Option<String>,
    #[serde(default)]
    pub current_state: Option<CurrentStatePayload>,
}
impl DisclosureOperation {
    pub fn new(
        k: impl Into<String>,
        s: impl Into<String>,
        g: i64,
        kind: DisclosureKind,
        r: impl Into<String>,
    ) -> Self {
        Self {
            idempotency_key: k.into(),
            subject: s.into(),
            access_generation: g,
            kind,
            resource_id: r.into(),
            activity: None,
            lifecycle_successor_id: None,
            superseded_resource_ids: Vec::new(),
            semantic_hash: None,
            current_state: None,
        }
    }

    pub fn with_activity(mut self, a: ActivityPayload) -> Self {
        self.activity = Some(a);
        self
    }

    pub fn with_lifecycle(mut self, successor: impl Into<String>) -> Self {
        self.lifecycle_successor_id = Some(successor.into());
        self
    }

    pub fn with_superseded_resources(
        mut self,
        resources: impl IntoIterator<Item = String>,
    ) -> Self {
        self.superseded_resource_ids = resources.into_iter().collect();
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptOutcome {
    Accepted,
    Revoked,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisclosureReceipt {
    pub deployment_id: String,
    pub account_id: String,
    pub subject: String,
    pub idempotency_key: String,
    pub access_generation: i64,
    pub operation: String,
    pub resource_id: String,
    pub lifecycle_successor_id: Option<String>,
    pub outcome: ReceiptOutcome,
    pub binding_version: i64,
    pub receipt_binding: String,
    /// Finance semantic hash for CurrentStateUpsert receipts.
    pub semantic_hash: Option<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum BridgePolicyError {
    #[error("stale access generation: expected {expected}, actual {actual}")]
    StaleGeneration { expected: i64, actual: i64 },
    #[error("subject has no deployment generation")]
    MissingGeneration,
    #[error("resource is not accessible to subject")]
    AccessDenied,
    #[error("idempotency key was reused for a different operation")]
    IdempotencyConflict,
    #[error("activity account does not match the authorized resource")]
    ActivityAccountMismatch,
    #[error("activity payload is invalid: {0}")]
    InvalidActivity(String),
    #[error("activity supersession is invalid: {0}")]
    InvalidActivitySupersession(String),
    #[error("current state payload is invalid: {0}")]
    InvalidCurrentState(String),
    #[error("current state semantic hash does not match payload")]
    SemanticHashMismatch,
    #[error("provider symbol and currency resolve ambiguously")]
    AmbiguousAsset,
    #[error("storage: {0}")]
    Storage(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeFault {
    None,
    BeforeReceipt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFault {
    None,
    AfterSidecar,
}
type Result<T> = std::result::Result<T, BridgePolicyError>;
#[derive(QueryableByName)]
struct Gen {
    #[diesel(sql_type=BigInt)]
    generation: i64,
}
#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    count: i64,
}
#[derive(QueryableByName)]
struct Account {
    #[diesel(sql_type=Text)]
    account_id: String,
}
#[derive(QueryableByName)]
struct ActivityIdentity {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    status: String,
}
#[derive(QueryableByName)]
struct ActivityIdRow {
    #[diesel(sql_type=Text)]
    id: String,
}
#[derive(QueryableByName)]
struct Household {
    #[diesel(sql_type=Text)]
    household_id: String,
}
#[derive(QueryableByName)]
struct ReceiptRow {
    #[diesel(sql_type=Text)]
    subject: String,
    #[diesel(sql_type=Text)]
    idempotency_key: String,
    #[diesel(sql_type=BigInt)]
    access_generation: i64,
    #[diesel(sql_type=Text)]
    operation: String,
    #[diesel(sql_type=Text)]
    resource_id: String,
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Nullable<Text>)]
    lifecycle_successor_id: Option<String>,
    #[diesel(sql_type=BigInt)]
    binding_version: i64,
    #[diesel(sql_type=Nullable<Text>)]
    receipt_binding: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    payload_digest: Option<String>,
    #[diesel(sql_type=Text)]
    outcome: String,
}
fn receipt(deployment_id: &str, r: ReceiptRow) -> DisclosureReceipt {
    let semantic_hash = (r.operation == "current_state_upsert")
        .then_some(r.payload_digest.clone())
        .flatten();
    DisclosureReceipt {
        deployment_id: deployment_id.to_owned(),
        account_id: r.account_id,
        subject: r.subject,
        idempotency_key: r.idempotency_key,
        access_generation: r.access_generation,
        operation: r.operation,
        resource_id: r.resource_id,
        lifecycle_successor_id: r.lifecycle_successor_id,
        outcome: if r.outcome == "revoked" {
            ReceiptOutcome::Revoked
        } else {
            ReceiptOutcome::Accepted
        },
        binding_version: r.binding_version,
        receipt_binding: r.receipt_binding.unwrap_or_default(),
        semantic_hash,
    }
}

const RECEIPT_BINDING_VERSION: i64 = 3;

// The binding is deliberately explicit so every receipt identity field is visible at call sites.
// V2 encodes successor presence separately so None cannot collide with a literal sentinel value.
#[allow(clippy::too_many_arguments)]
fn canonical_receipt_binding(
    deployment: &str,
    subject: &str,
    key: &str,
    generation: i64,
    operation: &str,
    resource: &str,
    account: &str,
    successor: Option<&str>,
    outcome: &str,
    semantic_hash: Option<&str>,
) -> String {
    let generation = generation.to_string();
    let successor_present = if successor.is_some() { "1" } else { "0" };
    let semantic_hash_present = if semantic_hash.is_some() { "1" } else { "0" };
    let fields = [
        deployment,
        subject,
        key,
        generation.as_str(),
        operation,
        resource,
        account,
        successor_present,
        successor.unwrap_or(""),
        outcome,
        semantic_hash_present,
        semantic_hash.unwrap_or(""),
    ];
    let mut out = String::from("v3;");
    for field in fields {
        out.push_str(&field.len().to_string());
        out.push(':');
        out.push_str(field);
    }
    out
}

fn is_canonical_receipt_binding(binding: &str, expected: &str) -> bool {
    if binding != expected || !binding.starts_with("v3;") {
        return false;
    }
    let mut rest = &binding.as_bytes()[3..];
    for _ in 0..12 {
        let Some(colon) = rest.iter().position(|b| *b == b':') else {
            return false;
        };
        let Ok(length) = std::str::from_utf8(&rest[..colon])
            .unwrap_or("")
            .parse::<usize>()
        else {
            return false;
        };
        rest = &rest[colon + 1..];
        if rest.len() < length || std::str::from_utf8(&rest[..length]).is_err() {
            return false;
        }
        rest = &rest[length..];
    }
    rest.is_empty()
}

#[derive(Debug, PartialEq, Eq)]
pub struct MigrationState {
    pub pending_exists: bool,
    pub posted_exists: bool,
    pub attribution: Option<String>,
    pub assignments: i64,
    pub splits: i64,
    pub event_tags: i64,
    pub ignored: bool,
    pub needs_review: bool,
}
#[derive(QueryableByName)]
struct State {
    #[diesel(sql_type=BigInt)]
    pending_exists: i64,
    #[diesel(sql_type=BigInt)]
    posted_exists: i64,
    #[diesel(sql_type=Nullable<Text>)]
    attribution: Option<String>,
    #[diesel(sql_type=BigInt)]
    assignments: i64,
    #[diesel(sql_type=BigInt)]
    splits: i64,
    #[diesel(sql_type=BigInt)]
    event_tags: i64,
    #[diesel(sql_type=Integer)]
    ignored: i32,
    #[diesel(sql_type=Integer)]
    needs_review: i32,
}

// ── SQL helpers that return diesel errors for the transaction closure to convert ──
pub fn disclosure_payload_digest(op: &DisclosureOperation) -> String {
    if matches!(op.kind, DisclosureKind::CurrentStateUpsert) {
        if let (Some(state), Some(semantic_hash)) = (&op.current_state, &op.semantic_hash) {
            if semantic_hash == &current_state_semantic_hash(state) {
                return semantic_hash.clone();
            }
        }
    }
    fn field(hasher: &mut Sha256, value: &str) {
        hasher.update(value.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(value.as_bytes());
    }
    fn optional_field(hasher: &mut Sha256, value: Option<&str>) {
        field(hasher, if value.is_some() { "1" } else { "0" });
        field(hasher, value.unwrap_or(""));
    }

    let mut hasher = Sha256::new();
    field(&mut hasher, "wealthfolio-bridge-payload-v1");
    field(&mut hasher, op.kind.as_str());
    field(&mut hasher, &op.resource_id);
    optional_field(&mut hasher, op.lifecycle_successor_id.as_deref());
    if !op.superseded_resource_ids.is_empty() {
        let mut superseded_resource_ids = op.superseded_resource_ids.clone();
        superseded_resource_ids.sort();
        field(&mut hasher, "superseded-resource-ids-v1");
        field(&mut hasher, &superseded_resource_ids.len().to_string());
        for resource_id in superseded_resource_ids {
            field(&mut hasher, &resource_id);
        }
    }
    field(&mut hasher, if op.activity.is_some() { "1" } else { "0" });
    if let Some(activity) = &op.activity {
        field(&mut hasher, &activity.account_id);
        field(&mut hasher, &activity.activity_type);
        field(&mut hasher, &activity.status);
        field(&mut hasher, &activity.activity_date);
        field(&mut hasher, &activity.amount);
        field(&mut hasher, &activity.currency);
        optional_field(&mut hasher, activity.notes.as_deref());
        optional_field(&mut hasher, activity.activity_type_override.as_deref());
        let needs_review = activity
            .needs_review
            .map(|value| if value { "true" } else { "false" });
        optional_field(&mut hasher, needs_review);
        field(
            &mut hasher,
            if activity.merchant_context.is_some() {
                "1"
            } else {
                "0"
            },
        );
        if let Some(context) = &activity.merchant_context {
            field(&mut hasher, &context.merchant_key);
            field(&mut hasher, &context.merchant_key_version.to_string());
            field(&mut hasher, &context.normalization_version.to_string());
        }
        let has_extended_fields = activity.settlement_date.is_some()
            || activity.fee.is_some()
            || activity.tax.is_some()
            || activity.fx_rate.is_some()
            || activity.source_type.is_some()
            || activity.subtype.is_some()
            || activity.source_record_id.is_some()
            || activity.source_group_id.is_some()
            || activity.metadata.is_some()
            || activity.original_amount.is_some()
            || activity.original_currency.is_some()
            || activity.settlement_amount.is_some()
            || activity.settlement_currency.is_some();
        if has_extended_fields {
            field(&mut hasher, "extended-activity-fields-v1");
            optional_field(&mut hasher, activity.settlement_date.as_deref());
            optional_field(&mut hasher, activity.fee.as_deref());
            optional_field(&mut hasher, activity.tax.as_deref());
            optional_field(&mut hasher, activity.fx_rate.as_deref());
            optional_field(&mut hasher, activity.source_type.as_deref());
            optional_field(&mut hasher, activity.subtype.as_deref());
            optional_field(&mut hasher, activity.source_record_id.as_deref());
            optional_field(&mut hasher, activity.source_group_id.as_deref());
            let metadata = activity.metadata.as_ref().map(canonical_json);
            optional_field(&mut hasher, metadata.as_deref());
            optional_field(&mut hasher, activity.original_amount.as_deref());
            optional_field(&mut hasher, activity.original_currency.as_deref());
            optional_field(&mut hasher, activity.settlement_amount.as_deref());
            optional_field(&mut hasher, activity.settlement_currency.as_deref());
        }
    }
    optional_field(&mut hasher, op.semantic_hash.as_deref());
    field(
        &mut hasher,
        if op.current_state.is_some() { "1" } else { "0" },
    );
    if let Some(state) = &op.current_state {
        field(&mut hasher, &state.account_id);
        field(&mut hasher, &state.provider_connection_id);
        field(&mut hasher, &state.provider_incarnation_id);
        field(&mut hasher, &state.provider_generation.to_string());
        field(&mut hasher, &state.observed_at);
        field(&mut hasher, &state.revision.to_string());
        field(&mut hasher, &state.as_of_date);
        field(&mut hasher, &state.currency);
        field(&mut hasher, &state.cash_balance);
        field(&mut hasher, &state.liability_balance);
        field(&mut hasher, &state.positions.len().to_string());
        for position in &state.positions {
            field(&mut hasher, &position.symbol);
            field(&mut hasher, &position.quantity);
            field(&mut hasher, &position.price);
            field(&mut hasher, &position.market_value);
            field(&mut hasher, &position.currency);
            optional_field(&mut hasher, position.cost_basis.as_deref());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => {
            serde_json::to_string(value).expect("string serialization")
        }
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
                        serde_json::to_string(key).expect("key serialization"),
                        canonical_json(&values[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

pub fn current_state_semantic_hash(state: &CurrentStatePayload) -> String {
    let positions = state
        .positions
        .iter()
        .map(|position| {
            let mut value = serde_json::json!({
                "symbol": position.symbol,
                "quantity": position.quantity,
                "price": position.price,
                "marketValue": position.market_value,
                "currency": position.currency,
            });
            if let Some(cost_basis) = &position.cost_basis {
                value.as_object_mut().expect("position object").insert(
                    "costBasis".into(),
                    serde_json::Value::String(cost_basis.clone()),
                );
            }
            value
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "accountId": state.account_id,
        "providerConnectionId": state.provider_connection_id,
        "providerIncarnationId": state.provider_incarnation_id,
        "providerGeneration": state.provider_generation,
        "observedAt": state.observed_at,
        "revision": state.revision,
        "asOfDate": state.as_of_date,
        "currency": state.currency,
        "cashBalance": state.cash_balance,
        "liabilityBalance": state.liability_balance,
        "positions": positions,
    });
    format!("{:x}", Sha256::digest(canonical_json(&value).as_bytes()))
}

fn validate_activity_payload(activity: &ActivityPayload) -> Result<()> {
    use chrono::{DateTime, NaiveDate};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    if activity.account_id.trim().is_empty()
        || activity.activity_type.trim().is_empty()
        || activity.activity_date.trim().is_empty()
    {
        return Err(BridgePolicyError::InvalidActivity(
            "account, type, and activity date are required".into(),
        ));
    }
    if !matches!(
        activity.status.as_str(),
        "POSTED" | "PENDING" | "DRAFT" | "VOID"
    ) {
        return Err(BridgePolicyError::InvalidActivity(
            "invalid activity status".into(),
        ));
    }
    let valid_date = |value: &str| {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
            || DateTime::parse_from_rfc3339(value).is_ok()
    };
    if !valid_date(&activity.activity_date)
        || activity
            .settlement_date
            .as_deref()
            .is_some_and(|value| !valid_date(value))
    {
        return Err(BridgePolicyError::InvalidActivity(
            "invalid activity date".into(),
        ));
    }
    let valid_currency = |value: &str| value.len() == 3 && value == value.to_uppercase();
    if !valid_currency(&activity.currency)
        || activity
            .original_currency
            .as_deref()
            .is_some_and(|value| !valid_currency(value))
        || activity
            .settlement_currency
            .as_deref()
            .is_some_and(|value| !valid_currency(value))
    {
        return Err(BridgePolicyError::InvalidActivity(
            "invalid activity currency".into(),
        ));
    }
    let decimal = |value: &str, allow_zero: bool| {
        Decimal::from_str(value)
            .ok()
            .filter(|number| !number.is_sign_negative() && (allow_zero || !number.is_zero()))
    };
    if decimal(&activity.amount, false).is_none()
        || activity
            .fee
            .as_deref()
            .is_some_and(|value| decimal(value, true).is_none())
        || activity
            .tax
            .as_deref()
            .is_some_and(|value| decimal(value, true).is_none())
        || activity
            .fx_rate
            .as_deref()
            .is_some_and(|value| decimal(value, false).is_none())
        || activity
            .original_amount
            .as_deref()
            .is_some_and(|value| decimal(value, false).is_none())
        || activity
            .settlement_amount
            .as_deref()
            .is_some_and(|value| decimal(value, false).is_none())
    {
        return Err(BridgePolicyError::InvalidActivity(
            "invalid activity decimal".into(),
        ));
    }
    if activity
        .metadata
        .as_ref()
        .is_some_and(|value| !value.is_object())
    {
        return Err(BridgePolicyError::InvalidActivity(
            "activity metadata must be an object".into(),
        ));
    }
    Ok(())
}

fn validate_activity_supersession(op: &DisclosureOperation) -> Result<()> {
    if op.superseded_resource_ids.is_empty() {
        return Ok(());
    }
    if op.kind != DisclosureKind::ActivityUpsert {
        return Err(BridgePolicyError::InvalidActivitySupersession(
            "only ActivityUpsert may supersede provider resources".into(),
        ));
    }
    if op.activity.is_none() {
        return Err(BridgePolicyError::InvalidActivitySupersession(
            "activity payload is required".into(),
        ));
    }
    if op.lifecycle_successor_id.is_some() {
        return Err(BridgePolicyError::InvalidActivitySupersession(
            "pending lifecycle and provider identity supersession cannot be combined".into(),
        ));
    }
    if op.superseded_resource_ids.len() > 64 {
        return Err(BridgePolicyError::InvalidActivitySupersession(
            "too many superseded resources".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for resource_id in &op.superseded_resource_ids {
        if resource_id.trim().is_empty()
            || resource_id == &op.resource_id
            || !seen.insert(resource_id)
        {
            return Err(BridgePolicyError::InvalidActivitySupersession(
                "resource IDs must be non-empty, distinct, and different from the canonical resource".into(),
            ));
        }
    }
    Ok(())
}

fn validate_current_state_operation(op: &DisclosureOperation) -> Result<()> {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    match op.kind {
        DisclosureKind::CurrentStateUpsert => {
            let state = op.current_state.as_ref().ok_or_else(|| {
                BridgePolicyError::InvalidCurrentState("current_state is required".into())
            })?;
            let expected_suffix = format!(":current-state:{}", state.account_id);
            if op.activity.is_some()
                || op.lifecycle_successor_id.is_some()
                || !op.resource_id.ends_with(&expected_suffix)
            {
                return Err(BridgePolicyError::InvalidCurrentState(
                    "operation fields do not match current state account".into(),
                ));
            }
            if state.provider_connection_id.trim().is_empty()
                || state.provider_incarnation_id.trim().is_empty()
                || state.provider_generation < 0
                || state.revision < 0
                || chrono::DateTime::parse_from_rfc3339(&state.observed_at).is_err()
                || state.currency.len() != 3
                || state.currency != state.currency.to_uppercase()
                || NaiveDate::parse_from_str(&state.as_of_date, "%Y-%m-%d").is_err()
            {
                return Err(BridgePolicyError::InvalidCurrentState(
                    "invalid date or account currency".into(),
                ));
            }
            Decimal::from_str(&state.cash_balance).map_err(|_| {
                BridgePolicyError::InvalidCurrentState("invalid cash balance".into())
            })?;
            let liability = Decimal::from_str(&state.liability_balance).map_err(|_| {
                BridgePolicyError::InvalidCurrentState("invalid liability balance".into())
            })?;
            if liability.is_sign_negative() {
                return Err(BridgePolicyError::InvalidCurrentState(
                    "liability must be a positive debt magnitude".into(),
                ));
            }
            let mut identities = std::collections::HashSet::new();
            for position in &state.positions {
                if position.symbol.trim().is_empty()
                    || position.currency.len() != 3
                    || position.currency != position.currency.to_uppercase()
                    || !identities.insert((
                        position.symbol.trim().to_uppercase(),
                        position.currency.clone(),
                    ))
                {
                    return Err(BridgePolicyError::InvalidCurrentState(
                        "invalid or duplicate position identity".into(),
                    ));
                }
                for value in [&position.quantity, &position.price, &position.market_value] {
                    Decimal::from_str(value).map_err(|_| {
                        BridgePolicyError::InvalidCurrentState("invalid position decimal".into())
                    })?;
                }
                if let Some(value) = &position.cost_basis {
                    Decimal::from_str(value).map_err(|_| {
                        BridgePolicyError::InvalidCurrentState("invalid cost basis".into())
                    })?;
                }
            }
            let supplied = op
                .semantic_hash
                .as_deref()
                .ok_or(BridgePolicyError::SemanticHashMismatch)?;
            if supplied != current_state_semantic_hash(state) {
                return Err(BridgePolicyError::SemanticHashMismatch);
            }
        }
        _ if op.current_state.is_some() || op.semantic_hash.is_some() => {
            return Err(BridgePolicyError::InvalidCurrentState(
                "current state fields are only valid for CurrentStateUpsert".into(),
            ))
        }
        _ => {}
    }
    Ok(())
}

fn upsert_activity_row(
    c: &mut SqliteConnection,
    resource_id: &str,
    subject: &str,
    act: &ActivityPayload,
) -> std::result::Result<(), diesel::result::Error> {
    #[derive(QueryableByName)]
    struct ExistingActivity {
        #[diesel(sql_type = Text)]
        account_id: String,
        #[diesel(sql_type = Nullable<Text>)]
        metadata: Option<String>,
    }

    let existing = sql_query("SELECT account_id, metadata FROM activities WHERE id=?")
        .bind::<Text, _>(resource_id)
        .get_result::<ExistingActivity>(c)
        .optional()?;
    if existing
        .as_ref()
        .is_some_and(|row| row.account_id != act.account_id)
    {
        return Err(diesel::result::Error::RollbackTransaction);
    }

    let mut metadata = existing
        .as_ref()
        .and_then(|row| row.metadata.as_deref())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let root = metadata.as_object_mut().expect("metadata object");
    let incoming_provider_owned = act
        .metadata
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .and_then(|value| value.get("providerOwned"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    root.insert("providerOwned".into(), incoming_provider_owned);
    let provider_owned = root
        .get_mut("providerOwned")
        .and_then(serde_json::Value::as_object_mut)
        .expect("provider-owned object");
    if let Some(context) = &act.merchant_context {
        provider_owned.insert(
            "merchantContext".into(),
            serde_json::json!({
                "merchantKey": context.merchant_key,
                "merchantKeyVersion": context.merchant_key_version,
                "normalizationVersion": context.normalization_version,
            }),
        );
    } else {
        provider_owned.remove("merchantContext");
    }
    let mut original = serde_json::Map::new();
    if let Some(value) = &act.original_amount {
        original.insert("amount".into(), serde_json::Value::String(value.clone()));
    }
    if let Some(value) = &act.original_currency {
        original.insert("currency".into(), serde_json::Value::String(value.clone()));
    }
    if !original.is_empty() {
        provider_owned.insert("original".into(), serde_json::Value::Object(original));
    } else {
        provider_owned.remove("original");
    }
    let mut settlement = serde_json::Map::new();
    if let Some(value) = &act.settlement_amount {
        settlement.insert("amount".into(), serde_json::Value::String(value.clone()));
    }
    if let Some(value) = &act.settlement_currency {
        settlement.insert("currency".into(), serde_json::Value::String(value.clone()));
    }
    if !settlement.is_empty() {
        provider_owned.insert("settlement".into(), serde_json::Value::Object(settlement));
    } else {
        provider_owned.remove("settlement");
    }
    let metadata = if provider_owned.is_empty() && root.len() == 1 {
        None
    } else {
        Some(metadata.to_string())
    };

    sql_query(
        "INSERT INTO activities(id,account_id,activity_type,status,activity_date,settlement_date,source_type,subtype,amount,fee,tax,currency,fx_rate,notes,activity_type_override,metadata,source_system,source_record_id,source_group_id,is_user_modified,needs_review,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,NULL,?,'FINANCE',?,?,0,COALESCE(?,0),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,activity_type=CASE WHEN activities.is_user_modified=1 THEN activities.activity_type ELSE excluded.activity_type END,status=excluded.status,activity_date=excluded.activity_date,settlement_date=excluded.settlement_date,source_type=excluded.source_type,subtype=CASE WHEN activities.is_user_modified=1 THEN activities.subtype ELSE excluded.subtype END,amount=excluded.amount,fee=excluded.fee,tax=excluded.tax,currency=excluded.currency,fx_rate=excluded.fx_rate,notes=CASE WHEN activities.is_user_modified=1 THEN activities.notes ELSE excluded.notes END,metadata=coalesce(excluded.metadata,activities.metadata),source_system=excluded.source_system,source_record_id=excluded.source_record_id,source_group_id=CASE WHEN activities.is_user_modified=1 THEN activities.source_group_id ELSE excluded.source_group_id END,needs_review=CASE WHEN activities.is_user_modified=1 THEN activities.needs_review ELSE COALESCE(excluded.needs_review,0) END,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')"
    )
        .bind::<Text,_>(resource_id)
        .bind::<Text,_>(&act.account_id)
        .bind::<Text,_>(&act.activity_type)
        .bind::<Text,_>(&act.status)
        .bind::<Text,_>(&act.activity_date)
        .bind::<Nullable<Text>,_>(act.settlement_date.as_deref())
        .bind::<Nullable<Text>,_>(act.source_type.as_deref())
        .bind::<Nullable<Text>,_>(act.subtype.as_deref())
        .bind::<Text,_>(&act.amount)
        .bind::<Nullable<Text>,_>(act.fee.as_deref())
        .bind::<Nullable<Text>,_>(act.tax.as_deref())
        .bind::<Text,_>(&act.currency)
        .bind::<Nullable<Text>,_>(act.fx_rate.as_deref())
        .bind::<Nullable<Text>,_>(act.notes.as_deref())
        .bind::<Nullable<Text>,_>(metadata)
        .bind::<Nullable<Text>,_>(act.source_record_id.as_deref())
        .bind::<Nullable<Text>,_>(act.source_group_id.as_deref())
        .bind::<Nullable<Integer>,_>(act.needs_review.map(i32::from))
        .execute(c)?;
    sql_query("INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity',?,'live_canonical','finance',?,?) ON CONFLICT(entity_type,entity_id) DO UPDATE SET role='live_canonical',source_system='finance',source_record_id=excluded.source_record_id,actor_id=excluded.actor_id,updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')")
        .bind::<Text, _>(resource_id)
        .bind::<Text, _>(act.source_record_id.as_deref().unwrap_or(resource_id))
        .bind::<Text, _>(subject)
        .execute(c)?;
    Ok(())
}

#[derive(QueryableByName)]
struct ActivitySupersessionRow {
    #[diesel(sql_type = Text)]
    activity_type: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    notes: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    activity_type_override: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    subtype: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    metadata: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source_record_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source_group_id: Option<String>,
    #[diesel(sql_type = Integer)]
    is_user_modified: i32,
    #[diesel(sql_type = Integer)]
    needs_review: i32,
}

fn activity_supersession_row(
    c: &mut SqliteConnection,
    id: &str,
) -> std::result::Result<Option<ActivitySupersessionRow>, diesel::result::Error> {
    sql_query("SELECT activity_type,account_id,notes,activity_type_override,subtype,metadata,source_record_id,source_group_id,is_user_modified,needs_review FROM activities WHERE id=?")
        .bind::<Text, _>(id)
        .get_result::<ActivitySupersessionRow>(c)
        .optional()
}

fn user_metadata_conflict(target: &serde_json::Value, loser: &serde_json::Value) -> bool {
    match (target, loser) {
        (serde_json::Value::Object(target), serde_json::Value::Object(loser)) => loser
            .iter()
            .filter(|(key, _)| key.as_str() != "providerOwned")
            .any(|(key, value)| {
                target
                    .get(key)
                    .is_some_and(|existing| user_metadata_conflict(existing, value))
            }),
        _ => target != loser,
    }
}

fn merge_metadata_value(target: &mut serde_json::Value, loser: &serde_json::Value) {
    match (target, loser) {
        (serde_json::Value::Object(target), serde_json::Value::Object(loser)) => {
            for (key, value) in loser {
                if key == "providerOwned" {
                    continue;
                }
                match target.get_mut(key) {
                    Some(existing) => merge_metadata_value(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        // The canonical successor wins whenever both sides contain a scalar or
        // incompatible object. The loser only fills absent nested provider/user keys.
        _ => {}
    }
}

fn merge_activity_metadata(
    target: Option<&str>,
    loser: Option<&str>,
) -> std::result::Result<Option<String>, diesel::result::Error> {
    let mut target_value = target
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let loser_value = loser
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .filter(serde_json::Value::is_object);
    if let Some(loser_value) = loser_value {
        if user_metadata_conflict(&target_value, &loser_value) {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        merge_metadata_value(&mut target_value, &loser_value);
    }
    if target_value
        .as_object()
        .is_some_and(|value| !value.is_empty())
    {
        Ok(Some(target_value.to_string()))
    } else {
        Ok(None)
    }
}

fn child_count(
    c: &mut SqliteConnection,
    sql: &str,
    loser: &str,
    canonical: &str,
) -> std::result::Result<i64, diesel::result::Error> {
    Ok(sql_query(sql)
        .bind::<Text, _>(loser)
        .bind::<Text, _>(canonical)
        .get_result::<Count>(c)?
        .count)
}

fn move_activity_children(
    c: &mut SqliteConnection,
    loser: &str,
    canonical: &str,
    deployment_id: &str,
    subject: &str,
) -> std::result::Result<(), diesel::result::Error> {
    let assignment_conflicts = child_count(
        c,
        "SELECT count(*) AS count FROM activity_taxonomy_assignments loser JOIN activity_taxonomy_assignments canonical ON loser.activity_id=? AND canonical.activity_id=? AND canonical.taxonomy_id=loser.taxonomy_id",
        loser,
        canonical,
    )?;
    if assignment_conflicts > 0 {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    if child_count(c, "SELECT count(*) AS count FROM spending_activity_splits loser WHERE loser.activity_id=? AND EXISTS (SELECT 1 FROM spending_activity_splits canonical WHERE canonical.activity_id=?)", loser, canonical)? > 0 {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    if child_count(c, "SELECT count(*) AS count FROM spending_activity_events loser WHERE loser.activity_id=? AND EXISTS (SELECT 1 FROM spending_activity_events canonical WHERE canonical.activity_id=?)", loser, canonical)? > 0 {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    if child_count(c, "SELECT count(*) AS count FROM activity_attribution_overlays loser WHERE loser.activity_id=? AND EXISTS (SELECT 1 FROM activity_attribution_overlays canonical WHERE canonical.activity_id=?)", loser, canonical)? > 0 {
        return Err(diesel::result::Error::RollbackTransaction);
    }

    sql_query("UPDATE activity_taxonomy_assignments SET activity_id=? WHERE activity_id=?")
        .bind::<Text, _>(canonical)
        .bind::<Text, _>(loser)
        .execute(c)?;
    sql_query("UPDATE spending_activity_splits SET activity_id=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE activity_id=?")
        .bind::<Text, _>(canonical)
        .bind::<Text, _>(loser)
        .execute(c)?;
    sql_query("UPDATE spending_activity_events SET activity_id=?,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE activity_id=?")
        .bind::<Text, _>(canonical)
        .bind::<Text, _>(loser)
        .execute(c)?;
    sql_query("UPDATE activity_attribution_overlays SET activity_id=? WHERE deployment_id=? AND activity_id=?")
        .bind::<Text, _>(canonical)
        .bind::<Text, _>(deployment_id)
        .bind::<Text, _>(loser)
        .execute(c)?;

    // Disclosure projections may exist for more than the currently delivering
    // subject. Move all non-conflicting rows, then let the current operation
    // refresh its own generation below.
    sql_query("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) SELECT old.deployment_id,old.person_subject_id,?,old.account_id,old.access_generation FROM activity_disclosure_projections old WHERE old.activity_id=? AND NOT EXISTS (SELECT 1 FROM activity_disclosure_projections current WHERE current.deployment_id=old.deployment_id AND current.person_subject_id=old.person_subject_id AND current.activity_id=?)")
        .bind::<Text, _>(canonical)
        .bind::<Text, _>(loser)
        .bind::<Text, _>(canonical)
        .execute(c)?;
    sql_query("DELETE FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id=? AND activity_id=?")
        .bind::<Text, _>(deployment_id)
        .bind::<Text, _>(subject)
        .bind::<Text, _>(loser)
        .execute(c)?;
    Ok(())
}

fn is_legacy_provider_identity_correction(
    loser: &str,
    loser_row: &ActivitySupersessionRow,
    canonical_row: &ActivitySupersessionRow,
) -> bool {
    // Finance Sync's exact PrimeSaver correction can replace a pre-0013
    // synthetic resource that has no source_record_id with the later
    // provider-referenced synthetic-v2 resource. This is deliberately narrow:
    // the receiver still requires Finance provenance, and the source side must
    // explicitly name the loser in superseded_resource_ids.
    loser.contains(":synthetic-v2:")
        && loser_row.source_record_id.is_none()
        && canonical_row
            .source_record_id
            .as_deref()
            .is_some_and(|value| value.starts_with("synthetic-v2:"))
}

fn append_superseded_provider_note(metadata: Option<String>, note: Option<&str>) -> Option<String> {
    let Some(note) = note.map(str::trim).filter(|value| !value.is_empty()) else {
        return metadata;
    };
    let mut value = metadata
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let root = value.as_object_mut().expect("metadata object");
    let provider_owned = root
        .entry("providerOwned")
        .or_insert_with(|| serde_json::json!({}));
    let provider_owned = provider_owned
        .as_object_mut()
        .expect("provider-owned metadata object");
    let notes = provider_owned
        .entry("supersededProviderNotes")
        .or_insert_with(|| serde_json::json!([]));
    let notes = notes
        .as_array_mut()
        .expect("superseded provider notes array");
    if !notes.iter().any(|value| value.as_str() == Some(note)) {
        notes.push(serde_json::Value::String(note.to_owned()));
    }
    Some(value.to_string())
}

fn supersede_activity_identity(
    c: &mut SqliteConnection,
    loser: &str,
    canonical: &str,
    account_id: &str,
    deployment_id: &str,
    subject: &str,
) -> std::result::Result<(), diesel::result::Error> {
    if loser == canonical {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let Some(loser_row) = activity_supersession_row(c, loser)? else {
        return Ok(());
    };
    let canonical_row =
        activity_supersession_row(c, canonical)?.ok_or(diesel::result::Error::NotFound)?;
    let legacy_provider_identity_correction =
        is_legacy_provider_identity_correction(loser, &loser_row, &canonical_row);
    if loser_row.account_id != account_id || canonical_row.account_id != account_id {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let loser_is_provider_owned = sql_query(
        "SELECT count(*) AS count FROM provenance_records WHERE entity_type='activity' AND entity_id=? AND role='live_canonical' AND source_system='finance'",
    )
    .bind::<Text, _>(loser)
    .get_result::<Count>(c)?
    .count
        == 1;
    if !loser_is_provider_owned {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    let loser_effective_activity_type = loser_row
        .activity_type_override
        .clone()
        .unwrap_or_else(|| loser_row.activity_type.clone());
    let canonical_effective_activity_type = canonical_row
        .activity_type_override
        .clone()
        .unwrap_or_else(|| canonical_row.activity_type.clone());
    if loser_row.is_user_modified == 1 && canonical_row.is_user_modified == 1 {
        if loser_effective_activity_type != canonical_effective_activity_type {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        for (field, left, right) in [
            ("notes", &loser_row.notes, &canonical_row.notes),
            (
                "activity_type_override",
                &loser_row.activity_type_override,
                &canonical_row.activity_type_override,
            ),
            ("subtype", &loser_row.subtype, &canonical_row.subtype),
            (
                "source_group_id",
                &loser_row.source_group_id,
                &canonical_row.source_group_id,
            ),
        ] {
            if field == "notes" && legacy_provider_identity_correction {
                continue;
            }
            if left.is_some() && right.is_some() && left != right {
                return Err(diesel::result::Error::RollbackTransaction);
            }
        }
    }
    if loser_row.source_group_id.is_some()
        && canonical_row.source_group_id.is_some()
        && loser_row.source_group_id != canonical_row.source_group_id
    {
        return Err(diesel::result::Error::RollbackTransaction);
    }

    move_activity_children(c, loser, canonical, deployment_id, subject)?;
    let notes = if canonical_row.is_user_modified == 1 {
        canonical_row.notes.clone().or(loser_row.notes.clone())
    } else if loser_row.is_user_modified == 1 {
        loser_row.notes.clone().or(canonical_row.notes.clone())
    } else {
        canonical_row.notes.clone().or(loser_row.notes.clone())
    };
    let activity_type_override =
        if canonical_row.is_user_modified == 0 && loser_row.is_user_modified == 1 {
            // A native transfer link may store its effective direction in
            // activity_type with no override. Preserve that user-owned semantic
            // when replacing the provider resource with a new base row.
            Some(loser_effective_activity_type)
        } else {
            canonical_row
                .activity_type_override
                .clone()
                .or(loser_row.activity_type_override.clone())
        };
    let subtype = if canonical_row.is_user_modified == 1 {
        canonical_row.subtype.clone().or(loser_row.subtype.clone())
    } else if loser_row.is_user_modified == 1 {
        loser_row.subtype.clone().or(canonical_row.subtype.clone())
    } else {
        canonical_row.subtype.clone().or(loser_row.subtype.clone())
    };
    let metadata = append_superseded_provider_note(
        merge_activity_metadata(
            canonical_row.metadata.as_deref(),
            loser_row.metadata.as_deref(),
        )?,
        if legacy_provider_identity_correction
            && loser_row.notes.is_some()
            && loser_row.notes != canonical_row.notes
        {
            loser_row.notes.as_deref()
        } else {
            None
        },
    );
    let source_group_id = canonical_row
        .source_group_id
        .clone()
        .or(loser_row.source_group_id.clone());
    sql_query("UPDATE activities SET notes=?,activity_type_override=?,subtype=?,metadata=?,source_group_id=?,is_user_modified=max(is_user_modified,?),needs_review=max(needs_review,?),updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
        .bind::<Nullable<Text>, _>(notes.as_deref())
        .bind::<Nullable<Text>, _>(activity_type_override.as_deref())
        .bind::<Nullable<Text>, _>(subtype.as_deref())
        .bind::<Nullable<Text>, _>(metadata.as_deref())
        .bind::<Nullable<Text>, _>(source_group_id.as_deref())
        .bind::<Integer, _>(loser_row.is_user_modified)
        .bind::<Integer, _>(loser_row.needs_review)
        .bind::<Text, _>(canonical)
        .execute(c)?;
    sql_query("DELETE FROM activities WHERE id=?")
        .bind::<Text, _>(loser)
        .execute(c)?;
    Ok(())
}

fn reject_conflicting_activity_overlays(
    c: &mut SqliteConnection,
    pending_id: &str,
    posted_id: &str,
    deployment_id: &str,
) -> std::result::Result<(), diesel::result::Error> {
    let conflicts = sql_query(
        "SELECT count(*) count FROM activities pending JOIN activities posted \
         WHERE pending.id=? AND posted.id=? AND (\
           (pending.is_user_modified=1 AND posted.is_user_modified=1 AND pending.notes IS NOT NULL AND posted.notes IS NOT NULL AND pending.notes<>posted.notes) OR \
           (pending.activity_type_override IS NOT NULL AND posted.activity_type_override IS NOT NULL AND pending.activity_type_override<>posted.activity_type_override) OR \
           (pending.is_user_modified=1 AND posted.is_user_modified=1 AND pending.subtype IS NOT NULL AND posted.subtype IS NOT NULL AND pending.subtype<>posted.subtype) OR \
           EXISTS (SELECT 1 FROM activity_attribution_overlays predecessor \
                   JOIN activity_attribution_overlays successor \
                     ON successor.deployment_id=predecessor.deployment_id \
                  WHERE predecessor.deployment_id=? AND predecessor.activity_id=pending.id \
                    AND successor.activity_id=posted.id \
                    AND predecessor.attributed_person_subject_id IS NOT NULL \
                    AND successor.attributed_person_subject_id IS NOT NULL \
                    AND predecessor.attributed_person_subject_id<>successor.attributed_person_subject_id)\
         )",
    )
    .bind::<Text, _>(pending_id)
    .bind::<Text, _>(posted_id)
    .bind::<Text, _>(deployment_id)
    .get_result::<Count>(c)?
    .count;
    if conflicts != 0 {
        return Err(diesel::result::Error::RollbackTransaction);
    }
    Ok(())
}

fn do_upsert_activity(
    c: &mut SqliteConnection,
    op: &DisclosureOperation,
    actual: i64,
    account_id: &str,
    d: &str,
) -> std::result::Result<(), diesel::result::Error> {
    if let Some(ref act) = op.activity {
        upsert_activity_row(c, &op.resource_id, &op.subject, act)?;
    }
    sql_query(
        "INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?) ON CONFLICT(deployment_id,person_subject_id,activity_id) DO UPDATE SET account_id=excluded.account_id,access_generation=excluded.access_generation"
    )
        .bind::<Text,_>(d)
        .bind::<Text,_>(&op.subject)
        .bind::<Text,_>(&op.resource_id)
        .bind::<Text,_>(account_id)
        .bind::<BigInt,_>(actual)
        .execute(c)?;
    Ok(())
}

fn do_delete_activity(
    c: &mut SqliteConnection,
    op: &DisclosureOperation,
    d: &str,
) -> std::result::Result<(), diesel::result::Error> {
    let provider_owned = sql_query("SELECT count(*) AS count FROM provenance_records WHERE entity_type='activity' AND entity_id=? AND role='live_canonical' AND source_system='finance'")
        .bind::<Text, _>(&op.resource_id)
        .get_result::<Count>(c)?
        .count > 0;
    if provider_owned {
        // Keep provider-owned rows as reversible tombstones. A later unchanged
        // ActivityUpsert can restore the row and all user-owned child overlays.
        sql_query("UPDATE activities SET status='VOID',needs_review=0,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
            .bind::<Text, _>(&op.resource_id)
            .execute(c)?;
    } else {
        sql_query("DELETE FROM activities WHERE id=?")
            .bind::<Text, _>(&op.resource_id)
            .execute(c)?;
    }
    sql_query("DELETE FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id=? AND activity_id=?")
        .bind::<Text,_>(d)
        .bind::<Text,_>(&op.subject)
        .bind::<Text,_>(&op.resource_id)
        .execute(c)?;
    Ok(())
}

#[derive(QueryableByName)]
struct AssetIdRow {
    #[diesel(sql_type = Text)]
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotProvenance {
    provider_connection_id: String,
    provider_incarnation_id: String,
    provider_generation: i64,
    observed_at: String,
    revision: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurrentStateWriteDecision {
    Apply,
    Superseded,
    Reject,
}

fn current_state_write_decision(
    c: &mut SqliteConnection,
    state: &CurrentStatePayload,
) -> std::result::Result<CurrentStateWriteDecision, diesel::result::Error> {
    let snapshot_id = wealthfolio_core::portfolio::snapshot::AccountStateSnapshot::stable_id(
        &state.account_id,
        chrono::NaiveDate::parse_from_str(&state.as_of_date, "%Y-%m-%d").expect("validated date"),
    );
    let Some(existing) = sql_query("SELECT snapshot.source, provenance.source_record_id FROM holdings_snapshots snapshot LEFT JOIN provenance_records provenance ON provenance.entity_type='snapshot' AND provenance.entity_id=snapshot.id WHERE snapshot.id=?")
        .bind::<Text, _>(&snapshot_id)
        .get_result::<ExistingSnapshot>(c)
        .optional()?
    else {
        return Ok(CurrentStateWriteDecision::Apply);
    };
    let Some(prior) = existing
        .source_record_id
        .as_deref()
        .and_then(|value| serde_json::from_str::<SnapshotProvenance>(value).ok())
    else {
        return Ok(CurrentStateWriteDecision::Reject);
    };
    if existing.source != "BROKER_IMPORTED"
        || prior.provider_connection_id != state.provider_connection_id
        || prior.provider_incarnation_id != state.provider_incarnation_id
        || state.provider_generation < prior.provider_generation
    {
        return Ok(CurrentStateWriteDecision::Reject);
    }
    let incoming_observed =
        chrono::DateTime::parse_from_rfc3339(&state.observed_at).expect("validated observation");
    let Ok(prior_observed) = chrono::DateTime::parse_from_rfc3339(&prior.observed_at) else {
        return Ok(CurrentStateWriteDecision::Reject);
    };
    if (incoming_observed, state.revision) > (prior_observed, prior.revision) {
        Ok(CurrentStateWriteDecision::Apply)
    } else {
        Ok(CurrentStateWriteDecision::Superseded)
    }
}

#[derive(QueryableByName)]
struct ExistingSnapshot {
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_record_id: Option<String>,
}

fn resolve_current_state_assets(
    c: &mut SqliteConnection,
    state: &CurrentStatePayload,
) -> Result<Vec<(CurrentStatePosition, String)>> {
    let mut resolved = Vec::with_capacity(state.positions.len());
    for position in &state.positions {
        let symbol = position.symbol.trim().to_uppercase();
        // Provider positions may only resolve through an immutable, provider-owned
        // mapping. Never co-opt a user's generic symbol/currency asset and never
        // manufacture a MANUAL asset in this ingestion transaction.
        let rows = sql_query("SELECT id FROM assets WHERE is_active=1 AND json_extract(provider_config,'$.providerConnectionId')=? AND json_extract(provider_config,'$.providerIncarnationId')=? AND json_extract(provider_config,'$.providerSymbol')=? AND json_extract(provider_config,'$.providerCurrency')=? ORDER BY id")
            .bind::<Text, _>(&state.provider_connection_id)
            .bind::<Text, _>(&state.provider_incarnation_id)
            .bind::<Text, _>(&symbol)
            .bind::<Text, _>(&position.currency)
            .load::<AssetIdRow>(c)
            .map_err(sqlerr)?;
        if rows.len() != 1 {
            return Err(BridgePolicyError::AmbiguousAsset);
        }
        resolved.push((position.clone(), rows[0].id.clone()));
    }
    Ok(resolved)
}

fn persist_current_state(
    c: &mut SqliteConnection,
    op: &DisclosureOperation,
    resolved: &[(CurrentStatePosition, String)],
) -> std::result::Result<(), diesel::result::Error> {
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let state = op.current_state.as_ref().expect("validated current state");
    let snapshot_id = wealthfolio_core::portfolio::snapshot::AccountStateSnapshot::stable_id(
        &state.account_id,
        chrono::NaiveDate::parse_from_str(&state.as_of_date, "%Y-%m-%d").expect("validated date"),
    );
    let now = chrono::Utc::now();

    if !matches!(
        current_state_write_decision(c, state)?,
        CurrentStateWriteDecision::Apply
    ) {
        return Err(diesel::result::Error::RollbackTransaction);
    }

    // Provider current state owns its account/date. A generated duplicate can
    // otherwise win an unordered same-day lookup and zero the visible balance.
    sql_query("DELETE FROM holdings_snapshots WHERE account_id=? AND snapshot_date=? AND source='CALCULATED'")
        .bind::<Text, _>(&state.account_id)
        .bind::<Text, _>(&state.as_of_date)
        .execute(c)?;

    let mut positions = serde_json::Map::new();
    let mut market_values = serde_json::Map::new();
    let mut total_cost_basis = Decimal::ZERO;

    sql_query("INSERT INTO holdings_snapshots(id,account_id,snapshot_date,currency,source) VALUES(?,?,?,?,'BROKER_IMPORTED') ON CONFLICT(id) DO NOTHING")
        .bind::<Text, _>(&snapshot_id)
        .bind::<Text, _>(&state.account_id)
        .bind::<Text, _>(&state.as_of_date)
        .bind::<Text, _>(&state.currency)
        .execute(c)?;
    sql_query("DELETE FROM snapshot_positions WHERE snapshot_id=?")
        .bind::<Text, _>(&snapshot_id)
        .execute(c)?;

    for (position, asset_id) in resolved {
        let quantity = Decimal::from_str(&position.quantity).expect("validated quantity");
        let price = Decimal::from_str(&position.price).expect("validated price");
        let cost_basis = position
            .cost_basis
            .as_deref()
            .map(|v| Decimal::from_str(v).expect("validated cost basis"))
            .unwrap_or(Decimal::ZERO);
        total_cost_basis += cost_basis;
        let position_json = serde_json::json!({
            "id": format!("POS-{asset_id}-{}", state.account_id),
            "accountId": state.account_id,
            "assetId": asset_id,
            "quantity": quantity,
            "averageCost": if quantity.is_zero() { Decimal::ZERO } else { cost_basis / quantity },
            "totalCostBasis": cost_basis,
            "currency": position.currency,
            "inceptionDate": now,
            "lots": [],
            "createdAt": now,
            "lastUpdated": now,
            "isAlternative": false,
            "contractMultiplier": Decimal::ONE,
        });
        positions.insert(asset_id.clone(), position_json);
        // Keep canonical provider precision. Serializing Decimal as a JSON
        // number passes through serde_json's f64 representation and truncates
        // significant digits; snapshot readers accept decimal strings.
        market_values.insert(
            asset_id.clone(),
            serde_json::Value::String(position.market_value.clone()),
        );

        sql_query("INSERT INTO snapshot_positions(snapshot_id,asset_id,quantity,average_cost,total_cost_basis,currency,inception_date,is_alternative,contract_multiplier,created_at,last_updated) VALUES(?,?,?,?,?,?,?,0,'1',?,?)")
            .bind::<Text, _>(&snapshot_id)
            .bind::<Text, _>(asset_id)
            .bind::<Text, _>(&position.quantity)
            .bind::<Text, _>(price.to_string())
            .bind::<Text, _>(cost_basis.to_string())
            .bind::<Text, _>(&position.currency)
            .bind::<Text, _>(now.to_rfc3339())
            .bind::<Text, _>(now.to_rfc3339())
            .bind::<Text, _>(now.to_rfc3339())
            .execute(c)?;
    }
    let cash = Decimal::from_str(&state.cash_balance).expect("validated cash");
    let cash_json = serde_json::json!({ state.currency.clone(): cash }).to_string();
    sql_query("INSERT INTO holdings_snapshots(id,account_id,snapshot_date,currency,positions,cash_balances,cost_basis,net_contribution,calculated_at,net_contribution_base,cash_total_account_currency,cash_total_base_currency,position_market_values,liability_balance,source) VALUES(?,?,?,?,?,?,?,'0',?,'0',?,?,?,?,'BROKER_IMPORTED') ON CONFLICT(id) DO UPDATE SET currency=excluded.currency,positions=excluded.positions,cash_balances=excluded.cash_balances,cost_basis=excluded.cost_basis,calculated_at=excluded.calculated_at,cash_total_account_currency=excluded.cash_total_account_currency,cash_total_base_currency=excluded.cash_total_base_currency,position_market_values=excluded.position_market_values,liability_balance=excluded.liability_balance,source=excluded.source")
        .bind::<Text, _>(&snapshot_id)
        .bind::<Text, _>(&state.account_id)
        .bind::<Text, _>(&state.as_of_date)
        .bind::<Text, _>(&state.currency)
        .bind::<Text, _>(serde_json::Value::Object(positions).to_string())
        .bind::<Text, _>(&cash_json)
        .bind::<Text, _>(total_cost_basis.to_string())
        .bind::<Text, _>(now.naive_utc().format("%Y-%m-%dT%H:%M:%S%.fZ").to_string())
        .bind::<Text, _>(cash.to_string())
        .bind::<Text, _>(cash.to_string())
        .bind::<Text, _>(serde_json::Value::Object(market_values).to_string())
        .bind::<Text, _>(&state.liability_balance)
        .execute(c)?;
    let provenance = serde_json::json!({
        "providerConnectionId": state.provider_connection_id,
        "providerIncarnationId": state.provider_incarnation_id,
        "providerGeneration": state.provider_generation,
        "observedAt": state.observed_at,
        "revision": state.revision,
        "semanticHash": op.semantic_hash.as_deref().expect("validated semantic hash"),
    })
    .to_string();
    sql_query("INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('snapshot',?,'live_canonical','finance',?,?) ON CONFLICT(entity_type,entity_id) DO UPDATE SET role='live_canonical',source_system='finance',source_record_id=excluded.source_record_id,actor_id=excluded.actor_id,updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')")
        .bind::<Text, _>(&snapshot_id)
        .bind::<Text, _>(provenance)
        .bind::<Text, _>(&op.subject)
        .execute(c)?;
    sql_query("INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('account',?,'live_canonical','finance',?,?) ON CONFLICT(entity_type,entity_id) DO UPDATE SET role='live_canonical',source_system='finance',source_record_id=excluded.source_record_id,actor_id=excluded.actor_id,updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')")
        .bind::<Text, _>(&state.account_id)
        .bind::<Text, _>(&state.provider_connection_id)
        .bind::<Text, _>(&op.subject)
        .execute(c)?;
    sql_query("INSERT INTO verified_receipts(id,account_id,basis,as_of,provider_receipt_ref,verification_state) VALUES(?,?,'current_snapshot',?,?,'verified') ON CONFLICT(id) DO UPDATE SET as_of=excluded.as_of,provider_receipt_ref=excluded.provider_receipt_ref,verification_state='verified',updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')")
        .bind::<Text, _>(format!("finance-current-state:{}", state.account_id))
        .bind::<Text, _>(&state.account_id)
        .bind::<Text, _>(&state.as_of_date)
        .bind::<Text, _>(&op.idempotency_key)
        .execute(c)?;
    sql_query("UPDATE accounts SET include_in_net_worth=1,updated_at=CURRENT_TIMESTAMP WHERE id=?")
        .bind::<Text, _>(&state.account_id)
        .execute(c)?;
    sql_query("INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) SELECT 'daily_valuation',id,'live_canonical','WEALTHFOLIO_DERIVED',NULL,? FROM daily_account_valuation WHERE account_id=? ON CONFLICT(entity_type,entity_id) DO UPDATE SET role='live_canonical',source_system='WEALTHFOLIO_DERIVED',actor_id=excluded.actor_id,updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')")
        .bind::<Text, _>(&op.subject)
        .bind::<Text, _>(&state.account_id)
        .execute(c)?;
    Ok(())
}

#[derive(Clone)]
pub struct BridgePolicyStore {
    deployment_id: String,
    pool: std::sync::Arc<DbPool>,
    writer: WriteHandle,
}
impl BridgePolicyStore {
    pub fn new(
        deployment_id: impl Into<String>,
        pool: std::sync::Arc<DbPool>,
        writer: WriteHandle,
    ) -> Self {
        Self {
            deployment_id: deployment_id.into(),
            pool,
            writer,
        }
    }
    pub async fn grant(&self, subject: &str, account: &str) -> Result<i64> {
        let (d, s, a) = (
            self.deployment_id.clone(),
            subject.to_owned(),
            account.to_owned(),
        );
        self.writer.exec(move|c|Ok(c.transaction::<_,crate::StorageError,_>(|c|{let h=sql_query("SELECT arrangement.household_id FROM account_ownership_arrangements arrangement JOIN account_ownership_participants participant ON participant.deployment_id=arrangement.deployment_id AND participant.arrangement_id=arrangement.id JOIN household_membership_epochs membership ON membership.deployment_id=arrangement.deployment_id AND membership.household_id=arrangement.household_id AND membership.person_subject_id=participant.person_subject_id WHERE arrangement.deployment_id=? AND arrangement.account_id=? AND arrangement.end_epoch IS NULL AND participant.person_subject_id=? AND membership.end_epoch IS NULL AND membership.role='owner'").bind::<Text,_>(&d).bind::<Text,_>(&a).bind::<Text,_>(&s).get_result::<Household>(c)?.household_id;sql_query("UPDATE policy_deployments SET access_generation=access_generation+1 WHERE id=?").bind::<Text,_>(&d).execute(c)?;let g=sql_query("SELECT access_generation generation FROM policy_deployments WHERE id=?").bind::<Text,_>(&d).get_result::<Gen>(c)?.generation;sql_query("INSERT INTO account_access_epochs(deployment_id,household_id,account_id,person_subject_id,start_generation) SELECT ?,?,?,?,? WHERE NOT EXISTS (SELECT 1 FROM account_access_epochs WHERE deployment_id=? AND account_id=? AND person_subject_id=? AND end_generation IS NULL)").bind::<Text,_>(&d).bind::<Text,_>(&h).bind::<Text,_>(&a).bind::<Text,_>(&s).bind::<BigInt,_>(g).bind::<Text,_>(&d).bind::<Text,_>(&a).bind::<Text,_>(&s).execute(c)?;Ok(g)})?)).await.map_err(err)
    }
    pub fn get_receipt(&self, subject: &str, key: &str) -> Result<Option<DisclosureReceipt>> {
        let mut connection = self
            .pool
            .get()
            .map_err(|error| BridgePolicyError::Storage(error.to_string()))?;
        let row = sql_query("SELECT person_subject_id subject,idempotency_key,access_generation,operation,resource_id,account_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome FROM disclosure_receipts WHERE deployment_id=? AND person_subject_id=? AND idempotency_key=?")
            .bind::<Text, _>(&self.deployment_id)
            .bind::<Text, _>(subject)
            .bind::<Text, _>(key)
            .get_result::<ReceiptRow>(&mut connection)
            .optional()
            .map_err(sqlerr)?;
        Ok(row.map(|value| receipt(&self.deployment_id, value)))
    }

    pub async fn commit(&self, op: DisclosureOperation) -> Result<DisclosureReceipt> {
        self.commit_with_fault(op, CommitFault::None).await
    }
    pub async fn commit_with_fault(
        &self,
        op: DisclosureOperation,
        fault: CommitFault,
    ) -> Result<DisclosureReceipt> {
        let d = self.deployment_id.clone();
        let result: std::result::Result<DisclosureReceipt, BridgePolicyError> = self.writer.exec(move |c| {
            let inner: std::result::Result<DisclosureReceipt, BridgePolicyError> = c.transaction::<_, crate::StorageError, _>(|c| {
                let payload_digest = disclosure_payload_digest(&op);
                // Resolve exact durable replays before checking the current generation. A
                // completed lifecycle removes the pending source row, and a later revoke may
                // advance the deployment generation without invalidating its durable receipt.
                if let Some(existing) = sql_query("SELECT person_subject_id subject,idempotency_key,access_generation,operation,resource_id,account_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome FROM disclosure_receipts WHERE deployment_id=? AND person_subject_id=? AND idempotency_key=?")
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&op.subject)
                    .bind::<Text,_>(&op.idempotency_key)
                    .get_result::<ReceiptRow>(c)
                    .optional()?
                {
                    let expected_binding = canonical_receipt_binding(&d, &existing.subject, &existing.idempotency_key, existing.access_generation, &existing.operation, &existing.resource_id, &existing.account_id, existing.lifecycle_successor_id.as_deref(), &existing.outcome, (existing.operation == "current_state_upsert").then_some(existing.payload_digest.as_deref()).flatten());
                    let payload_matches = existing.payload_digest.as_deref() == Some(payload_digest.as_str());
                    if existing.binding_version != RECEIPT_BINDING_VERSION
                        || existing.outcome != "accepted"
                        || existing.operation != op.kind.as_str()
                        || existing.resource_id != op.resource_id
                        || existing.access_generation != op.access_generation
                        || existing.lifecycle_successor_id != op.lifecycle_successor_id
                        || op.activity.as_ref().is_some_and(|a| a.account_id != existing.account_id)
                        || !payload_matches
                        || existing.receipt_binding.as_deref().is_none_or(|b| !is_canonical_receipt_binding(b, &expected_binding))
                    {
                        return Ok(Err(BridgePolicyError::IdempotencyConflict));
                    }
                    return Ok(Ok(receipt(&d, existing)));
                }

                // Validation applies only to new work. Exact durable replays must
                // remain idempotent even when a later schema rule rejects their
                // historical payload shape.
                if let Err(error) = validate_current_state_operation(&op) {
                    return Ok(Err(error));
                }
                if let Some(activity) = op.activity.as_ref() {
                    if let Err(error) = validate_activity_payload(activity) {
                        return Ok(Err(error));
                    }
                }
                if let Err(error) = validate_activity_supersession(&op) {
                    return Ok(Err(error));
                }

                // Reject new work at a stale generation.
                let actual = match sql_query("SELECT access_generation generation FROM policy_deployments WHERE id=?")
                    .bind::<Text,_>(&d)
                    .get_result::<Gen>(c)
                    .optional()?
                    .map(|x| x.generation)
                {
                    Some(g) => g,
                    None => return Ok(Err(BridgePolicyError::MissingGeneration)),
                };
                if actual != op.access_generation {
                    return Ok(Err(BridgePolicyError::StaleGeneration { expected: op.access_generation, actual }));
                }

                // 2. Resolve account
                let account = match op.kind {
                    DisclosureKind::ProjectionFence => Account { account_id: op.resource_id.clone() },
                    DisclosureKind::CurrentStateUpsert => Account {
                        account_id: op.current_state.as_ref().expect("validated current state").account_id.clone(),
                    },
                    _ => {
                        match sql_query("SELECT account_id FROM activities WHERE id=?")
                            .bind::<Text,_>(&op.resource_id)
                            .get_result::<Account>(c)
                            .optional()?
                        {
                            Some(a) => a,
                            None => {
                                // If activity payload is provided, resolve account from it
                                if let Some(ref act) = op.activity {
                                    Account { account_id: act.account_id.clone() }
                                } else {
                                    return Ok(Err(BridgePolicyError::AccessDenied));
                                }
                            }
                        }
                    }
                };

                if op.activity.as_ref().is_some_and(|activity| activity.account_id != account.account_id) {
                    return Ok(Err(BridgePolicyError::ActivityAccountMismatch));
                }

                // 3. Check access
                let allowed = sql_query("SELECT count(*) count FROM account_access_epochs WHERE deployment_id=? AND person_subject_id=? AND account_id=? AND start_generation<=? AND end_generation IS NULL")
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&op.subject)
                    .bind::<Text,_>(&account.account_id)
                    .bind::<BigInt,_>(actual)
                    .get_result::<Count>(c)?
                    .count > 0;
                if !allowed {
                    return Ok(Err(BridgePolicyError::AccessDenied));
                }

                // Concurrent projection workers can deliver an older same-generation
                // snapshot after a newer one. Treat that already-superseded state as
                // an accepted no-op so an idempotent outbox row cannot dead-letter
                // the entire provider cycle.
                let current_state_decision = if let Some(state) = op.current_state.as_ref() {
                    match current_state_write_decision(c, state)? {
                        CurrentStateWriteDecision::Apply => Some(CurrentStateWriteDecision::Apply),
                        CurrentStateWriteDecision::Superseded => {
                            Some(CurrentStateWriteDecision::Superseded)
                        }
                        CurrentStateWriteDecision::Reject => {
                            return Err(diesel::result::Error::RollbackTransaction.into());
                        }
                    }
                } else {
                    None
                };
                let resolved_current_state_assets = if current_state_decision
                    == Some(CurrentStateWriteDecision::Apply)
                {
                    match resolve_current_state_assets(
                        c,
                        op.current_state.as_ref().expect("validated current state"),
                    ) {
                        Ok(value) => value,
                        Err(error) => return Ok(Err(error)),
                    }
                } else {
                    Vec::new()
                };

                if let Some(activity) = op.activity.as_ref() {
                    if let Some(source_record_id) = activity.source_record_id.as_deref() {
                        let existing_provider_rows = sql_query(
                            "SELECT activities.id FROM activities LEFT JOIN provenance_records ON provenance_records.entity_type='activity' AND provenance_records.entity_id=activities.id WHERE activities.account_id=? AND activities.source_record_id=? AND activities.id<>? AND (activities.source_system='FINANCE' OR (provenance_records.role='live_canonical' AND provenance_records.source_system='finance'))",
                        )
                        .bind::<Text, _>(&activity.account_id)
                        .bind::<Text, _>(source_record_id)
                        .bind::<Text, _>(&op.resource_id)
                        .load::<ActivityIdRow>(c)?;
                        let allowed_ids = op
                            .superseded_resource_ids
                            .iter()
                            .chain(op.lifecycle_successor_id.iter())
                            .cloned()
                            .collect::<std::collections::HashSet<_>>();
                        if existing_provider_rows
                            .iter()
                            .any(|row| !allowed_ids.contains(&row.id))
                        {
                            return Ok(Err(BridgePolicyError::InvalidActivity(
                                "provider record identity already belongs to another activity".into(),
                            )));
                        }
                    }
                }

                // 4. Handle provider identity supersession atomically. The new
                // resource is materialized first, then user overlays and child
                // rows move from every still-present legacy identity.
                if !op.superseded_resource_ids.is_empty() {
                    let activity = op.activity.as_ref().expect("validated activity supersession");
                    upsert_activity_row(c, &op.resource_id, &op.subject, activity)?;
                    for loser in &op.superseded_resource_ids {
                        supersede_activity_identity(c, loser, &op.resource_id, &account.account_id, &d, &op.subject)?;
                    }
                }

                // 5. Handle lifecycle migration (pending->posted) atomically
                if let Some(ref successor) = op.lifecycle_successor_id {
                    let p = &op.resource_id;
                    let n = successor;
                    let pending = sql_query("SELECT account_id,status FROM activities WHERE id=?")
                        .bind::<Text,_>(p)
                        .get_result::<ActivityIdentity>(c)?;
                    // A posted observation may be the first sighting of the successor.
                    // Create/update its provider-owned row inside this same transaction,
                    // then migrate the pending row's user-owned overlays onto it.
                    if let Some(ref activity) = op.activity {
                        upsert_activity_row(c, n, &op.subject, activity)?;
                    }
                    let posted = sql_query("SELECT account_id,status FROM activities WHERE id=?")
                        .bind::<Text,_>(n)
                        .get_result::<ActivityIdentity>(c)?;
                    if pending.status != "PENDING"
                        || posted.status != "POSTED"
                        || pending.account_id != posted.account_id
                        || pending.account_id != account.account_id
                    {
                        return Err(diesel::result::Error::NotFound.into());
                    }
                    reject_conflicting_activity_overlays(c, p, n, &d)?;
                    let pending_overlay = activity_supersession_row(c, p)?.ok_or(diesel::result::Error::NotFound)?;
                    let posted_overlay = activity_supersession_row(c, n)?.ok_or(diesel::result::Error::NotFound)?;
                    if pending_overlay.source_group_id.is_some()
                        && posted_overlay.source_group_id.is_some()
                        && pending_overlay.source_group_id != posted_overlay.source_group_id
                    {
                        return Err(diesel::result::Error::RollbackTransaction.into());
                    }
                    let pending_effective_type = pending_overlay
                        .activity_type_override
                        .clone()
                        .unwrap_or_else(|| pending_overlay.activity_type.clone());
                    let activity_type_override = if posted_overlay.is_user_modified == 0
                        && pending_overlay.is_user_modified == 1
                    {
                        Some(pending_effective_type)
                    } else {
                        posted_overlay
                            .activity_type_override
                            .clone()
                            .or(pending_overlay.activity_type_override.clone())
                    };
                    let subtype = if pending_overlay.is_user_modified == 1 {
                        pending_overlay.subtype.clone().or(posted_overlay.subtype.clone())
                    } else {
                        posted_overlay.subtype.clone()
                    };
                    let metadata = merge_activity_metadata(
                        posted_overlay.metadata.as_deref(),
                        pending_overlay.metadata.as_deref(),
                    )?;
                    let source_group_id = posted_overlay
                        .source_group_id
                        .clone()
                        .or(pending_overlay.source_group_id.clone());
                    let projection_count = sql_query("SELECT count(*) AS count FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id=? AND activity_id=? AND account_id=? AND access_generation=?")
                        .bind::<Text,_>(&d)
                        .bind::<Text,_>(&op.subject)
                        .bind::<Text,_>(p)
                        .bind::<Text,_>(&account.account_id)
                        .bind::<BigInt,_>(actual)
                        .get_result::<Count>(c)?.count;
                    if projection_count != 1 {
                        return Err(diesel::result::Error::NotFound.into());
                    }
                    for table in &["activity_taxonomy_assignments", "spending_activity_splits", "spending_activity_events"] {
                        sql_query(format!("UPDATE {table} SET activity_id=? WHERE activity_id=?"))
                            .bind::<Text,_>(n)
                            .bind::<Text,_>(p)
                            .execute(c)?;
                    }
                    let notes = if pending_overlay.is_user_modified == 1 {
                        pending_overlay.notes.as_deref().or(posted_overlay.notes.as_deref())
                    } else {
                        posted_overlay.notes.as_deref().or(pending_overlay.notes.as_deref())
                    };
                    let needs_review = if posted_overlay.is_user_modified == 1
                        && pending_overlay.is_user_modified == 1
                    {
                        std::cmp::max(posted_overlay.needs_review, pending_overlay.needs_review)
                    } else if posted_overlay.is_user_modified == 1 {
                        posted_overlay.needs_review
                    } else if pending_overlay.is_user_modified == 1 {
                        pending_overlay.needs_review
                    } else {
                        std::cmp::max(posted_overlay.needs_review, pending_overlay.needs_review)
                    };
                    sql_query("UPDATE activities SET notes=coalesce(?,notes),activity_type_override=coalesce(?,activity_type_override),subtype=?,metadata=?,source_group_id=?,is_user_modified=max(is_user_modified,?),needs_review=? WHERE id=?")
                        .bind::<Nullable<Text>,_>(notes)
                        .bind::<Nullable<Text>,_>(activity_type_override.as_deref())
                        .bind::<Nullable<Text>,_>(subtype.as_deref())
                        .bind::<Nullable<Text>,_>(metadata.as_deref())
                        .bind::<Nullable<Text>,_>(source_group_id.as_deref())
                        .bind::<Integer,_>(pending_overlay.is_user_modified)
                        .bind::<Integer,_>(needs_review)
                        .bind::<Text,_>(n)
                        .execute(c)?;
                    sql_query("UPDATE activity_disclosure_projections SET activity_id=? WHERE deployment_id=? AND person_subject_id=? AND activity_id=?")
                        .bind::<Text,_>(n)
                        .bind::<Text,_>(&d)
                        .bind::<Text,_>(&op.subject)
                        .bind::<Text,_>(p)
                        .execute(c)?;
                    sql_query("UPDATE activity_attribution_overlays SET activity_id=?,account_id=(SELECT account_id FROM activities WHERE id=?) WHERE deployment_id=? AND activity_id=?")
                        .bind::<Text,_>(n)
                        .bind::<Text,_>(n)
                        .bind::<Text,_>(&d)
                        .bind::<Text,_>(p)
                        .execute(c)?;
                    sql_query("DELETE FROM activities WHERE id=?")
                        .bind::<Text,_>(p)
                        .execute(c)?;
                }

                // 6. Handle activity upsert/delete
                match op.kind {
                    DisclosureKind::ActivityUpsert if op.lifecycle_successor_id.is_some() => {}
                    DisclosureKind::ActivityUpsert => {
                        do_upsert_activity(c, &op, actual, &account.account_id, &d)?;
                    }
                    DisclosureKind::ActivityDelete => {
                        do_delete_activity(c, &op, &d)?;
                    }
                    DisclosureKind::ProjectionFence => {}
                    DisclosureKind::CurrentStateUpsert
                        if current_state_decision == Some(CurrentStateWriteDecision::Superseded) => {}
                    DisclosureKind::CurrentStateUpsert => {
                        persist_current_state(c, &op, &resolved_current_state_assets)?;
                    }
                }

                if fault == CommitFault::AfterSidecar {
                    return Err(crate::StorageError::from(diesel::result::Error::RollbackTransaction));
                }

                // 7. Write receipt
                let binding = canonical_receipt_binding(&d, &op.subject, &op.idempotency_key, actual, op.kind.as_str(), &op.resource_id, &account.account_id, op.lifecycle_successor_id.as_deref(), "accepted", op.semantic_hash.as_deref());
                sql_query("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome) VALUES(?,?,?,?,?,?,?,?,3,?,?,'accepted')")
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&op.subject)
                    .bind::<Text,_>(&op.idempotency_key)
                    .bind::<Text,_>(&account.account_id)
                    .bind::<BigInt,_>(actual)
                    .bind::<Text,_>(op.kind.as_str())
                    .bind::<Text,_>(&op.resource_id)
                    .bind::<Nullable<Text>,_>(op.lifecycle_successor_id.as_deref())
                    .bind::<Text,_>(&binding)
                    .bind::<Text,_>(&payload_digest)
                    .execute(c)?;

                Ok(Ok(DisclosureReceipt {
                    deployment_id: d,
                    account_id: account.account_id,
                    subject: op.subject,
                    idempotency_key: op.idempotency_key,
                    access_generation: actual,
                    operation: op.kind.as_str().into(),
                    resource_id: op.resource_id,
                    lifecycle_successor_id: op.lifecycle_successor_id,
                    outcome: ReceiptOutcome::Accepted,
                    binding_version: RECEIPT_BINDING_VERSION,
                    receipt_binding: binding,
                    semantic_hash: op.semantic_hash,
                }))
            }).map_err(|e: crate::StorageError| wealthfolio_core::Error::from(e))?;
            Ok(inner)
        }).await.map_err(|e: wealthfolio_core::Error| BridgePolicyError::Storage(e.to_string()))?;
        result
    }
    pub async fn revoke(&self, s: &str, a: &str, k: &str) -> Result<DisclosureReceipt> {
        self.revoke_with_fault(s, a, k, RevokeFault::None).await
    }
    pub async fn revoke_with_fault(
        &self,
        subject: &str,
        account: &str,
        key: &str,
        fault: RevokeFault,
    ) -> Result<DisclosureReceipt> {
        let (d, s, a, k) = (
            self.deployment_id.clone(),
            subject.to_owned(),
            account.to_owned(),
            key.to_owned(),
        );
        let result: std::result::Result<DisclosureReceipt, BridgePolicyError> = self.writer.exec(move |c| {
            let inner: std::result::Result<DisclosureReceipt, BridgePolicyError> = c.transaction::<_, crate::StorageError, _>(|c| {
                // Idempotency check: look up existing receipt before mutating
                if let Some(existing) = sql_query("SELECT person_subject_id subject,idempotency_key,access_generation,operation,resource_id,account_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome FROM disclosure_receipts WHERE deployment_id=? AND person_subject_id=? AND idempotency_key=?")
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&s)
                    .bind::<Text,_>(&k)
                    .get_result::<ReceiptRow>(c)
                    .optional()?
                {
                    let expected_binding = canonical_receipt_binding(&d, &s, &k, existing.access_generation, "projection_fence", &a, &a, None, "revoked", None);
                    if existing.binding_version != RECEIPT_BINDING_VERSION || existing.operation != "projection_fence" || existing.outcome != "revoked" || existing.resource_id != a || existing.account_id != a || existing.receipt_binding.as_deref().is_none_or(|b| !is_canonical_receipt_binding(b, &expected_binding)) {
                        return Ok(Err(BridgePolicyError::IdempotencyConflict));
                    }
                    return Ok(Ok(receipt(&d, existing)));
                }

                sql_query("UPDATE policy_deployments SET access_generation=access_generation+1 WHERE id=?")
                    .bind::<Text,_>(&d)
                    .execute(c)?;
                let g = sql_query("SELECT access_generation generation FROM policy_deployments WHERE id=?")
                    .bind::<Text,_>(&d)
                    .get_result::<Gen>(c)?
                    .generation;
                sql_query("UPDATE account_access_epochs SET end_generation=? WHERE deployment_id=? AND person_subject_id=? AND account_id=? AND end_generation IS NULL")
                    .bind::<BigInt,_>(g)
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&s)
                    .bind::<Text,_>(&a)
                    .execute(c)?;
                sql_query("DELETE FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id=? AND account_id=?")
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&s)
                    .bind::<Text,_>(&a)
                    .execute(c)?;
                if fault == RevokeFault::BeforeReceipt {
                    return Err(crate::StorageError::from(diesel::result::Error::RollbackTransaction));
                }
                let binding = canonical_receipt_binding(&d, &s, &k, g, "projection_fence", &a, &a, None, "revoked", None);
                sql_query("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,outcome) VALUES(?,?,?,?,?,'projection_fence',?,NULL,3,?,'revoked')")
                    .bind::<Text,_>(&d)
                    .bind::<Text,_>(&s)
                    .bind::<Text,_>(&k)
                    .bind::<Text,_>(&a)
                    .bind::<BigInt,_>(g)
                    .bind::<Text,_>(&a)
                    .bind::<Text,_>(&binding)
                    .execute(c)?;
                Ok(Ok(DisclosureReceipt {
                    deployment_id: d,
                    account_id: a.clone(),
                    subject: s,
                    idempotency_key: k,
                    access_generation: g,
                    operation: "projection_fence".into(),
                    resource_id: a,
                    lifecycle_successor_id: None,
                    outcome: ReceiptOutcome::Revoked,
                    binding_version: RECEIPT_BINDING_VERSION,
                    receipt_binding: binding,
                    semantic_hash: None,
                }))
            }).map_err(|e: crate::StorageError| wealthfolio_core::Error::from(e))?;
            Ok(inner)
        }).await.map_err(|e: wealthfolio_core::Error| BridgePolicyError::Storage(e.to_string()))?;
        result
    }
    pub async fn commit_fanout(
        &self,
        ops: Vec<DisclosureOperation>,
    ) -> Vec<Result<DisclosureReceipt>> {
        let mut out = Vec::with_capacity(ops.len());
        for op in ops {
            out.push(self.commit(op).await)
        }
        out
    }
    pub async fn generation(&self, _: &str) -> Result<i64> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| BridgePolicyError::Storage(e.to_string()))?;
        sql_query("SELECT access_generation generation FROM policy_deployments WHERE id=?")
            .bind::<Text, _>(&self.deployment_id)
            .get_result::<Gen>(&mut c)
            .map(|x| x.generation)
            .map_err(sqlerr)
    }
    pub async fn has_access(&self, s: &str, a: &str) -> Result<bool> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| BridgePolicyError::Storage(e.to_string()))?;
        Ok(sql_query("SELECT count(*) count FROM account_access_epochs WHERE deployment_id=? AND person_subject_id=? AND account_id=? AND end_generation IS NULL").bind::<Text,_>(&self.deployment_id).bind::<Text,_>(s).bind::<Text,_>(a).get_result::<Count>(&mut c).map_err(sqlerr)?.count>0)
    }
    pub async fn receipt_count(&self) -> Result<i64> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| BridgePolicyError::Storage(e.to_string()))?;
        Ok(
            sql_query("SELECT count(*) count FROM disclosure_receipts WHERE deployment_id=?")
                .bind::<Text, _>(&self.deployment_id)
                .get_result::<Count>(&mut c)
                .map_err(sqlerr)?
                .count,
        )
    }
    pub async fn receipt(&self, s: &str, k: &str) -> Result<Option<DisclosureReceipt>> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| BridgePolicyError::Storage(e.to_string()))?;
        Ok(sql_query("SELECT person_subject_id subject,idempotency_key,access_generation,operation,resource_id,account_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome FROM disclosure_receipts WHERE deployment_id=? AND person_subject_id=? AND idempotency_key=?").bind::<Text,_>(&self.deployment_id).bind::<Text,_>(s).bind::<Text,_>(k).get_result::<ReceiptRow>(&mut c).optional().map_err(sqlerr)?.map(|row| receipt(&self.deployment_id, row)))
    }
    pub async fn seed_migration_fixture(&self, p: &str, n: &str) -> Result<()> {
        let (p, n, d) = (p.to_owned(), n.to_owned(), self.deployment_id.clone());
        self.writer.exec(move |c| Ok(c.transaction::<_, crate::StorageError, _>(|c| {
            sql_query("INSERT OR IGNORE INTO taxonomies(id,name,color,is_system,is_single_select,sort_order,created_at,updated_at,scope) VALUES('bridge-taxonomy','Bridge','#000',0,1,0,'now','now','activity')").execute(c)?;
            sql_query("INSERT OR IGNORE INTO taxonomy_categories(id,taxonomy_id,name,key,color,sort_order,created_at,updated_at) VALUES('bridge-category','bridge-taxonomy','Bridge','bridge','#000',0,'now','now')").execute(c)?;
            sql_query("INSERT OR IGNORE INTO spending_event_types(id,name) VALUES('bridge-event-type','Bridge')").execute(c)?;
            sql_query("INSERT OR IGNORE INTO spending_events(id,name,event_type_id,start_date,end_date) VALUES('bridge-event','Bridge','bridge-event-type','2026-01-01','2026-01-01')").execute(c)?;
            for (id, status, review) in [(&p, "PENDING", 1_i32), (&n, "POSTED", 0_i32)] {
                sql_query("INSERT INTO activities(id,account_id,activity_type,status,activity_date,amount,currency,notes,activity_type_override,is_user_modified,needs_review,created_at,updated_at) VALUES(?,'a1', 'DEPOSIT',?,'2026-01-01','1','USD',?,'IGNORE',1,?,'now','now')")
                    .bind::<Text,_>(id).bind::<Text,_>(status).bind::<Nullable<Text>,_>(if id == &p { Some("pending note") } else { None }).bind::<Integer,_>(review).execute(c)?;
                sql_query("INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity',?,'live_canonical','finance',?,?)")
                    .bind::<Text,_>(id).bind::<Text,_>(id).bind::<Text,_>(&d).execute(c)?;
            }
            sql_query("INSERT INTO activity_attribution_overlays(deployment_id,activity_id,account_id,attributed_person_subject_id,revision) VALUES(?,?,'a1','alice',1)").bind::<Text,_>(&d).bind::<Text,_>(&p).execute(c)?;
            sql_query("INSERT INTO activity_taxonomy_assignments(id,activity_id,taxonomy_id,category_id) VALUES('bridge-assignment',?,'bridge-taxonomy','bridge-category')").bind::<Text,_>(&p).execute(c)?;
            sql_query("INSERT INTO spending_activity_splits(id,activity_id,taxonomy_id,category_id,amount) VALUES('bridge-split',?,'bridge-taxonomy','bridge-category','1')").bind::<Text,_>(&p).execute(c)?;
            sql_query("INSERT INTO spending_activity_events(activity_id,event_id) VALUES(?,'bridge-event')").bind::<Text,_>(&p).execute(c)?;
            Ok(())
        })?)).await.map_err(err)
    }
    pub async fn migrate_pending_to_posted(&self, p: &str, n: &str) -> Result<()> {
        let (p, n, d) = (p.to_owned(), n.to_owned(), self.deployment_id.clone());
        self.writer.exec(move|c|Ok(c.transaction::<_,crate::StorageError,_>(|c|{let pending: i64=sql_query("SELECT count(*) AS count FROM activities WHERE id=? AND status='PENDING'").bind::<Text,_>(&p).get_result::<Count>(c)?.count;let posted: i64=sql_query("SELECT count(*) AS count FROM activities WHERE id=? AND status='POSTED'").bind::<Text,_>(&n).get_result::<Count>(c)?.count;if pending!=1||posted!=1{return Err(diesel::result::Error::NotFound.into());}for table in ["activity_taxonomy_assignments","spending_activity_splits","spending_activity_events"]{sql_query(format!("UPDATE {table} SET activity_id=? WHERE activity_id=?")).bind::<Text,_>(&n).bind::<Text,_>(&p).execute(c)?;}sql_query("UPDATE activities SET notes=coalesce((SELECT notes FROM activities WHERE id=?),notes),activity_type_override=coalesce((SELECT activity_type_override FROM activities WHERE id=?),activity_type_override),is_user_modified=max(is_user_modified,coalesce((SELECT is_user_modified FROM activities WHERE id=?),0)),needs_review=max(needs_review,coalesce((SELECT needs_review FROM activities WHERE id=?),0)) WHERE id=?").bind::<Text,_>(&p).bind::<Text,_>(&p).bind::<Text,_>(&p).bind::<Text,_>(&p).bind::<Text,_>(&n).execute(c)?;sql_query("UPDATE activity_attribution_overlays SET activity_id=?,account_id=(SELECT account_id FROM activities WHERE id=?) WHERE deployment_id=? AND activity_id=?").bind::<Text,_>(&n).bind::<Text,_>(&n).bind::<Text,_>(&d).bind::<Text,_>(&p).execute(c)?;sql_query("DELETE FROM activities WHERE id=?").bind::<Text,_>(&p).execute(c)?;Ok(())})?)).await.map_err(err)
    }
    pub async fn migration_state(&self, p: &str, n: &str) -> Result<MigrationState> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| BridgePolicyError::Storage(e.to_string()))?;
        let s=sql_query("SELECT (SELECT count(*) FROM activities WHERE id=?) pending_exists,(SELECT count(*) FROM activities WHERE id=?) posted_exists,(SELECT attributed_person_subject_id FROM activity_attribution_overlays WHERE deployment_id=? AND activity_id=?) attribution,(SELECT count(*) FROM activity_taxonomy_assignments WHERE activity_id=?) assignments,(SELECT count(*) FROM spending_activity_splits WHERE activity_id=?) splits,(SELECT count(*) FROM spending_activity_events WHERE activity_id=?) event_tags,coalesce((SELECT activity_type_override='IGNORE' FROM activities WHERE id=?),0) ignored,coalesce((SELECT needs_review FROM activities WHERE id=?),0) needs_review").bind::<Text,_>(p).bind::<Text,_>(n).bind::<Text,_>(&self.deployment_id).bind::<Text,_>(n).bind::<Text,_>(n).bind::<Text,_>(n).bind::<Text,_>(n).bind::<Text,_>(n).bind::<Text,_>(n).get_result::<State>(&mut c).map_err(sqlerr)?;
        Ok(MigrationState {
            pending_exists: s.pending_exists > 0,
            posted_exists: s.posted_exists > 0,
            attribution: s.attribution,
            assignments: s.assignments,
            splits: s.splits,
            event_tags: s.event_tags,
            ignored: s.ignored != 0,
            needs_review: s.needs_review != 0,
        })
    }
}
fn err(e: wealthfolio_core::Error) -> BridgePolicyError {
    BridgePolicyError::Storage(e.to_string())
}
fn sqlerr(e: diesel::result::Error) -> BridgePolicyError {
    BridgePolicyError::Storage(e.to_string())
}

#[cfg(test)]
mod contract_fixture_tests {
    use super::*;

    #[test]
    fn finance_current_state_and_v3_receipt_fixture_match_receiver_authority() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/bridge-v3-current-state.json"
        ))
        .unwrap();
        let operation: DisclosureOperation =
            serde_json::from_value(fixture["operation"].clone()).unwrap();
        let state = operation.current_state.as_ref().unwrap();
        let semantic_hash = current_state_semantic_hash(state);
        assert_eq!(
            operation.semantic_hash.as_deref(),
            Some(semantic_hash.as_str())
        );
        let binding = canonical_receipt_binding(
            fixture["receipt"]["deploymentId"].as_str().unwrap(),
            &operation.subject,
            &operation.idempotency_key,
            operation.access_generation,
            operation.kind.as_str(),
            &operation.resource_id,
            &state.account_id,
            operation.lifecycle_successor_id.as_deref(),
            "accepted",
            Some(&semantic_hash),
        );
        assert_eq!(
            fixture["receipt"]["bindingVersion"],
            RECEIPT_BINDING_VERSION
        );
        assert_eq!(fixture["receipt"]["receiptBinding"], binding);
        assert_eq!(fixture["receipt"]["semanticHash"], semantic_hash);
        assert_eq!(
            fixture["receiptBindingFields"].as_array().unwrap().len(),
            12
        );
    }
}
