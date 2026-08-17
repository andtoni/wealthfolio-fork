use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use wealthfolio_core::accounts::{
    account_supports_purpose, AccountPurpose, AccountRepositoryTrait,
    AggregateEligibilityServiceTrait,
};
use wealthfolio_core::activities::{
    Activity, ActivityRepositoryTrait, ActivityStatus, TransferPairResolution,
};
use wealthfolio_core::taxonomies::TaxonomyServiceTrait;

use super::model::{
    CategoryBreakdownRow, CategorySpending, DayBucket, DayCategoryBucket, EventCategorySpending,
    EventSpendingSummary, EventSummariesRequest, MonthlyReport, PeriodSummary, ReportRequest,
    SpendingSummary, SubcategorySpending,
};
use crate::activity_allocations::{
    allocations_for_taxonomy, group_assignments, group_splits, AssignmentsByActivity,
    SplitsByActivity,
};
use crate::activity_assignments::ActivityTaxonomyAssignmentRepositoryTrait;
use crate::activity_classification::{
    activity_abs_amount, classify_activity, classify_activity_for_aggregation, decimal_to_f64,
    within_spending_transfer_groups, SpendingClassification,
};
use crate::activity_splits::ActivitySplitRepositoryTrait;
use crate::aggregate_eligibility::{
    filter_eligible_activity_rows, resolve_eligible_spending_accounts,
};
use crate::events::EventsService;
use crate::settings::SpendingSettingsService;

const SPENDING_TAXONOMY: &str = "spending_categories";
/// Sentinel category id used in spending_breakdown rows for activities that
/// have no spending_categories assignment. Mirrors the insight pipeline's
/// `UncategorizedBucket` so the two reports agree on totals. Keep in sync
/// with `insight-projection.ts::UNCATEGORIZED_CATEGORY_ID`.
const UNCATEGORIZED_CATEGORY_ID: &str = "__uncategorized__";
const INCOME_TAXONOMY: &str = "income_sources";
const SAVINGS_TAXONOMY: &str = "savings_categories";

type CategoryAccumulator = (Option<String>, String, Option<String>, Decimal, usize);
type SubcategoryAccumulator = (
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    Decimal,
    usize,
);

pub struct AnalyticsService {
    activity_repo: Arc<dyn ActivityRepositoryTrait>,
    account_repo: Arc<dyn AccountRepositoryTrait>,
    assignment_repo: Arc<dyn ActivityTaxonomyAssignmentRepositoryTrait>,
    split_repo: Arc<dyn ActivitySplitRepositoryTrait>,
    settings: Arc<SpendingSettingsService>,
    taxonomy_service: Arc<dyn TaxonomyServiceTrait>,
    events_service: Arc<EventsService>,
    fx_service: Arc<dyn wealthfolio_core::fx::FxServiceTrait>,
    activity_events: Arc<dyn crate::activity_events::ActivityEventsRepositoryTrait>,
    aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
}

impl AnalyticsService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        activity_repo: Arc<dyn ActivityRepositoryTrait>,
        account_repo: Arc<dyn AccountRepositoryTrait>,
        assignment_repo: Arc<dyn ActivityTaxonomyAssignmentRepositoryTrait>,
        split_repo: Arc<dyn ActivitySplitRepositoryTrait>,
        settings: Arc<SpendingSettingsService>,
        taxonomy_service: Arc<dyn TaxonomyServiceTrait>,
        events_service: Arc<EventsService>,
        fx_service: Arc<dyn wealthfolio_core::fx::FxServiceTrait>,
        activity_events: Arc<dyn crate::activity_events::ActivityEventsRepositoryTrait>,
        aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
    ) -> Self {
        Self {
            activity_repo,
            account_repo,
            assignment_repo,
            split_repo,
            settings,
            taxonomy_service,
            events_service,
            fx_service,
            activity_events,
            aggregate_eligibility_service,
        }
    }

    fn resolve_spending_account_types(
        &self,
        account_ids: &[String],
    ) -> Result<(Vec<String>, HashMap<String, String>)> {
        let accounts = self
            .account_repo
            .list(None, Some(false), Some(account_ids))
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let account_types: HashMap<String, String> = accounts
            .into_iter()
            .filter(|a| account_supports_purpose(&a.account_type, AccountPurpose::Spending))
            .map(|a| (a.id, a.account_type))
            .collect();
        let spending_account_ids = account_ids
            .iter()
            .filter(|id| account_types.contains_key(id.as_str()))
            .cloned()
            .collect();

        Ok((spending_account_ids, account_types))
    }

    /// Compute a monthly report covering [start_date, end_date].
    /// "Prior" period uses an equally-sized window immediately preceding the current one.
    /// `timezone` (IANA name, may be empty) drives per-day bucketing so a
    /// midnight-local activity lands on the date the user perceives. Empty/
    /// invalid values fall back to UTC.
    /// `base_currency` is the FX target — every activity amount is converted
    /// to it at `end_date` (snapshot-date convention, matches insight).
    pub async fn monthly_report(
        &self,
        req: ReportRequest,
        timezone: &str,
        base_currency: &str,
    ) -> Result<MonthlyReport> {
        let s = self.settings.get().await?;
        if !s.enabled || s.account_ids.is_empty() {
            return Ok(MonthlyReport {
                current: PeriodSummary::default(),
                prior: PeriodSummary::default(),
                spending_breakdown: vec![],
                income_breakdown: vec![],
                savings_breakdown: vec![],
                by_day: vec![],
                by_day_by_category: vec![],
            });
        }
        let requested_accounts: Vec<String> = match req.account_ids.clone() {
            Some(ids) => ids
                .into_iter()
                .filter(|id| s.account_ids.contains(id))
                .collect(),
            None => s.account_ids.clone(),
        };
        if requested_accounts.is_empty() {
            return Ok(MonthlyReport {
                current: PeriodSummary::default(),
                prior: PeriodSummary::default(),
                spending_breakdown: vec![],
                income_breakdown: vec![],
                savings_breakdown: vec![],
                by_day: vec![],
                by_day_by_category: vec![],
            });
        }
        let start = DateTime::parse_from_rfc3339(&req.start_date)?.with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339(&req.end_date)?.with_timezone(&Utc);

        let (all_spending_account_ids, _) = self.resolve_spending_account_types(&s.account_ids)?;
        if all_spending_account_ids.is_empty() {
            return Ok(MonthlyReport {
                current: PeriodSummary::default(),
                prior: PeriodSummary::default(),
                spending_breakdown: vec![],
                income_breakdown: vec![],
                savings_breakdown: vec![],
                by_day: vec![],
                by_day_by_category: vec![],
            });
        }

        let eligible_accounts = resolve_eligible_spending_accounts(
            self.account_repo.as_ref(),
            self.aggregate_eligibility_service.as_ref(),
            &requested_accounts,
            end.date_naive(),
        )
        .await?;
        if eligible_accounts.ids.is_empty() {
            return Ok(MonthlyReport {
                current: PeriodSummary::default(),
                prior: PeriodSummary::default(),
                spending_breakdown: vec![],
                income_breakdown: vec![],
                savings_breakdown: vec![],
                by_day: vec![],
                by_day_by_category: vec![],
            });
        }
        let eligible_account_ids: HashSet<&str> =
            eligible_accounts.ids.iter().map(String::as_str).collect();
        let target_account_ids: HashSet<String> = requested_accounts
            .into_iter()
            .filter(|id| eligible_account_ids.contains(id.as_str()))
            .collect();
        if target_account_ids.is_empty() {
            return Ok(MonthlyReport {
                current: PeriodSummary::default(),
                prior: PeriodSummary::default(),
                spending_breakdown: vec![],
                income_breakdown: vec![],
                savings_breakdown: vec![],
                by_day: vec![],
                by_day_by_category: vec![],
            });
        }
        // Prior window: same inclusive length, immediately preceding `start`.
        // `period_secs` includes both endpoints because activity filters use
        // inclusive comparisons.
        let period_secs = (end - start).num_seconds().max(0) + 1;
        let prior_end = start - Duration::seconds(1);
        let prior_start = prior_end - Duration::seconds((period_secs - 1).max(0));

        let context_activities = self
            .activity_repo
            .get_activities_by_account_ids(&all_spending_account_ids)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let transfer_context_acts: Vec<&Activity> = context_activities.iter().collect();
        let transfer_groups = within_spending_transfer_groups(&transfer_context_acts);

        let in_window = |a: &Activity, lo: DateTime<Utc>, hi: DateTime<Utc>| {
            a.activity_date >= lo && a.activity_date <= hi
        };

        let mut activities = context_activities
            .iter()
            .filter(|a| target_account_ids.contains(&a.account_id) && in_window(a, start, end))
            .chain(context_activities.iter().filter(|a| {
                target_account_ids.contains(&a.account_id) && in_window(a, prior_start, prior_end)
            }))
            .cloned()
            .collect::<Vec<_>>();
        let transfer_policy_context =
            aggregate_transfer_context_rows(&activities, &context_activities);
        activities = filter_eligible_activity_rows(
            self.aggregate_eligibility_service.as_ref(),
            activities,
            &transfer_policy_context,
        )
        .await?;

        let current_acts: Vec<&Activity> = activities
            .iter()
            .filter(|a| in_window(a, start, end))
            .collect();
        let prior_acts: Vec<&Activity> = activities
            .iter()
            .filter(|a| in_window(a, prior_start, prior_end))
            .collect();

        // Assignments for current + prior windows. Totals are bucketed by
        // activity flow; assignments only label spending/income breakdowns.
        let assignment_ids: Vec<String> = current_acts
            .iter()
            .chain(prior_acts.iter())
            .map(|a| a.id.clone())
            .collect();
        let all_assignments = self
            .assignment_repo
            .list_for_activities(&assignment_ids)
            .await?;
        let assignments_by_activity = group_assignments(all_assignments);
        let splits_by_activity =
            group_splits(self.split_repo.list_for_activities(&assignment_ids).await?);
        // FX as-of: end of the active window for current, end of the prior
        // window for prior. Matches insight's per-window snapshot convention.
        let fx_as_of_current = end.date_naive();
        let fx_as_of_prior = prior_end.date_naive();
        let fx = self.fx_service.as_ref();
        let current = summarize(
            &current_acts,
            &eligible_accounts.account_types,
            &transfer_groups,
            fx,
            base_currency,
            fx_as_of_current,
        );
        let prior = summarize(
            &prior_acts,
            &eligible_accounts.account_types,
            &transfer_groups,
            fx,
            base_currency,
            fx_as_of_prior,
        );

        // Per-day buckets (current period only). All amounts FX-converted to
        // base_currency at fx_as_of_current so daily totals roll up to the
        // headline outflow within rounding tolerance.
        let mut by_day_map: HashMap<NaiveDate, (Decimal, Decimal)> = HashMap::new();
        for a in &current_acts {
            let Some(classification) = classification_for(a, &eligible_accounts.account_types)
            else {
                continue;
            };
            let amt = activity_abs_amount(a);
            let income_native = classification.income_amount(amt);
            let spending_native = classification.spending_amount(amt);
            if income_native == Decimal::ZERO && spending_native == Decimal::ZERO {
                continue;
            }
            let income_amount = fx_to_target(
                fx,
                income_native,
                &a.currency,
                base_currency,
                fx_as_of_current,
            )
            .unwrap_or(Decimal::ZERO);
            let spending_amount = fx_to_target(
                fx,
                spending_native,
                &a.currency,
                base_currency,
                fx_as_of_current,
            )
            .unwrap_or(Decimal::ZERO);
            let d = wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(
                a.activity_date,
                timezone,
            );
            let entry = by_day_map
                .entry(d)
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            entry.0 += income_amount;
            entry.1 += spending_amount;
        }
        // Signed per-day outflow so `Σ by_day.outflow == current.net` minus
        // income, matching the headline. Refund days emit a negative outflow;
        // chart consumers that want non-negative bars should clamp at render.
        let mut by_day: Vec<DayBucket> = by_day_map
            .into_iter()
            .map(|(d, (income, outflow))| DayBucket {
                date: format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()),
                income: decimal_to_f64(income),
                outflow: decimal_to_f64(outflow),
            })
            .filter(|bucket| bucket.income != 0.0 || bucket.outflow != 0.0)
            .collect();
        by_day.sort_by(|a, b| a.date.cmp(&b.date));

        // Category breakdown — reuses `assignments_by_activity` loaded above
        // (covers current + prior in a single batched call).
        let mut spending_acc: HashMap<(String, String), (Decimal, usize)> = HashMap::new();
        let mut income_acc: HashMap<(String, String), (Decimal, usize)> = HashMap::new();
        let mut savings_acc: HashMap<(String, String), (Decimal, usize)> = HashMap::new();
        // (date, taxonomy_id, category_id) → (amount, count)
        let mut by_day_cat_acc: HashMap<(String, String, String), (Decimal, usize)> =
            HashMap::new();
        for a in &current_acts {
            let Some(account_type) = eligible_accounts.account_types.get(&a.account_id) else {
                continue;
            };
            let classification =
                classify_activity_for_aggregation(a, account_type, &transfer_groups);
            let amt = activity_abs_amount(a);
            let income_native = classification.income_amount(amt);
            let spending_native = classification.spending_amount(amt);
            let saving_native = classification.saving_amount(amt);
            if income_native == Decimal::ZERO
                && spending_native == Decimal::ZERO
                && saving_native == Decimal::ZERO
            {
                continue;
            }
            let day = wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(
                a.activity_date,
                timezone,
            );
            let day_str = format!("{:04}-{:02}-{:02}", day.year(), day.month(), day.day());
            add_report_breakdown_allocations(
                &mut spending_acc,
                &mut by_day_cat_acc,
                &a.id,
                SPENDING_TAXONOMY,
                spending_native,
                &assignments_by_activity,
                &splits_by_activity,
                fx,
                &a.currency,
                base_currency,
                fx_as_of_current,
                &day_str,
                true,
            );
            add_report_breakdown_allocations(
                &mut income_acc,
                &mut by_day_cat_acc,
                &a.id,
                INCOME_TAXONOMY,
                income_native,
                &assignments_by_activity,
                &splits_by_activity,
                fx,
                &a.currency,
                base_currency,
                fx_as_of_current,
                &day_str,
                false,
            );
            add_report_breakdown_allocations(
                &mut savings_acc,
                &mut by_day_cat_acc,
                &a.id,
                SAVINGS_TAXONOMY,
                saving_native,
                &assignments_by_activity,
                &splits_by_activity,
                fx,
                &a.currency,
                base_currency,
                fx_as_of_current,
                &day_str,
                false,
            );
        }

        let mut spending_breakdown: Vec<CategoryBreakdownRow> = spending_acc
            .into_iter()
            .filter(|(_, (amount, _))| *amount != Decimal::ZERO)
            .map(
                |((taxonomy_id, category_id), (amount, count))| CategoryBreakdownRow {
                    taxonomy_id,
                    category_id,
                    amount: decimal_to_f64(amount),
                    count,
                },
            )
            .collect();
        spending_breakdown.sort_by(|a, b| {
            b.amount
                .partial_cmp(&a.amount)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut income_breakdown: Vec<CategoryBreakdownRow> = income_acc
            .into_iter()
            .map(
                |((taxonomy_id, category_id), (amount, count))| CategoryBreakdownRow {
                    taxonomy_id,
                    category_id,
                    amount: decimal_to_f64(amount),
                    count,
                },
            )
            .collect();
        income_breakdown.sort_by(|a, b| {
            b.amount
                .partial_cmp(&a.amount)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut savings_breakdown: Vec<CategoryBreakdownRow> = savings_acc
            .into_iter()
            .map(
                |((taxonomy_id, category_id), (amount, count))| CategoryBreakdownRow {
                    taxonomy_id,
                    category_id,
                    amount: decimal_to_f64(amount),
                    count,
                },
            )
            .collect();
        savings_breakdown.sort_by(|a, b| {
            b.amount
                .partial_cmp(&a.amount)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut by_day_by_category: Vec<DayCategoryBucket> = by_day_cat_acc
            .into_iter()
            .filter(|(_, (amount, _))| *amount != Decimal::ZERO)
            .map(
                |((date, taxonomy_id, category_id), (amount, count))| DayCategoryBucket {
                    date,
                    taxonomy_id,
                    category_id,
                    amount: decimal_to_f64(amount),
                    count,
                },
            )
            .collect();
        by_day_by_category.sort_by(|a, b| a.date.cmp(&b.date));

        Ok(MonthlyReport {
            current,
            prior,
            spending_breakdown,
            income_breakdown,
            savings_breakdown,
            by_day,
            by_day_by_category,
        })
    }
}

fn summarize(
    acts: &[&Activity],
    account_types: &HashMap<String, String>,
    within_groups: &std::collections::HashSet<String>,
    fx: &dyn wealthfolio_core::fx::FxServiceTrait,
    target_currency: &str,
    fx_as_of: NaiveDate,
) -> PeriodSummary {
    let mut income = Decimal::ZERO;
    let mut outflow = Decimal::ZERO;
    let mut saved = Decimal::ZERO;
    let mut count = 0;
    for a in acts {
        let Some(account_type) = account_types.get(&a.account_id) else {
            continue;
        };
        // Income-pattern buckets: classification decides spend/income/saving;
        // a cross-boundary transfer-out → Saving. Amounts never overlap.
        let classification = classify_activity_for_aggregation(a, account_type, within_groups);
        let amt = activity_abs_amount(a);
        let income_native = classification.income_amount(amt);
        let spending_native = classification.spending_amount(amt);
        let saving_native = classification.saving_amount(amt);
        if income_native == Decimal::ZERO
            && spending_native == Decimal::ZERO
            && saving_native == Decimal::ZERO
        {
            continue;
        }
        // FX-convert each activity to the report currency at `fx_as_of`,
        // matching insight::aggregate_spend so the two services agree.
        income += fx_to_target(fx, income_native, &a.currency, target_currency, fx_as_of)
            .unwrap_or(Decimal::ZERO);
        outflow += fx_to_target(fx, spending_native, &a.currency, target_currency, fx_as_of)
            .unwrap_or(Decimal::ZERO);
        saved += fx_to_target(fx, saving_native, &a.currency, target_currency, fx_as_of)
            .unwrap_or(Decimal::ZERO);
        // `count` is "activities that contributed income OR outflow" — it
        // counts each activity once, regardless of how many spending/income
        // category assignments it carries. Consumers that need spending-only
        // counts should read `Σ spending_breakdown.count` (per-assignment),
        // which will be `<= count` when income-only activities exist. The
        // two fields measure different things; they're not expected to match.
        count += 1;
    }
    // Monetary fields are signed so `current.net == income - outflow - saved`
    // holds by construction, matching the insight pipeline's
    // `Headline.net_cashflow`. `outflow` is consumption-only (savings excluded);
    // UI that wants a non-negative "Spent" badge clamps at render time.
    PeriodSummary {
        income: decimal_to_f64(income),
        outflow: decimal_to_f64(outflow),
        saved: decimal_to_f64(saved),
        net: decimal_to_f64(income - outflow - saved),
        count,
    }
}

#[allow(clippy::too_many_arguments)]
fn add_report_breakdown_allocations(
    acc: &mut HashMap<(String, String), (Decimal, usize)>,
    by_day_cat_acc: &mut HashMap<(String, String, String), (Decimal, usize)>,
    activity_id: &str,
    taxonomy_id: &str,
    native_amount: Decimal,
    assignments_by_activity: &AssignmentsByActivity,
    splits_by_activity: &SplitsByActivity,
    fx: &dyn wealthfolio_core::fx::FxServiceTrait,
    from_currency: &str,
    target_currency: &str,
    fx_as_of: NaiveDate,
    day: &str,
    uncategorized_day_bucket: bool,
) {
    if native_amount == Decimal::ZERO {
        return;
    }

    let allocations = allocations_for_taxonomy(
        activity_id,
        taxonomy_id,
        native_amount,
        assignments_by_activity,
        splits_by_activity,
    );
    if allocations.is_empty() {
        let amount = fx_to_target(fx, native_amount, from_currency, target_currency, fx_as_of)
            .unwrap_or(Decimal::ZERO);
        if amount == Decimal::ZERO {
            return;
        }
        let entry = acc
            .entry((
                taxonomy_id.to_string(),
                UNCATEGORIZED_CATEGORY_ID.to_string(),
            ))
            .or_insert((Decimal::ZERO, 0));
        entry.0 += amount;
        entry.1 += 1;
        if taxonomy_id == SPENDING_TAXONOMY && uncategorized_day_bucket {
            let dc = by_day_cat_acc
                .entry((
                    day.to_string(),
                    taxonomy_id.to_string(),
                    UNCATEGORIZED_CATEGORY_ID.to_string(),
                ))
                .or_insert((Decimal::ZERO, 0));
            dc.0 += amount;
            dc.1 += 1;
        }
        return;
    }

    for allocation in allocations {
        let amount = fx_to_target(
            fx,
            allocation.amount,
            from_currency,
            target_currency,
            fx_as_of,
        )
        .unwrap_or(Decimal::ZERO);
        if amount == Decimal::ZERO {
            continue;
        }
        let entry = acc
            .entry((taxonomy_id.to_string(), allocation.category_id.clone()))
            .or_insert((Decimal::ZERO, 0));
        entry.0 += amount;
        entry.1 += 1;
        if taxonomy_id == SPENDING_TAXONOMY {
            let dc = by_day_cat_acc
                .entry((
                    day.to_string(),
                    taxonomy_id.to_string(),
                    allocation.category_id,
                ))
                .or_insert((Decimal::ZERO, 0));
            dc.0 += amount;
            dc.1 += 1;
        }
    }
}

fn classification_for(
    activity: &Activity,
    account_types: &HashMap<String, String>,
) -> Option<SpendingClassification> {
    account_types
        .get(&activity.account_id)
        .map(|account_type| classify_activity(activity, account_type))
}

fn aggregate_transfer_context_rows(candidates: &[Activity], context: &[Activity]) -> Vec<Activity> {
    let resolution = TransferPairResolution::from_activities(context);
    let mut context_ids = HashSet::new();

    for activity in candidates {
        if let Some(pair) = resolution.pair_for_activity(&activity.id) {
            context_ids.insert(pair.transfer_in.id.as_str());
            context_ids.insert(pair.transfer_out.id.as_str());
        }
        if let Some(group) = resolution.invalid_group_for_activity(&activity.id) {
            for activity_id in &group.activity_ids {
                context_ids.insert(activity_id.as_str());
            }
        }
    }

    context
        .iter()
        .filter(|activity| context_ids.contains(activity.id.as_str()))
        .cloned()
        .collect()
}

/// Convert a native amount to the report's target currency at `as_of`.
/// Mirrors `insight::service::fx_to_target` — same convention (one rate per
/// report, snapshot-date style) so analytics and insight surfaces agree.
/// Same-currency short-circuit; on FxService error, returns None so callers
/// exclude the native amount instead of mixing currencies into the target total.
fn fx_to_target(
    fx: &dyn wealthfolio_core::fx::FxServiceTrait,
    amount: Decimal,
    from: &str,
    to: &str,
    as_of: NaiveDate,
) -> Option<Decimal> {
    if amount == Decimal::ZERO || from == to || from.is_empty() {
        return Some(amount);
    }
    match fx.convert_currency_for_date(amount, from, to, as_of) {
        Ok(converted) => Some(converted),
        Err(e) => {
            log::warn!(
                "spending analytics FX conversion {}→{} on {} failed ({}); excluding native amount",
                from,
                to,
                as_of,
                e,
            );
            None
        }
    }
}

// ====================== SpendingSummary (PR-style multi-period rollup) ======================

impl AnalyticsService {
    /// Compute spending summaries for the periods consumed by the spending overview UI:
    /// `TOTAL`, `YTD`, `LAST_YEAR`, `TWO_YEARS_AGO`. The frontend picks the relevant one.
    ///
    /// `include_event_ids` — if Some(non-empty), only activities with `event_id` in this set are counted.
    /// `include_all_events` — if true, only activities that ARE tagged with any event are counted.
    /// `base_currency` is the FX target (every amount is converted to it).
    /// `timezone` drives by-month bucketing inside `build_summary`.
    pub async fn spending_summary(
        &self,
        include_event_ids: Option<Vec<String>>,
        include_all_events: Option<bool>,
        base_currency: &str,
        timezone: &str,
    ) -> Result<Vec<SpendingSummary>> {
        let s = self.settings.get().await?;
        let mut out = Vec::with_capacity(4);
        if !s.enabled || s.account_ids.is_empty() {
            for period in ["TOTAL", "YTD", "LAST_YEAR", "TWO_YEARS_AGO"] {
                out.push(empty_summary(period));
            }
            return Ok(out);
        }
        let now = Utc::now();
        let today_local =
            wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(now, timezone);
        let (all_spending_accounts, _) = self.resolve_spending_account_types(&s.account_ids)?;
        if all_spending_accounts.is_empty() {
            for period in ["TOTAL", "YTD", "LAST_YEAR", "TWO_YEARS_AGO"] {
                out.push(empty_summary(period));
            }
            return Ok(out);
        }

        let eligible_accounts = resolve_eligible_spending_accounts(
            self.account_repo.as_ref(),
            self.aggregate_eligibility_service.as_ref(),
            &s.account_ids,
            today_local,
        )
        .await?;
        if eligible_accounts.ids.is_empty() {
            for period in ["TOTAL", "YTD", "LAST_YEAR", "TWO_YEARS_AGO"] {
                out.push(empty_summary(period));
            }
            return Ok(out);
        }

        // Load all activities for the configured spending accounts so transfer
        // pair eligibility can fail closed before any classification or joins.
        let context_activities = self
            .activity_repo
            .get_activities_by_account_ids(&all_spending_accounts)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let eligible_account_ids: HashSet<&str> =
            eligible_accounts.ids.iter().map(String::as_str).collect();
        let mut activities = context_activities
            .iter()
            .filter(|activity| eligible_account_ids.contains(activity.account_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let transfer_policy_context =
            aggregate_transfer_context_rows(&activities, &context_activities);
        activities = filter_eligible_activity_rows(
            self.aggregate_eligibility_service.as_ref(),
            activities,
            &transfer_policy_context,
        )
        .await?;

        // Pull category metadata for spending_categories (for names + colors + parent map)
        let taxonomy_with_cats = self
            .taxonomy_service
            .get_taxonomy(SPENDING_TAXONOMY)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let categories = taxonomy_with_cats
            .map(|tw| tw.categories)
            .unwrap_or_default();
        let mut cat_meta: HashMap<String, (String, Option<String>, Option<String>)> =
            HashMap::new();
        for c in &categories {
            // (name, color_opt, parent_id_opt)
            cat_meta.insert(
                c.id.clone(),
                (c.name.clone(), Some(c.color.clone()), c.parent_id.clone()),
            );
        }

        // Pre-load assignments per activity in scope (only for outflow + spending taxonomy).
        // Single batched lookup, then group by activity_id — avoids the N+1
        // round-trip that the previous per-activity loop performed.
        let spending_ids: Vec<String> = activities
            .iter()
            .filter(|a| {
                classification_for(a, &eligible_accounts.account_types)
                    .map(|c| c.spending_amount(activity_abs_amount(a)) != Decimal::ZERO)
                    .unwrap_or(false)
            })
            .map(|a| a.id.clone())
            .collect();
        let all_assignments = self
            .assignment_repo
            .list_for_activities(&spending_ids)
            .await?;
        let assignments_by_activity = group_assignments(all_assignments);
        let splits_by_activity =
            group_splits(self.split_repo.list_for_activities(&spending_ids).await?);

        // Event filter set
        let include_set: Option<HashSet<String>> = include_event_ids
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| v.iter().cloned().collect());
        let only_with_events = include_all_events.unwrap_or(false);

        // Preload activity → event_id map once. Used by the filter helper
        // below in place of the old `a.event_id` field. Only loaded if at
        // least one event-related filter is active.
        let event_tags: HashMap<String, String> = if only_with_events || include_set.is_some() {
            let ids: Vec<String> = spending_ids.clone();
            self.activity_events.list_for_activities(&ids).await?
        } else {
            HashMap::new()
        };

        // Year boundaries are user-perceived calendar dates: a UTC+12 user
        // just before midnight on New Year's Eve still considers themselves
        // in the outgoing year. We derive `year_now` from the user's local
        // date and compare each activity's user-local date against
        // [year-01-01, year-12-31] ranges. Sub-day precision isn't needed
        // (the bounds are whole calendar days), so we work in NaiveDate.
        let year_now = today_local.year();
        let ytd_start = NaiveDate::from_ymd_opt(year_now, 1, 1).unwrap();
        let last_year_start = NaiveDate::from_ymd_opt(year_now - 1, 1, 1).unwrap();
        let last_year_end = NaiveDate::from_ymd_opt(year_now - 1, 12, 31).unwrap();
        let two_years_ago_start = NaiveDate::from_ymd_opt(year_now - 2, 1, 1).unwrap();
        let two_years_ago_end = NaiveDate::from_ymd_opt(year_now - 2, 12, 31).unwrap();

        // Report currency = caller's base. Per-activity native amounts are
        // FX-converted to this inside build_summary. Previous behavior picked
        // `activities.first().currency` which mislabeled multi-currency
        // accounts and produced naive cross-currency sums.
        let currency = base_currency.to_string();

        // Filter helper for an activity — event membership is consulted via
        // the preloaded tag map rather than a per-row column.
        let activity_passes = |a: &Activity| -> bool {
            let tag = event_tags.get(&a.id);
            if only_with_events && tag.is_none() {
                return false;
            }
            if let Some(set) = &include_set {
                match tag {
                    Some(eid) if set.contains(eid) => {}
                    _ => return false,
                }
            }
            true
        };

        for period in ["TOTAL", "YTD", "LAST_YEAR", "TWO_YEARS_AGO"] {
            let in_window: Vec<&Activity> = activities
                .iter()
                .filter(|a| {
                    let Some(classification) =
                        classification_for(a, &eligible_accounts.account_types)
                    else {
                        return false;
                    };
                    if classification.spending_amount(activity_abs_amount(a)) == Decimal::ZERO {
                        return false;
                    }
                    // Bucket by user-local date so an activity logged at 11pm
                    // local on Dec 31 lands in the year the user perceives,
                    // not the UTC year.
                    let act_date =
                        wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(
                            a.activity_date,
                            timezone,
                        );
                    let in_period = match period {
                        "TOTAL" => true,
                        "YTD" => act_date >= ytd_start,
                        "LAST_YEAR" => act_date >= last_year_start && act_date <= last_year_end,
                        "TWO_YEARS_AGO" => {
                            act_date >= two_years_ago_start && act_date <= two_years_ago_end
                        }
                        _ => false,
                    };
                    in_period && activity_passes(a)
                })
                .collect();

            // FX as-of for each named period: end of that period for closed
            // years (LAST_YEAR / TWO_YEARS_AGO), today (user-local) for
            // TOTAL/YTD.
            let fx_as_of: NaiveDate = match period {
                "LAST_YEAR" => last_year_end,
                "TWO_YEARS_AGO" => two_years_ago_end,
                _ => today_local,
            };
            out.push(build_summary(
                period,
                &in_window,
                &assignments_by_activity,
                &splits_by_activity,
                &cat_meta,
                &eligible_accounts.account_types,
                &currency,
                self.fx_service.as_ref(),
                fx_as_of,
                timezone,
            ));
        }

        Ok(out)
    }
}

fn event_overlaps_window(
    event_start: &str,
    event_end: &str,
    window_start: Option<&DateTime<Utc>>,
    window_end: Option<&DateTime<Utc>>,
) -> bool {
    // Events have YYYY-MM-DD date strings; compare date keys lexicographically.
    if let Some(ws) = window_start {
        let window_start_key = format!("{}-{:02}-{:02}", ws.year(), ws.month(), ws.day());
        if event_end < window_start_key.as_str() {
            return false;
        }
    }
    if let Some(we) = window_end {
        let window_end_key = format!("{}-{:02}-{:02}", we.year(), we.month(), we.day());
        if event_start > window_end_key.as_str() {
            return false;
        }
    }
    true
}

fn group_activities_by_visible_event(
    activities: Vec<Activity>,
    tag_map: HashMap<String, String>,
    visible_event_ids: &HashSet<String>,
    account_types: &HashMap<String, String>,
) -> HashMap<String, Vec<Activity>> {
    let mut by_event: HashMap<String, Vec<Activity>> = HashMap::new();
    for activity in activities {
        if activity.status != ActivityStatus::Posted {
            continue;
        }
        let Some(event_id) = tag_map.get(&activity.id).cloned() else {
            continue;
        };
        if !visible_event_ids.contains(&event_id) {
            continue;
        }
        let Some(classification) = classification_for(&activity, account_types) else {
            continue;
        };
        if classification.spending_amount(activity_abs_amount(&activity)) == Decimal::ZERO {
            continue;
        }
        by_event.entry(event_id).or_default().push(activity);
    }
    by_event
}

fn empty_summary(period: &str) -> SpendingSummary {
    SpendingSummary {
        period: period.to_string(),
        by_month: HashMap::new(),
        by_category: HashMap::new(),
        by_subcategory: HashMap::new(),
        by_account: HashMap::new(),
        by_month_by_category: HashMap::new(),
        by_month_by_subcategory: HashMap::new(),
        total_spending: 0.0,
        currency: "USD".to_string(),
        monthly_average: 0.0,
        transaction_count: 0,
        yoy_growth: None,
    }
}

fn add_event_category_allocation(
    category_id: Option<&str>,
    amount: Decimal,
    cat_meta: &HashMap<String, (String, Option<String>, Option<String>)>,
    by_category: &mut HashMap<String, CategoryAccumulator>,
) {
    let (cat_id_opt, cat_name, cat_color) = match category_id {
        Some(category_id) => match cat_meta.get(category_id) {
            Some((name, color, _parent)) => {
                (Some(category_id.to_string()), name.clone(), color.clone())
            }
            None => (Some(category_id.to_string()), category_id.to_string(), None),
        },
        None => (None, "Uncategorized".to_string(), None),
    };
    let key = cat_id_opt
        .clone()
        .unwrap_or_else(|| "uncategorized".to_string());
    let entry =
        by_category
            .entry(key)
            .or_insert((cat_id_opt, cat_name, cat_color, Decimal::ZERO, 0));
    entry.3 += amount;
    entry.4 += 1;
}

// 9 args is intentional — every parameter serves a distinct concern (period
// label, activities, assignment lookup, category metadata, account types,
// target currency, FX, snapshot date, timezone). No call site repetition to
// extract into a struct.
#[allow(clippy::too_many_arguments)]
fn build_summary(
    period: &str,
    activities: &[&Activity],
    assignments_by_activity: &AssignmentsByActivity,
    splits_by_activity: &SplitsByActivity,
    cat_meta: &HashMap<String, (String, Option<String>, Option<String>)>,
    account_types: &HashMap<String, String>,
    currency: &str,
    fx: &dyn wealthfolio_core::fx::FxServiceTrait,
    fx_as_of: NaiveDate,
    timezone: &str,
) -> SpendingSummary {
    let mut by_month: HashMap<String, Decimal> = HashMap::new();
    let mut by_account: HashMap<String, Decimal> = HashMap::new();
    let mut by_category: HashMap<String, CategoryAccumulator> = HashMap::new();
    let mut by_subcategory: HashMap<String, SubcategoryAccumulator> = HashMap::new();
    let mut by_month_by_category: HashMap<String, HashMap<String, Decimal>> = HashMap::new();
    let mut by_month_by_subcategory: HashMap<String, HashMap<String, Decimal>> = HashMap::new();
    let mut transaction_count = 0;
    for a in activities {
        let Some(classification) = classification_for(a, account_types) else {
            continue;
        };
        let amt_native = classification.spending_amount(activity_abs_amount(a));
        if amt_native == Decimal::ZERO {
            continue;
        }
        // FX-convert each activity to the report currency at `fx_as_of`
        // (snapshot-date convention, matches insight + monthly_report).
        let Some(amt) = fx_to_target(fx, amt_native, &a.currency, currency, fx_as_of) else {
            continue;
        };
        if amt == Decimal::ZERO {
            continue;
        }
        if amt > Decimal::ZERO {
            transaction_count += 1;
        }
        // Bucket by user-local calendar month so the by_month roll-up matches
        // what the user perceives at boundaries (was `naive_utc()`).
        let dt = wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(
            a.activity_date,
            timezone,
        );
        let month_key = format!("{:04}-{:02}", dt.year(), dt.month());
        *by_month.entry(month_key.clone()).or_insert(Decimal::ZERO) += amt;
        *by_account
            .entry(a.account_id.clone())
            .or_insert(Decimal::ZERO) += amt;

        let allocations = allocations_for_taxonomy(
            &a.id,
            SPENDING_TAXONOMY,
            amt_native,
            assignments_by_activity,
            splits_by_activity,
        );
        if allocations.is_empty() {
            add_summary_category_allocation(
                None,
                amt,
                &month_key,
                cat_meta,
                &mut by_category,
                &mut by_subcategory,
                &mut by_month_by_category,
                &mut by_month_by_subcategory,
            );
            continue;
        }

        for allocation in allocations {
            let Some(allocation_amount) =
                fx_to_target(fx, allocation.amount, &a.currency, currency, fx_as_of)
            else {
                continue;
            };
            if allocation_amount == Decimal::ZERO {
                continue;
            }
            add_summary_category_allocation(
                Some(&allocation.category_id),
                allocation_amount,
                &month_key,
                cat_meta,
                &mut by_category,
                &mut by_subcategory,
                &mut by_month_by_category,
                &mut by_month_by_subcategory,
            );
        }
    }

    let total: Decimal = by_month.values().copied().sum();
    if total <= Decimal::ZERO {
        by_month.clear();
        by_account.clear();
        by_category.clear();
        by_subcategory.clear();
        by_month_by_category.clear();
        by_month_by_subcategory.clear();
    } else {
        by_month.retain(|_, amount| *amount != Decimal::ZERO);
        by_account.retain(|_, amount| *amount != Decimal::ZERO);
        by_month_by_category.retain(|_, inner| {
            inner.retain(|_, amount| *amount != Decimal::ZERO);
            !inner.is_empty()
        });
        by_month_by_subcategory.retain(|_, inner| {
            inner.retain(|_, amount| *amount != Decimal::ZERO);
            !inner.is_empty()
        });
        by_category.retain(|_, value| value.3 != Decimal::ZERO);
        by_subcategory.retain(|_, value| value.5 != Decimal::ZERO);
    }

    let n_months = by_month.len() as f64;
    let total_spending = decimal_to_f64(total.max(Decimal::ZERO));
    let monthly_average = if n_months > 0.0 {
        total_spending / n_months
    } else {
        0.0
    };
    let transaction_count = if total > Decimal::ZERO {
        transaction_count
    } else {
        0
    };
    let by_month = by_month
        .into_iter()
        .map(|(key, amount)| (key, decimal_to_f64(amount)))
        .collect();
    let by_account = by_account
        .into_iter()
        .map(|(key, amount)| (key, decimal_to_f64(amount)))
        .collect();
    let by_month_by_category = by_month_by_category
        .into_iter()
        .map(|(month, values)| {
            (
                month,
                values
                    .into_iter()
                    .map(|(key, amount)| (key, decimal_to_f64(amount)))
                    .collect(),
            )
        })
        .collect();
    let by_month_by_subcategory = by_month_by_subcategory
        .into_iter()
        .map(|(month, values)| {
            (
                month,
                values
                    .into_iter()
                    .map(|(key, amount)| (key, decimal_to_f64(amount)))
                    .collect(),
            )
        })
        .collect();
    let by_category = by_category
        .into_iter()
        .map(
            |(key, (category_id, category_name, color, amount, transaction_count))| {
                (
                    key,
                    CategorySpending {
                        category_id,
                        category_name,
                        color,
                        amount: decimal_to_f64(amount),
                        transaction_count,
                    },
                )
            },
        )
        .collect();
    let by_subcategory = by_subcategory
        .into_iter()
        .map(
            |(
                key,
                (
                    subcategory_id,
                    subcategory_name,
                    category_id,
                    category_name,
                    color,
                    amount,
                    transaction_count,
                ),
            )| {
                (
                    key,
                    SubcategorySpending {
                        subcategory_id,
                        subcategory_name,
                        category_id,
                        category_name,
                        color,
                        amount: decimal_to_f64(amount),
                        transaction_count,
                    },
                )
            },
        )
        .collect();

    SpendingSummary {
        period: period.to_string(),
        by_month,
        by_category,
        by_subcategory,
        by_account,
        by_month_by_category,
        by_month_by_subcategory,
        total_spending,
        currency: currency.to_string(),
        monthly_average,
        transaction_count,
        yoy_growth: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn add_summary_category_allocation(
    assigned_cat_id: Option<&str>,
    amount: Decimal,
    month_key: &str,
    cat_meta: &HashMap<String, (String, Option<String>, Option<String>)>,
    by_category: &mut HashMap<String, CategoryAccumulator>,
    by_subcategory: &mut HashMap<String, SubcategoryAccumulator>,
    by_month_by_category: &mut HashMap<String, HashMap<String, Decimal>>,
    by_month_by_subcategory: &mut HashMap<String, HashMap<String, Decimal>>,
) {
    let (top_cat_id, sub_cat_id, top_name, top_color, sub_name) = match assigned_cat_id {
        Some(cid) => match cat_meta.get(cid) {
            Some((name, color, parent_id)) => match parent_id {
                Some(pid) => {
                    let parent = cat_meta.get(pid);
                    let parent_name = parent
                        .map(|(n, _, _)| n.clone())
                        .unwrap_or_else(|| pid.clone());
                    let parent_color = parent.and_then(|(_, c, _)| c.clone());
                    (
                        Some(pid.clone()),
                        Some(cid.to_string()),
                        parent_name,
                        parent_color,
                        name.clone(),
                    )
                }
                None => (
                    Some(cid.to_string()),
                    None,
                    name.clone(),
                    color.clone(),
                    String::new(),
                ),
            },
            None => (
                Some(cid.to_string()),
                None,
                cid.to_string(),
                None,
                String::new(),
            ),
        },
        None => (None, None, "Uncategorized".to_string(), None, String::new()),
    };

    let top_key = top_cat_id
        .clone()
        .unwrap_or_else(|| "uncategorized".to_string());
    let cat_entry = by_category.entry(top_key.clone()).or_insert((
        top_cat_id.clone(),
        top_name.clone(),
        top_color.clone(),
        Decimal::ZERO,
        0,
    ));
    cat_entry.3 += amount;
    cat_entry.4 += 1;

    if let Some(sub_id) = sub_cat_id.clone() {
        let sub_entry = by_subcategory.entry(sub_id.clone()).or_insert((
            Some(sub_id.clone()),
            sub_name,
            top_cat_id.clone(),
            top_name,
            top_color,
            Decimal::ZERO,
            0,
        ));
        sub_entry.5 += amount;
        sub_entry.6 += 1;

        *by_month_by_subcategory
            .entry(month_key.to_string())
            .or_default()
            .entry(sub_id)
            .or_insert(Decimal::ZERO) += amount;
    }

    *by_month_by_category
        .entry(month_key.to_string())
        .or_default()
        .entry(top_key)
        .or_insert(Decimal::ZERO) += amount;
}

// Helper to silence unused warning when Duration not referenced elsewhere
#[allow(dead_code)]
fn _silence_duration() {
    let _ = Duration::seconds(0);
}

// ====================== EventSpendingSummary (per-event rollups) ======================

impl AnalyticsService {
    /// Compute per-event spending summaries. Each event in the events table is intersected
    /// with the optional date window (events whose date range overlaps with [start, end]).
    /// Tagged activities count according to account-aware spending classification.
    /// `timezone` (IANA name, may be empty) drives the per-day daily bucketing.
    /// FX conversion target is `req.currency` (defaults to "USD") — every
    /// activity is converted at the report's end window (or "now" when no
    /// end was supplied).
    pub async fn event_spending_summaries(
        &self,
        req: EventSummariesRequest,
        timezone: &str,
    ) -> Result<Vec<EventSpendingSummary>> {
        let s = self.settings.get().await?;
        if !s.enabled || s.account_ids.is_empty() {
            return Ok(Vec::new());
        }
        let (target_accounts, account_types) =
            self.resolve_spending_account_types(&s.account_ids)?;
        if target_accounts.is_empty() {
            return Ok(Vec::new());
        }

        let events = self.events_service.list_events_with_names().await?;
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let event_types = self.events_service.list_types().await?;
        let type_color: HashMap<String, Option<String>> = event_types
            .iter()
            .map(|t| (t.id.clone(), t.color.clone()))
            .collect();

        // Optional date window
        let window_start = req
            .start_date
            .as_deref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)))
            .transpose()?;
        let window_end = req
            .end_date
            .as_deref()
            .map(|s| DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)))
            .transpose()?;

        let visible_events: Vec<_> = events
            .into_iter()
            .filter(|ev| {
                event_overlaps_window(
                    &ev.event.start_date,
                    &ev.event.end_date,
                    window_start.as_ref(),
                    window_end.as_ref(),
                )
            })
            .collect();
        if visible_events.is_empty() {
            return Ok(Vec::new());
        }
        let visible_event_ids: Vec<String> = visible_events
            .iter()
            .map(|ev| ev.event.id.clone())
            .collect();
        let visible_event_id_set: HashSet<String> = visible_event_ids.iter().cloned().collect();

        // Load category metadata for spending taxonomy
        let taxonomy_with_cats = self
            .taxonomy_service
            .get_taxonomy(SPENDING_TAXONOMY)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let categories = taxonomy_with_cats
            .map(|tw| tw.categories)
            .unwrap_or_default();
        let cat_meta: HashMap<String, (String, Option<String>, Option<String>)> = categories
            .iter()
            .map(|c| {
                (
                    c.id.clone(),
                    (c.name.clone(), Some(c.color.clone()), c.parent_id.clone()),
                )
            })
            .collect();

        // Event reporting is tag-based: the request window chooses which
        // events are visible, but every in-scope activity tagged to those
        // events contributes to the event total, even when the activity date is
        // before/after the event's own reporting window. Start from the
        // visible event tags so this stays bounded by tagged activity count
        // instead of scanning all spending history for the account set.
        let tag_map = self
            .activity_events
            .list_for_events(&visible_event_ids)
            .await?;
        let mut tagged_activity_ids: Vec<String> = tag_map.keys().cloned().collect();
        tagged_activity_ids.sort();
        tagged_activity_ids.dedup();
        let target_account_ids: HashSet<&str> =
            target_accounts.iter().map(String::as_str).collect();
        let activities = self
            .activity_repo
            .get_activities_by_ids(&tagged_activity_ids)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .into_iter()
            .filter(|activity| target_account_ids.contains(activity.account_id.as_str()))
            .collect::<Vec<_>>();
        let activities = filter_eligible_activity_rows(
            self.aggregate_eligibility_service.as_ref(),
            activities.clone(),
            &activities,
        )
        .await?;
        let mut by_event = group_activities_by_visible_event(
            activities,
            tag_map,
            &visible_event_id_set,
            &account_types,
        );

        let currency = req.currency.unwrap_or_else(|| "USD".to_string());
        // FX as-of: end of the requested window if provided; otherwise today.
        // Matches the snapshot-date convention used by insight + monthly_report.
        let fx_as_of: NaiveDate = window_end
            .map(|d| d.date_naive())
            .unwrap_or_else(|| Utc::now().date_naive());
        let fx = self.fx_service.as_ref();

        // Batch assignment lookup for every in-scope activity at once,
        // grouped by activity_id. Replaces a per-activity `list_for_activity`
        // call inside the inner loop (N+1 against the assignments table).
        let all_activity_ids: Vec<String> =
            by_event.values().flatten().map(|a| a.id.clone()).collect();
        let all_assignments = self
            .assignment_repo
            .list_for_activities(&all_activity_ids)
            .await?;
        let assignments_by_activity = group_assignments(all_assignments);
        let splits_by_activity = group_splits(
            self.split_repo
                .list_for_activities(&all_activity_ids)
                .await?,
        );

        let mut out = Vec::with_capacity(visible_events.len());
        for ev in visible_events {
            let acts = by_event.remove(&ev.event.id).unwrap_or_default();

            let mut total = Decimal::ZERO;
            let mut by_category: HashMap<String, CategoryAccumulator> = HashMap::new();
            let mut daily: HashMap<String, Decimal> = HashMap::new();
            let mut transaction_count = 0;

            for a in &acts {
                let Some(classification) = classification_for(a, &account_types) else {
                    continue;
                };
                let amt_native = classification.spending_amount(activity_abs_amount(a));
                if amt_native == Decimal::ZERO {
                    continue;
                }
                // FX-convert to the report currency at fx_as_of, matching
                // insight + monthly_report so event totals reconcile with
                // the broader period numbers.
                let Some(amt) = fx_to_target(fx, amt_native, &a.currency, &currency, fx_as_of)
                else {
                    continue;
                };
                if amt == Decimal::ZERO {
                    continue;
                }
                if amt > Decimal::ZERO {
                    transaction_count += 1;
                }
                total += amt;
                // Bucket by user-local day so daily counts match what the
                // user perceives at boundaries (was `naive_utc()`).
                let dt = wealthfolio_core::utils::time_utils::activity_date_in_user_timezone(
                    a.activity_date,
                    timezone,
                );
                let day = format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day());
                *daily.entry(day).or_insert(Decimal::ZERO) += amt;

                let allocations = allocations_for_taxonomy(
                    &a.id,
                    SPENDING_TAXONOMY,
                    amt_native,
                    &assignments_by_activity,
                    &splits_by_activity,
                );
                if allocations.is_empty() {
                    add_event_category_allocation(None, amt, &cat_meta, &mut by_category);
                } else {
                    for allocation in allocations {
                        let Some(allocation_amount) =
                            fx_to_target(fx, allocation.amount, &a.currency, &currency, fx_as_of)
                        else {
                            continue;
                        };
                        if allocation_amount == Decimal::ZERO {
                            continue;
                        }
                        add_event_category_allocation(
                            Some(&allocation.category_id),
                            allocation_amount,
                            &cat_meta,
                            &mut by_category,
                        );
                    }
                }
            }
            if total <= Decimal::ZERO {
                total = Decimal::ZERO;
                daily.clear();
                by_category.clear();
            } else {
                daily.retain(|_, amount| *amount != Decimal::ZERO);
                by_category.retain(|_, value| value.3 != Decimal::ZERO);
            }
            let total_spending = decimal_to_f64(total);
            let daily = daily
                .into_iter()
                .map(|(day, amount)| (day, decimal_to_f64(amount)))
                .collect();
            let by_category = by_category
                .into_iter()
                .map(
                    |(key, (category_id, category_name, color, amount, transaction_count))| {
                        (
                            key,
                            EventCategorySpending {
                                category_id,
                                category_name,
                                color,
                                amount: decimal_to_f64(amount),
                                transaction_count,
                            },
                        )
                    },
                )
                .collect();

            out.push(EventSpendingSummary {
                event_id: ev.event.id,
                event_name: ev.event.name,
                event_type_id: ev.event.event_type_id.clone(),
                event_type_name: ev.event_type_name,
                event_type_color: type_color.get(&ev.event.event_type_id).cloned().flatten(),
                start_date: ev.event.start_date,
                end_date: ev.event.end_date,
                total_spending,
                transaction_count: if total > Decimal::ZERO {
                    transaction_count
                } else {
                    0
                },
                currency: currency.clone(),
                by_category,
                daily_spending: daily,
            });
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::type_complexity)]

    use super::*;
    use async_trait::async_trait;
    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use serde_json::Value;
    use std::sync::Mutex;
    use wealthfolio_core::accounts::{
        account_types, Account, AccountRepositoryTrait, AccountUpdate,
        AggregateEligibilityServiceTrait, NewAccount, TrackingMode, ENTITY_ACTIVITY,
    };
    use wealthfolio_core::activities::{
        ActivityBulkMutationResult, ActivityRepositoryTrait, ActivitySearchResponse,
        ActivityStatus, ActivityUpdate, ActivityUpsert, ImportMapping, ImportTemplate, IncomeData,
        NewActivity, Sort, ACTIVITY_TYPE_TRANSFER_IN, ACTIVITY_TYPE_TRANSFER_OUT,
    };
    use wealthfolio_core::fx::{ExchangeRate, FxServiceTrait, NewExchangeRate};
    use wealthfolio_core::limits::ContributionActivity;
    use wealthfolio_core::taxonomies::{
        AssetTaxonomyAssignment, Category, NewAssetTaxonomyAssignment, NewCategory, NewTaxonomy,
        Taxonomy, TaxonomyServiceTrait, TaxonomyWithCategories,
    };

    use crate::activity_assignments::NewActivityTaxonomyAssignment;
    use crate::activity_events::ActivityEvent;
    use crate::activity_splits::{ActivitySplit, NewActivitySplit};
    use crate::events::{Event, EventType, NewEvent, NewEventType, UpdateEvent};
    use crate::settings::{
        SpendingSettingsRepositoryTrait, SETTING_KEY_ACCOUNT_IDS, SETTING_KEY_ENABLED,
    };

    /// Identity FX stub for tests — returns the input amount unchanged. Lets
    /// build_summary / summarize be exercised without a real FxService + DB.
    /// Same pattern as the insight service's PassthroughFx.
    pub(super) struct PassthroughFx;
    struct DoubleEurFx;
    struct FailingFx;

    type CoreResult<T> = std::result::Result<T, wealthfolio_core::Error>;

    #[async_trait]
    impl FxServiceTrait for PassthroughFx {
        fn initialize(&self) -> CoreResult<()> {
            Ok(())
        }
        fn get_historical_rates(&self, _: &str, _: &str, _: i64) -> CoreResult<Vec<ExchangeRate>> {
            Ok(vec![])
        }
        fn get_latest_exchange_rate(&self, _: &str, _: &str) -> CoreResult<Decimal> {
            Ok(Decimal::ONE)
        }
        fn get_exchange_rate_for_date(
            &self,
            _: &str,
            _: &str,
            _: NaiveDate,
        ) -> CoreResult<Decimal> {
            Ok(Decimal::ONE)
        }
        fn convert_currency(&self, amount: Decimal, _: &str, _: &str) -> CoreResult<Decimal> {
            Ok(amount)
        }
        fn convert_currency_for_date(
            &self,
            amount: Decimal,
            _: &str,
            _: &str,
            _: NaiveDate,
        ) -> CoreResult<Decimal> {
            Ok(amount)
        }
        fn get_latest_exchange_rates(&self) -> CoreResult<Vec<ExchangeRate>> {
            Ok(vec![])
        }
        async fn add_exchange_rate(&self, _: NewExchangeRate) -> CoreResult<ExchangeRate> {
            unimplemented!("PassthroughFx is read-only")
        }
        async fn update_exchange_rate(
            &self,
            _: &str,
            _: &str,
            _: Decimal,
        ) -> CoreResult<ExchangeRate> {
            unimplemented!("PassthroughFx is read-only")
        }
        async fn delete_exchange_rate(&self, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn register_currency_pair(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn register_currency_pair_manual(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn ensure_fx_pairs(&self, _: Vec<(String, String)>) -> CoreResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl FxServiceTrait for DoubleEurFx {
        fn initialize(&self) -> CoreResult<()> {
            Ok(())
        }
        fn get_historical_rates(&self, _: &str, _: &str, _: i64) -> CoreResult<Vec<ExchangeRate>> {
            Ok(vec![])
        }
        fn get_latest_exchange_rate(&self, _: &str, _: &str) -> CoreResult<Decimal> {
            Ok(Decimal::from(2))
        }
        fn get_exchange_rate_for_date(
            &self,
            _: &str,
            _: &str,
            _: NaiveDate,
        ) -> CoreResult<Decimal> {
            Ok(Decimal::from(2))
        }
        fn convert_currency(&self, amount: Decimal, from: &str, to: &str) -> CoreResult<Decimal> {
            self.convert_currency_for_date(
                amount,
                from,
                to,
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            )
        }
        fn convert_currency_for_date(
            &self,
            amount: Decimal,
            from: &str,
            to: &str,
            _: NaiveDate,
        ) -> CoreResult<Decimal> {
            Ok(if from == "EUR" && to == "USD" {
                amount * Decimal::from(2)
            } else {
                amount
            })
        }
        fn get_latest_exchange_rates(&self) -> CoreResult<Vec<ExchangeRate>> {
            Ok(vec![])
        }
        async fn add_exchange_rate(&self, _: NewExchangeRate) -> CoreResult<ExchangeRate> {
            unimplemented!("DoubleEurFx is read-only")
        }
        async fn update_exchange_rate(
            &self,
            _: &str,
            _: &str,
            _: Decimal,
        ) -> CoreResult<ExchangeRate> {
            unimplemented!("DoubleEurFx is read-only")
        }
        async fn delete_exchange_rate(&self, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn register_currency_pair(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn register_currency_pair_manual(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn ensure_fx_pairs(&self, _: Vec<(String, String)>) -> CoreResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl FxServiceTrait for FailingFx {
        fn initialize(&self) -> CoreResult<()> {
            Ok(())
        }
        fn get_historical_rates(&self, _: &str, _: &str, _: i64) -> CoreResult<Vec<ExchangeRate>> {
            Ok(vec![])
        }
        fn get_latest_exchange_rate(&self, _: &str, _: &str) -> CoreResult<Decimal> {
            Err(wealthfolio_core::Error::CurrencyConversionFailed(
                "missing rate".to_string(),
            ))
        }
        fn get_exchange_rate_for_date(
            &self,
            _: &str,
            _: &str,
            _: NaiveDate,
        ) -> CoreResult<Decimal> {
            Err(wealthfolio_core::Error::CurrencyConversionFailed(
                "missing rate".to_string(),
            ))
        }
        fn convert_currency(&self, amount: Decimal, from: &str, to: &str) -> CoreResult<Decimal> {
            self.convert_currency_for_date(
                amount,
                from,
                to,
                NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            )
        }
        fn convert_currency_for_date(
            &self,
            _: Decimal,
            _: &str,
            _: &str,
            _: NaiveDate,
        ) -> CoreResult<Decimal> {
            Err(wealthfolio_core::Error::CurrencyConversionFailed(
                "missing rate".to_string(),
            ))
        }
        fn get_latest_exchange_rates(&self) -> CoreResult<Vec<ExchangeRate>> {
            Ok(vec![])
        }
        async fn add_exchange_rate(&self, _: NewExchangeRate) -> CoreResult<ExchangeRate> {
            unimplemented!("FailingFx is read-only")
        }
        async fn update_exchange_rate(
            &self,
            _: &str,
            _: &str,
            _: Decimal,
        ) -> CoreResult<ExchangeRate> {
            unimplemented!("FailingFx is read-only")
        }
        async fn delete_exchange_rate(&self, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn register_currency_pair(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn register_currency_pair_manual(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
        async fn ensure_fx_pairs(&self, _: Vec<(String, String)>) -> CoreResult<()> {
            Ok(())
        }
    }

    fn spending_activity(
        id: &str,
        activity_type: &str,
        amount: i64,
        category_id: &str,
        month: u32,
    ) -> (
        Activity,
        crate::activity_assignments::ActivityTaxonomyAssignment,
    ) {
        let activity = Activity {
            id: id.to_string(),
            account_id: "card-account".to_string(),
            asset_id: None,
            activity_type: activity_type.to_string(),
            activity_type_override: None,
            source_type: None,
            subtype: None,
            status: ActivityStatus::Posted,
            activity_date: Utc.with_ymd_and_hms(2024, month, 10, 12, 0, 0).unwrap(),
            settlement_date: None,
            quantity: None,
            unit_price: None,
            amount: Some(Decimal::new(amount, 0)),
            fee: None,
            tax: None,
            currency: "USD".to_string(),
            fx_rate: None,
            notes: None,
            metadata: None::<Value>,
            source_system: None,
            source_record_id: None,
            source_group_id: None,
            idempotency_key: None,
            import_run_id: None,
            is_user_modified: false,
            needs_review: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        (activity, assignment(id, SPENDING_TAXONOMY, category_id))
    }

    fn assignment(
        activity_id: &str,
        taxonomy_id: &str,
        category_id: &str,
    ) -> crate::activity_assignments::ActivityTaxonomyAssignment {
        crate::activity_assignments::ActivityTaxonomyAssignment {
            id: format!("{activity_id}-{taxonomy_id}"),
            activity_id: activity_id.to_string(),
            taxonomy_id: taxonomy_id.to_string(),
            category_id: category_id.to_string(),
            weight: 10_000,
            source: "manual".to_string(),
            created_at: Utc::now().naive_utc(),
            updated_at: Utc::now().naive_utc(),
        }
    }

    fn account(id: &str, account_type: &str) -> Account {
        Account {
            include_in_net_worth: true,
            id: id.to_string(),
            name: id.to_string(),
            name_source: "manual".to_string(),
            account_type: account_type.to_string(),
            group: None,
            currency: "USD".to_string(),
            is_default: false,
            is_active: true,
            created_at: Utc::now().naive_utc(),
            updated_at: Utc::now().naive_utc(),
            platform_id: None,
            account_number: None,
            meta: None,
            provider: None,
            provider_account_id: None,
            is_archived: false,
            tracking_mode: TrackingMode::Transactions,
        }
    }

    fn analytics_activity(
        id: &str,
        account_id: &str,
        activity_type: &str,
        amount: i64,
        date: chrono::DateTime<Utc>,
    ) -> Activity {
        let (mut activity, _) = spending_activity(id, activity_type, amount, "groceries", 1);
        activity.account_id = account_id.to_string();
        activity.activity_date = date;
        activity
    }

    #[derive(Default)]
    struct MockSettingsRepo {
        account_ids: Vec<String>,
    }

    #[async_trait]
    impl SpendingSettingsRepositoryTrait for MockSettingsRepo {
        async fn get_setting(&self, key: &str) -> Result<Option<String>> {
            match key {
                SETTING_KEY_ENABLED => Ok(Some("true".to_string())),
                SETTING_KEY_ACCOUNT_IDS => Ok(Some(serde_json::to_string(&self.account_ids)?)),
                _ => Ok(None),
            }
        }

        async fn set_setting(&self, _: &str, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn set_settings(&self, _: Vec<(String, String)>) -> Result<()> {
            unimplemented!()
        }

        async fn has_concrete_budget_records(&self) -> Result<bool> {
            Ok(false)
        }
    }

    struct MockAccountRepo {
        accounts: Vec<Account>,
    }

    #[async_trait]
    impl AccountRepositoryTrait for MockAccountRepo {
        async fn create(&self, _: NewAccount) -> wealthfolio_core::Result<Account> {
            unimplemented!()
        }
        async fn update(&self, _: AccountUpdate) -> wealthfolio_core::Result<Account> {
            unimplemented!()
        }
        async fn delete(&self, _: &str) -> wealthfolio_core::Result<usize> {
            unimplemented!()
        }
        fn get_by_id(&self, _: &str) -> wealthfolio_core::Result<Account> {
            unimplemented!()
        }
        fn list(
            &self,
            is_active_filter: Option<bool>,
            is_archived_filter: Option<bool>,
            account_ids: Option<&[String]>,
        ) -> wealthfolio_core::Result<Vec<Account>> {
            Ok(self
                .accounts
                .iter()
                .filter(|account| {
                    account_ids
                        .map(|ids| ids.iter().any(|id| id == &account.id))
                        .unwrap_or(true)
                        && is_active_filter
                            .map(|active| active == account.is_active)
                            .unwrap_or(true)
                        && is_archived_filter
                            .map(|archived| archived == account.is_archived)
                            .unwrap_or(true)
                })
                .cloned()
                .collect())
        }
    }

    #[derive(Default)]
    struct MockEligibility {
        allowed_accounts: Mutex<HashSet<String>>,
        allowed_activity_ids: Mutex<HashSet<String>>,
        fail_accounts: Mutex<bool>,
        fail_rows: Mutex<bool>,
        account_calls: Mutex<Vec<(Vec<String>, NaiveDate)>>,
        row_calls: Mutex<Vec<(String, Vec<(String, NaiveDate)>)>>,
    }

    #[async_trait]
    impl AggregateEligibilityServiceTrait for MockEligibility {
        async fn eligible_account_ids(
            &self,
            requested_account_ids: &[String],
            as_of: NaiveDate,
        ) -> wealthfolio_core::Result<HashSet<String>> {
            self.account_calls
                .lock()
                .unwrap()
                .push((requested_account_ids.to_vec(), as_of));
            if *self.fail_accounts.lock().unwrap() {
                return Err(wealthfolio_core::Error::Validation(
                    wealthfolio_core::errors::ValidationError::InvalidInput(
                        "account policy failed".to_string(),
                    ),
                ));
            }
            let allowed = self.allowed_accounts.lock().unwrap();
            Ok(requested_account_ids
                .iter()
                .filter(|id| allowed.contains(*id))
                .cloned()
                .collect())
        }

        async fn eligible_entity_ids_for_rows(
            &self,
            entity_type: &str,
            rows: &[(String, NaiveDate)],
        ) -> wealthfolio_core::Result<HashSet<String>> {
            self.row_calls
                .lock()
                .unwrap()
                .push((entity_type.to_string(), rows.to_vec()));
            if *self.fail_rows.lock().unwrap() {
                return Err(wealthfolio_core::Error::Validation(
                    wealthfolio_core::errors::ValidationError::InvalidInput(
                        "row policy failed".to_string(),
                    ),
                ));
            }
            let allowed = self.allowed_activity_ids.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|(id, _)| allowed.contains(id))
                .map(|(id, _)| id.clone())
                .collect())
        }
    }

    struct MockActivityRepo {
        activities: Vec<Activity>,
    }

    #[async_trait]
    impl ActivityRepositoryTrait for MockActivityRepo {
        fn get_activity(&self, activity_id: &str) -> wealthfolio_core::Result<Activity> {
            self.activities
                .iter()
                .find(|activity| activity.id == activity_id)
                .cloned()
                .ok_or_else(|| {
                    wealthfolio_core::Error::Validation(
                        wealthfolio_core::errors::ValidationError::InvalidInput(
                            "not found".to_string(),
                        ),
                    )
                })
        }
        fn find_transfer_counterpart(
            &self,
            _: &str,
            _: &str,
        ) -> wealthfolio_core::Result<Option<Activity>> {
            Ok(None)
        }
        fn get_activities(&self) -> wealthfolio_core::Result<Vec<Activity>> {
            Ok(self.activities.clone())
        }
        fn get_activities_by_account_id(
            &self,
            account_id: &str,
        ) -> wealthfolio_core::Result<Vec<Activity>> {
            Ok(self
                .activities
                .iter()
                .filter(|activity| activity.account_id == account_id)
                .cloned()
                .collect())
        }
        fn get_activities_by_account_ids(
            &self,
            account_ids: &[String],
        ) -> wealthfolio_core::Result<Vec<Activity>> {
            Ok(self
                .activities
                .iter()
                .filter(|activity| account_ids.iter().any(|id| id == &activity.account_id))
                .cloned()
                .collect())
        }
        fn get_trading_activities(&self) -> wealthfolio_core::Result<Vec<Activity>> {
            unimplemented!()
        }
        fn get_income_activities(&self) -> wealthfolio_core::Result<Vec<Activity>> {
            unimplemented!()
        }
        fn get_contribution_activities(
            &self,
            _: &[String],
            _: chrono::DateTime<Utc>,
            _: chrono::DateTime<Utc>,
        ) -> wealthfolio_core::Result<Vec<ContributionActivity>> {
            unimplemented!()
        }
        fn search_activities(
            &self,
            _: i64,
            _: i64,
            _: Option<Vec<String>>,
            _: Option<Vec<String>>,
            _: Option<String>,
            _: Option<Sort>,
            _: Option<bool>,
            _: Option<NaiveDate>,
            _: Option<NaiveDate>,
            _: Option<Vec<String>>,
            _: Option<Vec<String>>,
        ) -> wealthfolio_core::Result<ActivitySearchResponse> {
            unimplemented!()
        }
        async fn create_activity(&self, _: NewActivity) -> wealthfolio_core::Result<Activity> {
            unimplemented!()
        }
        async fn update_activity(&self, _: ActivityUpdate) -> wealthfolio_core::Result<Activity> {
            unimplemented!()
        }
        async fn delete_activity(&self, _: String) -> wealthfolio_core::Result<Activity> {
            unimplemented!()
        }
        async fn link_transfer_activities(
            &self,
            _: String,
            _: String,
        ) -> wealthfolio_core::Result<(Activity, Activity)> {
            unimplemented!()
        }
        async fn unlink_transfer_activities(
            &self,
            _: String,
            _: String,
        ) -> wealthfolio_core::Result<(Activity, Activity)> {
            unimplemented!()
        }
        async fn bulk_mutate_activities(
            &self,
            _: Vec<NewActivity>,
            _: Vec<ActivityUpdate>,
            _: Vec<String>,
        ) -> wealthfolio_core::Result<ActivityBulkMutationResult> {
            unimplemented!()
        }
        async fn create_activities(&self, _: Vec<NewActivity>) -> wealthfolio_core::Result<usize> {
            unimplemented!()
        }
        fn get_first_activity_date(
            &self,
            _: Option<&[String]>,
        ) -> wealthfolio_core::Result<Option<chrono::DateTime<Utc>>> {
            unimplemented!()
        }
        fn get_import_mapping(
            &self,
            _: &str,
            _: &str,
        ) -> wealthfolio_core::Result<Option<ImportMapping>> {
            unimplemented!()
        }
        async fn save_import_mapping(&self, _: &ImportMapping) -> wealthfolio_core::Result<()> {
            unimplemented!()
        }
        async fn link_account_template(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> wealthfolio_core::Result<()> {
            unimplemented!()
        }
        fn list_import_templates(&self) -> wealthfolio_core::Result<Vec<ImportTemplate>> {
            unimplemented!()
        }
        fn get_import_template(&self, _: &str) -> wealthfolio_core::Result<Option<ImportTemplate>> {
            unimplemented!()
        }
        async fn save_import_template(&self, _: &ImportTemplate) -> wealthfolio_core::Result<()> {
            unimplemented!()
        }
        async fn delete_import_template(&self, _: &str) -> wealthfolio_core::Result<()> {
            unimplemented!()
        }
        fn get_broker_sync_profile(
            &self,
            _: &str,
            _: &str,
        ) -> wealthfolio_core::Result<Option<ImportTemplate>> {
            unimplemented!()
        }
        async fn save_broker_sync_profile(
            &self,
            _: &ImportTemplate,
        ) -> wealthfolio_core::Result<()> {
            unimplemented!()
        }
        async fn link_broker_sync_profile(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> wealthfolio_core::Result<()> {
            unimplemented!()
        }
        fn calculate_average_cost(&self, _: &str, _: &str) -> wealthfolio_core::Result<Decimal> {
            unimplemented!()
        }
        fn get_income_activities_data(
            &self,
            _: Option<&[String]>,
        ) -> wealthfolio_core::Result<Vec<IncomeData>> {
            unimplemented!()
        }
        fn get_first_activity_date_overall(
            &self,
        ) -> wealthfolio_core::Result<chrono::DateTime<Utc>> {
            unimplemented!()
        }
        fn get_activity_bounds_for_assets(
            &self,
            _: &[String],
        ) -> wealthfolio_core::Result<HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>>
        {
            unimplemented!()
        }
        fn get_holdings_snapshot_bounds_for_assets(
            &self,
            _: &[String],
        ) -> wealthfolio_core::Result<HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>>
        {
            unimplemented!()
        }
        fn check_existing_duplicates(
            &self,
            _: &[String],
        ) -> wealthfolio_core::Result<HashMap<String, String>> {
            unimplemented!()
        }
        async fn bulk_upsert(
            &self,
            _: Vec<ActivityUpsert>,
        ) -> wealthfolio_core::Result<wealthfolio_core::activities::BulkUpsertResult> {
            unimplemented!()
        }
        async fn reassign_asset(&self, _: &str, _: &str) -> wealthfolio_core::Result<u32> {
            unimplemented!()
        }
        async fn get_activity_accounts_and_currencies_by_asset_id(
            &self,
            _: &str,
        ) -> wealthfolio_core::Result<(Vec<String>, Vec<String>)> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockAssignmentRepo {
        assignments: Mutex<Vec<crate::activity_assignments::ActivityTaxonomyAssignment>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ActivityTaxonomyAssignmentRepositoryTrait for MockAssignmentRepo {
        async fn list_for_activity(
            &self,
            _: &str,
        ) -> Result<Vec<crate::activity_assignments::ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn list_for_activities(
            &self,
            activity_ids: &[String],
        ) -> Result<Vec<crate::activity_assignments::ActivityTaxonomyAssignment>> {
            self.calls.lock().unwrap().push(activity_ids.to_vec());
            Ok(self
                .assignments
                .lock()
                .unwrap()
                .iter()
                .filter(|assignment| activity_ids.iter().any(|id| id == &assignment.activity_id))
                .cloned()
                .collect())
        }

        async fn categories_belong_to_taxonomy(&self, _: &str, _: &[String]) -> Result<bool> {
            Ok(true)
        }

        async fn upsert(
            &self,
            _: NewActivityTaxonomyAssignment,
        ) -> Result<crate::activity_assignments::ActivityTaxonomyAssignment> {
            unimplemented!()
        }

        async fn assign_many_single_select(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
        ) -> Result<Vec<crate::activity_assignments::ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn assign_many_single_select_clearing_splits(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
        ) -> Result<Vec<crate::activity_assignments::ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn assign_rule_many_single_select(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
            _: bool,
        ) -> Result<Vec<crate::activity_assignments::ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn delete(&self, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn clear_for_taxonomy(&self, _: &str, _: &str) -> Result<()> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockSplitRepo {
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ActivitySplitRepositoryTrait for MockSplitRepo {
        async fn list_for_activity(&self, _: &str) -> Result<Vec<ActivitySplit>> {
            unimplemented!()
        }
        async fn list_for_activities(&self, activity_ids: &[String]) -> Result<Vec<ActivitySplit>> {
            self.calls.lock().unwrap().push(activity_ids.to_vec());
            Ok(Vec::new())
        }
        async fn categories_belong_to_taxonomy(&self, _: &str, _: &[String]) -> Result<bool> {
            unimplemented!()
        }
        async fn replace_for_activity(
            &self,
            _: &str,
            _: Vec<NewActivitySplit>,
        ) -> Result<Vec<ActivitySplit>> {
            unimplemented!()
        }
        async fn replace_for_activity_clearing_assignment(
            &self,
            _: &str,
            _: &str,
            _: Vec<NewActivitySplit>,
        ) -> Result<Vec<ActivitySplit>> {
            unimplemented!()
        }
        async fn clear_for_activity(&self, _: &str) -> Result<()> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockActivityEventsRepo {
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl crate::activity_events::ActivityEventsRepositoryTrait for MockActivityEventsRepo {
        async fn list_for_activities(&self, ids: &[String]) -> Result<HashMap<String, String>> {
            self.calls.lock().unwrap().push(ids.to_vec());
            Ok(HashMap::new())
        }
        async fn list_for_event(&self, _: &str) -> Result<Vec<String>> {
            Ok(Vec::new())
        }
        async fn set_activity_event_tag(&self, _: &str, _: Option<String>) -> Result<()> {
            unimplemented!()
        }
        async fn delete_by_event(&self, _: &str) -> Result<usize> {
            unimplemented!()
        }
        async fn list_all(&self) -> Result<Vec<ActivityEvent>> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockTaxonomyService;

    #[async_trait]
    impl TaxonomyServiceTrait for MockTaxonomyService {
        fn get_taxonomies(&self) -> wealthfolio_core::Result<Vec<Taxonomy>> {
            Ok(Vec::new())
        }
        fn get_taxonomy(
            &self,
            _: &str,
        ) -> wealthfolio_core::Result<Option<TaxonomyWithCategories>> {
            Ok(None)
        }
        fn get_taxonomies_with_categories(
            &self,
        ) -> wealthfolio_core::Result<Vec<TaxonomyWithCategories>> {
            Ok(Vec::new())
        }
        async fn create_taxonomy(&self, _: NewTaxonomy) -> wealthfolio_core::Result<Taxonomy> {
            unimplemented!()
        }
        async fn update_taxonomy(&self, _: Taxonomy) -> wealthfolio_core::Result<Taxonomy> {
            unimplemented!()
        }
        async fn delete_taxonomy(&self, _: &str) -> wealthfolio_core::Result<usize> {
            unimplemented!()
        }
        async fn create_category(&self, _: NewCategory) -> wealthfolio_core::Result<Category> {
            unimplemented!()
        }
        async fn update_category(&self, _: Category) -> wealthfolio_core::Result<Category> {
            unimplemented!()
        }
        async fn delete_category(&self, _: &str, _: &str) -> wealthfolio_core::Result<usize> {
            unimplemented!()
        }
        async fn move_category(
            &self,
            _: &str,
            _: &str,
            _: Option<String>,
            _: i32,
        ) -> wealthfolio_core::Result<Category> {
            unimplemented!()
        }
        async fn import_taxonomy_json(&self, _: &str) -> wealthfolio_core::Result<Taxonomy> {
            unimplemented!()
        }
        fn export_taxonomy_json(&self, _: &str) -> wealthfolio_core::Result<String> {
            unimplemented!()
        }
        fn get_asset_assignments(
            &self,
            _: &str,
        ) -> wealthfolio_core::Result<Vec<AssetTaxonomyAssignment>> {
            unimplemented!()
        }
        fn get_category_assignments(
            &self,
            _: &str,
            _: &str,
        ) -> wealthfolio_core::Result<Vec<AssetTaxonomyAssignment>> {
            unimplemented!()
        }
        async fn assign_asset_to_category(
            &self,
            _: NewAssetTaxonomyAssignment,
        ) -> wealthfolio_core::Result<AssetTaxonomyAssignment> {
            unimplemented!()
        }
        async fn replace_asset_taxonomy_assignments(
            &self,
            _: &str,
            _: &str,
            _: Vec<NewAssetTaxonomyAssignment>,
        ) -> wealthfolio_core::Result<Vec<AssetTaxonomyAssignment>> {
            unimplemented!()
        }
        async fn remove_asset_assignment(&self, _: &str) -> wealthfolio_core::Result<usize> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockEventTypesRepo;

    #[async_trait]
    impl crate::events::EventTypesRepositoryTrait for MockEventTypesRepo {
        async fn list(&self) -> Result<Vec<EventType>> {
            Ok(Vec::new())
        }
        async fn create(&self, _: NewEventType) -> Result<EventType> {
            unimplemented!()
        }
        async fn update(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<Option<String>>,
        ) -> Result<EventType> {
            unimplemented!()
        }
        async fn delete(&self, _: &str) -> Result<()> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockEventsRepo;

    #[async_trait]
    impl crate::events::EventsRepositoryTrait for MockEventsRepo {
        async fn list(&self) -> Result<Vec<Event>> {
            Ok(Vec::new())
        }
        async fn get(&self, _: &str) -> Result<Option<Event>> {
            Ok(None)
        }
        async fn create(&self, _: NewEvent) -> Result<Event> {
            unimplemented!()
        }
        async fn update(&self, _: &str, _: UpdateEvent) -> Result<Event> {
            unimplemented!()
        }
        async fn delete(&self, _: &str) -> Result<()> {
            unimplemented!()
        }
        async fn count_by_type(&self, _: &str) -> Result<usize> {
            Ok(0)
        }
    }

    struct TestAnalytics {
        service: AnalyticsService,
        eligibility: Arc<MockEligibility>,
        assignments: Arc<MockAssignmentRepo>,
        splits: Arc<MockSplitRepo>,
        activity_events: Arc<MockActivityEventsRepo>,
    }

    fn make_analytics_service(
        activities: Vec<Activity>,
        accounts: Vec<Account>,
        settings_account_ids: Vec<String>,
        allowed_accounts: Vec<String>,
        allowed_activity_ids: Vec<String>,
    ) -> TestAnalytics {
        let activity_repo = Arc::new(MockActivityRepo { activities });
        let account_repo = Arc::new(MockAccountRepo { accounts });
        let assignments = Arc::new(MockAssignmentRepo::default());
        let splits = Arc::new(MockSplitRepo::default());
        let activity_events = Arc::new(MockActivityEventsRepo::default());
        let settings = Arc::new(SpendingSettingsService::new(Arc::new(MockSettingsRepo {
            account_ids: settings_account_ids,
        })));
        let events_service = Arc::new(EventsService::new(
            Arc::new(MockEventTypesRepo),
            Arc::new(MockEventsRepo),
            activity_repo.clone(),
            activity_events.clone(),
        ));
        let eligibility = Arc::new(MockEligibility::default());
        *eligibility.allowed_accounts.lock().unwrap() = allowed_accounts.into_iter().collect();
        *eligibility.allowed_activity_ids.lock().unwrap() =
            allowed_activity_ids.into_iter().collect();

        TestAnalytics {
            service: AnalyticsService::new(
                activity_repo,
                account_repo,
                assignments.clone(),
                splits.clone(),
                settings,
                Arc::new(MockTaxonomyService),
                events_service,
                Arc::new(PassthroughFx),
                activity_events.clone(),
                eligibility.clone(),
            ),
            eligibility,
            assignments,
            splits,
            activity_events,
        }
    }

    #[tokio::test]
    async fn analytics_monthly_report_filters_denied_rows_before_joins_and_totals() {
        let allowed = analytics_activity(
            "allowed",
            "cash",
            "WITHDRAWAL",
            100,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        let denied = analytics_activity(
            "denied",
            "cash",
            "WITHDRAWAL",
            200,
            Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap(),
        );
        let analytics = make_analytics_service(
            vec![allowed, denied],
            vec![account("cash", account_types::CASH)],
            vec!["cash".to_string()],
            vec!["cash".to_string()],
            vec!["allowed".to_string()],
        );

        let report = analytics
            .service
            .monthly_report(
                ReportRequest {
                    start_date: "2026-07-01T00:00:00Z".to_string(),
                    end_date: "2026-07-31T23:59:59Z".to_string(),
                    account_ids: None,
                },
                "UTC",
                "USD",
            )
            .await
            .unwrap();

        assert_eq!(report.current.outflow, 100.0);
        assert_eq!(
            analytics.assignments.calls.lock().unwrap().as_slice(),
            &[vec!["allowed".to_string()]]
        );
        assert_eq!(
            analytics.splits.calls.lock().unwrap().as_slice(),
            &[vec!["allowed".to_string()]]
        );
        assert_eq!(analytics.activity_events.calls.lock().unwrap().len(), 0);
        assert_eq!(
            analytics.eligibility.row_calls.lock().unwrap().as_slice(),
            &[(
                ENTITY_ACTIVITY.to_string(),
                vec![
                    (
                        "allowed".to_string(),
                        NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
                    ),
                    (
                        "denied".to_string(),
                        NaiveDate::from_ymd_opt(2026, 7, 11).unwrap()
                    ),
                ],
            )]
        );
    }

    #[tokio::test]
    async fn analytics_monthly_report_excludes_provider_pocket_conversion_from_totals() {
        let mut conversion = analytics_activity(
            "usd-pocket-conversion",
            "cash",
            "TRANSFER_OUT",
            2_000_000,
            Utc.with_ymd_and_hms(2026, 8, 14, 12, 0, 0).unwrap(),
        );
        conversion.amount = Some(Decimal::new(2_000_000, 2));
        conversion.currency = "ZAR".to_string();
        conversion.source_type = Some("MCPBAPOCKETTRANSFERS".to_string());
        conversion.subtype = Some("POCKET_TRANSFER".to_string());

        let analytics = make_analytics_service(
            vec![conversion],
            vec![account("cash", account_types::CASH)],
            vec!["cash".to_string()],
            vec!["cash".to_string()],
            vec!["usd-pocket-conversion".to_string()],
        );

        let report = analytics
            .service
            .monthly_report(
                ReportRequest {
                    start_date: "2026-08-01T00:00:00Z".to_string(),
                    end_date: "2026-08-31T23:59:59Z".to_string(),
                    account_ids: None,
                },
                "UTC",
                "ZAR",
            )
            .await
            .unwrap();

        assert_eq!(report.current.outflow, 0.0);
        assert_eq!(report.current.income, 0.0);
        assert_eq!(report.current.saved, 0.0);
        assert_eq!(report.current.net, 0.0);
        assert_eq!(report.current.count, 0);
        assert!(report.spending_breakdown.is_empty());
        assert!(report.by_day.is_empty());
    }

    #[tokio::test]
    async fn analytics_monthly_report_intersects_requested_accounts_with_account_gate() {
        let cash = analytics_activity(
            "cash-row",
            "cash",
            "WITHDRAWAL",
            100,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        let denied_account = analytics_activity(
            "denied-account-row",
            "denied-cash",
            "WITHDRAWAL",
            200,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        let analytics = make_analytics_service(
            vec![cash, denied_account],
            vec![
                account("cash", account_types::CASH),
                account("denied-cash", account_types::CASH),
            ],
            vec!["cash".to_string(), "denied-cash".to_string()],
            vec!["cash".to_string()],
            vec!["cash-row".to_string(), "denied-account-row".to_string()],
        );

        let report = analytics
            .service
            .monthly_report(
                ReportRequest {
                    start_date: "2026-07-01T00:00:00Z".to_string(),
                    end_date: "2026-07-31T23:59:59Z".to_string(),
                    account_ids: Some(vec!["cash".to_string(), "denied-cash".to_string()]),
                },
                "UTC",
                "USD",
            )
            .await
            .unwrap();

        assert_eq!(report.current.outflow, 100.0);
        assert_eq!(
            analytics
                .eligibility
                .account_calls
                .lock()
                .unwrap()
                .as_slice(),
            &[(
                vec!["cash".to_string(), "denied-cash".to_string()],
                NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            )]
        );
        assert_eq!(
            analytics.eligibility.row_calls.lock().unwrap()[0].1,
            vec![(
                "cash-row".to_string(),
                NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
            )]
        );
        assert_eq!(
            analytics.assignments.calls.lock().unwrap().as_slice(),
            &[vec!["cash-row".to_string()]]
        );
    }

    #[tokio::test]
    async fn analytics_spending_summary_filters_mixed_role_rows_before_joins() {
        let allowed = analytics_activity(
            "allowed-current",
            "cash",
            "WITHDRAWAL",
            100,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        let historical = analytics_activity(
            "historical-role",
            "cash",
            "WITHDRAWAL",
            300,
            Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap(),
        );
        let analytics = make_analytics_service(
            vec![allowed, historical],
            vec![account("cash", account_types::CASH)],
            vec!["cash".to_string()],
            vec!["cash".to_string()],
            vec!["allowed-current".to_string()],
        );

        let summaries = analytics
            .service
            .spending_summary(None, None, "USD", "UTC")
            .await
            .unwrap();
        let total = summaries
            .iter()
            .find(|summary| summary.period == "TOTAL")
            .unwrap();

        assert_eq!(total.total_spending, 100.0);
        assert_eq!(
            analytics.assignments.calls.lock().unwrap()[0],
            vec!["allowed-current".to_string()]
        );
        assert_eq!(analytics.activity_events.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn analytics_monthly_report_transfer_pair_is_all_or_none() {
        let mut transfer_in = analytics_activity(
            "transfer-in",
            "cash-a",
            ACTIVITY_TYPE_TRANSFER_IN,
            100,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        transfer_in.source_group_id = Some("group-1".to_string());
        let mut transfer_out = analytics_activity(
            "transfer-out",
            "cash-b",
            ACTIVITY_TYPE_TRANSFER_OUT,
            100,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        transfer_out.source_group_id = Some("group-1".to_string());
        let analytics = make_analytics_service(
            vec![transfer_in, transfer_out],
            vec![
                account("cash-a", account_types::CASH),
                account("cash-b", account_types::CASH),
            ],
            vec!["cash-a".to_string(), "cash-b".to_string()],
            vec!["cash-a".to_string(), "cash-b".to_string()],
            vec!["transfer-in".to_string()],
        );

        let report = analytics
            .service
            .monthly_report(
                ReportRequest {
                    start_date: "2026-07-01T00:00:00Z".to_string(),
                    end_date: "2026-07-31T23:59:59Z".to_string(),
                    account_ids: None,
                },
                "UTC",
                "USD",
            )
            .await
            .unwrap();

        assert_eq!(report.current.count, 0);
        assert_eq!(
            analytics.assignments.calls.lock().unwrap().as_slice(),
            &[Vec::<String>::new()]
        );
    }

    #[tokio::test]
    async fn analytics_spending_summary_propagates_policy_errors() {
        let row = analytics_activity(
            "row",
            "cash",
            "WITHDRAWAL",
            100,
            Utc.with_ymd_and_hms(2026, 7, 10, 12, 0, 0).unwrap(),
        );
        let analytics = make_analytics_service(
            vec![row],
            vec![account("cash", account_types::CASH)],
            vec!["cash".to_string()],
            vec!["cash".to_string()],
            vec!["row".to_string()],
        );
        *analytics.eligibility.fail_rows.lock().unwrap() = true;

        let err = analytics
            .service
            .spending_summary(None, None, "USD", "UTC")
            .await
            .unwrap_err();

        assert!(err.to_string().contains("row policy failed"));
        assert!(analytics.assignments.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn report_breakdown_keeps_income_out_of_daily_category_buckets() {
        let assignments = group_assignments(vec![assignment("income", INCOME_TAXONOMY, "salary")]);
        let splits = SplitsByActivity::new();
        let mut income_acc: HashMap<(String, String), (Decimal, usize)> = HashMap::new();
        let mut by_day_cat_acc: HashMap<(String, String, String), (Decimal, usize)> =
            HashMap::new();

        add_report_breakdown_allocations(
            &mut income_acc,
            &mut by_day_cat_acc,
            "income",
            INCOME_TAXONOMY,
            Decimal::new(100, 0),
            &assignments,
            &splits,
            &PassthroughFx,
            "USD",
            "USD",
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
            "2024-01-10",
            false,
        );

        assert_eq!(
            income_acc.get(&(INCOME_TAXONOMY.to_string(), "salary".to_string())),
            Some(&(Decimal::new(100, 0), 1)),
        );
        assert!(by_day_cat_acc.is_empty());
    }

    fn build_credit_card_summary(activities: &[Activity]) -> SpendingSummary {
        build_credit_card_summary_in_timezone(activities, "")
    }

    fn build_credit_card_summary_in_timezone(
        activities: &[Activity],
        timezone: &str,
    ) -> SpendingSummary {
        let activity_refs: Vec<&Activity> = activities.iter().collect();
        let account_types = HashMap::from([(
            "card-account".to_string(),
            account_types::CREDIT_CARD.to_string(),
        )]);
        let cat_meta = HashMap::from([
            (
                "groceries".to_string(),
                ("Groceries".to_string(), Some("#1".to_string()), None),
            ),
            (
                "travel".to_string(),
                ("Travel".to_string(), Some("#2".to_string()), None),
            ),
        ]);
        let assign_by_act = group_assignments(
            activities
                .iter()
                .map(|activity| {
                    let category = if activity.id == "charge" {
                        "groceries"
                    } else {
                        "travel"
                    };
                    assignment(&activity.id, SPENDING_TAXONOMY, category)
                })
                .collect(),
        );
        let splits_by_act = SplitsByActivity::new();

        build_summary(
            "TOTAL",
            &activity_refs,
            &assign_by_act,
            &splits_by_act,
            &cat_meta,
            &account_types,
            "USD",
            &PassthroughFx,
            NaiveDate::from_ymd_opt(2024, 12, 31).unwrap(),
            timezone,
        )
    }

    #[test]
    fn build_summary_preserves_refund_buckets_when_period_stays_positive() {
        let (charge, _) = spending_activity("charge", "WITHDRAWAL", 200, "groceries", 1);
        let (refund, _) = spending_activity("refund", "CREDIT", 50, "travel", 2);

        let summary = build_credit_card_summary(&[charge, refund]);

        assert_eq!(summary.total_spending, 150.0);
        assert_eq!(summary.by_month.get("2024-01"), Some(&200.0));
        assert_eq!(summary.by_month.get("2024-02"), Some(&-50.0));
        assert_eq!(summary.by_category["groceries"].amount, 200.0);
        assert_eq!(summary.by_category["travel"].amount, -50.0);
        assert_eq!(summary.transaction_count, 1);
    }

    #[test]
    fn build_summary_clears_buckets_when_refunds_exceed_charges() {
        let (charge, _) = spending_activity("charge", "WITHDRAWAL", 100, "groceries", 1);
        let (refund, _) = spending_activity("refund", "CREDIT", 150, "travel", 2);

        let summary = build_credit_card_summary(&[charge, refund]);

        assert_eq!(summary.total_spending, 0.0);
        assert!(summary.by_month.is_empty());
        assert!(summary.by_category.is_empty());
        assert!(summary.by_account.is_empty());
        assert_eq!(summary.transaction_count, 0);
    }

    #[test]
    fn event_window_selects_events_by_event_dates() {
        let start = Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 2, 29, 23, 59, 59).unwrap();
        assert!(event_overlaps_window(
            "2024-01-30",
            "2024-02-02",
            Some(&start),
            Some(&end)
        ));
        assert!(event_overlaps_window(
            "2024-02-10",
            "2024-02-12",
            Some(&start),
            Some(&end)
        ));
        assert!(!event_overlaps_window(
            "2024-01-01",
            "2024-01-31",
            Some(&start),
            Some(&end)
        ));
        assert!(!event_overlaps_window(
            "2024-03-01",
            "2024-03-03",
            Some(&start),
            Some(&end)
        ));
    }

    #[test]
    fn event_summary_groups_all_tagged_activities_for_visible_event() {
        let (mut before, _) = spending_activity("flight", "WITHDRAWAL", 300, "travel", 5);
        before.activity_date = Utc.with_ymd_and_hms(2024, 5, 20, 12, 0, 0).unwrap();
        let (mut during, _) = spending_activity("meal", "WITHDRAWAL", 80, "travel", 6);
        during.activity_date = Utc.with_ymd_and_hms(2024, 6, 12, 12, 0, 0).unwrap();
        let (mut after, _) = spending_activity("deposit", "WITHDRAWAL", 120, "travel", 7);
        after.activity_date = Utc.with_ymd_and_hms(2024, 7, 2, 12, 0, 0).unwrap();

        let grouped = group_activities_by_visible_event(
            vec![before, during, after],
            HashMap::from([
                ("flight".to_string(), "holiday".to_string()),
                ("meal".to_string(), "holiday".to_string()),
                ("deposit".to_string(), "holiday".to_string()),
            ]),
            &HashSet::from(["holiday".to_string()]),
            &HashMap::from([(
                "card-account".to_string(),
                account_types::CREDIT_CARD.to_string(),
            )]),
        );

        let mut ids = grouped
            .get("holiday")
            .unwrap()
            .iter()
            .map(|activity| activity.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, vec!["deposit", "flight", "meal"]);
    }

    #[test]
    fn event_summary_excludes_pending_and_void_activities() {
        let (posted, _) = spending_activity("posted", "WITHDRAWAL", 100, "travel", 6);
        let (mut pending, _) = spending_activity("pending", "WITHDRAWAL", 100, "travel", 6);
        pending.status = ActivityStatus::Pending;
        let (mut void, _) = spending_activity("void", "WITHDRAWAL", 100, "travel", 6);
        void.status = ActivityStatus::Void;

        let grouped = group_activities_by_visible_event(
            vec![posted, pending, void],
            HashMap::from([
                ("posted".to_string(), "holiday".to_string()),
                ("pending".to_string(), "holiday".to_string()),
                ("void".to_string(), "holiday".to_string()),
            ]),
            &HashSet::from(["holiday".to_string()]),
            &HashMap::from([(
                "card-account".to_string(),
                account_types::CREDIT_CARD.to_string(),
            )]),
        );

        assert_eq!(
            grouped
                .get("holiday")
                .unwrap()
                .iter()
                .map(|activity| activity.id.as_str())
                .collect::<Vec<_>>(),
            vec!["posted"]
        );
    }

    #[test]
    fn event_summary_ignores_tags_for_events_outside_requested_window() {
        let (visible, _) = spending_activity("visible", "WITHDRAWAL", 100, "travel", 6);
        let (hidden, _) = spending_activity("hidden", "WITHDRAWAL", 100, "travel", 6);

        let grouped = group_activities_by_visible_event(
            vec![visible, hidden],
            HashMap::from([
                ("visible".to_string(), "visible-event".to_string()),
                ("hidden".to_string(), "hidden-event".to_string()),
            ]),
            &HashSet::from(["visible-event".to_string()]),
            &HashMap::from([(
                "card-account".to_string(),
                account_types::CREDIT_CARD.to_string(),
            )]),
        );

        let ids = grouped
            .get("visible-event")
            .unwrap()
            .iter()
            .map(|activity| activity.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["visible"]);
        assert!(!grouped.contains_key("hidden-event"));
    }

    #[test]
    fn build_summary_converts_activity_amounts_to_report_currency() {
        let (mut charge, _) = spending_activity("charge", "WITHDRAWAL", 100, "groceries", 1);
        charge.currency = "EUR".to_string();
        let activity_refs = vec![&charge];
        let account_types = HashMap::from([(
            "card-account".to_string(),
            account_types::CREDIT_CARD.to_string(),
        )]);
        let cat_meta = HashMap::from([(
            "groceries".to_string(),
            ("Groceries".to_string(), Some("#1".to_string()), None),
        )]);
        let assignments =
            group_assignments(vec![assignment("charge", SPENDING_TAXONOMY, "groceries")]);
        let splits = SplitsByActivity::new();

        let summary = build_summary(
            "TOTAL",
            &activity_refs,
            &assignments,
            &splits,
            &cat_meta,
            &account_types,
            "USD",
            &DoubleEurFx,
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            "",
        );

        assert_eq!(summary.total_spending, 200.0);
        assert_eq!(summary.by_category["groceries"].amount, 200.0);
    }

    #[test]
    fn build_summary_excludes_failed_fx_conversion() {
        let (mut charge, _) = spending_activity("charge", "WITHDRAWAL", 100, "groceries", 1);
        charge.currency = "EUR".to_string();
        let activity_refs = vec![&charge];
        let account_types = HashMap::from([(
            "card-account".to_string(),
            account_types::CREDIT_CARD.to_string(),
        )]);
        let cat_meta = HashMap::from([(
            "groceries".to_string(),
            ("Groceries".to_string(), Some("#1".to_string()), None),
        )]);
        let assignments =
            group_assignments(vec![assignment("charge", SPENDING_TAXONOMY, "groceries")]);
        let splits = SplitsByActivity::new();

        let summary = build_summary(
            "TOTAL",
            &activity_refs,
            &assignments,
            &splits,
            &cat_meta,
            &account_types,
            "USD",
            &FailingFx,
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            "",
        );

        assert_eq!(summary.total_spending, 0.0);
        assert!(summary.by_category.is_empty());
    }

    #[test]
    fn build_summary_accumulates_small_amounts_with_decimal_precision() {
        let activities = (0..1000)
            .map(|idx| {
                let (mut activity, _) =
                    spending_activity(&format!("charge-{idx}"), "WITHDRAWAL", 0, "groceries", 1);
                activity.amount = Some(Decimal::new(1, 2));
                activity
            })
            .collect::<Vec<_>>();
        let activity_refs = activities.iter().collect::<Vec<_>>();
        let account_types = HashMap::from([(
            "card-account".to_string(),
            account_types::CREDIT_CARD.to_string(),
        )]);
        let cat_meta = HashMap::from([(
            "groceries".to_string(),
            ("Groceries".to_string(), Some("#1".to_string()), None),
        )]);
        let assignments = activities
            .iter()
            .map(|activity| assignment(&activity.id, SPENDING_TAXONOMY, "groceries"))
            .collect::<Vec<_>>();
        let assignments = group_assignments(assignments);
        let splits = SplitsByActivity::new();

        let summary = build_summary(
            "TOTAL",
            &activity_refs,
            &assignments,
            &splits,
            &cat_meta,
            &account_types,
            "USD",
            &PassthroughFx,
            NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(),
            "",
        );

        assert_eq!(summary.total_spending, 10.0);
        assert_eq!(summary.by_category["groceries"].amount, 10.0);
    }

    #[test]
    fn build_summary_buckets_by_user_timezone() {
        let (mut charge, _) = spending_activity("charge", "WITHDRAWAL", 100, "groceries", 3);
        charge.activity_date = Utc.with_ymd_and_hms(2024, 3, 1, 2, 30, 0).unwrap();

        let summary = build_credit_card_summary_in_timezone(&[charge], "America/Toronto");

        assert_eq!(summary.by_month.get("2024-02"), Some(&100.0));
        assert!(!summary.by_month.contains_key("2024-03"));
    }
}
