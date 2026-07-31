//! Spending-tracker settings: enable toggle, opted-in accounts, and budget cycle.
//! Stored in the existing `app_settings` k/v table:
//!   - `spending.enabled`         → "true" | "false"
//!   - `spending.account_ids`     → JSON array of opted-in spending account IDs
//!   - `spending.budget_cycle`    → JSON cadence + anchor date

pub mod model;
pub mod service;
pub mod traits;

pub use model::{
    BudgetCadence, BudgetCycleSettings, SpendingSettings, SpendingSettingsUpdate,
    DEFAULT_BUDGET_ANCHOR_DATE,
};
pub use service::SpendingSettingsService;
pub use traits::SpendingSettingsRepositoryTrait;

pub const SETTING_KEY_ENABLED: &str = "spending.enabled";
pub const SETTING_KEY_ACCOUNT_IDS: &str = "spending.account_ids";
pub const SETTING_KEY_BUDGET_CYCLE: &str = "spending.budget_cycle";
