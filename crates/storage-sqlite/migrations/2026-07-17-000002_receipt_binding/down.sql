CREATE TEMP TABLE receipt_binding_downgrade_guard (ok INTEGER NOT NULL CHECK (ok = 1));
INSERT INTO receipt_binding_downgrade_guard(ok)
SELECT CASE WHEN EXISTS (SELECT 1 FROM disclosure_receipts WHERE binding_version != 0 OR receipt_binding IS NOT NULL) THEN 0 ELSE 1 END;
DROP TABLE receipt_binding_downgrade_guard;
ALTER TABLE disclosure_receipts DROP COLUMN receipt_binding;
ALTER TABLE disclosure_receipts DROP COLUMN binding_version;
