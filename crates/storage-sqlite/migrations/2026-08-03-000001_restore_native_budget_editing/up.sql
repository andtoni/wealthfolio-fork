-- Finance-v1 seeded the native budget tables, then added database triggers that
-- made the seed immutable. Keep the seeded categories, groups, assignments, and
-- targets as ordinary user data and remove only the enforcement layer.
DROP TRIGGER IF EXISTS finance_v1_lock_policy_delete;
DROP TRIGGER IF EXISTS finance_v1_lock_policy_update;
DROP TRIGGER IF EXISTS finance_v1_lock_assignment_delete;
DROP TRIGGER IF EXISTS finance_v1_lock_assignment_update;
DROP TRIGGER IF EXISTS finance_v1_lock_category_delete;
DROP TRIGGER IF EXISTS finance_v1_lock_category_update;
DROP TRIGGER IF EXISTS finance_v1_lock_target_delete;
DROP TRIGGER IF EXISTS finance_v1_lock_target_update;
DROP TRIGGER IF EXISTS finance_v1_no_rollover_insert;
DROP TRIGGER IF EXISTS finance_v1_no_rollover_update;

DROP TABLE IF EXISTS finance_v1_locked_budget_objects;
DROP TABLE IF EXISTS finance_v1_budget_policy;
