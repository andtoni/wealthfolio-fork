-- Drop indexes and tables in reverse order of creation
DROP INDEX IF EXISTS idx_verified_receipts_account;
DROP TABLE IF EXISTS verified_receipts;
DROP TABLE IF EXISTS provider_identities;

-- Remove include_in_net_worth column (SQLite doesn't support DROP COLUMN in older versions,
-- but since we're on a modern SQLite we can use ALTER TABLE DROP COLUMN)
ALTER TABLE accounts DROP COLUMN include_in_net_worth;
