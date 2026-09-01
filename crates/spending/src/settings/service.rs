use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use super::model::{BudgetCycleSettings, SpendingSettings, SpendingSettingsUpdate};
use super::traits::SpendingSettingsRepositoryTrait;
use super::{SETTING_KEY_ACCOUNT_IDS, SETTING_KEY_BUDGET_CYCLE, SETTING_KEY_ENABLED};
use crate::error::SpendingError;

pub struct SpendingSettingsService {
    repo: Arc<dyn SpendingSettingsRepositoryTrait>,
    op_lock: Arc<Mutex<()>>,
}

impl SpendingSettingsService {
    pub fn new(repo: Arc<dyn SpendingSettingsRepositoryTrait>) -> Self {
        Self {
            repo,
            op_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn get(&self) -> Result<SpendingSettings> {
        let _guard = self.op_lock.lock().await;
        self.read_settings().await
    }

    async fn read_settings(&self) -> Result<SpendingSettings> {
        let enabled = self
            .repo
            .get_setting(SETTING_KEY_ENABLED)
            .await?
            .map(|s| s == "true")
            .unwrap_or(false);

        let account_ids = self
            .repo
            .get_setting(SETTING_KEY_ACCOUNT_IDS)
            .await?
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default();

        let budget_cycle = self
            .repo
            .get_setting(SETTING_KEY_BUDGET_CYCLE)
            .await?
            .and_then(|s| serde_json::from_str::<BudgetCycleSettings>(&s).ok())
            .filter(|cycle| cycle.parsed_anchor_date().is_some())
            .unwrap_or_default();

        Ok(SpendingSettings {
            enabled,
            account_ids,
            budget_cycle,
        })
    }

    pub async fn update(&self, patch: SpendingSettingsUpdate) -> Result<SpendingSettings> {
        let (_, after) = self.update_with_previous(patch).await?;
        Ok(after)
    }

    pub async fn update_with_previous(
        &self,
        patch: SpendingSettingsUpdate,
    ) -> Result<(SpendingSettings, SpendingSettings)> {
        let _guard = self.op_lock.lock().await;
        let before = self.read_settings().await?;
        let mut values = Vec::new();
        if let Some(enabled) = patch.enabled {
            values.push((
                SETTING_KEY_ENABLED.to_string(),
                (if enabled { "true" } else { "false" }).to_string(),
            ));
        }
        if let Some(ids) = patch.account_ids {
            let json = serde_json::to_string(&ids)?;
            values.push((SETTING_KEY_ACCOUNT_IDS.to_string(), json));
        }
        if let Some(cycle) = patch.budget_cycle {
            validate_budget_cycle(&cycle)?;
            if cycle != before.budget_cycle {
                self.repo
                    .update_budget_cycle(before.budget_cycle.clone(), cycle, values)
                    .await?;
            } else if !values.is_empty() {
                self.repo.set_settings(values).await?;
            }
        } else if !values.is_empty() {
            self.repo.set_settings(values).await?;
        }
        let after = self.read_settings().await?;
        Ok((before, after))
    }

    /// Auto-include a new CASH account in the spending-tracked set when spending
    /// is enabled. No-op when disabled.
    pub async fn auto_include_account(&self, account_id: &str) -> Result<()> {
        let _guard = self.op_lock.lock().await;
        let mut current = self.read_settings().await?;
        if !current.enabled {
            return Ok(());
        }
        if !current.account_ids.iter().any(|id| id == account_id) {
            current.account_ids.push(account_id.to_string());
            let json = serde_json::to_string(&current.account_ids)?;
            self.repo
                .set_setting(SETTING_KEY_ACCOUNT_IDS, &json)
                .await?;
        }
        Ok(())
    }
}

fn validate_budget_cycle(cycle: &BudgetCycleSettings) -> Result<()> {
    if cycle.parsed_anchor_date().is_none() {
        return Err(SpendingError::InvalidInput {
            message: "Budget cycle anchorDate must be a valid YYYY-MM-DD date".to_string(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;

    use super::*;
    use crate::settings::BudgetCadence;

    struct MockSettingsRepo {
        values: StdMutex<HashMap<String, String>>,
        has_concrete_budget_records: bool,
    }

    #[async_trait]
    impl SpendingSettingsRepositoryTrait for MockSettingsRepo {
        async fn get_setting(&self, key: &str) -> Result<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        async fn set_settings(&self, values: Vec<(String, String)>) -> Result<()> {
            self.values.lock().unwrap().extend(values);
            Ok(())
        }

        async fn update_budget_cycle(
            &self,
            old_cycle: BudgetCycleSettings,
            new_cycle: BudgetCycleSettings,
            mut other_settings: Vec<(String, String)>,
        ) -> Result<()> {
            if old_cycle.cadence != new_cycle.cadence && self.has_concrete_budget_records {
                return Err(anyhow::anyhow!(
                    "Changing budget cadence requires no cycle-specific budget records"
                ));
            }
            other_settings.push((
                SETTING_KEY_BUDGET_CYCLE.to_string(),
                serde_json::to_string(&new_cycle)?,
            ));
            self.set_settings(other_settings).await
        }
    }

    #[test]
    fn validates_budget_cycle_anchor_date() {
        assert!(validate_budget_cycle(&BudgetCycleSettings {
            cadence: BudgetCadence::Weekly,
            anchor_date: "2026-07-25".to_string(),
        })
        .is_ok());
        for anchor_date in ["2026-02-30", "not-a-date", "2026-7-5"] {
            assert!(validate_budget_cycle(&BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: anchor_date.to_string(),
            })
            .is_err());
        }
    }

    #[tokio::test]
    async fn updates_cycle_when_only_default_targets_exist() {
        let service = SpendingSettingsService::new(Arc::new(MockSettingsRepo {
            values: StdMutex::new(HashMap::new()),
            has_concrete_budget_records: false,
        }));
        let cycle = BudgetCycleSettings {
            cadence: BudgetCadence::Biweekly,
            anchor_date: "2026-08-07".to_string(),
        };

        let updated = service
            .update(SpendingSettingsUpdate {
                budget_cycle: Some(cycle.clone()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(updated.budget_cycle, cycle);
    }

    #[tokio::test]
    async fn allows_same_cadence_anchor_changes_with_concrete_budget_records() {
        let service = SpendingSettingsService::new(Arc::new(MockSettingsRepo {
            values: StdMutex::new(HashMap::new()),
            has_concrete_budget_records: true,
        }));
        let cycle = BudgetCycleSettings {
            cadence: BudgetCadence::Monthly,
            anchor_date: "2026-08-01".to_string(),
        };

        let updated = service
            .update(SpendingSettingsUpdate {
                budget_cycle: Some(cycle.clone()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(updated.budget_cycle, cycle);
    }

    #[tokio::test]
    async fn rejects_cadence_changes_with_concrete_budget_records() {
        let service = SpendingSettingsService::new(Arc::new(MockSettingsRepo {
            values: StdMutex::new(HashMap::new()),
            has_concrete_budget_records: true,
        }));

        let error = service
            .update(SpendingSettingsUpdate {
                budget_cycle: Some(BudgetCycleSettings {
                    cadence: BudgetCadence::Weekly,
                    anchor_date: "2026-07-25".to_string(),
                }),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("Changing budget cadence requires no cycle-specific budget records"));
        assert_eq!(
            service.get().await.unwrap().budget_cycle,
            BudgetCycleSettings::default()
        );
    }
}
