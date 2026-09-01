//! Storage adapter for `wealthfolio-spending::settings`.
//! Backed by the existing `app_settings` k/v table.

use std::{
    collections::{BTreeMap, HashSet},
    str::FromStr,
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use diesel::{prelude::*, sqlite::SqliteConnection};
use rust_decimal::Decimal;

use crate::db::{get_connection, DbPool, WriteHandle};
use crate::errors::StorageError;
use crate::schema::app_settings::dsl::*;
use crate::schema::{budget_rollover_settings, budget_targets};
use crate::settings::model::AppSettingDB;
use crate::spending::budget::{BudgetRolloverSettingDB, BudgetTargetDB, NewBudgetTargetDB};
use crate::spending::deterministic_ids::budget_target_id;
use wealthfolio_spending::budget::period::{period_for_date, period_for_key};
use wealthfolio_spending::settings::{BudgetCycleSettings, SpendingSettingsRepositoryTrait};

pub(crate) struct BudgetCycleRebaseChanges {
    pub(crate) deleted_target_ids: Vec<String>,
    pub(crate) inserted_targets: Vec<BudgetTargetDB>,
    pub(crate) updated_targets: Vec<BudgetTargetDB>,
    pub(crate) updated_rollovers: Vec<BudgetRolloverSettingDB>,
}

fn invalid_rebase(message: impl Into<String>) -> wealthfolio_core::Error {
    StorageError::SerializationError(message.into()).into()
}

fn canonical_target_for_cycle(row: &BudgetTargetDB, cycle: &BudgetCycleSettings) -> bool {
    period_for_key(&row.period_key, cycle)
        .ok()
        .is_some_and(|period| {
            period.key == row.period_key
                && budget_target_id(
                    &row.period_key,
                    &row.target_type,
                    row.taxonomy_id.as_deref(),
                    row.category_id.as_deref(),
                    row.group_id.as_deref(),
                ) == row.id
        })
}

/// Re-key period-dependent budget rows for a cycle change.
///
/// The caller owns the surrounding transaction. When `allow_already_rebased` is
/// true, rows already canonical for `new_cycle` are retained; this is needed
/// when an incoming sync setting arrives after its target events.
pub(crate) fn rebase_budget_cycle_rows(
    conn: &mut SqliteConnection,
    old_cycle: &BudgetCycleSettings,
    new_cycle: &BudgetCycleSettings,
    allow_already_rebased: bool,
) -> wealthfolio_core::Result<BudgetCycleRebaseChanges> {
    if old_cycle.cadence != new_cycle.cadence {
        return Err(invalid_rebase(
            "Budget cycle rebasing requires the cadence to remain unchanged",
        ));
    }

    let now = Utc::now().to_rfc3339();
    let mut targets = budget_targets::table
        .filter(budget_targets::period_key.ne("default"))
        .load::<BudgetTargetDB>(conn)
        .map_err(StorageError::from)?;
    targets.sort_by(|left, right| left.id.cmp(&right.id));
    let old_ids: HashSet<String> = targets.iter().map(|row| row.id.clone()).collect();
    let mut rebased_targets: BTreeMap<String, NewBudgetTargetDB> = BTreeMap::new();

    for row in &targets {
        let new_period_key = if allow_already_rebased && canonical_target_for_cycle(row, new_cycle)
        {
            row.period_key.clone()
        } else {
            match period_for_key(&row.period_key, old_cycle) {
                Ok(old_period) => {
                    period_for_date(old_period.start, new_cycle)
                        .map_err(|error| invalid_rebase(error.to_string()))?
                        .key
                }
                Err(error) if allow_already_rebased => {
                    if period_for_key(&row.period_key, new_cycle).is_ok() {
                        row.period_key.clone()
                    } else {
                        return Err(invalid_rebase(error.to_string()));
                    }
                }
                Err(error) => return Err(invalid_rebase(error.to_string())),
            }
        };
        let id = budget_target_id(
            &new_period_key,
            &row.target_type,
            row.taxonomy_id.as_deref(),
            row.category_id.as_deref(),
            row.group_id.as_deref(),
        );
        let candidate = NewBudgetTargetDB {
            id: id.clone(),
            period_key: new_period_key,
            target_type: row.target_type.clone(),
            taxonomy_id: row.taxonomy_id.clone(),
            category_id: row.category_id.clone(),
            group_id: row.group_id.clone(),
            amount: row.amount.clone(),
            created_at: row.created_at.clone(),
            updated_at: now.clone(),
        };
        if let Some(existing) = rebased_targets.get_mut(&id) {
            let total = Decimal::from_str(&existing.amount).map_err(|error| {
                invalid_rebase(format!("Invalid budget target amount: {error}"))
            })?;
            let amount = Decimal::from_str(&candidate.amount).map_err(|error| {
                invalid_rebase(format!("Invalid budget target amount: {error}"))
            })?;
            existing.amount = (total + amount).to_string();
            if candidate.created_at < existing.created_at {
                existing.created_at = candidate.created_at;
            }
        } else {
            rebased_targets.insert(id, candidate);
        }
    }

    let new_ids: HashSet<String> = rebased_targets.keys().cloned().collect();
    for id in &new_ids {
        if old_ids.contains(id) {
            continue;
        }
        let exists = budget_targets::table
            .find(id)
            .first::<BudgetTargetDB>(conn)
            .optional()
            .map_err(StorageError::from)?
            .is_some();
        if exists {
            return Err(invalid_rebase(format!(
                "Budget cycle change conflicts with target {id}"
            )));
        }
    }

    let mut deleted_target_ids: Vec<String> = old_ids.difference(&new_ids).cloned().collect();
    deleted_target_ids.sort();
    for id in &deleted_target_ids {
        diesel::delete(budget_targets::table.find(id))
            .execute(conn)
            .map_err(StorageError::from)?;
    }

    let mut inserted_targets = Vec::new();
    let mut updated_targets = Vec::new();
    for (_, mut row) in rebased_targets {
        row.updated_at = now.clone();
        if old_ids.contains(&row.id) {
            let updated = diesel::update(budget_targets::table.find(&row.id))
                .set((
                    budget_targets::period_key.eq(&row.period_key),
                    budget_targets::target_type.eq(&row.target_type),
                    budget_targets::taxonomy_id.eq(&row.taxonomy_id),
                    budget_targets::category_id.eq(&row.category_id),
                    budget_targets::group_id.eq(&row.group_id),
                    budget_targets::amount.eq(&row.amount),
                    budget_targets::created_at.eq(&row.created_at),
                    budget_targets::updated_at.eq(&row.updated_at),
                ))
                .get_result::<BudgetTargetDB>(conn)
                .map_err(StorageError::from)?;
            updated_targets.push(updated);
        } else {
            let inserted = diesel::insert_into(budget_targets::table)
                .values(&row)
                .returning(BudgetTargetDB::as_returning())
                .get_result::<BudgetTargetDB>(conn)
                .map_err(StorageError::from)?;
            inserted_targets.push(inserted);
        }
    }

    let mut updated_rollovers = Vec::new();
    let rollovers = budget_rollover_settings::table
        .load::<BudgetRolloverSettingDB>(conn)
        .map_err(StorageError::from)?;
    for row in rollovers {
        let new_start_month = match period_for_key(&row.start_month, old_cycle) {
            Ok(old_period) => {
                period_for_date(old_period.start, new_cycle)
                    .map_err(|error| invalid_rebase(error.to_string()))?
                    .key
            }
            Err(_error)
                if allow_already_rebased && period_for_key(&row.start_month, new_cycle).is_ok() =>
            {
                row.start_month.clone()
            }
            Err(error) => return Err(invalid_rebase(error.to_string())),
        };
        if new_start_month == row.start_month {
            continue;
        }
        let updated = diesel::update(budget_rollover_settings::table.find(&row.id))
            .set((
                budget_rollover_settings::start_month.eq(new_start_month),
                budget_rollover_settings::updated_at.eq(&now),
            ))
            .get_result::<BudgetRolloverSettingDB>(conn)
            .map_err(StorageError::from)?;
        updated_rollovers.push(updated);
    }

    Ok(BudgetCycleRebaseChanges {
        deleted_target_ids,
        inserted_targets,
        updated_targets,
        updated_rollovers,
    })
}

pub struct SpendingSettingsRepository {
    pool: Arc<DbPool>,
    writer: WriteHandle,
}

impl SpendingSettingsRepository {
    pub fn new(pool: Arc<DbPool>, writer: WriteHandle) -> Self {
        Self { pool, writer }
    }
}

#[async_trait]
impl SpendingSettingsRepositoryTrait for SpendingSettingsRepository {
    async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut conn = get_connection(&self.pool).map_err(|e| anyhow::anyhow!(e))?;
        let value = app_settings
            .filter(setting_key.eq(key))
            .select(setting_value)
            .first::<String>(&mut conn)
            .optional()
            .map_err(StorageError::from)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(value)
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.set_settings(vec![(key.to_string(), value.to_string())])
            .await
    }

    async fn set_settings(&self, values: Vec<(String, String)>) -> Result<()> {
        let rows: Vec<AppSettingDB> = values
            .into_iter()
            .map(|(key, value)| AppSettingDB {
                setting_key: key,
                setting_value: value,
            })
            .collect();
        self.writer
            .exec_tx(move |tx| {
                for row in rows {
                    diesel::replace_into(app_settings)
                        .values(&row)
                        .execute(tx.conn())
                        .map_err(StorageError::from)?;
                    tx.update(&row)?;
                }
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }

    async fn update_budget_cycle(
        &self,
        old_cycle: wealthfolio_spending::settings::BudgetCycleSettings,
        new_cycle: wealthfolio_spending::settings::BudgetCycleSettings,
        other_settings: Vec<(String, String)>,
    ) -> Result<()> {
        let cycle_json = serde_json::to_string(&new_cycle)?;
        self.writer
            .exec_tx(move |tx| {
                let stored_cycle_json = app_settings
                    .filter(
                        setting_key.eq(wealthfolio_spending::settings::SETTING_KEY_BUDGET_CYCLE),
                    )
                    .select(setting_value)
                    .first::<String>(tx.conn())
                    .optional()
                    .map_err(StorageError::from)?;
                let stored_cycle = stored_cycle_json
                    .map(|value| {
                        serde_json::from_str::<BudgetCycleSettings>(&value)
                            .map_err(|error| invalid_rebase(error.to_string()))
                    })
                    .transpose()?
                    .unwrap_or_default();
                if stored_cycle != old_cycle {
                    return Err(invalid_rebase(
                        "Budget cycle changed while the update was in progress",
                    ));
                }

                let changes = if old_cycle.cadence == new_cycle.cadence {
                    rebase_budget_cycle_rows(tx.conn(), &old_cycle, &new_cycle, false)?
                } else {
                    let target_count = budget_targets::table
                        .filter(budget_targets::period_key.ne("default"))
                        .count()
                        .get_result::<i64>(tx.conn())
                        .map_err(StorageError::from)?;
                    let rollover_count = budget_rollover_settings::table
                        .count()
                        .get_result::<i64>(tx.conn())
                        .map_err(StorageError::from)?;
                    if target_count > 0 || rollover_count > 0 {
                        return Err(invalid_rebase(
                            "Changing budget cadence requires no cycle-specific budget records",
                        ));
                    }
                    BudgetCycleRebaseChanges {
                        deleted_target_ids: Vec::new(),
                        inserted_targets: Vec::new(),
                        updated_targets: Vec::new(),
                        updated_rollovers: Vec::new(),
                    }
                };
                for id in changes.deleted_target_ids {
                    tx.delete::<BudgetTargetDB>(id);
                }
                for row in changes.updated_targets {
                    tx.update(&row)?;
                }
                for row in changes.inserted_targets {
                    tx.insert(&row)?;
                }
                for row in changes.updated_rollovers {
                    tx.update(&row)?;
                }
                for (key, value) in other_settings {
                    let setting = AppSettingDB {
                        setting_key: key,
                        setting_value: value,
                    };
                    diesel::replace_into(app_settings)
                        .values(&setting)
                        .execute(tx.conn())
                        .map_err(StorageError::from)?;
                    tx.update(&setting)?;
                }
                let cycle = AppSettingDB {
                    setting_key: wealthfolio_spending::settings::SETTING_KEY_BUDGET_CYCLE
                        .to_string(),
                    setting_value: cycle_json,
                };
                diesel::replace_into(app_settings)
                    .values(&cycle)
                    .execute(tx.conn())
                    .map_err(StorageError::from)?;
                tx.update(&cycle)?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }
}
