-- Preserve the checksum of the already-published name_source migration while
-- correcting its fail-open ownership classification. Application inserts now
-- explicitly default to user ownership.
-- Existing names are user-owned unless current provider provenance proves that
-- the stored name still exactly matches the provider's last display name.
UPDATE accounts
SET name_source = 'user'
WHERE name_source = 'provider'
  AND NOT EXISTS (
    SELECT 1
    FROM provider_identities pi
    WHERE pi.account_id = accounts.id
      AND pi.provider_display_name IS NOT NULL
      AND pi.provider_display_name = accounts.name
  );
