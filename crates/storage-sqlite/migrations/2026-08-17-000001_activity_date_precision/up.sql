ALTER TABLE activities ADD COLUMN activity_date_precision TEXT NOT NULL DEFAULT 'INSTANT';

UPDATE activities
SET activity_date_precision = CASE
    WHEN length(activity_date) = 10 THEN 'DATE'
    ELSE 'INSTANT'
END,
activity_date = CASE
    WHEN length(activity_date) = 10 THEN activity_date || 'T00:00:00.000Z'
    ELSE strftime('%Y-%m-%dT%H:%M:%fZ', activity_date)
END;
