CREATE TRIGGER spending_budget_cycle_guard_insert BEFORE INSERT ON app_settings
WHEN NEW.setting_key = 'spending.budget_cycle'
  AND EXISTS (
    SELECT 1 FROM app_settings
    WHERE setting_key = NEW.setting_key AND setting_value <> NEW.setting_value
  )
  AND (
    EXISTS (SELECT 1 FROM budget_targets WHERE period_key <> 'default')
    OR EXISTS (SELECT 1 FROM budget_rollover_settings)
  )
BEGIN SELECT RAISE(ABORT, 'Remove cycle-specific budget overrides before changing the budget cycle'); END;

CREATE TRIGGER spending_budget_cycle_guard_update BEFORE UPDATE ON app_settings
WHEN NEW.setting_key = 'spending.budget_cycle'
  AND NEW.setting_value <> OLD.setting_value
  AND (
    EXISTS (SELECT 1 FROM budget_targets WHERE period_key <> 'default')
    OR EXISTS (SELECT 1 FROM budget_rollover_settings)
  )
BEGIN SELECT RAISE(ABORT, 'Remove cycle-specific budget overrides before changing the budget cycle'); END;
