-- Durable, subject-owned control and staging records for Finance historical imports.
-- Batch writes only stage reviewed data. Finalize publishes the complete run in one transaction.
CREATE TABLE historical_import_runs (
    import_run_id TEXT NOT NULL PRIMARY KEY,
    subject TEXT NOT NULL,
    manifest_hash TEXT NOT NULL,
    manifest_payload_digest TEXT NOT NULL,
    manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json)),
    status TEXT NOT NULL DEFAULT 'staged' CHECK(status IN ('staged', 'active', 'rolled_back')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK(length(trim(import_run_id)) > 0),
    CHECK(length(trim(subject)) > 0)
);

CREATE TABLE historical_import_batch_receipts (
    subject TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    import_run_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN ('stage_ledger', 'stage_learning_evidence', 'finalize', 'rollback')),
    payload_digest TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY(subject, idempotency_key),
    FOREIGN KEY(import_run_id) REFERENCES historical_import_runs(import_run_id) ON DELETE RESTRICT
);
CREATE INDEX idx_historical_import_receipts_run
    ON historical_import_batch_receipts(subject, import_run_id);

CREATE TABLE historical_import_staged_ledger (
    subject TEXT NOT NULL,
    import_run_id TEXT NOT NULL,
    manifest_hash TEXT NOT NULL,
    identity TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    row_json TEXT NOT NULL CHECK(json_valid(row_json)),
    PRIMARY KEY(subject, import_run_id, manifest_hash, identity),
    FOREIGN KEY(import_run_id) REFERENCES historical_import_runs(import_run_id) ON DELETE RESTRICT
);

CREATE TABLE historical_import_staged_evidence (
    subject TEXT NOT NULL,
    import_run_id TEXT NOT NULL,
    manifest_hash TEXT NOT NULL,
    identity TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    row_json TEXT NOT NULL CHECK(json_valid(row_json)),
    PRIMARY KEY(subject, import_run_id, manifest_hash, identity),
    FOREIGN KEY(import_run_id) REFERENCES historical_import_runs(import_run_id) ON DELETE RESTRICT
);

CREATE TABLE historical_import_activities (
    subject TEXT NOT NULL,
    import_run_id TEXT NOT NULL,
    activity_id TEXT NOT NULL UNIQUE,
    PRIMARY KEY(subject, import_run_id, activity_id),
    FOREIGN KEY(import_run_id) REFERENCES historical_import_runs(import_run_id) ON DELETE RESTRICT,
    FOREIGN KEY(activity_id) REFERENCES activities(id) ON DELETE CASCADE
);

CREATE TABLE historical_import_evidence (
    subject TEXT NOT NULL,
    import_run_id TEXT NOT NULL,
    evidence_id TEXT NOT NULL UNIQUE,
    PRIMARY KEY(subject, import_run_id, evidence_id),
    FOREIGN KEY(import_run_id) REFERENCES historical_import_runs(import_run_id) ON DELETE RESTRICT,
    FOREIGN KEY(evidence_id) REFERENCES spending_categorization_evidence(id) ON DELETE CASCADE
);
