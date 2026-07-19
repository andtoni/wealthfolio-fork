-- Local, advisory evidence aggregated from trustworthy historical manual labels.
-- This sidecar has no write path to activities, assignments, overlays, or balances.
CREATE TABLE spending_categorization_evidence (
    id TEXT NOT NULL PRIMARY KEY,
    identity_key TEXT NOT NULL UNIQUE,
    merchant_key TEXT NOT NULL,
    merchant_key_version INTEGER NOT NULL,
    normalization_version INTEGER NOT NULL,
    taxonomy_id TEXT NOT NULL,
    category_id TEXT NOT NULL,
    direction TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    provider_context TEXT,
    account_context TEXT,
    label_count INTEGER NOT NULL,
    agreement_state TEXT NOT NULL,
    first_label_date TEXT NOT NULL,
    last_label_date TEXT NOT NULL,
    provenance TEXT NOT NULL,
    import_run_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

    FOREIGN KEY (taxonomy_id, category_id)
        REFERENCES taxonomy_categories(taxonomy_id, id) ON DELETE RESTRICT,
    CHECK (merchant_key_version > 0),
    CHECK (normalization_version > 0),
    CHECK (label_count > 0),
    CHECK (agreement_state IN ('agreement', 'conflict')),
    CHECK (length(trim(merchant_key)) > 0),
    CHECK (length(trim(direction)) > 0),
    CHECK (length(trim(activity_type)) > 0),
    CHECK (length(trim(provenance)) > 0)
);

CREATE INDEX idx_spending_categorization_evidence_lookup
    ON spending_categorization_evidence(
        merchant_key, merchant_key_version, normalization_version,
        direction, activity_type
    );
CREATE INDEX idx_spending_categorization_evidence_import_run
    ON spending_categorization_evidence(import_run_id);
CREATE INDEX idx_spending_categorization_evidence_category
    ON spending_categorization_evidence(taxonomy_id, category_id);
