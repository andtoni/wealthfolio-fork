-- Forward-only taxonomy normalization. Recreating Finance-v1 duplicate
-- categories would make historical references ambiguous.
CREATE TRIGGER restore_native_spending_taxonomy_down_abort
BEFORE INSERT ON app_settings
BEGIN
  SELECT RAISE(ABORT, 'Restoring native spending taxonomy is irreversible');
END;
INSERT INTO app_settings(setting_key, setting_value)
VALUES ('restore_native_spending_taxonomy_down_abort', '1');
