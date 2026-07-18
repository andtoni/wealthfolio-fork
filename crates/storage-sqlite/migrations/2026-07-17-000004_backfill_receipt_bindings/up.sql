-- Legacy receipts already contain every identity field covered by binding v1.
-- Backfill a deterministic byte-length-prefixed binding so exact replays remain
-- safe after the binding contract is enabled.
UPDATE disclosure_receipts
SET binding_version = 1,
    receipt_binding =
      'v1;'
      || length(CAST(deployment_id AS BLOB)) || ':' || deployment_id
      || length(CAST(person_subject_id AS BLOB)) || ':' || person_subject_id
      || length(CAST(idempotency_key AS BLOB)) || ':' || idempotency_key
      || length(CAST(CAST(access_generation AS TEXT) AS BLOB)) || ':' || CAST(access_generation AS TEXT)
      || length(CAST(operation AS BLOB)) || ':' || operation
      || length(CAST(resource_id AS BLOB)) || ':' || resource_id
      || length(CAST(account_id AS BLOB)) || ':' || account_id
      || length(CAST(COALESCE(lifecycle_successor_id, '<null>') AS BLOB)) || ':' || COALESCE(lifecycle_successor_id, '<null>')
      || length(CAST(outcome AS BLOB)) || ':' || outcome
WHERE binding_version = 0
  AND receipt_binding IS NULL;
