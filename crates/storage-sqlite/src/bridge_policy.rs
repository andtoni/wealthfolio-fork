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
}
impl DisclosureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ActivityUpsert => "activity_upsert",
            Self::ActivityDelete => "activity_delete",
            Self::ProjectionFence => "projection_fence",
        }
    }
}

/// Payload for creating or updating an activity row via the bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityPayload {
    pub account_id: String,
    pub activity_type: String,
    pub status: String,
    pub activity_date: String,
    pub amount: String,
    pub currency: String,
    pub notes: Option<String>,
    pub activity_type_override: Option<String>,
    pub needs_review: Option<bool>,
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
    }
}

const RECEIPT_BINDING_VERSION: i64 = 2;

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
) -> String {
    let generation = generation.to_string();
    let successor_present = if successor.is_some() { "1" } else { "0" };
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
    ];
    let mut out = String::from("v2;");
    for field in fields {
        out.push_str(&field.len().to_string());
        out.push(':');
        out.push_str(field);
    }
    out
}

fn is_canonical_receipt_binding(binding: &str, expected: &str) -> bool {
    if binding != expected || !binding.starts_with("v2;") {
        return false;
    }
    let mut rest = &binding.as_bytes()[3..];
    for _ in 0..10 {
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
fn disclosure_payload_digest(op: &DisclosureOperation) -> String {
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
    }
    format!("{:x}", hasher.finalize())
}

fn upsert_activity_row(
    c: &mut SqliteConnection,
    resource_id: &str,
    act: &ActivityPayload,
) -> std::result::Result<(), diesel::result::Error> {
    if let Some(existing) = sql_query("SELECT account_id FROM activities WHERE id=?")
        .bind::<Text, _>(resource_id)
        .get_result::<Account>(c)
        .optional()?
    {
        if existing.account_id != act.account_id {
            return Err(diesel::result::Error::RollbackTransaction);
        }
    }
    sql_query(
        "INSERT INTO activities(id,account_id,activity_type,status,activity_date,amount,currency,notes,activity_type_override,is_user_modified,needs_review,created_at,updated_at) VALUES(?,?,?,?,?,?,?,NULL,NULL,0,0,'now','now') ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,activity_type=excluded.activity_type,status=excluded.status,activity_date=excluded.activity_date,amount=excluded.amount,currency=excluded.currency,updated_at='now'"
    )
        .bind::<Text,_>(resource_id)
        .bind::<Text,_>(&act.account_id)
        .bind::<Text,_>(&act.activity_type)
        .bind::<Text,_>(&act.status)
        .bind::<Text,_>(&act.activity_date)
        .bind::<Text,_>(&act.amount)
        .bind::<Text,_>(&act.currency)
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
           (pending.notes IS NOT NULL AND posted.notes IS NOT NULL AND pending.notes<>posted.notes) OR \
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
        upsert_activity_row(c, &op.resource_id, act)?;
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
    sql_query("DELETE FROM activities WHERE id=?")
        .bind::<Text, _>(&op.resource_id)
        .execute(c)?;
    sql_query("DELETE FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id=? AND activity_id=?")
        .bind::<Text,_>(d)
        .bind::<Text,_>(&op.subject)
        .bind::<Text,_>(&op.resource_id)
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
                    let expected_binding = canonical_receipt_binding(&d, &existing.subject, &existing.idempotency_key, existing.access_generation, &existing.operation, &existing.resource_id, &existing.account_id, existing.lifecycle_successor_id.as_deref(), &existing.outcome);
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

                // 4. Handle lifecycle migration (pending->posted) atomically
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
                        upsert_activity_row(c, n, activity)?;
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
                    sql_query("UPDATE activities SET notes=coalesce((SELECT notes FROM activities WHERE id=?),notes),activity_type_override=coalesce((SELECT activity_type_override FROM activities WHERE id=?),activity_type_override),subtype=CASE WHEN coalesce((SELECT is_user_modified FROM activities WHERE id=?),0)=1 THEN coalesce((SELECT subtype FROM activities WHERE id=?),subtype) ELSE subtype END,is_user_modified=max(is_user_modified,coalesce((SELECT is_user_modified FROM activities WHERE id=?),0)),needs_review=max(needs_review,coalesce((SELECT needs_review FROM activities WHERE id=?),0)) WHERE id=?")
                        .bind::<Text,_>(p)
                        .bind::<Text,_>(p)
                        .bind::<Text,_>(p)
                        .bind::<Text,_>(p)
                        .bind::<Text,_>(p)
                        .bind::<Text,_>(p)
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
                }

                if fault == CommitFault::AfterSidecar {
                    return Err(crate::StorageError::from(diesel::result::Error::RollbackTransaction));
                }

                // 7. Write receipt
                let binding = canonical_receipt_binding(&d, &op.subject, &op.idempotency_key, actual, op.kind.as_str(), &op.resource_id, &account.account_id, op.lifecycle_successor_id.as_deref(), "accepted");
                sql_query("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome) VALUES(?,?,?,?,?,?,?,?,2,?,?,'accepted')")
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
                    let expected_binding = canonical_receipt_binding(&d, &s, &k, existing.access_generation, "projection_fence", &a, &a, None, "revoked");
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
                let binding = canonical_receipt_binding(&d, &s, &k, g, "projection_fence", &a, &a, None, "revoked");
                sql_query("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,outcome) VALUES(?,?,?,?,?,'projection_fence',?,NULL,2,?,'revoked')")
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
