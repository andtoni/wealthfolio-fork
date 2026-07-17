CREATE TEMP TABLE receipt_identity_downgrade_guard (ok INTEGER NOT NULL CHECK (ok = 1));
INSERT INTO receipt_identity_downgrade_guard(ok)
SELECT CASE WHEN EXISTS (SELECT 1 FROM disclosure_receipts WHERE lifecycle_successor_id IS NOT NULL) THEN 0 ELSE 1 END;
DROP TABLE receipt_identity_downgrade_guard;
ALTER TABLE disclosure_receipts DROP COLUMN lifecycle_successor_id;
