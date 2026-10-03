//! Activity → budget period overlay.
//!
//! Lets the user count a transaction toward a budget period other than the one
//! its date falls in (groceries bought two days before the next period starts).
//! The stored activity, including its date, is never changed: budget and
//! insight aggregation work on an in-memory copy moved to the nearest day
//! inside the chosen period.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{NaiveDate, TimeZone, Utc};
use wealthfolio_core::activities::{Activity, ActivityDatePrecision};
use wealthfolio_core::utils::time_utils::parse_user_timezone_or_default;

use crate::budget::period::{period_for_date, BudgetPeriod};
use crate::settings::BudgetCycleSettings;

/// activity_id → start date (`YYYY-MM-DD`) of the chosen budget period.
pub type BudgetPeriodsByActivity = HashMap<String, String>;

#[async_trait]
pub trait ActivityBudgetPeriodRepositoryTrait: Send + Sync {
    async fn list_all(&self) -> Result<BudgetPeriodsByActivity>;

    /// Activities without a chosen period are absent from the map.
    async fn list_for_activities(&self, ids: &[String]) -> Result<BudgetPeriodsByActivity>;

    /// Sets the chosen period for every id, or clears it when `period_start`
    /// is `None`. All rows commit in one transaction.
    async fn set_many(&self, activity_ids: &[String], period_start: Option<String>) -> Result<()>;
}

/// Resolves a stored period start against the current cycle. A cycle change
/// after the choice was made maps it to the period containing that date, the
/// same rule the budget target rebase uses.
pub(crate) fn chosen_period(
    period_start: &str,
    cycle: &BudgetCycleSettings,
) -> Option<BudgetPeriod> {
    let date = NaiveDate::parse_from_str(period_start, "%Y-%m-%d").ok()?;
    period_for_date(date, cycle).ok()
}

/// Moves each assigned activity to the nearest day inside its chosen period
/// (local noon), so every date-based aggregation downstream counts it there.
pub(crate) fn apply_budget_period_overrides(
    activities: &mut [Activity],
    overrides: &BudgetPeriodsByActivity,
    timezone: &str,
    cycle: &BudgetCycleSettings,
) {
    if overrides.is_empty() {
        return;
    }
    let tz = parse_user_timezone_or_default(timezone);
    for activity in activities {
        let Some(period) = overrides
            .get(&activity.id)
            .and_then(|period_start| chosen_period(period_start, cycle))
        else {
            continue;
        };
        let date = activity.calendar_date(timezone);
        let moved = date.clamp(period.start, period.end);
        if moved == date {
            continue;
        }
        let Some(noon) = moved.and_hms_opt(12, 0, 0) else {
            continue;
        };
        activity.activity_date = tz
            .from_local_datetime(&noon)
            .earliest()
            .map(|local| local.with_timezone(&Utc))
            .unwrap_or_else(|| Utc.from_utc_datetime(&noon));
        activity.activity_date_precision = Some(ActivityDatePrecision::Instant);
    }
}

/// In-memory repository for service tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct StaticBudgetPeriods(pub std::sync::Mutex<BudgetPeriodsByActivity>);

#[cfg(test)]
#[async_trait]
impl ActivityBudgetPeriodRepositoryTrait for StaticBudgetPeriods {
    async fn list_all(&self) -> Result<BudgetPeriodsByActivity> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn list_for_activities(&self, ids: &[String]) -> Result<BudgetPeriodsByActivity> {
        let all = self.0.lock().unwrap();
        Ok(ids
            .iter()
            .filter_map(|id| all.get(id).map(|period| (id.clone(), period.clone())))
            .collect())
    }

    async fn set_many(&self, activity_ids: &[String], period_start: Option<String>) -> Result<()> {
        let mut all = self.0.lock().unwrap();
        for id in activity_ids {
            match &period_start {
                Some(period_start) => all.insert(id.clone(), period_start.clone()),
                None => all.remove(id),
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use wealthfolio_core::activities::ActivityStatus;

    use super::*;
    use crate::settings::BudgetCadence;

    fn cycle() -> BudgetCycleSettings {
        BudgetCycleSettings {
            cadence: BudgetCadence::Monthly,
            anchor_date: "2026-01-25".to_string(),
        }
    }

    fn activity(id: &str, date: &str) -> Activity {
        Activity {
            id: id.to_string(),
            account_id: "account".to_string(),
            asset_id: None,
            activity_type: "WITHDRAWAL".to_string(),
            activity_type_override: None,
            source_type: None,
            subtype: None,
            status: ActivityStatus::Posted,
            activity_date: date.parse().unwrap(),
            activity_date_precision: Some(ActivityDatePrecision::Instant),
            settlement_date: None,
            quantity: None,
            unit_price: None,
            amount: None,
            fee: None,
            tax: None,
            currency: "ZAR".to_string(),
            fx_rate: None,
            notes: None,
            metadata: None,
            source_system: None,
            source_record_id: None,
            source_group_id: None,
            idempotency_key: None,
            import_run_id: None,
            is_user_modified: false,
            needs_review: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn overrides(entries: &[(&str, &str)]) -> BudgetPeriodsByActivity {
        entries
            .iter()
            .map(|(id, period)| (id.to_string(), period.to_string()))
            .collect()
    }

    #[test]
    fn moves_an_activity_forward_to_the_first_day_of_the_chosen_period() {
        let mut activities = vec![activity("groceries", "2026-09-23T16:30:00Z")];
        apply_budget_period_overrides(
            &mut activities,
            &overrides(&[("groceries", "2026-09-25")]),
            "Africa/Johannesburg",
            &cycle(),
        );
        assert_eq!(
            activities[0].activity_date.to_rfc3339(),
            "2026-09-25T10:00:00+00:00"
        );
    }

    #[test]
    fn moves_an_activity_back_to_the_last_day_of_the_chosen_period() {
        let mut activities = vec![activity("dinner", "2026-09-25T18:00:00Z")];
        apply_budget_period_overrides(
            &mut activities,
            &overrides(&[("dinner", "2026-08-25")]),
            "Africa/Johannesburg",
            &cycle(),
        );
        assert_eq!(
            activities[0].activity_date.to_rfc3339(),
            "2026-09-24T10:00:00+00:00"
        );
    }

    #[test]
    fn leaves_unassigned_and_already_in_period_activities_untouched() {
        let mut activities = vec![
            activity("unassigned", "2026-09-23T16:30:00Z"),
            activity("in-period", "2026-09-26T08:00:00Z"),
        ];
        apply_budget_period_overrides(
            &mut activities,
            &overrides(&[("in-period", "2026-09-25")]),
            "Africa/Johannesburg",
            &cycle(),
        );
        assert_eq!(
            activities[0].activity_date.to_rfc3339(),
            "2026-09-23T16:30:00+00:00"
        );
        assert_eq!(
            activities[1].activity_date.to_rfc3339(),
            "2026-09-26T08:00:00+00:00"
        );
    }

    #[test]
    fn a_cycle_change_resolves_the_stored_start_to_its_containing_period() {
        let first_of_month = BudgetCycleSettings {
            cadence: BudgetCadence::Monthly,
            anchor_date: "2026-01-01".to_string(),
        };
        let period = chosen_period("2026-09-25", &first_of_month).unwrap();
        assert_eq!(period.key, "2026-09-01");
    }
}
