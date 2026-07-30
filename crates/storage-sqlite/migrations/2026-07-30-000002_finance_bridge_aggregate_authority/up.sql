-- Materialize aggregate authority for provider facts that were already accepted by
-- the authenticated Finance Sync bridge. Earlier bridge versions persisted the
-- rows and disclosure receipts but omitted the account/row authority sidecars,
-- causing every aggregate read to fail closed.

UPDATE activities
SET created_at = CASE
        WHEN length(activity_date) = 10 THEN activity_date || 'T00:00:00Z'
        ELSE replace(activity_date, ' ', 'T')
    END
WHERE created_at = 'now';

UPDATE activities
SET updated_at = CASE
        WHEN length(activity_date) = 10 THEN activity_date || 'T00:00:00Z'
        ELSE replace(activity_date, ' ', 'T')
    END
WHERE updated_at = 'now';

-- Provider current state owns an account/date over a generated duplicate.
DELETE FROM holdings_snapshots
WHERE source = 'CALCULATED'
  AND EXISTS (
      SELECT 1
      FROM holdings_snapshots provider
      WHERE provider.account_id = holdings_snapshots.account_id
        AND provider.snapshot_date = holdings_snapshots.snapshot_date
        AND provider.source = 'BROKER_IMPORTED'
  );

INSERT INTO provenance_records(
    entity_type, entity_id, role, source_system, source_record_id, actor_id
)
SELECT
    'account', account.id, 'live_canonical', 'finance',
    account.provider_account_id,
    (
        SELECT receipt.person_subject_id
        FROM disclosure_receipts receipt
        WHERE receipt.account_id = account.id
          AND receipt.operation = 'current_state_upsert'
          AND receipt.outcome = 'accepted'
        ORDER BY receipt.access_generation DESC, receipt.idempotency_key DESC
        LIMIT 1
    )
FROM accounts account
WHERE EXISTS (
    SELECT 1 FROM disclosure_receipts receipt
    WHERE receipt.account_id = account.id
      AND receipt.operation = 'current_state_upsert'
      AND receipt.outcome = 'accepted'
)
ON CONFLICT(entity_type, entity_id) DO UPDATE SET
    role = 'live_canonical',
    source_system = 'finance',
    source_record_id = excluded.source_record_id,
    actor_id = excluded.actor_id,
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');

INSERT INTO verified_receipts(
    id, account_id, basis, as_of, provider_receipt_ref, verification_state
)
SELECT
    'finance-current-state:' || account.id,
    account.id,
    'current_snapshot',
    (SELECT max(snapshot.snapshot_date) FROM holdings_snapshots snapshot
     WHERE snapshot.account_id = account.id AND snapshot.source = 'BROKER_IMPORTED'),
    (
        SELECT receipt.idempotency_key
        FROM disclosure_receipts receipt
        WHERE receipt.account_id = account.id
          AND receipt.operation = 'current_state_upsert'
          AND receipt.outcome = 'accepted'
        ORDER BY receipt.access_generation DESC, receipt.idempotency_key DESC
        LIMIT 1
    ),
    'verified'
FROM accounts account
WHERE EXISTS (
    SELECT 1 FROM holdings_snapshots snapshot
    WHERE snapshot.account_id = account.id AND snapshot.source = 'BROKER_IMPORTED'
)
  AND EXISTS (
    SELECT 1 FROM disclosure_receipts receipt
    WHERE receipt.account_id = account.id
      AND receipt.operation = 'current_state_upsert'
      AND receipt.outcome = 'accepted'
)
ON CONFLICT(id) DO UPDATE SET
    as_of = excluded.as_of,
    provider_receipt_ref = excluded.provider_receipt_ref,
    verification_state = 'verified',
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');

UPDATE accounts
SET include_in_net_worth = 1,
    updated_at = CURRENT_TIMESTAMP
WHERE is_active = 1
  AND is_archived = 0
  AND EXISTS (
      SELECT 1 FROM verified_receipts receipt
      WHERE receipt.account_id = accounts.id
        AND receipt.verification_state = 'verified'
  );

INSERT INTO provenance_records(
    entity_type, entity_id, role, source_system, source_record_id, actor_id
)
SELECT
    'activity', projection.activity_id, 'live_canonical', 'finance',
    projection.activity_id, projection.person_subject_id
FROM activity_disclosure_projections projection
WHERE EXISTS (SELECT 1 FROM activities activity WHERE activity.id = projection.activity_id)
ON CONFLICT(entity_type, entity_id) DO UPDATE SET
    role = 'live_canonical',
    source_system = 'finance',
    source_record_id = excluded.source_record_id,
    actor_id = excluded.actor_id,
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');

INSERT INTO provenance_records(
    entity_type, entity_id, role, source_system, source_record_id, actor_id
)
SELECT
    'daily_valuation', valuation.id, 'live_canonical', 'WEALTHFOLIO_DERIVED',
    NULL,
    (SELECT provenance.actor_id FROM provenance_records provenance
     WHERE provenance.entity_type = 'account'
       AND provenance.entity_id = valuation.account_id)
FROM daily_account_valuation valuation
WHERE EXISTS (
    SELECT 1 FROM verified_receipts receipt
    WHERE receipt.account_id = valuation.account_id
      AND receipt.verification_state = 'verified'
)
ON CONFLICT(entity_type, entity_id) DO UPDATE SET
    role = 'live_canonical',
    source_system = 'WEALTHFOLIO_DERIVED',
    actor_id = excluded.actor_id,
    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');

-- Refresh same-day derived values from the authoritative provider keyframe.
UPDATE daily_account_valuation
SET cash_balance = coalesce((
        SELECT snapshot.cash_total_account_currency
        FROM holdings_snapshots snapshot
        WHERE snapshot.account_id = daily_account_valuation.account_id
          AND snapshot.snapshot_date = daily_account_valuation.valuation_date
          AND snapshot.source = 'BROKER_IMPORTED'
        LIMIT 1
    ), cash_balance),
    cash_balance_base = coalesce((
        SELECT snapshot.cash_total_base_currency
        FROM holdings_snapshots snapshot
        WHERE snapshot.account_id = daily_account_valuation.account_id
          AND snapshot.snapshot_date = daily_account_valuation.valuation_date
          AND snapshot.source = 'BROKER_IMPORTED'
        LIMIT 1
    ), cash_balance_base),
    total_value = coalesce((
        SELECT CAST(CAST(snapshot.cash_total_account_currency AS REAL)
                    + CAST(snapshot.liability_balance AS REAL) AS TEXT)
        FROM holdings_snapshots snapshot
        WHERE snapshot.account_id = daily_account_valuation.account_id
          AND snapshot.snapshot_date = daily_account_valuation.valuation_date
          AND snapshot.source = 'BROKER_IMPORTED'
        LIMIT 1
    ), total_value),
    total_value_base = coalesce((
        SELECT CAST(CAST(snapshot.cash_total_base_currency AS REAL)
                    + CAST(snapshot.liability_balance AS REAL) AS TEXT)
        FROM holdings_snapshots snapshot
        WHERE snapshot.account_id = daily_account_valuation.account_id
          AND snapshot.snapshot_date = daily_account_valuation.valuation_date
          AND snapshot.source = 'BROKER_IMPORTED'
        LIMIT 1
    ), total_value_base),
    value_status = 'COMPLETE',
    calculated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE EXISTS (
    SELECT 1 FROM holdings_snapshots snapshot
    WHERE snapshot.account_id = daily_account_valuation.account_id
      AND snapshot.snapshot_date = daily_account_valuation.valuation_date
      AND snapshot.source = 'BROKER_IMPORTED'
);
