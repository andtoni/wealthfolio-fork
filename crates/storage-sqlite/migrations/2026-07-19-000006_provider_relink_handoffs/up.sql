CREATE TABLE provider_account_relink_handoffs (
    account_id TEXT NOT NULL,
    provider_generation INTEGER NOT NULL,
    prior_provider_generation INTEGER NOT NULL,
    receiver_access_generation INTEGER NOT NULL,
    database_state_digest TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','acknowledged')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    acknowledged_at TEXT,
    PRIMARY KEY(account_id, provider_generation),
    FOREIGN KEY(account_id) REFERENCES provider_account_connections(account_id) ON DELETE CASCADE,
    CHECK(provider_generation > prior_provider_generation),
    CHECK(length(database_state_digest) = 71 AND database_state_digest LIKE 'sha256:%'),
    CHECK((state = 'pending' AND acknowledged_at IS NULL) OR (state = 'acknowledged' AND acknowledged_at IS NOT NULL))
);

CREATE UNIQUE INDEX provider_account_relink_handoffs_pending
ON provider_account_relink_handoffs(account_id)
WHERE state = 'pending';
