-- Forward-only normalization: restoring the former Finance-v1 locks would
-- reintroduce non-native behavior and could discard user edits. Fail inside the
-- migration transaction so Diesel leaves the migration applied.
CREATE TRIGGER restore_native_budget_editing_down_abort
BEFORE INSERT ON app_settings
BEGIN
  SELECT RAISE(ABORT, 'Restoring native budget editing is irreversible');
END;
INSERT INTO app_settings(setting_key, setting_value)
VALUES ('restore_native_budget_editing_down_abort', '1');
