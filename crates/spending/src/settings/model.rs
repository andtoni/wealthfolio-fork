use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BUDGET_ANCHOR_DATE: &str = "2026-07-25";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetCadence {
    Weekly,
    Biweekly,
    Monthly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetCycleSettings {
    pub cadence: BudgetCadence,
    pub anchor_date: String,
}

impl Default for BudgetCycleSettings {
    fn default() -> Self {
        Self {
            cadence: BudgetCadence::Monthly,
            anchor_date: DEFAULT_BUDGET_ANCHOR_DATE.to_string(),
        }
    }
}

impl BudgetCycleSettings {
    pub fn parsed_anchor_date(&self) -> Option<NaiveDate> {
        let date = NaiveDate::parse_from_str(&self.anchor_date, "%Y-%m-%d").ok()?;
        (date.format("%Y-%m-%d").to_string() == self.anchor_date).then_some(date)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpendingSettings {
    pub enabled: bool,
    pub account_ids: Vec<String>,
    #[serde(default)]
    pub budget_cycle: BudgetCycleSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendingSettingsUpdate {
    pub enabled: Option<bool>,
    pub account_ids: Option<Vec<String>>,
    pub budget_cycle: Option<BudgetCycleSettings>,
}
