-- Add include_in_net_worth to accounts
-- Default true for backward compatibility; provider-discovered accounts set this to false.
ALTER TABLE accounts ADD COLUMN include_in_net_worth BOOLEAN NOT NULL DEFAULT true;

-- Provider identities: immutable provider identity for each account.
CREATE TABLE provider_identities (
    account_id TEXT NOT NULL PRIMARY KEY,
    provider TEXT NOT NULL,
    provider_account_id TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL DEFAULT 'active'
        CHECK(lifecycle_state IN ('active','closed','suspended')),
    last_sync_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Verified receipts: make an account eligible for current financial aggregates.
CREATE TABLE verified_receipts (
    id TEXT NOT NULL PRIMARY KEY,
    account_id TEXT NOT NULL,
    basis TEXT NOT NULL CHECK(basis IN ('current_snapshot','opening_baseline')),
    as_of TEXT NOT NULL,
    provider_receipt_ref TEXT,
    verification_state TEXT NOT NULL DEFAULT 'pending'
        CHECK(verification_state IN ('pending','verified','rejected')),
    coverage_start_date TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
);

CREATE INDEX idx_verified_receipts_account ON verified_receipts(account_id);
