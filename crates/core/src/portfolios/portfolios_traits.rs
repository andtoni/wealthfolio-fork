use async_trait::async_trait;
use chrono::NaiveDate;

use super::portfolios_model::{
    AccountScope, NewPortfolio, PortfolioUpdate, PortfolioWithAccounts, ResolvedAccountScope,
};
use crate::accounts::AccountPurpose;
use crate::errors::Result;

#[async_trait]
pub trait PortfolioRepositoryTrait: Send + Sync {
    async fn create(&self, new: NewPortfolio) -> Result<PortfolioWithAccounts>;
    async fn update(&self, update: PortfolioUpdate) -> Result<PortfolioWithAccounts>;
    async fn delete(&self, id: &str) -> Result<usize>;
    fn get_by_id(&self, id: &str) -> Result<PortfolioWithAccounts>;
    fn list(&self) -> Result<Vec<PortfolioWithAccounts>>;
    /// Resolve an AccountScope to the concrete list of account IDs.
    fn resolve_account_ids(&self, filter: &AccountScope) -> Result<Vec<String>>;
}

#[async_trait]
pub trait PortfolioServiceTrait: Send + Sync {
    async fn create_portfolio(&self, new: NewPortfolio) -> Result<PortfolioWithAccounts>;
    async fn update_portfolio(&self, update: PortfolioUpdate) -> Result<PortfolioWithAccounts>;
    async fn delete_portfolio(&self, id: &str) -> Result<()>;
    fn get_portfolio(&self, id: &str) -> Result<PortfolioWithAccounts>;
    fn list_portfolios(&self) -> Result<Vec<PortfolioWithAccounts>>;

    /// Resolve an AccountScope into its runtime reporting form.
    async fn resolve_account_scope(
        &self,
        filter: &AccountScope,
        base_currency: &str,
    ) -> Result<ResolvedAccountScope>;

    /// Resolve an AccountScope into its runtime reporting form for a specific
    /// aggregate as-of date.
    async fn resolve_account_scope_as_of(
        &self,
        filter: &AccountScope,
        base_currency: &str,
        as_of: NaiveDate,
    ) -> Result<ResolvedAccountScope>;

    /// Resolve an AccountScope and keep only accounts eligible for a product surface.
    async fn resolve_account_scope_for_purpose(
        &self,
        filter: &AccountScope,
        base_currency: &str,
        purpose: AccountPurpose,
    ) -> Result<ResolvedAccountScope>;

    /// Resolve an AccountScope into its runtime reporting form using the app
    /// default base currency. Prefer `resolve_account_scope` where the current
    /// base currency is available.
    async fn resolve_account_scope_default_base(
        &self,
        filter: &AccountScope,
    ) -> Result<ResolvedAccountScope> {
        self.resolve_account_scope(filter, "").await
    }
}
