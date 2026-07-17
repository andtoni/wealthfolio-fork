ALTER TABLE disclosure_receipts ADD COLUMN binding_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE disclosure_receipts ADD COLUMN receipt_binding TEXT;
