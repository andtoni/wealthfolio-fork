-- Intentionally irreversible: these rows record already-accepted provider facts
-- and their derived read-model authority. Rolling back must not revoke verified
-- account inclusion or remove provenance from live financial data.
SELECT 1;
