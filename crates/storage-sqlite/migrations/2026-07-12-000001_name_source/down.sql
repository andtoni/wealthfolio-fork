-- Reverse the name_source migration.
ALTER TABLE accounts DROP COLUMN name_source;
ALTER TABLE provider_identities DROP COLUMN provider_display_name;
