-- Allow a budget group to remain visible in spending reports while being
-- omitted from budget totals, actuals, pace, and rollover calculations.
ALTER TABLE budget_groups
ADD COLUMN exclude_from_budget INTEGER NOT NULL DEFAULT 0;

-- The catch-all group represents unplanned spending by default. Preserve it in
-- the report/'Where it went' views, but do not let it make every budget cycle
-- appear overspent until the user explicitly opts it back in.
UPDATE budget_groups
SET exclude_from_budget = 1
WHERE key = 'other';
