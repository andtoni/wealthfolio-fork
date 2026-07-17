//! Accounts module - domain models, services, and traits.

use async_trait::async_trait;
use chrono::NaiveDate;
use std::collections::HashSet;

mod accounts_constants;
mod accounts_model;
mod accounts_service;
mod accounts_traits;
mod aggregate_eligibility;
mod eligibility;
mod receipts_traits;

// Re-export the public interface
pub use accounts_constants::*;
pub use accounts_model::{
    Account, AccountAccountingSettings, AccountUpdate, CostBasisMethod, CostBasisProfile,
    LotSelectionStrategy, NewAccount, PoolingScope, ProvenanceRecord, ProvenanceRole,
    ProviderIdentity, TrackingMode, VerifiedReceipt, VerifiedReceiptBasis,
};
pub use accounts_service::AccountService;
pub use accounts_traits::{AccountRepositoryTrait, AccountServiceTrait};
pub use aggregate_eligibility::{AggregateEligibilityService, AggregateEligibilityServiceTrait};
pub use eligibility::{account_is_eligible, filter_eligible_accounts};
pub use receipts_traits::{ProviderIdentityRepositoryTrait, VerifiedReceiptRepositoryTrait};

pub const ENTITY_ACCOUNT: &str = "account";
pub const ENTITY_ACTIVITY: &str = "activity";
pub const ENTITY_SNAPSHOT: &str = "snapshot";
pub const ENTITY_POSITION: &str = "position";
pub const ENTITY_DAILY_VALUATION: &str = "daily_valuation";

#[async_trait]
pub trait ProvenanceRepositoryTrait: Send + Sync {
    async fn get(
        &self,
        entity_type: &str,
        entity_id: &str,
    ) -> crate::errors::Result<Option<ProvenanceRecord>>;
    async fn upsert(&self, record: ProvenanceRecord) -> crate::errors::Result<ProvenanceRecord>;
    async fn eligible_entity_ids(
        &self,
        entity_type: &str,
        row_date: &str,
    ) -> crate::errors::Result<Vec<String>>;
    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> crate::errors::Result<HashSet<String>>;

    async fn eligible_entity_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> crate::errors::Result<HashSet<(String, NaiveDate)>> {
        let eligible = self.eligible_entity_ids_for_rows(entity_type, rows).await?;
        Ok(rows
            .iter()
            .filter(|(id, _)| eligible.contains(id))
            .cloned()
            .collect())
    }
}

/// Returns true when an account belongs in portfolio totals/history.
pub fn account_in_portfolio_scope(account: &Account) -> bool {
    !account.is_archived
}

/// Returns true when an account belongs in a portfolio reporting scope for a purpose.
pub fn account_supports_portfolio_scope(account: &Account, purpose: AccountPurpose) -> bool {
    account_in_portfolio_scope(account) && account_supports_purpose(&account.account_type, purpose)
}

#[cfg(test)]
mod accounts_model_tests;
