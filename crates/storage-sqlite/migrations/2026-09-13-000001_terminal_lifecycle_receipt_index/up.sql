CREATE INDEX idx_disclosure_receipts_terminal_lifecycle
ON disclosure_receipts(deployment_id, resource_id, account_id, person_subject_id, lifecycle_successor_id)
WHERE operation = 'activity_upsert'
  AND lifecycle_successor_id IS NOT NULL
  AND outcome = 'accepted';
