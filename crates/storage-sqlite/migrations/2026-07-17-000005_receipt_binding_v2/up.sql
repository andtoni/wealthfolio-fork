-- Binding v1 represented a NULL successor with the literal text "<null>",
-- which collided with a real successor having that value. V2 encodes nullable
-- presence independently and then length-prefixes the value.
UPDATE disclosure_receipts
SET binding_version = 2,
    receipt_binding =
      'v2;'
      || length(CAST(deployment_id AS BLOB)) || ':' || deployment_id
      || length(CAST(person_subject_id AS BLOB)) || ':' || person_subject_id
      || length(CAST(idempotency_key AS BLOB)) || ':' || idempotency_key
      || length(CAST(CAST(access_generation AS TEXT) AS BLOB)) || ':' || CAST(access_generation AS TEXT)
      || length(CAST(operation AS BLOB)) || ':' || operation
      || length(CAST(resource_id AS BLOB)) || ':' || resource_id
      || length(CAST(account_id AS BLOB)) || ':' || account_id
      || '1:' || CASE WHEN lifecycle_successor_id IS NULL THEN '0' ELSE '1' END
      || length(CAST(COALESCE(lifecycle_successor_id, '') AS BLOB)) || ':' || COALESCE(lifecycle_successor_id, '')
      || length(CAST(outcome AS BLOB)) || ':' || outcome;
