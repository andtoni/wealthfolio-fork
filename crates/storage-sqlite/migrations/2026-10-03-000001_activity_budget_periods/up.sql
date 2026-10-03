-- Lets the user count a transaction toward a budget period other than the one
-- its date falls in (e.g. groceries bought two days before the next period
-- starts). The activity itself, including its date, is never changed; only
-- budget and insight aggregation read this overlay.
--
-- `period_start` is the start date (YYYY-MM-DD) of the chosen budget period.
-- It is resolved against the current budget cycle at read time, so a later
-- cycle change needs no rewrite here.
CREATE TABLE spending_activity_budget_periods (
    activity_id  TEXT NOT NULL PRIMARY KEY,
    period_start TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (activity_id) REFERENCES activities(id) ON DELETE CASCADE
);
