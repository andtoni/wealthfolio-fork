use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;
use wealthfolio_core::accounts::{AccountRepositoryTrait, AggregateEligibilityServiceTrait};
use wealthfolio_core::activities::ActivityRepositoryTrait;
use wealthfolio_core::taxonomies::{Category, TaxonomyServiceTrait};

use super::model::{
    BudgetCategoryRow, BudgetGroup, BudgetGroupRow, BudgetRolloverSetting,
    BudgetRolloverTargetType, BudgetSnapshot, BudgetSnapshotComputed, BudgetSnapshotState,
    BudgetTarget, BudgetTargetType, BudgetTotals, NewBudgetGroup, NewBudgetGroupAssignment,
    NewBudgetRolloverSetting, NewBudgetTarget, UpdateBudgetGroup,
};
use super::period::{
    current_period_key, local_period_bounds_utc, period_for_key, period_key_for_datetime,
    period_keys_between,
};
use super::traits::BudgetRepositoryTrait;
use crate::activity_allocations::{
    allocations_for_taxonomy, group_assignments, group_splits, SplitsByActivity,
};
use crate::activity_assignments::ActivityTaxonomyAssignmentRepositoryTrait;
use crate::activity_classification::{activity_abs_amount, classify_activity, decimal_to_f64};
use crate::activity_splits::ActivitySplitRepositoryTrait;
use crate::aggregate_eligibility::{
    filter_eligible_activity_rows, resolve_eligible_spending_accounts,
};
use crate::error::SpendingError;
use crate::settings::{BudgetCycleSettings, SpendingSettingsService};

const SPENDING_TAXONOMY: &str = "spending_categories";
const INCOME_TAXONOMY: &str = "income_sources";
const DEFAULT_PERIOD_KEY: &str = "default";
const OTHER_GROUP_KEY: &str = "other";
const OTHER_GROUP_NAME: &str = "Other";
const OTHER_GROUP_COLOR: &str = "#9C998E";
const OTHER_GROUP_ICON: &str = "MoreHorizontal";
const OTHER_GROUP_SORT_ORDER: i32 = 99;

type MonthActuals = HashMap<(String, String), Decimal>;

pub struct BudgetService {
    repo: Arc<dyn BudgetRepositoryTrait>,
    activity_repo: Arc<dyn ActivityRepositoryTrait>,
    account_repo: Arc<dyn AccountRepositoryTrait>,
    assignment_repo: Arc<dyn ActivityTaxonomyAssignmentRepositoryTrait>,
    split_repo: Arc<dyn ActivitySplitRepositoryTrait>,
    spending_settings: Arc<SpendingSettingsService>,
    taxonomy_service: Arc<dyn TaxonomyServiceTrait>,
    fx_service: Arc<dyn wealthfolio_core::fx::FxServiceTrait>,
    aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
}

impl BudgetService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: Arc<dyn BudgetRepositoryTrait>,
        activity_repo: Arc<dyn ActivityRepositoryTrait>,
        account_repo: Arc<dyn AccountRepositoryTrait>,
        assignment_repo: Arc<dyn ActivityTaxonomyAssignmentRepositoryTrait>,
        split_repo: Arc<dyn ActivitySplitRepositoryTrait>,
        spending_settings: Arc<SpendingSettingsService>,
        taxonomy_service: Arc<dyn TaxonomyServiceTrait>,
        fx_service: Arc<dyn wealthfolio_core::fx::FxServiceTrait>,
        aggregate_eligibility_service: Arc<dyn AggregateEligibilityServiceTrait>,
    ) -> Self {
        Self {
            repo,
            activity_repo,
            account_repo,
            assignment_repo,
            split_repo,
            spending_settings,
            taxonomy_service,
            fx_service,
            aggregate_eligibility_service,
        }
    }

    pub async fn get(
        &self,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        let settings = self.spending_settings.get().await?;
        let cycle = settings.budget_cycle.clone();
        let period_key = normalize_period_key(period_key, timezone, &cycle)?;
        let period = (period_key != DEFAULT_PERIOD_KEY)
            .then(|| period_for_key(&period_key, &cycle))
            .transpose()?;
        let period_start = period.as_ref().map(|value| value.start.to_string());
        let period_end = period.as_ref().map(|value| value.end.to_string());
        if !settings.enabled {
            return Ok(BudgetSnapshot::empty(
                period_key,
                currency.to_string(),
                cycle,
                period_start,
                period_end,
            ));
        }

        self.ensure_other_group().await?;
        let groups = self.repo.list_groups().await?;
        let assignments = self.repo.list_group_assignments().await?;
        let targets = self.repo.list_targets().await?;
        let rollover_settings = self.repo.list_rollover_settings().await?;

        let spending_categories = self.taxonomy_categories(SPENDING_TAXONOMY)?;
        let income_categories = self.taxonomy_categories(INCOME_TAXONOMY)?;
        let spending_category_meta = category_meta(&spending_categories);
        let income_meta = category_meta(&income_categories);
        let top_spending_categories = top_level_categories(&spending_categories);
        let top_income_categories = top_level_categories(&income_categories);

        let is_month_view = period_key != DEFAULT_PERIOD_KEY;
        let actuals_by_month = if is_month_view {
            let earliest_rollover_month = rollover_settings
                .iter()
                .filter(|s| s.enabled && s.start_month <= period_key)
                .map(|s| s.start_month.clone())
                .min()
                .unwrap_or_else(|| period_key.clone());
            self.actuals_by_period(
                &earliest_rollover_month,
                &period_key,
                &spending_category_meta,
                &income_meta,
                currency,
                timezone,
                &cycle,
            )
            .await?
        } else {
            HashMap::new()
        };
        let current_actuals = actuals_by_month
            .get(&period_key)
            .cloned()
            .unwrap_or_default();

        let target_index = TargetIndex::new(&targets);
        let rollover_index = RolloverIndex::new(&rollover_settings);
        let group_by_key: HashMap<String, BudgetGroup> =
            groups.iter().map(|g| (g.key.clone(), g.clone())).collect();
        let other_group_id = group_by_key
            .get(OTHER_GROUP_KEY)
            .map(|g| g.id.clone())
            .ok_or_else(|| anyhow!("Missing Other budget group"))?;

        let group_ids: HashSet<&str> = groups.iter().map(|group| group.id.as_str()).collect();
        let assignment_by_category: HashMap<String, String> = assignments
            .iter()
            .filter(|a| {
                a.taxonomy_id == SPENDING_TAXONOMY && group_ids.contains(a.group_id.as_str())
            })
            .map(|a| (a.category_id.clone(), a.group_id.clone()))
            .collect();
        let group_for_category = |category_id: &str| -> String {
            resolve_group_for_category(
                category_id,
                &assignment_by_category,
                &spending_category_meta,
                &other_group_id,
            )
        };

        let mut rows_by_group: HashMap<String, Vec<BudgetCategoryRow>> = HashMap::new();
        for category in &top_spending_categories {
            let group_id = group_for_category(&category.id);
            let excluded_from_budget = group_by_key
                .get(&group_id)
                .is_some_and(|group| group.exclude_from_budget);
            let configured_actual = current_actuals
                .get(&(SPENDING_TAXONOMY.to_string(), category.id.clone()))
                .copied()
                .unwrap_or(Decimal::ZERO);
            let configured_target = target_index.effective_category_decimal(
                &period_key,
                SPENDING_TAXONOMY,
                &category.id,
            );
            let actual = if excluded_from_budget {
                Decimal::ZERO
            } else {
                configured_actual
            };
            let target = if excluded_from_budget {
                Decimal::ZERO
            } else {
                configured_target
            };
            let rollover = (!excluded_from_budget && is_month_view)
                .then(|| rollover_index.category(SPENDING_TAXONOMY, &category.id))
                .flatten();
            let (rollover_in, rollover_out, remaining) = if let Some(setting) = rollover {
                compute_rollover_for_period(
                    setting,
                    &period_key,
                    &cycle,
                    |month| {
                        target_index.effective_category_decimal(
                            month,
                            SPENDING_TAXONOMY,
                            &category.id,
                        )
                    },
                    |month| {
                        actuals_by_month
                            .get(month)
                            .and_then(|m| {
                                m.get(&(SPENDING_TAXONOMY.to_string(), category.id.clone()))
                            })
                            .copied()
                            .unwrap_or(Decimal::ZERO)
                    },
                )?
            } else {
                (Decimal::ZERO, Decimal::ZERO, target - actual)
            };
            rows_by_group
                .entry(group_id.clone())
                .or_default()
                .push(BudgetCategoryRow {
                    taxonomy_id: SPENDING_TAXONOMY.to_string(),
                    category_id: category.id.clone(),
                    group_id: Some(group_id.clone()),
                    parent_id: category.parent_id.clone(),
                    name: category.name.clone(),
                    color: Some(category.color.clone()),
                    icon: category.icon.clone(),
                    target: decimal_to_f64(target),
                    actual: decimal_to_f64(actual),
                    rollover_in: decimal_to_f64(rollover_in),
                    rollover_out: decimal_to_f64(rollover_out),
                    remaining: decimal_to_f64(remaining),
                    overspent: remaining < Decimal::ZERO,
                    has_default_target: target_index
                        .has_default_category(SPENDING_TAXONOMY, &category.id),
                    has_month_override: target_index.has_month_category(
                        &period_key,
                        SPENDING_TAXONOMY,
                        &category.id,
                    ),
                    rollover_enabled: rollover.is_some(),
                    excluded_from_budget,
                });
        }

        let mut group_rows = Vec::with_capacity(groups.len());
        let mut spending_planned_total = Decimal::ZERO;
        let mut spending_actual_total = Decimal::ZERO;
        let mut spending_remaining_total = Decimal::ZERO;
        let mut group_buffer_total = Decimal::ZERO;
        let mut rollover_in_total = Decimal::ZERO;
        let mut rollover_out_total = Decimal::ZERO;
        for group in &groups {
            let mut categories = rows_by_group.remove(&group.id).unwrap_or_default();
            categories.sort_by(|a, b| a.name.cmp(&b.name));
            let category_ids = categories
                .iter()
                .map(|category| category.category_id.clone())
                .collect::<Vec<_>>();
            let excluded_from_budget = group.exclude_from_budget;
            let category_target_total_decimal = if excluded_from_budget {
                Decimal::ZERO
            } else {
                category_ids
                    .iter()
                    .map(|category_id| {
                        target_index.effective_category_decimal(
                            &period_key,
                            SPENDING_TAXONOMY,
                            category_id,
                        )
                    })
                    .sum::<Decimal>()
            };
            let actual_decimal = if excluded_from_budget {
                Decimal::ZERO
            } else {
                category_ids
                    .iter()
                    .map(|category_id| {
                        current_actuals
                            .get(&(SPENDING_TAXONOMY.to_string(), category_id.clone()))
                            .copied()
                            .unwrap_or(Decimal::ZERO)
                    })
                    .sum::<Decimal>()
            };
            let buffer_decimal = if excluded_from_budget {
                Decimal::ZERO
            } else {
                target_index.effective_group_buffer_decimal(&period_key, &group.id)
            };
            let planned_total_decimal = category_target_total_decimal + buffer_decimal;
            let category_target_total = decimal_to_f64(category_target_total_decimal);
            let actual = decimal_to_f64(actual_decimal);
            let buffer = decimal_to_f64(buffer_decimal);
            let planned_total = decimal_to_f64(planned_total_decimal);
            let rollover = (!excluded_from_budget && is_month_view)
                .then(|| rollover_index.group(&group.id))
                .flatten();
            let (rollover_in, rollover_out, remaining) = if let Some(setting) = rollover {
                compute_rollover_for_period(
                    setting,
                    &period_key,
                    &cycle,
                    |month| {
                        let child_total = category_ids
                            .iter()
                            .map(|category_id| {
                                target_index.effective_category_decimal(
                                    month,
                                    SPENDING_TAXONOMY,
                                    category_id,
                                )
                            })
                            .sum::<Decimal>();
                        child_total + target_index.effective_group_buffer_decimal(month, &group.id)
                    },
                    |month| {
                        category_ids
                            .iter()
                            .map(|category_id| {
                                actuals_by_month
                                    .get(month)
                                    .and_then(|m| {
                                        m.get(&(SPENDING_TAXONOMY.to_string(), category_id.clone()))
                                    })
                                    .copied()
                                    .unwrap_or(Decimal::ZERO)
                            })
                            .sum::<Decimal>()
                    },
                )?
            } else {
                (
                    Decimal::ZERO,
                    Decimal::ZERO,
                    planned_total_decimal - actual_decimal,
                )
            };
            spending_planned_total += planned_total_decimal;
            spending_actual_total += actual_decimal;
            spending_remaining_total += remaining;
            group_buffer_total += buffer_decimal;
            rollover_in_total += rollover_in;
            rollover_out_total += rollover_out;
            group_rows.push(BudgetGroupRow {
                group: group.clone(),
                category_target_total,
                buffer,
                planned_total,
                actual,
                rollover_in: decimal_to_f64(rollover_in),
                rollover_out: decimal_to_f64(rollover_out),
                remaining: decimal_to_f64(remaining),
                overspent: remaining < Decimal::ZERO,
                rollover_enabled: rollover.is_some(),
                categories,
            });
        }
        group_rows.sort_by(|a, b| {
            a.group
                .sort_order
                .cmp(&b.group.sort_order)
                .then(a.group.name.cmp(&b.group.name))
        });

        let mut income_rows = Vec::with_capacity(top_income_categories.len());
        let mut income_planned_total = Decimal::ZERO;
        let mut income_actual_total = Decimal::ZERO;
        for category in &top_income_categories {
            let actual = current_actuals
                .get(&(INCOME_TAXONOMY.to_string(), category.id.clone()))
                .copied()
                .unwrap_or(Decimal::ZERO);
            let target =
                target_index.effective_category_decimal(&period_key, INCOME_TAXONOMY, &category.id);
            income_planned_total += target;
            income_actual_total += actual;
            income_rows.push(BudgetCategoryRow {
                taxonomy_id: INCOME_TAXONOMY.to_string(),
                category_id: category.id.clone(),
                group_id: None,
                parent_id: category.parent_id.clone(),
                name: category.name.clone(),
                color: Some(category.color.clone()),
                icon: category.icon.clone(),
                target: decimal_to_f64(target),
                actual: decimal_to_f64(actual),
                rollover_in: 0.0,
                rollover_out: 0.0,
                remaining: decimal_to_f64(target - actual),
                overspent: false,
                has_default_target: target_index
                    .has_default_category(INCOME_TAXONOMY, &category.id),
                has_month_override: target_index.has_month_category(
                    &period_key,
                    INCOME_TAXONOMY,
                    &category.id,
                ),
                rollover_enabled: false,
                excluded_from_budget: false,
            });
        }
        income_rows.sort_by(|a, b| a.name.cmp(&b.name));

        let totals = BudgetTotals {
            spending_planned: decimal_to_f64(spending_planned_total),
            spending_actual: decimal_to_f64(spending_actual_total),
            spending_remaining: decimal_to_f64(spending_remaining_total),
            income_planned: decimal_to_f64(income_planned_total),
            income_actual: decimal_to_f64(income_actual_total),
            group_buffer: decimal_to_f64(group_buffer_total),
            rollover_in: decimal_to_f64(rollover_in_total),
            rollover_out: decimal_to_f64(rollover_out_total),
            overspent_count: group_rows.iter().filter(|g| g.overspent).count()
                + group_rows
                    .iter()
                    .flat_map(|g| &g.categories)
                    .filter(|c| c.overspent)
                    .count(),
        };
        let fx_as_of = period_end.clone();

        Ok(BudgetSnapshot {
            state: BudgetSnapshotState {
                groups,
                group_assignments: assignments,
                targets,
                rollover_settings,
            },
            computed: BudgetSnapshotComputed {
                currency: currency.to_string(),
                period_key,
                period_start,
                period_end,
                budget_cycle: cycle,
                fx_as_of,
                group_rows,
                ungrouped_rows: vec![],
                income_rows,
                totals,
            },
        })
    }

    pub async fn create_group(
        &self,
        input: NewBudgetGroup,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        self.repo
            .create_group(NewBudgetGroup {
                id: input.id,
                key: Some(format!("custom_{}", Uuid::new_v4())),
                name: input.name,
                color: input.color,
                icon: input.icon,
                sort_order: input.sort_order,
                is_system: false,
                exclude_from_budget: input.exclude_from_budget,
            })
            .await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn update_group(
        &self,
        id: &str,
        patch: UpdateBudgetGroup,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        self.repo.update_group(id, patch).await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn delete_group(
        &self,
        id: &str,
        reassign_to_group_id: &str,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        let groups = self.repo.list_groups().await?;
        let _group = groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| invalid_budget_input("Budget group not found"))?;
        if reassign_to_group_id == id {
            return Err(invalid_budget_input(
                "Cannot reassign categories to the group being deleted",
            ));
        }
        if !groups.iter().any(|g| g.id == reassign_to_group_id) {
            return Err(invalid_budget_input("Reassignment budget group not found"));
        }
        if self
            .repo
            .list_rollover_settings()
            .await?
            .into_iter()
            .any(|r| {
                matches!(r.target_type, BudgetRolloverTargetType::Group)
                    && r.group_id.as_deref() == Some(id)
            })
        {
            return Err(invalid_budget_input(
                "Delete the group's rollover setting before deleting the group",
            ));
        }
        let assignments = self.repo.list_group_assignments().await?;
        let reassignments = assignments
            .into_iter()
            .filter(|a| a.group_id == id)
            .map(|a| NewBudgetGroupAssignment {
                id: Some(a.id),
                group_id: reassign_to_group_id.to_string(),
                taxonomy_id: a.taxonomy_id,
                category_id: a.category_id,
            })
            .collect::<Vec<_>>();
        self.repo
            .delete_group_and_reassign(id, reassign_to_group_id, reassignments)
            .await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn assign_category_to_group(
        &self,
        category_id: String,
        group_id: String,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        self.repo
            .upsert_group_assignment(NewBudgetGroupAssignment {
                id: None,
                group_id,
                taxonomy_id: SPENDING_TAXONOMY.to_string(),
                category_id,
            })
            .await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn reset_groups(
        &self,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        // Keep this endpoint harmless for older clients. Resetting must not
        // recreate the removed canonical group/assignment seed.
        self.ensure_other_group().await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn upsert_target(
        &self,
        target: NewBudgetTarget,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        let cycle = self.spending_settings.get().await?.budget_cycle;
        validate_period_key(&target.period_key, &cycle)?;
        validate_budget_target(&target)?;
        self.repo.upsert_target(target).await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn delete_target(
        &self,
        id: &str,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        self.repo.delete_target(id).await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn upsert_rollover_setting(
        &self,
        setting: NewBudgetRolloverSetting,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        let cycle = self.spending_settings.get().await?.budget_cycle;
        period_for_key(&setting.start_month, &cycle)?;
        validate_rollover_setting(&setting)?;
        match setting.target_type {
            BudgetRolloverTargetType::Group if setting.enabled => {
                let group_id = setting
                    .group_id
                    .as_ref()
                    .ok_or_else(|| invalid_budget_input("Group rollover requires groupId"))?;
                let categories = self.categories_for_group(group_id).await?;
                self.repo
                    .disable_category_rollovers(SPENDING_TAXONOMY, &categories)
                    .await?;
            }
            BudgetRolloverTargetType::Category if setting.enabled => {
                let category_id = setting
                    .category_id
                    .as_ref()
                    .ok_or_else(|| invalid_budget_input("Category rollover requires categoryId"))?;
                let group_id = self.group_id_for_category(category_id).await?;
                let group_rollover_enabled = self
                    .repo
                    .list_rollover_settings()
                    .await?
                    .into_iter()
                    .any(|r| {
                        r.enabled
                            && matches!(r.target_type, BudgetRolloverTargetType::Group)
                            && r.group_id.as_deref() == Some(group_id.as_str())
                    });
                if group_rollover_enabled {
                    return Err(invalid_budget_input(
                        "Disable group rollover before enabling category rollover",
                    ));
                }
            }
            _ => {}
        }
        self.repo.upsert_rollover_setting(setting).await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn delete_rollover_setting(
        &self,
        id: &str,
        period_key: Option<String>,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        self.repo.delete_rollover_setting(id).await?;
        self.get(period_key, currency, timezone).await
    }

    pub async fn copy_period_targets(
        &self,
        source_period_key: &str,
        target_period_key: &str,
        overwrite: bool,
        currency: &str,
        timezone: &str,
    ) -> Result<BudgetSnapshot> {
        let cycle = self.spending_settings.get().await?.budget_cycle;
        validate_period_key(source_period_key, &cycle)?;
        period_for_key(target_period_key, &cycle)?;
        if source_period_key == target_period_key {
            return Err(invalid_budget_input(
                "Source and target periods must differ",
            ));
        }
        self.repo
            .copy_period_targets(source_period_key, target_period_key, overwrite)
            .await?;
        self.get(Some(target_period_key.to_string()), currency, timezone)
            .await
    }

    async fn ensure_other_group(&self) -> Result<()> {
        self.repo
            .ensure_group(NewBudgetGroup {
                id: None,
                name: OTHER_GROUP_NAME.to_string(),
                key: Some(OTHER_GROUP_KEY.to_string()),
                color: Some(OTHER_GROUP_COLOR.to_string()),
                icon: Some(OTHER_GROUP_ICON.to_string()),
                sort_order: Some(OTHER_GROUP_SORT_ORDER),
                is_system: false,
                exclude_from_budget: true,
            })
            .await?;
        Ok(())
    }

    fn taxonomy_categories(&self, taxonomy_id: &str) -> Result<Vec<Category>> {
        Ok(self
            .taxonomy_service
            .get_taxonomy(taxonomy_id)?
            .map(|t| t.categories)
            .unwrap_or_default())
    }

    async fn categories_for_group(&self, group_id: &str) -> Result<Vec<String>> {
        let assignments = self.repo.list_group_assignments().await?;
        let categories = self.taxonomy_categories(SPENDING_TAXONOMY)?;
        let meta = category_meta(&categories);
        let groups = self.repo.list_groups().await?;
        let group_ids: HashSet<&str> = groups.iter().map(|group| group.id.as_str()).collect();
        let other_group_id = groups
            .iter()
            .find(|g| g.key == OTHER_GROUP_KEY)
            .map(|g| g.id.clone())
            .ok_or_else(|| anyhow!("Missing Other budget group"))?;
        let assignment_by_category = assignments
            .into_iter()
            .filter(|a| {
                a.taxonomy_id == SPENDING_TAXONOMY && group_ids.contains(a.group_id.as_str())
            })
            .map(|a| (a.category_id, a.group_id))
            .collect::<HashMap<_, _>>();

        Ok(top_level_categories(&categories)
            .into_iter()
            .filter(|category| {
                resolve_group_for_category(
                    &category.id,
                    &assignment_by_category,
                    &meta,
                    &other_group_id,
                ) == group_id
            })
            .map(|category| category.id)
            .collect())
    }

    async fn group_id_for_category(&self, category_id: &str) -> Result<String> {
        let assignments = self.repo.list_group_assignments().await?;
        let categories = self.taxonomy_categories(SPENDING_TAXONOMY)?;
        let meta = category_meta(&categories);
        let groups = self.repo.list_groups().await?;
        let group_ids: HashSet<&str> = groups.iter().map(|group| group.id.as_str()).collect();
        let other_group_id = groups
            .iter()
            .find(|g| g.key == OTHER_GROUP_KEY)
            .map(|g| g.id.clone())
            .ok_or_else(|| anyhow!("Missing Other budget group"))?;
        let assignment_by_category = assignments
            .into_iter()
            .filter(|a| group_ids.contains(a.group_id.as_str()))
            .map(|a| (a.category_id, a.group_id))
            .collect::<HashMap<_, _>>();
        Ok(resolve_group_for_category(
            category_id,
            &assignment_by_category,
            &meta,
            &other_group_id,
        ))
    }

    /// `currency` is the FX target — every counted activity is converted at
    /// the selected recurring period's end. `timezone` (IANA name, may be
    /// empty) drives local-date bucketing from the configured anchor.
    async fn actuals_by_period(
        &self,
        start_period: &str,
        end_period: &str,
        spending_meta: &HashMap<String, Category>,
        income_meta: &HashMap<String, Category>,
        currency: &str,
        timezone: &str,
        cycle: &BudgetCycleSettings,
    ) -> Result<HashMap<String, MonthActuals>> {
        let settings = self.spending_settings.get().await?;
        if !settings.enabled || settings.account_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let (_, end) = local_period_bounds_utc(end_period, timezone, cycle)?;
        let eligible_accounts = resolve_eligible_spending_accounts(
            self.account_repo.as_ref(),
            self.aggregate_eligibility_service.as_ref(),
            &settings.account_ids,
            period_for_key(end_period, cycle)?.end,
        )
        .await?;
        if eligible_accounts.ids.is_empty() {
            return Ok(HashMap::new());
        }
        let (start, _) = local_period_bounds_utc(start_period, timezone, cycle)?;
        let activities = self
            .activity_repo
            .get_activities_by_account_ids_in_date_range(&eligible_accounts.ids, start, end)
            .map_err(|e| anyhow!(e.to_string()))?
            .into_iter()
            .collect::<Vec<_>>();
        let activities = filter_eligible_activity_rows(
            self.aggregate_eligibility_service.as_ref(),
            activities.clone(),
            &activities,
        )
        .await?;
        let account_types = eligible_accounts.account_types;
        let activity_ids = activities.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        let assignments = self
            .assignment_repo
            .list_for_activities(&activity_ids)
            .await?;
        let assignments_by_activity = group_assignments(assignments);
        let splits_by_activity =
            group_splits(self.split_repo.list_for_activities(&activity_ids).await?);

        // FX: one as_of date for the entire window (the latest cycle's end).
        let fx_as_of = period_for_key(end_period, cycle)?.end;
        let fx = self.fx_service.as_ref();
        let mut actuals: HashMap<String, MonthActuals> = HashMap::new();
        for activity in activities {
            let Some(account_type) = account_types.get(&activity.account_id) else {
                continue;
            };
            let classification = classify_activity(&activity, account_type);
            let amount = activity_abs_amount(&activity);
            let spending_native = classification.spending_amount(amount);
            let income_native = classification.income_amount(amount);
            if spending_native == Decimal::ZERO && income_native == Decimal::ZERO {
                continue;
            }
            let period_key = period_key_for_datetime(activity.activity_date, timezone, cycle)?;
            let month_actuals = actuals.entry(period_key).or_default();
            add_allocated_actuals(
                month_actuals,
                &activity.id,
                SPENDING_TAXONOMY,
                spending_native,
                spending_meta,
                &assignments_by_activity,
                &splits_by_activity,
                fx,
                &activity.currency,
                currency,
                fx_as_of,
            );
            add_allocated_actuals(
                month_actuals,
                &activity.id,
                INCOME_TAXONOMY,
                income_native,
                income_meta,
                &assignments_by_activity,
                &splits_by_activity,
                fx,
                &activity.currency,
                currency,
                fx_as_of,
            );
        }
        Ok(actuals)
    }
}

#[allow(clippy::too_many_arguments)]
fn add_allocated_actuals(
    month_actuals: &mut MonthActuals,
    activity_id: &str,
    taxonomy_id: &str,
    amount: Decimal,
    meta: &HashMap<String, Category>,
    assignments_by_activity: &crate::activity_allocations::AssignmentsByActivity,
    splits_by_activity: &SplitsByActivity,
    fx: &dyn wealthfolio_core::fx::FxServiceTrait,
    from_currency: &str,
    target_currency: &str,
    fx_as_of: NaiveDate,
) {
    for allocation in allocations_for_taxonomy(
        activity_id,
        taxonomy_id,
        amount,
        assignments_by_activity,
        splits_by_activity,
    ) {
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
        let top_id = top_category_id(&allocation.category_id, meta);
        *month_actuals
            .entry((taxonomy_id.to_string(), top_id))
            .or_insert(Decimal::ZERO) += amount;
    }
}

pub(crate) struct TargetIndex<'a> {
    targets: &'a [BudgetTarget],
}

impl<'a> TargetIndex<'a> {
    pub(crate) fn new(targets: &'a [BudgetTarget]) -> Self {
        Self { targets }
    }

    pub(crate) fn effective_category_decimal(
        &self,
        period: &str,
        taxonomy_id: &str,
        category_id: &str,
    ) -> Decimal {
        self.month_category(period, taxonomy_id, category_id)
            .or_else(|| self.default_category(taxonomy_id, category_id))
            .map(parse_amount)
            .unwrap_or(Decimal::ZERO)
    }

    pub(crate) fn effective_group_buffer_decimal(&self, period: &str, group_id: &str) -> Decimal {
        self.month_group_buffer(period, group_id)
            .or_else(|| self.default_group_buffer(group_id))
            .map(parse_amount)
            .unwrap_or(Decimal::ZERO)
    }

    pub(crate) fn has_default_category(&self, taxonomy_id: &str, category_id: &str) -> bool {
        self.default_category(taxonomy_id, category_id).is_some()
    }

    pub(crate) fn has_month_category(
        &self,
        period: &str,
        taxonomy_id: &str,
        category_id: &str,
    ) -> bool {
        self.month_category(period, taxonomy_id, category_id)
            .is_some()
    }

    pub(crate) fn has_month_group_buffer(&self, period: &str, group_id: &str) -> bool {
        self.month_group_buffer(period, group_id).is_some()
    }

    fn month_category(&self, period: &str, taxonomy_id: &str, category_id: &str) -> Option<&str> {
        self.targets
            .iter()
            .find(|t| {
                matches!(t.target_type, BudgetTargetType::Category)
                    && t.period_key == period
                    && t.taxonomy_id.as_deref() == Some(taxonomy_id)
                    && t.category_id.as_deref() == Some(category_id)
            })
            .map(|t| t.amount.as_str())
    }

    fn default_category(&self, taxonomy_id: &str, category_id: &str) -> Option<&str> {
        self.month_category(DEFAULT_PERIOD_KEY, taxonomy_id, category_id)
    }

    fn month_group_buffer(&self, period: &str, group_id: &str) -> Option<&str> {
        self.targets
            .iter()
            .find(|t| {
                matches!(t.target_type, BudgetTargetType::GroupBuffer)
                    && t.period_key == period
                    && t.group_id.as_deref() == Some(group_id)
            })
            .map(|t| t.amount.as_str())
    }

    fn default_group_buffer(&self, group_id: &str) -> Option<&str> {
        self.month_group_buffer(DEFAULT_PERIOD_KEY, group_id)
    }
}

struct RolloverIndex<'a> {
    settings: &'a [BudgetRolloverSetting],
}

impl<'a> RolloverIndex<'a> {
    fn new(settings: &'a [BudgetRolloverSetting]) -> Self {
        Self { settings }
    }

    fn category(&self, taxonomy_id: &str, category_id: &str) -> Option<&'a BudgetRolloverSetting> {
        self.settings.iter().find(|s| {
            s.enabled
                && matches!(s.target_type, BudgetRolloverTargetType::Category)
                && s.taxonomy_id.as_deref() == Some(taxonomy_id)
                && s.category_id.as_deref() == Some(category_id)
        })
    }

    fn group(&self, group_id: &str) -> Option<&'a BudgetRolloverSetting> {
        self.settings.iter().find(|s| {
            s.enabled
                && matches!(s.target_type, BudgetRolloverTargetType::Group)
                && s.group_id.as_deref() == Some(group_id)
        })
    }
}

fn compute_rollover_for_period(
    setting: &BudgetRolloverSetting,
    period_key: &str,
    cycle: &BudgetCycleSettings,
    target_for_period: impl Fn(&str) -> Decimal,
    actual_for_period: impl Fn(&str) -> Decimal,
) -> Result<(Decimal, Decimal, Decimal)> {
    if setting.start_month.as_str() > period_key {
        let target = target_for_period(period_key);
        let actual = actual_for_period(period_key);
        return Ok((Decimal::ZERO, Decimal::ZERO, target - actual));
    }
    let mut carry = parse_amount(&setting.starting_balance);
    for period in period_keys_between(&setting.start_month, period_key, cycle)? {
        let rollover_in = carry;
        let target = target_for_period(&period);
        let actual = actual_for_period(&period);
        let rollover_out = rollover_in + target - actual;
        if period == period_key {
            return Ok((rollover_in, rollover_out, rollover_in + target - actual));
        }
        carry = rollover_out;
    }
    Ok((
        Decimal::ZERO,
        Decimal::ZERO,
        target_for_period(period_key) - actual_for_period(period_key),
    ))
}

pub(crate) fn category_meta(categories: &[Category]) -> HashMap<String, Category> {
    categories
        .iter()
        .map(|c| (c.id.clone(), c.clone()))
        .collect()
}

pub(crate) fn top_level_categories(categories: &[Category]) -> Vec<Category> {
    let mut categories = categories
        .iter()
        .filter(|c| c.parent_id.is_none())
        .cloned()
        .collect::<Vec<_>>();
    categories.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
    categories
}

pub(crate) fn resolve_group_for_category(
    category_id: &str,
    assignment_by_category: &HashMap<String, String>,
    category_meta: &HashMap<String, Category>,
    other_group_id: &str,
) -> String {
    let mut current = Some(category_id.to_string());
    while let Some(id) = current {
        if let Some(group_id) = assignment_by_category.get(&id) {
            return group_id.clone();
        }
        current = category_meta.get(&id).and_then(|c| c.parent_id.clone());
    }
    other_group_id.to_string()
}

pub(crate) fn top_category_id(category_id: &str, meta: &HashMap<String, Category>) -> String {
    let mut current = category_id.to_string();
    // Guard against a corrupted taxonomy with a cyclic parent_id chain: bound
    // the walk to a depth far above any realistic hierarchy depth (the seeded
    // taxonomies are 2 levels deep). Without the bound, a `parent_id` loop
    // would hang the entire request thread.
    const MAX_DEPTH: usize = 32;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(current.clone());
    for _ in 0..MAX_DEPTH {
        match meta.get(&current).and_then(|c| c.parent_id.clone()) {
            Some(parent_id) if !seen.contains(&parent_id) => {
                seen.insert(parent_id.clone());
                current = parent_id;
            }
            _ => break,
        }
    }
    current
}

/// Parse a string-encoded decimal amount. Garbage input degrades to
/// 0.0 (callers treat that as "no budget set"), but log a warning so a
/// corrupted row is at least visible in operator logs.
fn parse_amount(value: &str) -> Decimal {
    match Decimal::from_str(value) {
        Ok(n) => n,
        Err(_) => {
            log::warn!(
                "budget parse_amount: ignoring non-numeric amount {:?} (treating as 0.0)",
                value
            );
            Decimal::ZERO
        }
    }
}

fn validate_decimal_amount(value: &str, field_name: &str) -> Result<()> {
    Decimal::from_str(value)
        .map(|_| ())
        .map_err(|_| invalid_budget_input(&format!("Invalid {field_name} amount")))
}

fn normalize_period_key(
    period_key: Option<String>,
    timezone: &str,
    cycle: &BudgetCycleSettings,
) -> Result<String> {
    match period_key {
        Some(key) if key == DEFAULT_PERIOD_KEY => Ok(key),
        Some(key) => {
            period_for_key(&key, cycle)?;
            Ok(key)
        }
        None => current_period_key(timezone, cycle),
    }
}

fn validate_period_key(period_key: &str, cycle: &BudgetCycleSettings) -> Result<()> {
    if period_key == DEFAULT_PERIOD_KEY {
        Ok(())
    } else {
        period_for_key(period_key, cycle).map(|_| ())
    }
}

fn validate_budget_target(target: &NewBudgetTarget) -> Result<()> {
    validate_decimal_amount(&target.amount, "budget target")?;

    match target.target_type {
        BudgetTargetType::Category => {
            if target.taxonomy_id.as_deref().unwrap_or_default().is_empty()
                || target.category_id.as_deref().unwrap_or_default().is_empty()
                || target.group_id.is_some()
            {
                return Err(invalid_budget_input(
                    "Category target requires taxonomyId and categoryId only",
                ));
            }
        }
        BudgetTargetType::GroupBuffer => {
            if target.group_id.as_deref().unwrap_or_default().is_empty()
                || target.taxonomy_id.is_some()
                || target.category_id.is_some()
            {
                return Err(invalid_budget_input(
                    "Group buffer target requires groupId only",
                ));
            }
        }
    }
    Ok(())
}

fn validate_rollover_setting(setting: &NewBudgetRolloverSetting) -> Result<()> {
    validate_decimal_amount(&setting.starting_balance, "rollover starting balance")?;

    match setting.target_type {
        BudgetRolloverTargetType::Category => {
            if setting.taxonomy_id.as_deref() != Some(SPENDING_TAXONOMY)
                || setting
                    .category_id
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                || setting.group_id.is_some()
            {
                return Err(invalid_budget_input(
                    "Category rollover requires spending taxonomyId and categoryId only",
                ));
            }
        }
        BudgetRolloverTargetType::Group => {
            if setting.group_id.as_deref().unwrap_or_default().is_empty()
                || setting.taxonomy_id.is_some()
                || setting.category_id.is_some()
            {
                return Err(invalid_budget_input("Group rollover requires groupId only"));
            }
        }
    }
    Ok(())
}

fn invalid_budget_input(message: &str) -> anyhow::Error {
    SpendingError::InvalidInput {
        message: message.to_string(),
    }
    .into()
}

/// Convert a native amount to the budget's target currency at `as_of`.
/// Mirrors `insight::service::fx_to_target` and `analytics::service::fx_to_target`
/// — same convention (one rate per report, snapshot-date style) so all three
/// services agree. Same-currency short-circuit; on FxService error, returns
/// None so callers exclude the native amount instead of mixing currencies into
/// the target total.
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
                "spending budget FX conversion {}→{} on {} failed ({}); excluding native amount",
                from,
                to,
                as_of,
                e,
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::type_complexity)]

    const BUDGET_GROUP_NEEDS_ID: &str = "032ecb02-5912-42e8-9724-2cd566fc08d5";
    const BUDGET_GROUP_WANTS_ID: &str = "a409e0d6-9152-49c8-a5b4-a147a8ac636e";
    const BUDGET_GROUP_SAVINGS_ID: &str = "1fb6f2a3-3245-4702-83e8-ab116458d13e";
    const BUDGET_GROUP_GIVING_ID: &str = "8cbd26c8-e3b2-4176-8c61-e5c11e10b808";
    const BUDGET_GROUP_PERSONAL_ID: &str = "3ff71753-5dd5-4372-9ca2-63d8d9a04851";
    const BUDGET_GROUP_OTHER_ID: &str = "6e25d097-0c73-4521-9407-d47e8dfb73e2";

    use async_trait::async_trait;
    use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
    use serde_json::Value;
    use std::sync::Mutex;
    use wealthfolio_core::accounts::{
        account_types, Account, AccountAccountingSettings, AccountRepositoryTrait, AccountUpdate,
        AggregateEligibilityServiceTrait, NewAccount, ProvenanceRole, TrackingMode,
    };
    use wealthfolio_core::activities::{
        Activity, ActivityBulkMutationResult, ActivitySearchResponse, ActivityStatus,
        ActivityUpdate, ActivityUpsert, BulkUpsertResult, ImportMapping, ImportTemplate,
        IncomeData, NewActivity, Sort, ACTIVITY_TYPE_TRANSFER_IN, ACTIVITY_TYPE_TRANSFER_OUT,
    };
    use wealthfolio_core::fx::{ExchangeRate, NewExchangeRate};
    use wealthfolio_core::limits::ContributionActivity;
    use wealthfolio_core::taxonomies::{
        AssetTaxonomyAssignment, Category, NewAssetTaxonomyAssignment, NewCategory, NewTaxonomy,
        Taxonomy, TaxonomyWithCategories,
    };
    use wealthfolio_core::{
        errors::{Error, ValidationError},
        Result as CoreResult,
    };

    use super::super::model::BudgetGroupAssignment;
    use super::*;
    use crate::activity_assignments::{ActivityTaxonomyAssignment, NewActivityTaxonomyAssignment};
    use crate::activity_splits::{ActivitySplit, NewActivitySplit};
    use crate::settings::{
        BudgetCadence, SpendingSettingsRepositoryTrait, SETTING_KEY_ACCOUNT_IDS,
        SETTING_KEY_ENABLED,
    };

    fn ts() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn account(id: &str, account_type: &str) -> Account {
        Account {
            id: id.to_string(),
            name: id.to_string(),
            account_type: account_type.to_string(),
            group: None,
            currency: "USD".to_string(),
            is_default: false,
            is_active: true,
            created_at: ts(),
            updated_at: ts(),
            platform_id: None,
            account_number: None,
            meta: None,
            provider: None,
            provider_account_id: None,
            is_archived: false,
            tracking_mode: TrackingMode::Transactions,
            include_in_net_worth: true,
            name_source: "manual".to_string(),
        }
    }

    fn activity(id: &str, account_id: &str, activity_type: &str, amount: i64) -> Activity {
        Activity {
            id: id.to_string(),
            account_id: account_id.to_string(),
            asset_id: None,
            activity_type: activity_type.to_string(),
            activity_type_override: None,
            source_type: None,
            subtype: None,
            status: ActivityStatus::Posted,
            activity_date: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
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
        }
    }

    fn assignment(activity_id: &str, category_id: &str) -> ActivityTaxonomyAssignment {
        ActivityTaxonomyAssignment {
            id: format!("{activity_id}-{category_id}"),
            activity_id: activity_id.to_string(),
            taxonomy_id: SPENDING_TAXONOMY.to_string(),
            category_id: category_id.to_string(),
            weight: 10_000,
            source: "manual".to_string(),
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn category(taxonomy_id: &str, category_id: &str, parent_id: Option<&str>) -> Category {
        Category {
            id: category_id.to_string(),
            taxonomy_id: taxonomy_id.to_string(),
            parent_id: parent_id.map(str::to_string),
            name: category_id.to_string(),
            key: category_id.to_string(),
            color: "#000000".to_string(),
            description: None,
            sort_order: 0,
            created_at: ts(),
            updated_at: ts(),
            icon: None,
        }
    }

    fn taxonomy_with_categories(id: &str, categories: Vec<Category>) -> TaxonomyWithCategories {
        TaxonomyWithCategories {
            taxonomy: Taxonomy {
                id: id.to_string(),
                name: id.to_string(),
                color: "#000000".to_string(),
                description: None,
                is_system: true,
                is_single_select: true,
                sort_order: 0,
                created_at: ts(),
                updated_at: ts(),
                scope: "activity".to_string(),
            },
            categories,
        }
    }

    fn budget_group(id: &str, key: &str) -> BudgetGroup {
        BudgetGroup {
            id: id.to_string(),
            name: key.to_string(),
            key: key.to_string(),
            color: None,
            icon: None,
            sort_order: 0,
            is_system: true,
            exclude_from_budget: key == OTHER_GROUP_KEY,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn budget_group_assignment(group_id: &str, category_id: &str) -> BudgetGroupAssignment {
        BudgetGroupAssignment {
            id: format!("{group_id}-{category_id}"),
            group_id: group_id.to_string(),
            taxonomy_id: SPENDING_TAXONOMY.to_string(),
            category_id: category_id.to_string(),
            created_at: ts(),
            updated_at: ts(),
        }
    }

    #[derive(Default)]
    struct MockEligibility {
        account_provenance: HashMap<String, ProvenanceInput>,
        row_provenance: Mutex<HashMap<String, ProvenanceInput>>,
        account_error: Mutex<bool>,
        row_error: Mutex<bool>,
        account_calls: Mutex<Vec<(Vec<String>, NaiveDate)>>,
        row_calls: Mutex<Vec<(String, Vec<(String, NaiveDate)>)>>,
    }

    #[derive(Clone)]
    struct ProvenanceInput {
        role: ProvenanceRole,
        coverage_start: Option<NaiveDate>,
        manual_current_approved: bool,
    }

    impl ProvenanceInput {
        fn eligible(&self, row_date: NaiveDate) -> bool {
            let coverage_start = self.coverage_start.map(|date| date.to_string());
            self.role.eligible_for_aggregate(
                &row_date.to_string(),
                coverage_start.as_deref(),
                self.manual_current_approved,
            )
        }
    }

    #[async_trait]
    impl AggregateEligibilityServiceTrait for MockEligibility {
        async fn eligible_account_ids(
            &self,
            requested_account_ids: &[String],
            as_of: NaiveDate,
        ) -> CoreResult<HashSet<String>> {
            self.account_calls
                .lock()
                .unwrap()
                .push((requested_account_ids.to_vec(), as_of));
            if *self.account_error.lock().unwrap() {
                return Err(Error::Validation(ValidationError::InvalidInput(
                    "account policy failed".to_string(),
                )));
            }
            Ok(requested_account_ids
                .iter()
                .filter(|id| {
                    self.account_provenance
                        .get(*id)
                        .is_some_and(|record| record.eligible(as_of))
                })
                .cloned()
                .collect())
        }

        async fn eligible_entity_ids_for_rows(
            &self,
            entity_type: &str,
            rows: &[(String, NaiveDate)],
        ) -> CoreResult<HashSet<String>> {
            self.row_calls
                .lock()
                .unwrap()
                .push((entity_type.to_string(), rows.to_vec()));
            if *self.row_error.lock().unwrap() {
                return Err(Error::Validation(ValidationError::InvalidInput(
                    "row policy failed".to_string(),
                )));
            }
            Ok(rows
                .iter()
                .filter(|(id, row_date)| {
                    self.row_provenance
                        .lock()
                        .unwrap()
                        .get(id)
                        .is_some_and(|record| record.eligible(*row_date))
                })
                .map(|(id, _)| id.clone())
                .collect())
        }
    }

    struct MockSettingsRepo {
        enabled: bool,
        account_ids: Vec<String>,
    }

    #[async_trait]
    impl SpendingSettingsRepositoryTrait for MockSettingsRepo {
        async fn get_setting(&self, key: &str) -> Result<Option<String>> {
            Ok(match key {
                SETTING_KEY_ENABLED => Some(self.enabled.to_string()),
                SETTING_KEY_ACCOUNT_IDS => Some(serde_json::to_string(&self.account_ids)?),
                _ => None,
            })
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
        async fn create(&self, _: NewAccount) -> CoreResult<Account> {
            unimplemented!()
        }

        async fn update(&self, _: AccountUpdate) -> CoreResult<Account> {
            unimplemented!()
        }

        async fn delete(&self, _: &str) -> CoreResult<usize> {
            unimplemented!()
        }

        fn get_by_id(&self, _: &str) -> CoreResult<Account> {
            unimplemented!()
        }

        fn list(
            &self,
            is_active_filter: Option<bool>,
            is_archived_filter: Option<bool>,
            account_ids: Option<&[String]>,
        ) -> CoreResult<Vec<Account>> {
            Ok(self
                .accounts
                .iter()
                .filter(|account| {
                    is_active_filter
                        .map(|is_active| account.is_active == is_active)
                        .unwrap_or(true)
                        && is_archived_filter
                            .map(|is_archived| account.is_archived == is_archived)
                            .unwrap_or(true)
                        && account_ids
                            .map(|ids| ids.iter().any(|id| id == &account.id))
                            .unwrap_or(true)
                })
                .cloned()
                .collect())
        }

        fn get_accounting_settings_by_account_ids(
            &self,
            account_ids: &[String],
        ) -> CoreResult<HashMap<String, AccountAccountingSettings>> {
            Ok(account_ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        AccountAccountingSettings::default_for_account(id.clone()),
                    )
                })
                .collect())
        }
    }

    struct MockActivityRepo {
        activities: Vec<Activity>,
        date_range_calls: Mutex<Vec<Vec<String>>>,
    }

    impl MockActivityRepo {
        fn new(activities: Vec<Activity>) -> Self {
            Self {
                activities,
                date_range_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ActivityRepositoryTrait for MockActivityRepo {
        fn get_activity(&self, _: &str) -> CoreResult<Activity> {
            unimplemented!()
        }

        fn find_transfer_counterpart(&self, _: &str, _: &str) -> CoreResult<Option<Activity>> {
            unimplemented!()
        }

        fn get_activities(&self) -> CoreResult<Vec<Activity>> {
            Ok(self.activities.clone())
        }

        fn get_activities_by_account_id(&self, account_id: &str) -> CoreResult<Vec<Activity>> {
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
        ) -> CoreResult<Vec<Activity>> {
            Ok(self
                .activities
                .iter()
                .filter(|activity| account_ids.iter().any(|id| id == &activity.account_id))
                .cloned()
                .collect())
        }

        fn get_activities_by_account_ids_in_date_range(
            &self,
            account_ids: &[String],
            start_utc: DateTime<Utc>,
            end_utc: DateTime<Utc>,
        ) -> CoreResult<Vec<Activity>> {
            self.date_range_calls
                .lock()
                .unwrap()
                .push(account_ids.to_vec());
            Ok(self
                .activities
                .iter()
                .filter(|activity| {
                    account_ids.iter().any(|id| id == &activity.account_id)
                        && activity.activity_date >= start_utc
                        && activity.activity_date <= end_utc
                })
                .cloned()
                .collect())
        }

        fn get_trading_activities(&self) -> CoreResult<Vec<Activity>> {
            unimplemented!()
        }

        fn get_income_activities(&self) -> CoreResult<Vec<Activity>> {
            unimplemented!()
        }

        fn get_contribution_activities(
            &self,
            _: &[String],
            _: DateTime<Utc>,
            _: DateTime<Utc>,
        ) -> CoreResult<Vec<ContributionActivity>> {
            unimplemented!()
        }

        #[allow(clippy::too_many_arguments)]
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
        ) -> CoreResult<ActivitySearchResponse> {
            unimplemented!()
        }

        async fn create_activity(&self, _: NewActivity) -> CoreResult<Activity> {
            unimplemented!()
        }

        async fn update_activity(&self, _: ActivityUpdate) -> CoreResult<Activity> {
            unimplemented!()
        }

        async fn delete_activity(&self, _: String) -> CoreResult<Activity> {
            unimplemented!()
        }

        async fn link_transfer_activities(
            &self,
            _: String,
            _: String,
        ) -> CoreResult<(Activity, Activity)> {
            unimplemented!()
        }

        async fn unlink_transfer_activities(
            &self,
            _: String,
            _: String,
        ) -> CoreResult<(Activity, Activity)> {
            unimplemented!()
        }

        async fn bulk_mutate_activities(
            &self,
            _: Vec<NewActivity>,
            _: Vec<ActivityUpdate>,
            _: Vec<String>,
        ) -> CoreResult<ActivityBulkMutationResult> {
            unimplemented!()
        }

        async fn create_activities(&self, _: Vec<NewActivity>) -> CoreResult<usize> {
            unimplemented!()
        }

        fn get_first_activity_date(
            &self,
            _: Option<&[String]>,
        ) -> CoreResult<Option<DateTime<Utc>>> {
            unimplemented!()
        }

        fn get_import_mapping(&self, _: &str, _: &str) -> CoreResult<Option<ImportMapping>> {
            unimplemented!()
        }

        async fn save_import_mapping(&self, _: &ImportMapping) -> CoreResult<()> {
            unimplemented!()
        }

        async fn link_account_template(&self, _: &str, _: &str, _: &str) -> CoreResult<()> {
            unimplemented!()
        }

        fn list_import_templates(&self) -> CoreResult<Vec<ImportTemplate>> {
            unimplemented!()
        }

        fn get_import_template(&self, _: &str) -> CoreResult<Option<ImportTemplate>> {
            unimplemented!()
        }

        async fn save_import_template(&self, _: &ImportTemplate) -> CoreResult<()> {
            unimplemented!()
        }

        async fn delete_import_template(&self, _: &str) -> CoreResult<()> {
            unimplemented!()
        }

        fn get_broker_sync_profile(&self, _: &str, _: &str) -> CoreResult<Option<ImportTemplate>> {
            unimplemented!()
        }

        async fn save_broker_sync_profile(&self, _: &ImportTemplate) -> CoreResult<()> {
            unimplemented!()
        }

        async fn link_broker_sync_profile(&self, _: &str, _: &str, _: &str) -> CoreResult<()> {
            unimplemented!()
        }

        fn calculate_average_cost(&self, _: &str, _: &str) -> CoreResult<Decimal> {
            unimplemented!()
        }

        fn get_income_activities_data(&self, _: Option<&[String]>) -> CoreResult<Vec<IncomeData>> {
            unimplemented!()
        }

        fn get_first_activity_date_overall(&self) -> CoreResult<DateTime<Utc>> {
            unimplemented!()
        }

        fn get_activity_bounds_for_assets(
            &self,
            _: &[String],
        ) -> CoreResult<HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>> {
            unimplemented!()
        }

        fn get_holdings_snapshot_bounds_for_assets(
            &self,
            _: &[String],
        ) -> CoreResult<HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>> {
            unimplemented!()
        }

        fn check_existing_duplicates(&self, _: &[String]) -> CoreResult<HashMap<String, String>> {
            unimplemented!()
        }

        async fn bulk_upsert(&self, _: Vec<ActivityUpsert>) -> CoreResult<BulkUpsertResult> {
            unimplemented!()
        }

        async fn reassign_asset(&self, _: &str, _: &str) -> CoreResult<u32> {
            unimplemented!()
        }

        async fn get_activity_accounts_and_currencies_by_asset_id(
            &self,
            _: &str,
        ) -> CoreResult<(Vec<String>, Vec<String>)> {
            unimplemented!()
        }
    }

    #[derive(Default)]
    struct MockAssignmentRepo {
        assignments: Vec<ActivityTaxonomyAssignment>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ActivityTaxonomyAssignmentRepositoryTrait for MockAssignmentRepo {
        async fn list_for_activity(
            &self,
            activity_id: &str,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            Ok(self
                .assignments
                .iter()
                .filter(|assignment| assignment.activity_id == activity_id)
                .cloned()
                .collect())
        }

        async fn list_for_activities(
            &self,
            activity_ids: &[String],
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            self.calls.lock().unwrap().push(activity_ids.to_vec());
            Ok(self
                .assignments
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
        ) -> Result<ActivityTaxonomyAssignment> {
            unimplemented!()
        }

        async fn assign_many_single_select(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn assign_many_single_select_clearing_splits(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn assign_rule_many_single_select(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
            _: bool,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
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
        splits: Vec<ActivitySplit>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ActivitySplitRepositoryTrait for MockSplitRepo {
        async fn list_for_activity(&self, activity_id: &str) -> Result<Vec<ActivitySplit>> {
            Ok(self
                .splits
                .iter()
                .filter(|split| split.activity_id == activity_id)
                .cloned()
                .collect())
        }

        async fn list_for_activities(&self, activity_ids: &[String]) -> Result<Vec<ActivitySplit>> {
            self.calls.lock().unwrap().push(activity_ids.to_vec());
            Ok(self
                .splits
                .iter()
                .filter(|split| activity_ids.iter().any(|id| id == &split.activity_id))
                .cloned()
                .collect())
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

    struct MockTaxonomyService {
        taxonomies: HashMap<String, TaxonomyWithCategories>,
    }

    #[async_trait]
    impl TaxonomyServiceTrait for MockTaxonomyService {
        fn get_taxonomies(&self) -> CoreResult<Vec<Taxonomy>> {
            Ok(self
                .taxonomies
                .values()
                .map(|taxonomy| taxonomy.taxonomy.clone())
                .collect())
        }

        fn get_taxonomy(&self, id: &str) -> CoreResult<Option<TaxonomyWithCategories>> {
            Ok(self.taxonomies.get(id).cloned())
        }

        fn get_taxonomies_with_categories(&self) -> CoreResult<Vec<TaxonomyWithCategories>> {
            Ok(self.taxonomies.values().cloned().collect())
        }

        async fn create_taxonomy(&self, _: NewTaxonomy) -> CoreResult<Taxonomy> {
            unimplemented!()
        }

        async fn update_taxonomy(&self, _: Taxonomy) -> CoreResult<Taxonomy> {
            unimplemented!()
        }

        async fn delete_taxonomy(&self, _: &str) -> CoreResult<usize> {
            unimplemented!()
        }

        async fn create_category(&self, _: NewCategory) -> CoreResult<Category> {
            unimplemented!()
        }

        async fn update_category(&self, _: Category) -> CoreResult<Category> {
            unimplemented!()
        }

        async fn delete_category(&self, _: &str, _: &str) -> CoreResult<usize> {
            unimplemented!()
        }

        async fn move_category(
            &self,
            _: &str,
            _: &str,
            _: Option<String>,
            _: i32,
        ) -> CoreResult<Category> {
            unimplemented!()
        }

        async fn import_taxonomy_json(&self, _: &str) -> CoreResult<Taxonomy> {
            unimplemented!()
        }

        fn export_taxonomy_json(&self, _: &str) -> CoreResult<String> {
            unimplemented!()
        }

        fn get_asset_assignments(&self, _: &str) -> CoreResult<Vec<AssetTaxonomyAssignment>> {
            unimplemented!()
        }

        fn get_category_assignments(
            &self,
            _: &str,
            _: &str,
        ) -> CoreResult<Vec<AssetTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn assign_asset_to_category(
            &self,
            _: NewAssetTaxonomyAssignment,
        ) -> CoreResult<AssetTaxonomyAssignment> {
            unimplemented!()
        }

        async fn replace_asset_taxonomy_assignments(
            &self,
            _: &str,
            _: &str,
            _: Vec<NewAssetTaxonomyAssignment>,
        ) -> CoreResult<Vec<AssetTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn remove_asset_assignment(&self, _: &str) -> CoreResult<usize> {
            unimplemented!()
        }
    }

    struct MockFxService;

    #[async_trait]
    impl wealthfolio_core::fx::FxServiceTrait for MockFxService {
        fn initialize(&self) -> CoreResult<()> {
            Ok(())
        }

        fn get_historical_rates(&self, _: &str, _: &str, _: i64) -> CoreResult<Vec<ExchangeRate>> {
            unimplemented!()
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
            unimplemented!()
        }

        async fn add_exchange_rate(&self, _: NewExchangeRate) -> CoreResult<ExchangeRate> {
            unimplemented!()
        }

        async fn update_exchange_rate(
            &self,
            _: &str,
            _: &str,
            _: Decimal,
        ) -> CoreResult<ExchangeRate> {
            unimplemented!()
        }

        async fn delete_exchange_rate(&self, _: &str) -> CoreResult<()> {
            unimplemented!()
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

    #[derive(Default)]
    struct MockBudgetRepo {
        groups: Vec<BudgetGroup>,
        group_assignments: Vec<BudgetGroupAssignment>,
        targets: Vec<BudgetTarget>,
        rollover_settings: Vec<BudgetRolloverSetting>,
    }

    #[async_trait]
    impl BudgetRepositoryTrait for MockBudgetRepo {
        async fn list_groups(&self) -> Result<Vec<BudgetGroup>> {
            Ok(self.groups.clone())
        }

        async fn create_group(&self, _: NewBudgetGroup) -> Result<BudgetGroup> {
            unimplemented!()
        }

        async fn update_group(&self, _: &str, _: UpdateBudgetGroup) -> Result<BudgetGroup> {
            unimplemented!()
        }

        async fn delete_group(&self, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn delete_group_and_reassign(
            &self,
            _: &str,
            _: &str,
            _: Vec<NewBudgetGroupAssignment>,
        ) -> Result<()> {
            unimplemented!()
        }

        async fn upsert_system_groups(
            &self,
            groups: Vec<NewBudgetGroup>,
        ) -> Result<Vec<BudgetGroup>> {
            Ok(groups
                .into_iter()
                .map(|group| BudgetGroup {
                    id: group.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                    name: group.name,
                    key: group.key.unwrap_or_default(),
                    color: group.color,
                    icon: group.icon,
                    sort_order: group.sort_order.unwrap_or_default(),
                    is_system: group.is_system,
                    exclude_from_budget: group.exclude_from_budget,
                    created_at: ts(),
                    updated_at: ts(),
                })
                .collect())
        }

        async fn list_group_assignments(&self) -> Result<Vec<BudgetGroupAssignment>> {
            Ok(self.group_assignments.clone())
        }

        async fn upsert_group_assignment(
            &self,
            _: NewBudgetGroupAssignment,
        ) -> Result<BudgetGroupAssignment> {
            unimplemented!()
        }

        async fn upsert_group_assignments(
            &self,
            _: Vec<NewBudgetGroupAssignment>,
        ) -> Result<Vec<BudgetGroupAssignment>> {
            unimplemented!()
        }

        async fn upsert_system_group_assignments(
            &self,
            _: Vec<NewBudgetGroupAssignment>,
        ) -> Result<Vec<BudgetGroupAssignment>> {
            Ok(Vec::new())
        }

        async fn list_targets(&self) -> Result<Vec<BudgetTarget>> {
            Ok(self.targets.clone())
        }

        async fn upsert_target(&self, _: NewBudgetTarget) -> Result<BudgetTarget> {
            unimplemented!()
        }

        async fn delete_target(&self, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn list_rollover_settings(&self) -> Result<Vec<BudgetRolloverSetting>> {
            Ok(self.rollover_settings.clone())
        }

        async fn upsert_rollover_setting(
            &self,
            _: NewBudgetRolloverSetting,
        ) -> Result<BudgetRolloverSetting> {
            unimplemented!()
        }

        async fn delete_rollover_setting(&self, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn disable_category_rollovers(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<Vec<BudgetRolloverSetting>> {
            unimplemented!()
        }

        async fn copy_period_targets(
            &self,
            _: &str,
            _: &str,
            _: bool,
        ) -> Result<Vec<BudgetTarget>> {
            unimplemented!()
        }
    }

    struct BudgetFixture {
        service: BudgetService,
        eligibility: Arc<MockEligibility>,
        activity_repo: Arc<MockActivityRepo>,
        assignment_repo: Arc<MockAssignmentRepo>,
        split_repo: Arc<MockSplitRepo>,
    }

    fn budget_fixture(
        activities: Vec<Activity>,
        assignments: Vec<ActivityTaxonomyAssignment>,
        targets: Vec<BudgetTarget>,
        allowed_accounts: &[&str],
        allowed_rows: &[&str],
    ) -> BudgetFixture {
        let groups = vec![
            budget_group(BUDGET_GROUP_NEEDS_ID, "needs"),
            budget_group(BUDGET_GROUP_WANTS_ID, "wants"),
            budget_group(BUDGET_GROUP_SAVINGS_ID, "savings"),
            budget_group(BUDGET_GROUP_GIVING_ID, "giving"),
            budget_group(BUDGET_GROUP_PERSONAL_ID, "personal"),
            budget_group(BUDGET_GROUP_OTHER_ID, OTHER_GROUP_KEY),
        ];
        let budget_repo = Arc::new(MockBudgetRepo {
            groups,
            group_assignments: vec![budget_group_assignment(
                BUDGET_GROUP_NEEDS_ID,
                "cat_groceries",
            )],
            targets,
            rollover_settings: Vec::new(),
        });
        let activity_repo = Arc::new(MockActivityRepo::new(activities));
        let assignment_repo = Arc::new(MockAssignmentRepo {
            assignments,
            calls: Mutex::new(Vec::new()),
        });
        let split_repo = Arc::new(MockSplitRepo::default());
        let live = || ProvenanceInput {
            role: ProvenanceRole::LiveCanonical,
            coverage_start: None,
            manual_current_approved: false,
        };
        let eligibility = Arc::new(MockEligibility {
            account_provenance: allowed_accounts
                .iter()
                .map(|id| (id.to_string(), live()))
                .collect(),
            row_provenance: Mutex::new(
                allowed_rows
                    .iter()
                    .map(|id| (id.to_string(), live()))
                    .collect(),
            ),
            ..Default::default()
        });
        let mut taxonomies = HashMap::new();
        taxonomies.insert(
            SPENDING_TAXONOMY.to_string(),
            taxonomy_with_categories(
                SPENDING_TAXONOMY,
                vec![
                    category(SPENDING_TAXONOMY, "cat_groceries", None),
                    category(SPENDING_TAXONOMY, "cat_unassigned", None),
                ],
            ),
        );
        taxonomies.insert(
            INCOME_TAXONOMY.to_string(),
            taxonomy_with_categories(INCOME_TAXONOMY, Vec::new()),
        );
        let service = BudgetService::new(
            budget_repo,
            activity_repo.clone(),
            Arc::new(MockAccountRepo {
                accounts: vec![
                    account("eligible-card", account_types::CREDIT_CARD),
                    account("denied-card", account_types::CREDIT_CARD),
                ],
            }),
            assignment_repo.clone(),
            split_repo.clone(),
            Arc::new(SpendingSettingsService::new(Arc::new(MockSettingsRepo {
                enabled: true,
                account_ids: vec!["eligible-card".to_string(), "denied-card".to_string()],
            }))),
            Arc::new(MockTaxonomyService { taxonomies }),
            Arc::new(MockFxService),
            eligibility.clone(),
        );

        BudgetFixture {
            service,
            eligibility,
            activity_repo,
            assignment_repo,
            split_repo,
        }
    }

    #[tokio::test]
    async fn budget_actuals_direct_provenance_roles_and_coverage_dates() {
        let ids = [
            "live-canonical",
            "historical-context",
            "opening-before",
            "opening-at-start",
            "opening-after",
            "missing-provenance",
            "unknown-role",
            "contextual-reference",
            "manual-approved",
            "manual-unapproved",
        ];
        let mut activities = ids
            .iter()
            .map(|id| activity(id, "eligible-card", "WITHDRAWAL", 10))
            .collect::<Vec<_>>();
        activities[2].activity_date = Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        activities[3].activity_date = Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap();
        activities[4].activity_date = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        let fixture = budget_fixture(
            activities,
            ids.iter()
                .map(|id| assignment(id, "cat_groceries"))
                .collect(),
            Vec::new(),
            &["eligible-card"],
            &ids,
        );
        let coverage = Some(NaiveDate::from_ymd_opt(2026, 5, 10).unwrap());
        {
            let provenance = &mut fixture.eligibility.row_provenance.lock().unwrap();
            provenance.clear();
            provenance.insert(
                "live-canonical".into(),
                ProvenanceInput {
                    role: ProvenanceRole::LiveCanonical,
                    coverage_start: None,
                    manual_current_approved: false,
                },
            );
            provenance.insert(
                "historical-context".into(),
                ProvenanceInput {
                    role: ProvenanceRole::HistoricalContext,
                    coverage_start: None,
                    manual_current_approved: false,
                },
            );
            for id in ["opening-before", "opening-at-start", "opening-after"] {
                provenance.insert(
                    id.into(),
                    ProvenanceInput {
                        role: ProvenanceRole::OpeningBaseline,
                        coverage_start: coverage,
                        manual_current_approved: false,
                    },
                );
            }

            provenance.insert(
                "contextual-reference".into(),
                ProvenanceInput {
                    role: ProvenanceRole::ContextualReference,
                    coverage_start: None,
                    manual_current_approved: false,
                },
            );
            provenance.insert(
                "manual-approved".into(),
                ProvenanceInput {
                    role: ProvenanceRole::ManualCurrent,
                    coverage_start: None,
                    manual_current_approved: true,
                },
            );
            provenance.insert(
                "manual-unapproved".into(),
                ProvenanceInput {
                    role: ProvenanceRole::ManualCurrent,
                    coverage_start: None,
                    manual_current_approved: false,
                },
            );
        }

        let meta = category_meta(&[category(SPENDING_TAXONOMY, "cat_groceries", None)]);
        let actuals = fixture
            .service
            .actuals_by_period(
                "2026-04-25",
                "2026-04-25",
                &meta,
                &HashMap::new(),
                "USD",
                "UTC",
                &BudgetCycleSettings::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            actuals["2026-04-25"][&(SPENDING_TAXONOMY.into(), "cat_groceries".into())],
            Decimal::new(40, 0)
        );
        assert_eq!(
            fixture.eligibility.row_calls.lock().unwrap().as_slice(),
            &[(
                wealthfolio_core::accounts::ENTITY_ACTIVITY.to_string(),
                vec![
                    (
                        "live-canonical".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "historical-context".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "opening-before".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 9).unwrap()
                    ),
                    (
                        "opening-at-start".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "opening-after".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 11).unwrap()
                    ),
                    (
                        "missing-provenance".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "unknown-role".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "contextual-reference".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "manual-approved".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "manual-unapproved".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                ]
            )]
        );
    }

    #[tokio::test]
    async fn budget_actuals_filter_eligible_rows_before_assignment_and_split_reads() {
        let activities = vec![
            activity("allowed-spend", "eligible-card", "WITHDRAWAL", 40),
            activity("denied-row", "eligible-card", "WITHDRAWAL", 70),
            activity("denied-account", "denied-card", "WITHDRAWAL", 90),
        ];
        let fixture = budget_fixture(
            activities,
            vec![
                assignment("allowed-spend", "cat_groceries"),
                assignment("denied-row", "cat_groceries"),
                assignment("denied-account", "cat_groceries"),
            ],
            Vec::new(),
            &["eligible-card"],
            &["allowed-spend", "denied-account"],
        );
        let spending_meta = category_meta(&[category(SPENDING_TAXONOMY, "cat_groceries", None)]);
        let actuals = fixture
            .service
            .actuals_by_period(
                "2026-04-25",
                "2026-04-25",
                &spending_meta,
                &HashMap::new(),
                "USD",
                "UTC",
                &BudgetCycleSettings::default(),
            )
            .await
            .unwrap();

        assert_eq!(
            fixture
                .activity_repo
                .date_range_calls
                .lock()
                .unwrap()
                .as_slice(),
            &[vec!["eligible-card".to_string()]]
        );
        assert_eq!(
            fixture.eligibility.account_calls.lock().unwrap().as_slice(),
            &[(
                vec!["eligible-card".to_string(), "denied-card".to_string()],
                NaiveDate::from_ymd_opt(2026, 5, 24).unwrap()
            )]
        );
        assert_eq!(
            fixture.eligibility.row_calls.lock().unwrap().as_slice(),
            &[(
                wealthfolio_core::accounts::ENTITY_ACTIVITY.to_string(),
                vec![
                    (
                        "allowed-spend".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                    (
                        "denied-row".to_string(),
                        NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                    ),
                ]
            )]
        );
        assert_eq!(
            fixture.assignment_repo.calls.lock().unwrap().as_slice(),
            &[vec!["allowed-spend".to_string()]]
        );
        assert_eq!(
            fixture.split_repo.calls.lock().unwrap().as_slice(),
            &[vec!["allowed-spend".to_string()]]
        );
        assert_eq!(
            actuals
                .get("2026-04-25")
                .and_then(|month| month
                    .get(&(SPENDING_TAXONOMY.to_string(), "cat_groceries".to_string()))),
            Some(&Decimal::new(40, 0))
        );
    }

    #[tokio::test]
    async fn budget_actuals_fetch_and_bucket_local_day_25_boundaries() {
        let row_ids = ["before", "start", "day-24", "day-25"];
        let mut activities = row_ids
            .iter()
            .map(|id| activity(id, "eligible-card", "WITHDRAWAL", 10))
            .collect::<Vec<_>>();
        activities[0].activity_date = Utc.with_ymd_and_hms(2026, 4, 25, 3, 59, 59).unwrap();
        activities[1].activity_date = Utc.with_ymd_and_hms(2026, 4, 25, 4, 0, 0).unwrap();
        activities[2].activity_date = Utc.with_ymd_and_hms(2026, 5, 25, 3, 59, 59).unwrap();
        activities[3].activity_date = Utc.with_ymd_and_hms(2026, 5, 25, 4, 0, 0).unwrap();
        let fixture = budget_fixture(
            activities,
            row_ids
                .iter()
                .map(|id| assignment(id, "cat_groceries"))
                .collect(),
            Vec::new(),
            &["eligible-card"],
            &row_ids,
        );
        let spending_meta = category_meta(&[category(SPENDING_TAXONOMY, "cat_groceries", None)]);

        let actuals = fixture
            .service
            .actuals_by_period(
                "2026-03-25",
                "2026-04-25",
                &spending_meta,
                &HashMap::new(),
                "USD",
                "America/Toronto",
                &BudgetCycleSettings::default(),
            )
            .await
            .unwrap();

        let key = (SPENDING_TAXONOMY.to_string(), "cat_groceries".to_string());
        assert_eq!(actuals["2026-03-25"][&key], Decimal::new(10, 0));
        assert_eq!(actuals["2026-04-25"][&key], Decimal::new(20, 0));
    }

    #[tokio::test]
    async fn budget_actuals_bucket_weekly_cycle_boundaries() {
        let row_ids = ["week-start", "week-end", "next-week"];
        let mut activities = row_ids
            .iter()
            .map(|id| activity(id, "eligible-card", "WITHDRAWAL", 10))
            .collect::<Vec<_>>();
        activities[0].activity_date = Utc.with_ymd_and_hms(2026, 1, 7, 0, 0, 0).unwrap();
        activities[1].activity_date = Utc.with_ymd_and_hms(2026, 1, 13, 23, 59, 59).unwrap();
        activities[2].activity_date = Utc.with_ymd_and_hms(2026, 1, 14, 0, 0, 0).unwrap();
        let fixture = budget_fixture(
            activities,
            row_ids
                .iter()
                .map(|id| assignment(id, "cat_groceries"))
                .collect(),
            Vec::new(),
            &["eligible-card"],
            &row_ids,
        );
        let spending_meta = category_meta(&[category(SPENDING_TAXONOMY, "cat_groceries", None)]);
        let cycle = BudgetCycleSettings {
            cadence: BudgetCadence::Weekly,
            anchor_date: "2026-01-07".to_string(),
        };

        let actuals = fixture
            .service
            .actuals_by_period(
                "2026-01-07",
                "2026-01-14",
                &spending_meta,
                &HashMap::new(),
                "USD",
                "UTC",
                &cycle,
            )
            .await
            .unwrap();

        let key = (SPENDING_TAXONOMY.to_string(), "cat_groceries".to_string());
        assert_eq!(actuals["2026-01-07"][&key], Decimal::new(20, 0));
        assert_eq!(actuals["2026-01-14"][&key], Decimal::new(10, 0));
    }

    #[tokio::test]
    async fn budget_actuals_transfer_pair_is_all_or_none_before_enrichment() {
        let mut transfer_in = activity(
            "transfer-in",
            "eligible-card",
            ACTIVITY_TYPE_TRANSFER_IN,
            50,
        );
        transfer_in.source_group_id = Some("transfer-group".to_string());
        let mut transfer_out = activity(
            "transfer-out",
            "eligible-card",
            ACTIVITY_TYPE_TRANSFER_OUT,
            50,
        );
        transfer_out.source_group_id = Some("transfer-group".to_string());
        let fixture = budget_fixture(
            vec![transfer_in, transfer_out],
            vec![
                assignment("transfer-in", "cat_groceries"),
                assignment("transfer-out", "cat_groceries"),
            ],
            Vec::new(),
            &["eligible-card"],
            &["transfer-in"],
        );
        let spending_meta = category_meta(&[category(SPENDING_TAXONOMY, "cat_groceries", None)]);
        let actuals = fixture
            .service
            .actuals_by_period(
                "2026-04-25",
                "2026-04-25",
                &spending_meta,
                &HashMap::new(),
                "USD",
                "UTC",
                &BudgetCycleSettings::default(),
            )
            .await
            .unwrap();

        assert_eq!(
            fixture.eligibility.row_calls.lock().unwrap()[0].1,
            vec![
                (
                    "transfer-in".to_string(),
                    NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                ),
                (
                    "transfer-out".to_string(),
                    NaiveDate::from_ymd_opt(2026, 5, 10).unwrap()
                ),
            ]
        );
        assert_eq!(
            fixture.assignment_repo.calls.lock().unwrap().as_slice(),
            &[Vec::<String>::new()]
        );
        assert_eq!(
            fixture.split_repo.calls.lock().unwrap().as_slice(),
            &[Vec::<String>::new()]
        );
        assert!(actuals.is_empty());
    }

    #[tokio::test]
    async fn budget_actuals_propagate_row_policy_error_before_enrichment() {
        let fixture = budget_fixture(
            vec![activity("row", "eligible-card", "WITHDRAWAL", 40)],
            vec![assignment("row", "cat_groceries")],
            Vec::new(),
            &["eligible-card"],
            &["row"],
        );
        *fixture.eligibility.row_error.lock().unwrap() = true;
        let spending_meta = category_meta(&[category(SPENDING_TAXONOMY, "cat_groceries", None)]);
        let err = fixture
            .service
            .actuals_by_period(
                "2026-04-25",
                "2026-04-25",
                &spending_meta,
                &HashMap::new(),
                "USD",
                "UTC",
                &BudgetCycleSettings::default(),
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("row policy failed"));
        assert!(fixture.assignment_repo.calls.lock().unwrap().is_empty());
        assert!(fixture.split_repo.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn budget_get_month_view_denies_non_aggregate_provenance_rows_and_records_policy_call() {
        for row_id in [
            "missing-provenance",
            "unknown-role",
            "historical-context",
            "contextual-reference",
        ] {
            let fixture = budget_fixture(
                vec![activity(row_id, "eligible-card", "WITHDRAWAL", 40)],
                vec![assignment(row_id, "cat_groceries")],
                Vec::new(),
                &["eligible-card"],
                &[],
            );
            let snapshot = fixture
                .service
                .get(Some("2026-04-25".to_string()), "USD", "UTC")
                .await
                .unwrap();
            assert_eq!(snapshot.computed.totals.spending_actual, 0.0, "{row_id}");
            assert_eq!(fixture.eligibility.row_calls.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn budget_get_month_view_propagates_account_policy_error_directly() {
        let fixture = budget_fixture(
            vec![activity("account-error", "eligible-card", "WITHDRAWAL", 40)],
            vec![assignment("account-error", "cat_groceries")],
            Vec::new(),
            &["eligible-card"],
            &["account-error"],
        );
        *fixture.eligibility.account_error.lock().unwrap() = true;
        let err = fixture
            .service
            .get(Some("2026-04-25".to_string()), "USD", "UTC")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("account policy failed"));
        assert!(fixture.eligibility.row_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn budget_get_month_view_uses_direct_policy_output_for_live_and_manual_roles() {
        for (row_id, allowed) in [
            ("live-canonical", true),
            ("manual-current-approved", true),
            ("manual-current-unapproved", false),
        ] {
            let allowed_rows = if allowed { vec![row_id] } else { Vec::new() };
            let fixture = budget_fixture(
                vec![activity(row_id, "eligible-card", "WITHDRAWAL", 40)],
                vec![assignment(row_id, "cat_groceries")],
                Vec::new(),
                &["eligible-card"],
                &allowed_rows,
            );
            let snapshot = fixture
                .service
                .get(Some("2026-04-25".to_string()), "USD", "UTC")
                .await
                .unwrap();
            assert_eq!(
                snapshot.computed.totals.spending_actual,
                if allowed { 40.0 } else { 0.0 },
                "{row_id}"
            );
            assert_eq!(fixture.eligibility.row_calls.lock().unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn budget_get_places_unassigned_top_level_categories_in_other() {
        let fixture = budget_fixture(Vec::new(), Vec::new(), Vec::new(), &[], &[]);
        let snapshot = fixture
            .service
            .get(Some(DEFAULT_PERIOD_KEY.to_string()), "USD", "UTC")
            .await
            .unwrap();

        let other = snapshot
            .computed
            .group_rows
            .iter()
            .find(|row| row.group.key == OTHER_GROUP_KEY)
            .expect("Other budget group");
        assert!(other
            .categories
            .iter()
            .any(|category| category.category_id == "cat_unassigned"));
        assert!(snapshot.computed.ungrouped_rows.is_empty());
    }

    #[tokio::test]
    async fn excluded_budget_group_does_not_contribute_to_budget_actuals() {
        let fixture = budget_fixture(
            vec![
                activity("included-spend", "eligible-card", "WITHDRAWAL", 25),
                activity("other-spend", "eligible-card", "WITHDRAWAL", 40),
            ],
            vec![
                assignment("included-spend", "cat_groceries"),
                assignment("other-spend", "cat_unassigned"),
            ],
            Vec::new(),
            &["eligible-card"],
            &["included-spend", "other-spend"],
        );
        let snapshot = fixture
            .service
            .get(Some("2026-04-25".to_string()), "USD", "UTC")
            .await
            .unwrap();

        let other = snapshot
            .computed
            .group_rows
            .iter()
            .find(|row| row.group.key == OTHER_GROUP_KEY)
            .expect("Other budget group");
        assert!(other.group.exclude_from_budget);
        assert_eq!(other.actual, 0.0);
        assert_eq!(
            snapshot.computed.totals.spending_actual, 25.0,
            "excluded Other spending must not affect budget actuals"
        );
    }

    #[tokio::test]
    async fn budget_get_default_planned_targets_do_not_read_actual_policy() {
        let fixture = budget_fixture(
            Vec::new(),
            Vec::new(),
            vec![target(
                DEFAULT_PERIOD_KEY,
                BudgetTargetType::Category,
                Some(SPENDING_TAXONOMY),
                Some("cat_groceries"),
                None,
                "125",
            )],
            &[],
            &[],
        );
        *fixture.eligibility.row_error.lock().unwrap() = true;
        let snapshot = fixture
            .service
            .get(Some(DEFAULT_PERIOD_KEY.to_string()), "USD", "UTC")
            .await
            .unwrap();

        assert_eq!(snapshot.computed.totals.spending_planned, 125.0);
        assert_eq!(snapshot.computed.totals.spending_actual, 0.0);
        assert!(fixture.eligibility.account_calls.lock().unwrap().is_empty());
        assert!(fixture.eligibility.row_calls.lock().unwrap().is_empty());
        assert!(fixture.assignment_repo.calls.lock().unwrap().is_empty());
        assert!(fixture.split_repo.calls.lock().unwrap().is_empty());
    }

    fn target(
        period_key: &str,
        target_type: BudgetTargetType,
        taxonomy_id: Option<&str>,
        category_id: Option<&str>,
        group_id: Option<&str>,
        amount: &str,
    ) -> BudgetTarget {
        BudgetTarget {
            id: format!(
                "{}-{}-{}",
                period_key,
                category_id.or(group_id).unwrap_or("target"),
                amount
            ),
            period_key: period_key.to_string(),
            target_type,
            taxonomy_id: taxonomy_id.map(str::to_string),
            category_id: category_id.map(str::to_string),
            group_id: group_id.map(str::to_string),
            amount: amount.to_string(),
            created_at: ts(),
            updated_at: ts(),
        }
    }

    fn rollover_setting(start_month: &str, starting_balance: &str) -> BudgetRolloverSetting {
        BudgetRolloverSetting {
            id: "rollover".to_string(),
            target_type: BudgetRolloverTargetType::Category,
            taxonomy_id: Some(SPENDING_TAXONOMY.to_string()),
            category_id: Some("cat_groceries".to_string()),
            group_id: None,
            enabled: true,
            start_month: start_month.to_string(),
            starting_balance: starting_balance.to_string(),
            created_at: ts(),
            updated_at: ts(),
        }
    }

    #[test]
    fn validates_budget_target_decimal_amount() {
        let target = NewBudgetTarget {
            id: None,
            period_key: DEFAULT_PERIOD_KEY.to_string(),
            target_type: BudgetTargetType::Category,
            taxonomy_id: Some(SPENDING_TAXONOMY.to_string()),
            category_id: Some("cat_groceries".to_string()),
            group_id: None,
            amount: "not-a-number".to_string(),
        };

        assert!(validate_budget_target(&target).is_err());
    }

    #[test]
    fn validates_rollover_starting_balance_decimal_amount() {
        let setting = NewBudgetRolloverSetting {
            id: None,
            target_type: BudgetRolloverTargetType::Category,
            taxonomy_id: Some(SPENDING_TAXONOMY.to_string()),
            category_id: Some("cat_groceries".to_string()),
            group_id: None,
            enabled: true,
            start_month: "2025-12-25".to_string(),
            starting_balance: "bad-balance".to_string(),
        };

        assert!(validate_rollover_setting(&setting).is_err());
    }

    #[test]
    fn target_index_uses_sparse_period_overrides_over_defaults() {
        let targets = vec![
            target(
                DEFAULT_PERIOD_KEY,
                BudgetTargetType::Category,
                Some(SPENDING_TAXONOMY),
                Some("cat_groceries"),
                None,
                "200",
            ),
            target(
                "2026-02-25",
                BudgetTargetType::Category,
                Some(SPENDING_TAXONOMY),
                Some("cat_groceries"),
                None,
                "300",
            ),
            target(
                DEFAULT_PERIOD_KEY,
                BudgetTargetType::GroupBuffer,
                None,
                None,
                Some(BUDGET_GROUP_NEEDS_ID),
                "500",
            ),
        ];
        let index = TargetIndex::new(&targets);

        assert_eq!(
            index.effective_category_decimal("2026-02-25", SPENDING_TAXONOMY, "cat_groceries",),
            Decimal::new(300, 0)
        );
        assert_eq!(
            index.effective_category_decimal("2026-03-25", SPENDING_TAXONOMY, "cat_groceries",),
            Decimal::new(200, 0)
        );
        assert_eq!(
            index.effective_category_decimal("2026-03-25", SPENDING_TAXONOMY, "cat_travel"),
            Decimal::ZERO
        );
        assert_eq!(
            index.effective_group_buffer_decimal("2026-03-25", BUDGET_GROUP_NEEDS_ID),
            Decimal::new(500, 0)
        );
        assert!(index.has_default_category(SPENDING_TAXONOMY, "cat_groceries"));
        assert!(index.has_month_category("2026-02-25", SPENDING_TAXONOMY, "cat_groceries"));
    }

    #[test]
    fn rollover_ignores_periods_before_future_start() {
        let setting = rollover_setting("2026-05-25", "25");

        let (rollover_in, rollover_out, remaining) = compute_rollover_for_period(
            &setting,
            "2026-04-25",
            &BudgetCycleSettings::default(),
            |_| Decimal::new(100, 0),
            |_| Decimal::new(40, 0),
        )
        .unwrap();

        assert_eq!(rollover_in, Decimal::ZERO);
        assert_eq!(rollover_out, Decimal::ZERO);
        assert_eq!(remaining, Decimal::new(60, 0));
    }

    #[test]
    fn rollover_recomputes_multi_year_cycle_chain() {
        let setting = rollover_setting("2024-12-25", "10");

        let (rollover_in, rollover_out, remaining) = compute_rollover_for_period(
            &setting,
            "2026-04-25",
            &BudgetCycleSettings::default(),
            |_| Decimal::new(100, 0),
            |period| match period {
                "2024-12-25" => Decimal::new(25, 0),
                "2026-04-25" => Decimal::new(40, 0),
                _ => Decimal::ZERO,
            },
        )
        .unwrap();

        assert_eq!(rollover_in, Decimal::new(1585, 0));
        assert_eq!(rollover_out, Decimal::new(1645, 0));
        assert_eq!(remaining, Decimal::new(1645, 0));
    }

    #[test]
    fn missing_target_with_spending_creates_negative_remaining() {
        let setting = rollover_setting("2026-04-25", "0");

        let (rollover_in, rollover_out, remaining) = compute_rollover_for_period(
            &setting,
            "2026-04-25",
            &BudgetCycleSettings::default(),
            |_| Decimal::ZERO,
            |_| Decimal::new(30, 0),
        )
        .unwrap();

        assert_eq!(rollover_in, Decimal::ZERO);
        assert_eq!(rollover_out, Decimal::new(-30, 0));
        assert_eq!(remaining, Decimal::new(-30, 0));
    }

    #[test]
    fn refund_period_increases_remaining() {
        let setting = rollover_setting("2026-04-25", "0");

        let (rollover_in, rollover_out, remaining) = compute_rollover_for_period(
            &setting,
            "2026-04-25",
            &BudgetCycleSettings::default(),
            |_| Decimal::new(100, 0),
            |_| Decimal::new(-25, 0),
        )
        .unwrap();

        assert_eq!(rollover_in, Decimal::ZERO);
        assert_eq!(rollover_out, Decimal::new(125, 0));
        assert_eq!(remaining, Decimal::new(125, 0));
    }

    #[test]
    fn period_keys_are_strictly_validated_against_the_cycle() {
        let cycle = BudgetCycleSettings::default();
        assert!(validate_period_key(DEFAULT_PERIOD_KEY, &cycle).is_ok());
        assert!(validate_period_key("2026-04-25", &cycle).is_ok());
        assert!(validate_period_key("2026-04-24", &cycle).is_err());
        assert!(validate_period_key("2026-13-25", &cycle).is_err());
    }

    #[test]
    fn planned_budget_targets_remain_independent_of_actual_activity_filtering() {
        let targets = vec![target(
            "2026-04-25",
            BudgetTargetType::Category,
            Some(SPENDING_TAXONOMY),
            Some("cat_groceries"),
            None,
            "125",
        )];
        let index = TargetIndex::new(&targets);

        assert_eq!(
            index.effective_category_decimal("2026-04-25", SPENDING_TAXONOMY, "cat_groceries",),
            Decimal::new(125, 0)
        );
    }
}
