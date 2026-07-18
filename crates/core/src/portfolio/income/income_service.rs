use crate::constants::DISPLAY_DECIMAL_PRECISION;
use crate::utils::time_utils::{parse_user_timezone_or_default, user_today};
use crate::{
    accounts::{AggregateEligibilityServiceTrait, ENTITY_ACTIVITY},
    activities::{ActivityRepositoryTrait, IncomeData},
    Result,
};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};

use super::IncomeSummary;
use crate::fx::FxServiceTrait;
use log::{debug, error};
use num_traits::Zero;
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
// Define the trait for the income service
#[async_trait]
pub trait IncomeServiceTrait: Send + Sync {
    async fn get_income_summary(
        &self,
        account_ids: Option<&[String]>,
    ) -> Result<Vec<IncomeSummary>>;
}

pub struct IncomeService {
    fx_service: Arc<dyn FxServiceTrait>,
    activity_repository: Arc<dyn ActivityRepositoryTrait>,
    aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
    base_currency: Arc<RwLock<String>>,
    timezone: Arc<RwLock<String>>,
}

impl IncomeService {
    pub fn new(
        fx_service: Arc<dyn FxServiceTrait>,
        activity_repository: Arc<dyn ActivityRepositoryTrait>,
        base_currency: Arc<RwLock<String>>,
        aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
    ) -> Self {
        Self::new_with_timezone(
            fx_service,
            activity_repository,
            base_currency,
            Arc::new(RwLock::new(String::new())),
            aggregate_eligibility_service,
        )
    }

    pub fn new_with_timezone(
        fx_service: Arc<dyn FxServiceTrait>,
        activity_repository: Arc<dyn ActivityRepositoryTrait>,
        base_currency: Arc<RwLock<String>>,
        timezone: Arc<RwLock<String>>,
        aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
    ) -> Self {
        IncomeService {
            fx_service,
            activity_repository,
            aggregate_eligibility_service,
            base_currency,
            timezone,
        }
    }

    fn today_in_user_timezone(&self) -> NaiveDate {
        let tz = parse_user_timezone_or_default(&self.timezone.read().unwrap());
        user_today(tz)
    }

    fn calculate_yoy_growth(current: Decimal, previous: Decimal) -> Decimal {
        if previous > Decimal::zero() {
            (current - previous) / previous
        } else {
            Decimal::zero()
        }
    }
}

// Implement the trait for IncomeService
#[async_trait]
impl IncomeServiceTrait for IncomeService {
    async fn get_income_summary(
        &self,
        account_ids: Option<&[String]>,
    ) -> Result<Vec<IncomeSummary>> {
        debug!("Getting income summary...");

        let current_date = self.today_in_user_timezone();
        let mut source_activities = match self.activity_repository.get_income_activities() {
            Ok(activities) => activities,
            Err(e) => {
                error!("Error getting source income activities: {:?}", e);
                return Err(e);
            }
        };

        let requested_account_ids: Vec<String> = match account_ids {
            Some(ids) => ids.to_vec(),
            None => {
                let mut ids: Vec<String> = source_activities
                    .iter()
                    .map(|activity| activity.account_id.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                ids.sort();
                ids
            }
        };

        if requested_account_ids.is_empty() {
            return Ok(Vec::new());
        }

        let eligible_account_ids = self
            .aggregate_eligibility_service
            .eligible_account_ids(&requested_account_ids, current_date)
            .await?;
        if eligible_account_ids.is_empty() {
            return Ok(Vec::new());
        }

        let requested_account_ids: HashSet<&str> =
            requested_account_ids.iter().map(String::as_str).collect();
        source_activities.retain(|activity| {
            requested_account_ids.contains(activity.account_id.as_str())
                && eligible_account_ids.contains(&activity.account_id)
                && matches!(
                    activity.activity_type.as_str(),
                    "DIVIDEND" | "INTEREST" | "OTHER_INCOME"
                )
        });

        if source_activities.is_empty() {
            return Ok(Vec::new());
        }

        let policy_rows: Vec<(String, NaiveDate)> = source_activities
            .iter()
            .map(|activity| (activity.id.clone(), activity.activity_date.date_naive()))
            .collect();
        let eligible_activity_ids = self
            .aggregate_eligibility_service
            .eligible_entity_ids_for_rows(ENTITY_ACTIVITY, &policy_rows)
            .await?;
        source_activities.retain(|activity| eligible_activity_ids.contains(&activity.id));

        if source_activities.is_empty() {
            return Ok(Vec::new());
        }

        let oldest_date = source_activities
            .iter()
            .map(|activity| activity.activity_date)
            .min()
            .expect("source_activities is not empty");
        let activity_ids: Vec<String> = source_activities
            .iter()
            .map(|activity| activity.id.clone())
            .collect();
        let activities = match self
            .activity_repository
            .get_income_activities_data_for_activity_ids(&activity_ids)
        {
            Ok(activity) => activity,
            Err(e) => {
                error!("Error getting aggregated income data: {:?}", e);
                return Err(e);
            }
        };

        if activities.is_empty() {
            return Ok(Vec::new());
        }

        let base_currency = self.base_currency.read().unwrap().clone();
        let current_year = current_date.year();
        let last_year = current_year - 1;
        let two_years_ago = current_year - 2;
        let current_month = current_date.month();

        let mut months_since_first_transaction: i32 =
            (current_date.year() - oldest_date.year()) * 12;
        months_since_first_transaction = months_since_first_transaction
            + current_date.month() as i32
            - oldest_date.month() as i32;

        let mut months_in_last_year: i32 = 12;
        if oldest_date.year() >= current_year - 1 {
            months_in_last_year = 13 - oldest_date.month() as i32
        }

        let mut months_two_years_ago: i32 = 12;
        if oldest_date.year() >= current_year - 2 {
            months_two_years_ago = 13 - oldest_date.month() as i32
        }

        let mut total_summary = IncomeSummary::new("ALL", base_currency.clone());
        let mut ytd_summary = IncomeSummary::new("YTD", base_currency.clone());
        let mut last_year_summary = IncomeSummary::new("LAST_YEAR", base_currency.clone());
        let mut two_years_ago_summary = IncomeSummary::new("TWO_YEARS_AGO", base_currency.clone());

        for activity in activities {
            let date = match NaiveDate::parse_from_str(&format!("{}-01", activity.date), "%Y-%m-%d")
            {
                Ok(d) => d,
                Err(e) => {
                    error!("Error parsing date {}: {:?}", activity.date, e);
                    continue;
                }
            };

            // Correctly call methods on the FxService instance within the Arc
            let converted_amount = self.fx_service.convert_currency(
                activity.amount,
                &activity.currency,
                &base_currency,
            )?;

            // Create a copy of the activity with cloned fields to avoid ownership issues
            let activity_copy = IncomeData {
                date: activity.date.clone(),
                income_type: activity.income_type.clone(),
                asset_id: activity.asset_id.clone(),
                asset_kind: activity.asset_kind.clone(),
                symbol: activity.symbol.clone(),
                symbol_name: activity.symbol_name.clone(),
                currency: activity.currency.clone(),
                amount: activity.amount,
                account_id: activity.account_id.clone(),
                account_name: activity.account_name.clone(),
            };

            total_summary.add_income(&activity_copy, converted_amount);

            if date.year() == current_year {
                ytd_summary.add_income(&activity_copy, converted_amount);
            } else if date.year() == last_year {
                last_year_summary.add_income(&activity_copy, converted_amount);
            } else if date.year() == two_years_ago {
                two_years_ago_summary.add_income(&activity_copy, converted_amount);
            }
        }

        total_summary.calculate_monthly_average(Some(months_since_first_transaction as u32));
        ytd_summary.calculate_monthly_average(Some(current_month));
        last_year_summary.calculate_monthly_average(Some(months_in_last_year as u32));
        two_years_ago_summary.calculate_monthly_average(Some(months_two_years_ago as u32));

        // Calculate Year-over-Year Growth using the static helper method
        let ytd_yoy_growth = IncomeService::calculate_yoy_growth(
            ytd_summary.total_income,
            last_year_summary.total_income,
        );
        let last_year_yoy_growth = IncomeService::calculate_yoy_growth(
            last_year_summary.total_income,
            two_years_ago_summary.total_income,
        );

        ytd_summary.yoy_growth = Some(ytd_yoy_growth);
        last_year_summary.yoy_growth = Some(last_year_yoy_growth);

        // Two years ago YoY growth can't be calculated without activity from three years ago
        two_years_ago_summary.yoy_growth = None;

        // Round the calculated values before returning
        let summaries = vec![
            total_summary,
            ytd_summary,
            last_year_summary,
            two_years_ago_summary,
        ];

        let rounded_summaries = summaries
            .into_iter()
            .map(|mut summary| {
                summary.total_income = summary.total_income.round_dp(DISPLAY_DECIMAL_PRECISION);
                summary.monthly_average =
                    summary.monthly_average.round_dp(DISPLAY_DECIMAL_PRECISION);
                if let Some(growth) = summary.yoy_growth {
                    summary.yoy_growth = Some(growth.round_dp(DISPLAY_DECIMAL_PRECISION));
                }

                for val in summary.by_month.values_mut() {
                    *val = val.round_dp(DISPLAY_DECIMAL_PRECISION);
                }
                for val in summary.by_type.values_mut() {
                    *val = val.round_dp(DISPLAY_DECIMAL_PRECISION);
                }
                for entry in summary.by_asset.values_mut() {
                    entry.income = entry.income.round_dp(DISPLAY_DECIMAL_PRECISION);
                }
                for val in summary.by_currency.values_mut() {
                    *val = val.round_dp(DISPLAY_DECIMAL_PRECISION);
                }
                for entry in summary.by_account.values_mut() {
                    entry.total = entry.total.round_dp(DISPLAY_DECIMAL_PRECISION);
                    for val in entry.by_month.values_mut() {
                        *val = val.round_dp(DISPLAY_DECIMAL_PRECISION);
                    }
                }
                summary
            })
            .collect();

        debug!("Income summary calculation and rounding completed successfully");
        Ok(rounded_summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AggregateEligibilityServiceTrait, ProvenanceRole, ENTITY_ACTIVITY};
    use crate::activities::{Activity, ActivityStatus};
    use crate::fx::{ExchangeRate, NewExchangeRate};
    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    struct MockFxService {
        fail_conversion: bool,
    }

    #[async_trait]
    impl FxServiceTrait for MockFxService {
        fn initialize(&self) -> Result<()> {
            Ok(())
        }

        fn get_historical_rates(
            &self,
            _from_currency: &str,
            _to_currency: &str,
            _days: i64,
        ) -> Result<Vec<ExchangeRate>> {
            Ok(Vec::new())
        }

        fn get_latest_exchange_rate(
            &self,
            _from_currency: &str,
            _to_currency: &str,
        ) -> Result<Decimal> {
            Ok(Decimal::ONE)
        }

        fn get_exchange_rate_for_date(
            &self,
            _from_currency: &str,
            _to_currency: &str,
            _date: NaiveDate,
        ) -> Result<Decimal> {
            Ok(Decimal::ONE)
        }

        fn convert_currency(
            &self,
            amount: Decimal,
            from_currency: &str,
            _to_currency: &str,
        ) -> Result<Decimal> {
            if self.fail_conversion {
                return Err(crate::Error::ConstraintViolation("fx-failure".into()));
            }
            Ok(if from_currency == "EUR" {
                amount * Decimal::new(2, 0)
            } else {
                amount
            })
        }

        fn convert_currency_for_date(
            &self,
            amount: Decimal,
            _from_currency: &str,
            _to_currency: &str,
            _date: NaiveDate,
        ) -> Result<Decimal> {
            Ok(amount)
        }

        fn get_latest_exchange_rates(&self) -> Result<Vec<ExchangeRate>> {
            Ok(Vec::new())
        }

        async fn add_exchange_rate(&self, _new_rate: NewExchangeRate) -> Result<ExchangeRate> {
            unimplemented!()
        }

        async fn update_exchange_rate(
            &self,
            _from_currency: &str,
            _to_currency: &str,
            _rate: Decimal,
        ) -> Result<ExchangeRate> {
            unimplemented!()
        }

        async fn delete_exchange_rate(&self, _rate_id: &str) -> Result<()> {
            Ok(())
        }

        async fn register_currency_pair(
            &self,
            _from_currency: &str,
            _to_currency: &str,
        ) -> Result<()> {
            Ok(())
        }

        async fn register_currency_pair_manual(
            &self,
            _from_currency: &str,
            _to_currency: &str,
        ) -> Result<()> {
            Ok(())
        }

        async fn ensure_fx_pairs(&self, _pairs: Vec<(String, String)>) -> Result<()> {
            Ok(())
        }
    }

    struct SemanticProvenance {
        role: ProvenanceRole,
        coverage_start: Option<NaiveDate>,
        manual_approved: bool,
    }

    struct MockEligibility {
        allowed_accounts: HashSet<String>,
        provenance: HashMap<String, SemanticProvenance>,
        checked_rows: Mutex<Vec<(String, NaiveDate)>>,
        account_error: Option<String>,
        row_error: Option<String>,
    }

    #[async_trait]
    impl AggregateEligibilityServiceTrait for MockEligibility {
        async fn eligible_account_ids(
            &self,
            requested_account_ids: &[String],
            _as_of: NaiveDate,
        ) -> Result<HashSet<String>> {
            if let Some(message) = &self.account_error {
                return Err(crate::Error::ConstraintViolation(message.clone()));
            }
            Ok(requested_account_ids
                .iter()
                .filter(|id| self.allowed_accounts.contains(*id))
                .cloned()
                .collect())
        }

        async fn eligible_entity_ids_for_rows(
            &self,
            entity_type: &str,
            rows: &[(String, NaiveDate)],
        ) -> Result<HashSet<String>> {
            if let Some(message) = &self.row_error {
                return Err(crate::Error::ConstraintViolation(message.clone()));
            }
            assert_eq!(entity_type, ENTITY_ACTIVITY);
            *self.checked_rows.lock().unwrap() = rows.to_vec();
            Ok(rows
                .iter()
                .filter(|(id, date)| {
                    self.provenance.get(id).is_some_and(|record| {
                        record.role.eligible_for_aggregate(
                            &date.to_string(),
                            record
                                .coverage_start
                                .map(|value| value.to_string())
                                .as_deref(),
                            record.manual_approved,
                        )
                    })
                })
                .map(|(id, _)| id.clone())
                .collect())
        }
    }

    struct MockActivityRepository {
        activities: Vec<Activity>,
        income_data_by_id: HashMap<String, IncomeData>,
        requested_income_ids: Mutex<Vec<String>>,
        account_wide_data_called: Mutex<bool>,
        source_error: Option<String>,
        derived_error: Option<String>,
    }

    impl MockActivityRepository {
        fn new(activities: Vec<Activity>, income_data: Vec<(String, IncomeData)>) -> Self {
            Self {
                activities,
                income_data_by_id: income_data.into_iter().collect(),
                requested_income_ids: Mutex::new(Vec::new()),
                account_wide_data_called: Mutex::new(false),
                source_error: None,
                derived_error: None,
            }
        }
    }

    #[async_trait]
    impl ActivityRepositoryTrait for MockActivityRepository {
        fn get_activity(&self, _activity_id: &str) -> Result<Activity> {
            unimplemented!()
        }

        fn find_transfer_counterpart(
            &self,
            _group_id: &str,
            _exclude_id: &str,
        ) -> Result<Option<Activity>> {
            unimplemented!()
        }

        fn get_activities(&self) -> Result<Vec<Activity>> {
            unimplemented!()
        }

        fn get_activities_by_account_id(&self, _account_id: &str) -> Result<Vec<Activity>> {
            unimplemented!()
        }

        fn get_activities_by_account_ids(&self, _account_ids: &[String]) -> Result<Vec<Activity>> {
            unimplemented!()
        }

        fn get_trading_activities(&self) -> Result<Vec<Activity>> {
            unimplemented!()
        }

        fn get_income_activities(&self) -> Result<Vec<Activity>> {
            if let Some(message) = &self.source_error {
                return Err(crate::Error::ConstraintViolation(message.clone()));
            }
            Ok(self.activities.clone())
        }

        fn get_contribution_activities(
            &self,
            _account_ids: &[String],
            _start_utc: chrono::DateTime<Utc>,
            _end_exclusive_utc: chrono::DateTime<Utc>,
        ) -> Result<Vec<crate::limits::ContributionActivity>> {
            unimplemented!()
        }

        fn search_activities(
            &self,
            _page: i64,
            _page_size: i64,
            _account_id_filter: Option<Vec<String>>,
            _activity_type_filter: Option<Vec<String>>,
            _asset_id_keyword: Option<String>,
            _sort: Option<crate::activities::Sort>,
            _needs_review_filter: Option<bool>,
            _date_from: Option<NaiveDate>,
            _date_to: Option<NaiveDate>,
            _instrument_type_filter: Option<Vec<String>>,
            _activity_id_filter: Option<Vec<String>>,
        ) -> Result<crate::activities::ActivitySearchResponse> {
            unimplemented!()
        }

        async fn create_activity(
            &self,
            _new_activity: crate::activities::NewActivity,
        ) -> Result<Activity> {
            unimplemented!()
        }

        async fn update_activity(
            &self,
            _activity_update: crate::activities::ActivityUpdate,
        ) -> Result<Activity> {
            unimplemented!()
        }

        async fn delete_activity(&self, _activity_id: String) -> Result<Activity> {
            unimplemented!()
        }

        async fn link_transfer_activities(
            &self,
            _activity_a_id: String,
            _activity_b_id: String,
        ) -> Result<(Activity, Activity)> {
            unimplemented!()
        }

        async fn unlink_transfer_activities(
            &self,
            _activity_a_id: String,
            _activity_b_id: String,
        ) -> Result<(Activity, Activity)> {
            unimplemented!()
        }

        async fn bulk_mutate_activities(
            &self,
            _creates: Vec<crate::activities::NewActivity>,
            _updates: Vec<crate::activities::ActivityUpdate>,
            _delete_ids: Vec<String>,
        ) -> Result<crate::activities::ActivityBulkMutationResult> {
            unimplemented!()
        }

        async fn create_activities(
            &self,
            _activities: Vec<crate::activities::NewActivity>,
        ) -> Result<usize> {
            unimplemented!()
        }

        fn get_first_activity_date(
            &self,
            _account_ids: Option<&[String]>,
        ) -> Result<Option<chrono::DateTime<Utc>>> {
            panic!("income summary must derive oldest date from eligible source rows")
        }

        fn get_import_mapping(
            &self,
            _account_id: &str,
            _context_kind: &str,
        ) -> Result<Option<crate::activities::ImportMapping>> {
            unimplemented!()
        }

        async fn save_import_mapping(
            &self,
            _mapping: &crate::activities::ImportMapping,
        ) -> Result<()> {
            unimplemented!()
        }

        async fn link_account_template(
            &self,
            _account_id: &str,
            _template_id: &str,
            _context_kind: &str,
        ) -> Result<()> {
            unimplemented!()
        }

        fn list_import_templates(&self) -> Result<Vec<crate::activities::ImportTemplate>> {
            unimplemented!()
        }

        fn get_import_template(
            &self,
            _template_id: &str,
        ) -> Result<Option<crate::activities::ImportTemplate>> {
            unimplemented!()
        }

        async fn save_import_template(
            &self,
            _template: &crate::activities::ImportTemplate,
        ) -> Result<()> {
            unimplemented!()
        }

        async fn delete_import_template(&self, _template_id: &str) -> Result<()> {
            unimplemented!()
        }

        fn get_broker_sync_profile(
            &self,
            _account_id: &str,
            _source_system: &str,
        ) -> Result<Option<crate::activities::ImportTemplate>> {
            unimplemented!()
        }

        async fn save_broker_sync_profile(
            &self,
            _template: &crate::activities::ImportTemplate,
        ) -> Result<()> {
            unimplemented!()
        }

        async fn link_broker_sync_profile(
            &self,
            _account_id: &str,
            _template_id: &str,
            _source_system: &str,
        ) -> Result<()> {
            unimplemented!()
        }

        fn calculate_average_cost(&self, _account_id: &str, _asset_id: &str) -> Result<Decimal> {
            unimplemented!()
        }

        fn get_income_activities_data(
            &self,
            _account_ids: Option<&[String]>,
        ) -> Result<Vec<IncomeData>> {
            *self.account_wide_data_called.lock().unwrap() = true;
            Ok(Vec::new())
        }

        fn get_income_activities_data_for_activity_ids(
            &self,
            activity_ids: &[String],
        ) -> Result<Vec<IncomeData>> {
            if let Some(message) = &self.derived_error {
                return Err(crate::Error::ConstraintViolation(message.clone()));
            }
            *self.requested_income_ids.lock().unwrap() = activity_ids.to_vec();
            Ok(activity_ids
                .iter()
                .filter_map(|id| self.income_data_by_id.get(id).cloned())
                .collect())
        }

        fn get_first_activity_date_overall(&self) -> Result<chrono::DateTime<Utc>> {
            panic!("income summary must derive oldest date from eligible source rows")
        }

        fn get_activity_bounds_for_assets(
            &self,
            _asset_ids: &[String],
        ) -> Result<HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>> {
            unimplemented!()
        }

        fn get_holdings_snapshot_bounds_for_assets(
            &self,
            _asset_ids: &[String],
        ) -> Result<HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>> {
            unimplemented!()
        }

        fn check_existing_duplicates(
            &self,
            _idempotency_keys: &[String],
        ) -> Result<HashMap<String, String>> {
            unimplemented!()
        }

        async fn bulk_upsert(
            &self,
            _activities: Vec<crate::activities::ActivityUpsert>,
        ) -> Result<crate::activities::BulkUpsertResult> {
            unimplemented!()
        }

        async fn reassign_asset(&self, _old_asset_id: &str, _new_asset_id: &str) -> Result<u32> {
            unimplemented!()
        }

        async fn get_activity_accounts_and_currencies_by_asset_id(
            &self,
            _asset_id: &str,
        ) -> Result<(Vec<String>, Vec<String>)> {
            unimplemented!()
        }
    }

    fn activity(
        id: &str,
        account_id: &str,
        activity_type: &str,
        y: i32,
        m: u32,
        d: u32,
    ) -> Activity {
        let now = Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap();
        Activity {
            id: id.to_string(),
            account_id: account_id.to_string(),
            asset_id: None,
            activity_type: activity_type.to_string(),
            activity_type_override: None,
            source_type: None,
            subtype: None,
            status: ActivityStatus::Posted,
            activity_date: now,
            settlement_date: None,
            quantity: None,
            unit_price: None,
            amount: Some(Decimal::new(100, 0)),
            fee: None,
            tax: None,
            currency: "USD".to_string(),
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
            created_at: now,
            updated_at: now,
        }
    }

    fn income_data(id: &str, account_id: &str, amount: i64) -> (String, IncomeData) {
        (
            id.to_string(),
            IncomeData {
                date: "2026-01".to_string(),
                income_type: "DIVIDEND".to_string(),
                asset_id: "CASH".to_string(),
                asset_kind: "CASH".to_string(),
                symbol: "CASH".to_string(),
                symbol_name: "Cash".to_string(),
                currency: "USD".to_string(),
                amount: Decimal::new(amount, 0),
                account_id: account_id.to_string(),
                account_name: account_id.to_string(),
            },
        )
    }

    #[tokio::test]
    async fn get_income_summary_filters_source_rows_before_derived_income_query() {
        let repo = Arc::new(MockActivityRepository::new(
            vec![
                activity("allowed-income", "acc-1", "DIVIDEND", 2026, 1, 10),
                activity("denied-income", "acc-1", "DIVIDEND", 2026, 1, 11),
                activity("wrong-account", "acc-2", "DIVIDEND", 2026, 1, 12),
                activity("wrong-type", "acc-1", "BUY", 2026, 1, 13),
            ],
            vec![
                income_data("allowed-income", "acc-1", 100),
                income_data("denied-income", "acc-1", 900),
            ],
        ));
        let policy = Arc::new(MockEligibility {
            allowed_accounts: HashSet::from([String::from("acc-1")]),
            provenance: HashMap::from([(
                String::from("allowed-income"),
                SemanticProvenance {
                    role: ProvenanceRole::LiveCanonical,
                    coverage_start: None,
                    manual_approved: false,
                },
            )]),
            checked_rows: Mutex::new(Vec::new()),
            account_error: None,
            row_error: None,
        });
        let service = IncomeService::new_with_timezone(
            Arc::new(MockFxService {
                fail_conversion: false,
            }),
            repo.clone(),
            Arc::new(RwLock::new(String::from("USD"))),
            Arc::new(RwLock::new(String::from("UTC"))),
            policy.clone(),
        );

        let summaries = service
            .get_income_summary(Some(&[String::from("acc-1")]))
            .await
            .expect("summary");

        assert_eq!(
            repo.requested_income_ids.lock().unwrap().as_slice(),
            ["allowed-income"]
        );
        assert!(!*repo.account_wide_data_called.lock().unwrap());
        assert_eq!(
            policy.checked_rows.lock().unwrap().as_slice(),
            [
                (
                    String::from("allowed-income"),
                    NaiveDate::from_ymd_opt(2026, 1, 10).unwrap()
                ),
                (
                    String::from("denied-income"),
                    NaiveDate::from_ymd_opt(2026, 1, 11).unwrap()
                )
            ]
        );
        assert_eq!(summaries[0].total_income, Decimal::new(100, 0));
    }

    #[tokio::test]
    async fn get_income_summary_returns_empty_without_derived_query_when_policy_denies_all_rows() {
        let repo = Arc::new(MockActivityRepository::new(
            vec![activity("denied-income", "acc-1", "DIVIDEND", 2026, 1, 11)],
            vec![income_data("denied-income", "acc-1", 900)],
        ));
        let policy = Arc::new(MockEligibility {
            allowed_accounts: HashSet::from([String::from("acc-1")]),
            provenance: HashMap::new(),
            checked_rows: Mutex::new(Vec::new()),
            account_error: None,
            row_error: None,
        });
        let service = IncomeService::new_with_timezone(
            Arc::new(MockFxService {
                fail_conversion: false,
            }),
            repo.clone(),
            Arc::new(RwLock::new(String::from("USD"))),
            Arc::new(RwLock::new(String::from("UTC"))),
            policy,
        );

        let summaries = service
            .get_income_summary(Some(&[String::from("acc-1")]))
            .await
            .expect("summary");

        assert!(summaries.is_empty());
        assert!(repo.requested_income_ids.lock().unwrap().is_empty());
        assert!(!*repo.account_wide_data_called.lock().unwrap());
    }

    #[tokio::test]
    async fn get_income_summary_applies_semantic_role_date_and_approval_matrix() {
        let rows = vec![
            ("live", ProvenanceRole::LiveCanonical, None, false),
            (
                "baseline-before",
                ProvenanceRole::OpeningBaseline,
                Some((2026, 1, 11)),
                false,
            ),
            (
                "baseline-equal",
                ProvenanceRole::OpeningBaseline,
                Some((2026, 1, 10)),
                false,
            ),
            (
                "baseline-after",
                ProvenanceRole::OpeningBaseline,
                Some((2026, 1, 9)),
                false,
            ),
            ("manual-approved", ProvenanceRole::ManualCurrent, None, true),
            (
                "manual-unapproved",
                ProvenanceRole::ManualCurrent,
                None,
                false,
            ),
            ("historical", ProvenanceRole::HistoricalContext, None, false),
            (
                "contextual",
                ProvenanceRole::ContextualReference,
                None,
                false,
            ),
            ("missing", ProvenanceRole::LiveCanonical, None, false),
        ];
        let activities = rows
            .iter()
            .map(|(id, _, _, _)| activity(id, "acc-1", "DIVIDEND", 2026, 1, 10))
            .collect();
        let income = rows
            .iter()
            .map(|(id, _, _, _)| income_data(id, "acc-1", 1))
            .collect();
        let provenance = rows
            .iter()
            .filter(|(id, _, _, _)| *id != "missing")
            .map(|(id, role, coverage, approved)| {
                (
                    (*id).to_string(),
                    SemanticProvenance {
                        role: *role,
                        coverage_start: coverage
                            .map(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).unwrap()),
                        manual_approved: *approved,
                    },
                )
            })
            .collect();
        let repo = Arc::new(MockActivityRepository::new(activities, income));
        let policy = Arc::new(MockEligibility {
            allowed_accounts: HashSet::from([String::from("acc-1")]),
            provenance,
            checked_rows: Mutex::new(Vec::new()),
            account_error: None,
            row_error: None,
        });
        let service = IncomeService::new_with_timezone(
            Arc::new(MockFxService {
                fail_conversion: false,
            }),
            repo.clone(),
            Arc::new(RwLock::new(String::from("USD"))),
            Arc::new(RwLock::new(String::from("UTC"))),
            policy,
        );
        let summaries = service
            .get_income_summary(Some(&[String::from("acc-1")]))
            .await
            .unwrap();
        assert_eq!(
            repo.requested_income_ids.lock().unwrap().as_slice(),
            [
                "live",
                "baseline-equal",
                "baseline-after",
                "manual-approved"
            ]
        );
        assert_eq!(summaries[0].total_income, Decimal::new(4, 0));
    }

    #[tokio::test]
    async fn get_income_summary_propagates_source_account_and_row_errors_without_derived_query() {
        let account_ids = [String::from("acc-1")];
        for case in ["source-read", "account-policy", "row-policy"] {
            let mut repo_inner = MockActivityRepository::new(
                vec![activity("income", "acc-1", "DIVIDEND", 2026, 1, 10)],
                vec![income_data("income", "acc-1", 100)],
            );
            repo_inner.source_error = (case == "source-read").then(|| case.to_string());
            let repo = Arc::new(repo_inner);
            let policy = Arc::new(MockEligibility {
                allowed_accounts: HashSet::from([String::from("acc-1")]),
                provenance: HashMap::new(),
                checked_rows: Mutex::new(Vec::new()),
                account_error: (case == "account-policy").then(|| case.to_string()),
                row_error: (case == "row-policy").then(|| case.to_string()),
            });
            let service = IncomeService::new_with_timezone(
                Arc::new(MockFxService {
                    fail_conversion: false,
                }),
                repo.clone(),
                Arc::new(RwLock::new(String::from("USD"))),
                Arc::new(RwLock::new(String::from("UTC"))),
                policy,
            );
            let error = service
                .get_income_summary(Some(&account_ids))
                .await
                .unwrap_err();
            match case {
                "source-read" => assert!(matches!(
                    error,
                    crate::Error::ConstraintViolation(message) if message == "source-read"
                )),
                "account-policy" => assert!(matches!(
                    error,
                    crate::Error::ConstraintViolation(message) if message == "account-policy"
                )),
                "row-policy" => assert!(matches!(
                    error,
                    crate::Error::ConstraintViolation(message) if message == "row-policy"
                )),
                _ => unreachable!(),
            }
            assert!(repo.requested_income_ids.lock().unwrap().is_empty());
            assert!(!*repo.account_wide_data_called.lock().unwrap());
        }
    }

    #[tokio::test]
    async fn get_income_summary_propagates_fx_error_without_partial_aggregate() {
        let repo = Arc::new(MockActivityRepository::new(
            vec![activity("income", "acc-1", "DIVIDEND", 2026, 1, 10)],
            vec![("income".into(), income_data("income", "acc-1", 100).1)],
        ));
        let policy = Arc::new(MockEligibility {
            allowed_accounts: HashSet::from([String::from("acc-1")]),
            provenance: HashMap::from([(
                String::from("income"),
                SemanticProvenance {
                    role: ProvenanceRole::LiveCanonical,
                    coverage_start: None,
                    manual_approved: false,
                },
            )]),
            checked_rows: Mutex::new(Vec::new()),
            account_error: None,
            row_error: None,
        });
        let service = IncomeService::new_with_timezone(
            Arc::new(MockFxService {
                fail_conversion: true,
            }),
            repo.clone(),
            Arc::new(RwLock::new(String::from("USD"))),
            Arc::new(RwLock::new(String::from("UTC"))),
            policy,
        );
        let error = service
            .get_income_summary(Some(&[String::from("acc-1")]))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            crate::Error::ConstraintViolation(message) if message == "fx-failure"
        ));
        assert_eq!(
            repo.requested_income_ids.lock().unwrap().as_slice(),
            ["income"]
        );
    }

    #[tokio::test]
    async fn get_income_summary_public_matrix_covers_periods_groupings_fx_and_income_types() {
        let mut rows = vec![
            activity("usd-dividend", "acc-1", "DIVIDEND", 2024, 1, 10),
            activity("usd-staking", "acc-1", "INTEREST", 2025, 3, 10),
            activity("eur-drip", "acc-2", "DIVIDEND", 2026, 5, 10),
            activity("usd-interest", "acc-2", "INTEREST", 2026, 7, 10),
        ];
        for (index, asset, currency, amount) in [
            (0, "CASH", "USD", 100),
            (1, "ETF", "USD", 50),
            (2, "STOCK", "EUR", 30),
            (3, "BOND", "USD", 20),
        ] {
            rows[index].asset_id = Some(asset.into());
            rows[index].currency = currency.into();
            rows[index].amount = Some(Decimal::new(amount, 0));
        }
        rows[1].subtype = Some("STAKING_REWARD".into());
        rows[2].subtype = Some("DRIP".into());
        let mut staking = income_data("usd-staking", "acc-1", 50).1;
        staking.date = "2025-03".into();
        staking.income_type = "STAKING".into();
        staking.asset_id = "ETF".into();
        let mut drip = income_data("eur-drip", "acc-2", 30).1;
        drip.date = "2026-05".into();
        drip.income_type = "DRIP".into();
        drip.asset_id = "STOCK".into();
        drip.currency = "EUR".into();
        let mut interest = income_data("usd-interest", "acc-2", 20).1;
        interest.date = "2026-07".into();
        interest.income_type = "INTEREST".into();
        interest.asset_id = "BOND".into();
        let mut dividend = income_data("usd-dividend", "acc-1", 100).1;
        dividend.date = "2024-01".into();
        let income = vec![
            ("usd-dividend".into(), dividend),
            ("usd-staking".into(), staking),
            ("eur-drip".into(), drip),
            ("usd-interest".into(), interest),
        ];
        let provenance = rows
            .iter()
            .map(|row| {
                (
                    row.id.clone(),
                    SemanticProvenance {
                        role: ProvenanceRole::LiveCanonical,
                        coverage_start: None,
                        manual_approved: false,
                    },
                )
            })
            .collect();
        let repo = Arc::new(MockActivityRepository::new(rows, income));
        let policy = Arc::new(MockEligibility {
            allowed_accounts: HashSet::from(["acc-1".into(), "acc-2".into()]),
            provenance,
            checked_rows: Mutex::new(Vec::new()),
            account_error: None,
            row_error: None,
        });
        let service = IncomeService::new_with_timezone(
            Arc::new(MockFxService {
                fail_conversion: false,
            }),
            repo,
            Arc::new(RwLock::new("USD".into())),
            Arc::new(RwLock::new("UTC".into())),
            policy,
        );
        let summaries = service.get_income_summary(None).await.unwrap();
        fn assert_summary(
            summary: &IncomeSummary,
            period: &str,
            by_month: HashMap<String, Decimal>,
            by_type: HashMap<String, Decimal>,
            by_asset: HashMap<String, (String, String, String, String, Decimal)>,
            by_currency: HashMap<String, Decimal>,
            by_account: HashMap<String, (String, HashMap<String, Decimal>, Decimal)>,
            total_income: Decimal,
            monthly_average: Decimal,
            yoy_growth: Option<Decimal>,
        ) {
            assert_eq!(summary.period, period);
            assert_eq!(summary.by_month, by_month);
            assert_eq!(summary.by_type, by_type);
            assert_eq!(summary.by_currency, by_currency);
            assert_eq!(summary.total_income, total_income);
            assert_eq!(summary.currency, "USD");
            assert_eq!(summary.monthly_average, monthly_average);
            assert_eq!(summary.yoy_growth, yoy_growth);
            assert_eq!(summary.by_asset.len(), by_asset.len());
            for (asset_id, (kind, symbol, name, expected_id, income)) in by_asset {
                let asset = summary.by_asset.get(&asset_id).unwrap();
                assert_eq!(
                    (&asset.kind, &asset.symbol, &asset.name),
                    (&kind, &symbol, &name)
                );
                assert_eq!(&asset.asset_id, &expected_id);
                assert_eq!(asset.income, income);
            }
            assert_eq!(summary.by_account.len(), by_account.len());
            for (account_id, (name, months, total)) in by_account {
                let account = summary.by_account.get(&account_id).unwrap();
                assert_eq!(account.account_id, account_id);
                assert_eq!(account.account_name, name);
                assert_eq!(account.by_month, months);
                assert_eq!(account.total, total);
            }
        }

        let assets = HashMap::from([
            (
                "CASH".into(),
                (
                    "CASH".into(),
                    "CASH".into(),
                    "Cash".into(),
                    "CASH".into(),
                    Decimal::new(100, 0),
                ),
            ),
            (
                "ETF".into(),
                (
                    "CASH".into(),
                    "CASH".into(),
                    "Cash".into(),
                    "ETF".into(),
                    Decimal::new(50, 0),
                ),
            ),
            (
                "STOCK".into(),
                (
                    "CASH".into(),
                    "CASH".into(),
                    "Cash".into(),
                    "STOCK".into(),
                    Decimal::new(60, 0),
                ),
            ),
            (
                "BOND".into(),
                (
                    "CASH".into(),
                    "CASH".into(),
                    "Cash".into(),
                    "BOND".into(),
                    Decimal::new(20, 0),
                ),
            ),
        ]);
        let accounts = HashMap::from([
            (
                "acc-1".into(),
                (
                    "acc-1".into(),
                    HashMap::from([
                        ("2024-01".into(), Decimal::new(100, 0)),
                        ("2025-03".into(), Decimal::new(50, 0)),
                    ]),
                    Decimal::new(150, 0),
                ),
            ),
            (
                "acc-2".into(),
                (
                    "acc-2".into(),
                    HashMap::from([
                        ("2026-05".into(), Decimal::new(60, 0)),
                        ("2026-07".into(), Decimal::new(20, 0)),
                    ]),
                    Decimal::new(80, 0),
                ),
            ),
        ]);
        assert_summary(
            &summaries[0],
            "ALL",
            HashMap::from([
                ("2024-01".into(), Decimal::new(100, 0)),
                ("2025-03".into(), Decimal::new(50, 0)),
                ("2026-05".into(), Decimal::new(60, 0)),
                ("2026-07".into(), Decimal::new(20, 0)),
            ]),
            HashMap::from([
                ("DIVIDEND".into(), Decimal::new(100, 0)),
                ("STAKING".into(), Decimal::new(50, 0)),
                ("DRIP".into(), Decimal::new(60, 0)),
                ("INTEREST".into(), Decimal::new(20, 0)),
            ]),
            assets.clone(),
            HashMap::from([
                ("USD".into(), Decimal::new(170, 0)),
                ("EUR".into(), Decimal::new(30, 0)),
            ]),
            accounts.clone(),
            Decimal::new(230, 0),
            Decimal::new(767, 2),
            None,
        );
        assert_summary(
            &summaries[1],
            "YTD",
            HashMap::from([
                ("2026-05".into(), Decimal::new(60, 0)),
                ("2026-07".into(), Decimal::new(20, 0)),
            ]),
            HashMap::from([
                ("DRIP".into(), Decimal::new(60, 0)),
                ("INTEREST".into(), Decimal::new(20, 0)),
            ]),
            HashMap::from([
                ("STOCK".into(), assets["STOCK"].clone()),
                ("BOND".into(), assets["BOND"].clone()),
            ]),
            HashMap::from([
                ("EUR".into(), Decimal::new(30, 0)),
                ("USD".into(), Decimal::new(20, 0)),
            ]),
            HashMap::from([("acc-2".into(), accounts["acc-2"].clone())]),
            Decimal::new(80, 0),
            Decimal::new(1143, 2),
            Some(Decimal::new(60, 2)),
        );
        assert_summary(
            &summaries[2],
            "LAST_YEAR",
            HashMap::from([("2025-03".into(), Decimal::new(50, 0))]),
            HashMap::from([("STAKING".into(), Decimal::new(50, 0))]),
            HashMap::from([("ETF".into(), assets["ETF"].clone())]),
            HashMap::from([("USD".into(), Decimal::new(50, 0))]),
            HashMap::from([(
                "acc-1".into(),
                (
                    "acc-1".into(),
                    HashMap::from([("2025-03".into(), Decimal::new(50, 0))]),
                    Decimal::new(50, 0),
                ),
            )]),
            Decimal::new(50, 0),
            Decimal::new(417, 2),
            Some(Decimal::new(-50, 2)),
        );
        assert_summary(
            &summaries[3],
            "TWO_YEARS_AGO",
            HashMap::from([("2024-01".into(), Decimal::new(100, 0))]),
            HashMap::from([("DIVIDEND".into(), Decimal::new(100, 0))]),
            HashMap::from([("CASH".into(), assets["CASH"].clone())]),
            HashMap::from([("USD".into(), Decimal::new(100, 0))]),
            HashMap::from([(
                "acc-1".into(),
                (
                    "acc-1".into(),
                    HashMap::from([("2024-01".into(), Decimal::new(100, 0))]),
                    Decimal::new(100, 0),
                ),
            )]),
            Decimal::new(100, 0),
            Decimal::new(833, 2),
            None,
        );
    }
}
