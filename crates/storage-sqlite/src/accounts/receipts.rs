//! SQLite storage for provider identities and verified receipts.
//!
//! Uses raw SQL queries (like bridge_policy.rs) for the new tables
//! to avoid Diesel schema regeneration requirements.

use crate::DbPool;
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{Nullable, Text},
};
use std::sync::Arc;
use wealthfolio_core::accounts::{
    ProviderIdentity, ProviderIdentityRepositoryTrait, VerifiedReceipt, VerifiedReceiptBasis,
    VerifiedReceiptRepositoryTrait,
};
use wealthfolio_core::errors::Result;

// ── Provider Identity Queries ──

/// Repository for provider identity storage.
pub struct ProviderIdentityRepository {
    pool: Arc<DbPool>,
}

impl ProviderIdentityRepository {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ProviderIdentityRepositoryTrait for ProviderIdentityRepository {
    async fn get_by_account(&self, account_id: &str) -> Result<Option<ProviderIdentity>> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        let row = sql_query(
            "SELECT account_id, provider, provider_account_id, lifecycle_state, provider_display_name, last_sync_at \
             FROM provider_identities WHERE account_id = ?",
        )
        .bind::<Text, _>(account_id)
        .get_result::<ProviderIdentityRow>(&mut c)
        .optional()
        .map_err(to_core_err)?;
        Ok(row.map(|r| r.into_domain()))
    }

    async fn upsert(&self, identity: ProviderIdentity) -> Result<ProviderIdentity> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        let affected = sql_query(
            "INSERT INTO provider_identities(account_id, provider, provider_account_id, lifecycle_state, provider_display_name, last_sync_at) \
             VALUES(?, ?, ?, ?, ?, ?) \
             ON CONFLICT(account_id) DO UPDATE SET \
               lifecycle_state=excluded.lifecycle_state, \
               provider_display_name=excluded.provider_display_name, \
               last_sync_at=excluded.last_sync_at, \
               updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') \
             WHERE provider_identities.provider=excluded.provider \
               AND provider_identities.provider_account_id=excluded.provider_account_id"
        )
        .bind::<Text, _>(&identity.account_id)
        .bind::<Text, _>(&identity.provider)
        .bind::<Text, _>(&identity.provider_account_id)
        .bind::<Text, _>(&identity.lifecycle_state)
        .bind::<Nullable<Text>, _>(identity.provider_display_name.as_deref())
        .bind::<Nullable<Text>, _>(identity.last_sync_at.as_deref())
        .execute(&mut c)
        .map_err(to_core_err)?;
        if affected != 1 {
            return Err(to_core_err("immutable provider identity mismatch"));
        }
        Ok(identity)
    }

    async fn update_lifecycle(&self, account_id: &str, lifecycle_state: &str) -> Result<()> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        sql_query(
            "UPDATE provider_identities SET lifecycle_state=?, updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE account_id=?"
        )
            .bind::<Text, _>(lifecycle_state)
            .bind::<Text, _>(account_id)
            .execute(&mut c)
            .map_err(to_core_err)?;
        Ok(())
    }

    async fn list_all(&self) -> Result<Vec<ProviderIdentity>> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        let rows: Vec<ProviderIdentityRow> = sql_query(
            "SELECT account_id, provider, provider_account_id, lifecycle_state, provider_display_name, last_sync_at \
             FROM provider_identities ORDER BY provider, account_id",
        )
        .load(&mut c)
        .map_err(to_core_err)?;
        Ok(rows.into_iter().map(|r| r.into_domain()).collect())
    }
}

// ── Verified Receipt Queries ──

/// Repository for verified receipt storage.
pub struct VerifiedReceiptRepository {
    pool: Arc<DbPool>,
}

impl VerifiedReceiptRepository {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl VerifiedReceiptRepositoryTrait for VerifiedReceiptRepository {
    async fn get_by_account(&self, account_id: &str) -> Result<Option<VerifiedReceipt>> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        let row = sql_query(
            "SELECT id, account_id, basis, as_of, provider_receipt_ref, \
             verification_state, coverage_start_date, created_at, updated_at \
             FROM verified_receipts WHERE account_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind::<Text, _>(account_id)
        .get_result::<VerifiedReceiptRow>(&mut c)
        .optional()
        .map_err(to_core_err)?;
        Ok(row.map(|r| r.into_domain()))
    }

    async fn upsert(&self, receipt: VerifiedReceipt) -> Result<VerifiedReceipt> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        sql_query(
            "INSERT INTO verified_receipts(id, account_id, basis, as_of, provider_receipt_ref, \
             verification_state, coverage_start_date) \
             VALUES(?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               basis=excluded.basis, \
               as_of=excluded.as_of, \
               provider_receipt_ref=excluded.provider_receipt_ref, \
               verification_state=excluded.verification_state, \
               coverage_start_date=excluded.coverage_start_date, \
               updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
        )
        .bind::<Text, _>(&receipt.id)
        .bind::<Text, _>(&receipt.account_id)
        .bind::<Text, _>(receipt_basis_to_str(receipt.basis))
        .bind::<Text, _>(&receipt.as_of)
        .bind::<Nullable<Text>, _>(receipt.provider_receipt_ref.as_deref())
        .bind::<Text, _>(&receipt.verification_state)
        .bind::<Nullable<Text>, _>(receipt.coverage_start_date.as_deref())
        .execute(&mut c)
        .map_err(to_core_err)?;
        Ok(receipt)
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        let affected = sql_query("DELETE FROM verified_receipts WHERE id=?")
            .bind::<Text, _>(id)
            .execute(&mut c)
            .map_err(to_core_err)?;
        Ok(affected > 0)
    }

    async fn get_eligible_account_ids(&self) -> Result<Vec<String>> {
        let mut c = self
            .pool
            .get()
            .map_err(|e| to_core_err(format!("pool error: {e}")))?;
        #[derive(QueryableByName)]
        struct IdRow {
            #[diesel(sql_type=Text)]
            id: String,
        }
        let rows: Vec<IdRow> = sql_query(
            "SELECT vr.account_id AS id FROM verified_receipts vr \
             INNER JOIN accounts a ON a.id = vr.account_id \
             WHERE vr.verification_state = 'verified' \
               AND a.include_in_net_worth = 1 \
               AND a.is_archived = 0 \
             ORDER BY vr.account_id",
        )
        .load(&mut c)
        .map_err(to_core_err)?;
        Ok(rows.into_iter().map(|r| r.id).collect())
    }
}

// ── Internal row helpers ──

#[derive(QueryableByName)]
struct ProviderIdentityRow {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    provider: String,
    #[diesel(sql_type=Text)]
    provider_account_id: String,
    #[diesel(sql_type=Text)]
    lifecycle_state: String,
    #[diesel(sql_type=Nullable<Text>)]
    provider_display_name: Option<String>,
    #[diesel(sql_type=Nullable<Text>)]
    last_sync_at: Option<String>,
}

impl ProviderIdentityRow {
    fn into_domain(self) -> ProviderIdentity {
        ProviderIdentity {
            account_id: self.account_id,
            provider: self.provider,
            provider_account_id: self.provider_account_id,
            lifecycle_state: self.lifecycle_state,
            provider_display_name: self.provider_display_name,
            last_sync_at: self.last_sync_at,
        }
    }
}

#[derive(QueryableByName)]
struct VerifiedReceiptRow {
    #[diesel(sql_type=Text)]
    id: String,
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    basis: String,
    #[diesel(sql_type=Text)]
    as_of: String,
    #[diesel(sql_type=Nullable<Text>)]
    provider_receipt_ref: Option<String>,
    #[diesel(sql_type=Text)]
    verification_state: String,
    #[diesel(sql_type=Nullable<Text>)]
    coverage_start_date: Option<String>,
    #[diesel(sql_type=Text)]
    created_at: String,
    #[diesel(sql_type=Text)]
    updated_at: String,
}

impl VerifiedReceiptRow {
    fn into_domain(self) -> VerifiedReceipt {
        let basis = match self.basis.as_str() {
            "current_snapshot" => VerifiedReceiptBasis::CurrentSnapshot,
            "opening_baseline" => VerifiedReceiptBasis::OpeningBaseline,
            _ => VerifiedReceiptBasis::CurrentSnapshot,
        };
        VerifiedReceipt {
            id: self.id,
            account_id: self.account_id,
            basis,
            as_of: self.as_of,
            provider_receipt_ref: self.provider_receipt_ref,
            verification_state: self.verification_state,
            coverage_start_date: self.coverage_start_date,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

fn receipt_basis_to_str(basis: VerifiedReceiptBasis) -> &'static str {
    match basis {
        VerifiedReceiptBasis::CurrentSnapshot => "current_snapshot",
        VerifiedReceiptBasis::OpeningBaseline => "opening_baseline",
    }
}

fn to_core_err(e: impl std::fmt::Display) -> wealthfolio_core::Error {
    wealthfolio_core::Error::Database(wealthfolio_core::errors::DatabaseError::QueryFailed(
        e.to_string(),
    ))
}
