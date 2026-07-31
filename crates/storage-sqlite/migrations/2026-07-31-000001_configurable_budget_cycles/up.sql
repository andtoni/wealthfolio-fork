-- Replace the locked 25th-24th calendar assumption with a user-owned recurring
-- budget cycle. Existing users retain the same schedule through the seeded
-- monthly anchor. No-rollover, ZAR, and transfer-exclusion policy remains locked.

DROP TRIGGER IF EXISTS finance_v1_lock_target_update;
DROP TRIGGER IF EXISTS finance_v1_lock_target_delete;
DROP TRIGGER IF EXISTS finance_v1_no_rollover_insert;
DROP TRIGGER IF EXISTS finance_v1_no_rollover_update;
DROP TRIGGER IF EXISTS finance_v1_lock_policy_update;
DROP TRIGGER IF EXISTS finance_v1_lock_policy_delete;

ALTER TABLE finance_v1_budget_policy RENAME TO finance_v1_budget_policy_legacy;
CREATE TABLE finance_v1_budget_policy (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  currency TEXT NOT NULL CHECK (currency = 'ZAR'),
  rollover_enabled INTEGER NOT NULL CHECK (rollover_enabled = 0),
  transfers_excluded INTEGER NOT NULL CHECK (transfers_excluded = 1),
  seeded_at TEXT NOT NULL
);
INSERT INTO finance_v1_budget_policy
  (id, currency, rollover_enabled, transfers_excluded, seeded_at)
SELECT id, currency, rollover_enabled, transfers_excluded, seeded_at
FROM finance_v1_budget_policy_legacy;
DROP TABLE finance_v1_budget_policy_legacy;

CREATE TABLE budget_targets_configurable_cycle (
  id TEXT NOT NULL PRIMARY KEY,
  period_key TEXT NOT NULL,
  target_type TEXT NOT NULL,
  taxonomy_id TEXT,
  category_id TEXT,
  group_id TEXT,
  amount TEXT NOT NULL DEFAULT '0',
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

  FOREIGN KEY (taxonomy_id, category_id) REFERENCES taxonomy_categories(taxonomy_id, id) ON DELETE CASCADE,
  FOREIGN KEY (group_id) REFERENCES budget_groups(id) ON DELETE CASCADE,

  CHECK (target_type IN ('category', 'group_buffer')),
  CHECK (
    period_key = 'default'
    OR (
      period_key GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'
      AND date(period_key) = period_key
    )
  ),
  CHECK (
    (target_type = 'category' AND taxonomy_id IS NOT NULL AND category_id IS NOT NULL AND group_id IS NULL)
    OR
    (target_type = 'group_buffer' AND taxonomy_id IS NULL AND category_id IS NULL AND group_id IS NOT NULL)
  )
);
INSERT INTO budget_targets_configurable_cycle
  (id, period_key, target_type, taxonomy_id, category_id, group_id, amount, created_at, updated_at)
SELECT
  id,
  CASE
    WHEN period_key = 'default' THEN 'default'
    ELSE date(period_key || '-01', '-1 month', '+24 days')
  END,
  target_type, taxonomy_id, category_id, group_id, amount, created_at, updated_at
FROM budget_targets;
DROP TABLE budget_targets;
ALTER TABLE budget_targets_configurable_cycle RENAME TO budget_targets;
CREATE UNIQUE INDEX idx_budget_targets_category_unique
  ON budget_targets(period_key, taxonomy_id, category_id)
  WHERE target_type = 'category';
CREATE UNIQUE INDEX idx_budget_targets_group_buffer_unique
  ON budget_targets(period_key, group_id)
  WHERE target_type = 'group_buffer';
CREATE INDEX idx_budget_targets_period ON budget_targets(period_key);
CREATE INDEX idx_budget_targets_category ON budget_targets(taxonomy_id, category_id);
CREATE INDEX idx_budget_targets_group ON budget_targets(group_id);

CREATE TABLE budget_rollover_settings_configurable_cycle (
  id TEXT NOT NULL PRIMARY KEY,
  target_type TEXT NOT NULL,
  taxonomy_id TEXT,
  category_id TEXT,
  group_id TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  start_month TEXT NOT NULL,
  starting_balance TEXT NOT NULL DEFAULT '0',
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

  FOREIGN KEY (taxonomy_id, category_id) REFERENCES taxonomy_categories(taxonomy_id, id) ON DELETE CASCADE,
  FOREIGN KEY (group_id) REFERENCES budget_groups(id) ON DELETE CASCADE,

  CHECK (target_type IN ('category', 'group')),
  CHECK (enabled IN (0, 1)),
  CHECK (
    start_month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'
    AND date(start_month) = start_month
  ),
  CHECK (
    (target_type = 'category' AND taxonomy_id IN ('spending_categories', 'savings_categories') AND category_id IS NOT NULL AND group_id IS NULL)
    OR
    (target_type = 'group' AND taxonomy_id IS NULL AND category_id IS NULL AND group_id IS NOT NULL)
  )
);
INSERT INTO budget_rollover_settings_configurable_cycle
  (id, target_type, taxonomy_id, category_id, group_id, enabled, start_month, starting_balance, created_at, updated_at)
SELECT
  id, target_type, taxonomy_id, category_id, group_id, enabled,
  date(start_month || '-01', '-1 month', '+24 days'),
  starting_balance, created_at, updated_at
FROM budget_rollover_settings;
DROP TABLE budget_rollover_settings;
ALTER TABLE budget_rollover_settings_configurable_cycle RENAME TO budget_rollover_settings;
CREATE UNIQUE INDEX idx_budget_rollover_settings_category_unique
  ON budget_rollover_settings(taxonomy_id, category_id)
  WHERE target_type = 'category';
CREATE UNIQUE INDEX idx_budget_rollover_settings_group_unique
  ON budget_rollover_settings(group_id)
  WHERE target_type = 'group';
CREATE INDEX idx_budget_rollover_settings_category ON budget_rollover_settings(taxonomy_id, category_id);
CREATE INDEX idx_budget_rollover_settings_group ON budget_rollover_settings(group_id);

INSERT OR IGNORE INTO app_settings(setting_key, setting_value)
VALUES ('spending.budget_cycle', '{"cadence":"monthly","anchorDate":"2026-07-25"}');

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

CREATE TRIGGER finance_v1_lock_policy_update BEFORE UPDATE ON finance_v1_budget_policy
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget policy is locked'); END;
CREATE TRIGGER finance_v1_lock_policy_delete BEFORE DELETE ON finance_v1_budget_policy
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget policy is locked'); END;
CREATE TRIGGER finance_v1_no_rollover_insert BEFORE INSERT ON budget_rollover_settings
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget rollover is disabled'); END;
CREATE TRIGGER finance_v1_no_rollover_update BEFORE UPDATE ON budget_rollover_settings
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget rollover is disabled'); END;
CREATE TRIGGER finance_v1_lock_target_update BEFORE UPDATE ON budget_targets
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='target' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget target is locked'); END;
CREATE TRIGGER finance_v1_lock_target_delete BEFORE DELETE ON budget_targets
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='target' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget target is locked'); END;
