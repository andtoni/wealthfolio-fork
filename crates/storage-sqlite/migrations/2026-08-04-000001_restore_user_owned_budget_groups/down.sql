-- Forward-only normalization: restoring the seeded canonical groups would
-- reintroduce the hard-coded budget behavior this migration removes.
CREATE TRIGGER restore_user_owned_budget_groups_down_abort
BEFORE INSERT ON app_settings
BEGIN
  SELECT RAISE(ABORT, 'Restoring seeded budget groups is irreversible');
END;
INSERT INTO app_settings(setting_key, setting_value)
VALUES ('restore_user_owned_budget_groups_down_abort', '1');
