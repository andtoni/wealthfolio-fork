CREATE TABLE provenance_records (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('live_canonical','opening_baseline','manual_current','historical_context','contextual_reference')),
    source_system TEXT,
    source_record_id TEXT,
    import_run_id TEXT,
    actor_id TEXT,
    coverage_start_date TEXT,
    manual_current_approved BOOLEAN NOT NULL DEFAULT false,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    PRIMARY KEY(entity_type, entity_id)
);
CREATE INDEX idx_provenance_records_role ON provenance_records(role);