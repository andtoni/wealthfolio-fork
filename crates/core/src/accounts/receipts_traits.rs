//! Repository trait for verified receipts and provider identities.

use crate::accounts::{ProviderIdentity, VerifiedReceipt};
use crate::errors::Result;

/// Repository trait for operating on verified receipts.
#[async_trait::async_trait]
pub trait VerifiedReceiptRepositoryTrait: Send + Sync {
    /// Get the verified receipt for an account, if one exists.
    async fn get_by_account(&self, account_id: &str) -> Result<Option<VerifiedReceipt>>;

    /// Upsert a verified receipt for an account.
    async fn upsert(&self, receipt: VerifiedReceipt) -> Result<VerifiedReceipt>;

    /// Delete a verified receipt.
    async fn delete(&self, id: &str) -> Result<bool>;

    /// Get all account IDs that have a verified receipt with state "verified".
    async fn get_eligible_account_ids(&self) -> Result<Vec<String>>;
}

/// Repository trait for operating on provider identities.
#[async_trait::async_trait]
pub trait ProviderIdentityRepositoryTrait: Send + Sync {
    /// Get the provider identity for an account, if one exists.
    async fn get_by_account(&self, account_id: &str) -> Result<Option<ProviderIdentity>>;

    /// Upsert a provider identity.
    async fn upsert(&self, identity: ProviderIdentity) -> Result<ProviderIdentity>;

    /// Update the lifecycle state of a provider identity.
    async fn update_lifecycle(&self, account_id: &str, lifecycle_state: &str) -> Result<()>;

    /// List all provider identities.
    async fn list_all(&self) -> Result<Vec<ProviderIdentity>>;
}
