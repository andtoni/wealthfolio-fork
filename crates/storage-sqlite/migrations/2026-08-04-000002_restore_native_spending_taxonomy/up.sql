-- Restore the native spending taxonomy after Finance-v1 changed canonical labels,
-- hierarchy, and category identities. Classification references are remapped before
-- the Finance-v1-only categories are removed; raw activities are untouched.

CREATE TEMP TABLE native_category_remap (
    old_id TEXT NOT NULL PRIMARY KEY,
    new_id TEXT NOT NULL
);

INSERT INTO native_category_remap (old_id, new_id) VALUES
    ('finance_v1_cat_fees_bills', 'cat_bills'),
    ('finance_v1_cat_giving', 'cat_gifts'),
    ('finance_v1_cat_dining', 'cat_food'),
    ('finance_v1_cat_miscellaneous', 'cat_other_expense'),
    -- The native taxonomy has no generic Insurance or Connectivity parent.
    -- Their detailed children are restored below; generic references belong in
    -- the clean Bills & Utilities bucket.
    ('finance_v1_cat_insurance', 'cat_bills'),
    ('finance_v1_cat_connectivity', 'cat_bills');

-- Restore the native Fees & Charges parent deleted by Finance-v1 correction.
INSERT OR IGNORE INTO taxonomy_categories
    (id, taxonomy_id, parent_id, name, key, color, icon, sort_order)
VALUES
    ('cat_fees', 'spending_categories', NULL, 'Fees & Charges', 'fees', '#9C998E', 'CreditCard', 13);

-- Restore native category names and hierarchy. These are stable native IDs; the
-- Finance-v1 labels were written onto them by the earlier correction migration.
-- Finance-v1 changed only these native labels. Restore a label only while the
-- row still has that exact Finance-v1 value, leaving any later user edits
-- untouched (the editable-category migration precedes this one).
UPDATE taxonomy_categories
SET name='Transportation'
WHERE taxonomy_id='spending_categories' AND id='cat_transport' AND name='Transport';
UPDATE taxonomy_categories
SET name='Bills & Utilities'
WHERE taxonomy_id='spending_categories' AND id='cat_bills' AND name='Fees & Bills';
UPDATE taxonomy_categories
SET name='Gifts & Donations'
WHERE taxonomy_id='spending_categories' AND id='cat_gifts' AND name='Giving';
UPDATE taxonomy_categories
SET name='Food & Dining'
WHERE taxonomy_id='spending_categories' AND id='cat_food' AND name='Dining';
UPDATE taxonomy_categories
SET name='Other Expenses'
WHERE taxonomy_id='spending_categories' AND id='cat_other_expense' AND name='Miscellaneous';

-- Restore native parents only while the row still has the Finance-v1 parent.
-- A different parent is an explicit user edit and must survive the upgrade.
UPDATE taxonomy_categories SET parent_id='cat_housing'
WHERE taxonomy_id='spending_categories' AND id='cat_housing_insurance'
  AND parent_id='finance_v1_cat_insurance';
UPDATE taxonomy_categories SET parent_id='cat_transport'
WHERE taxonomy_id='spending_categories' AND id='cat_transport_insurance'
  AND parent_id='finance_v1_cat_insurance';
UPDATE taxonomy_categories SET parent_id='cat_health'
WHERE taxonomy_id='spending_categories' AND id='cat_health_insurance'
  AND parent_id='finance_v1_cat_insurance';
UPDATE taxonomy_categories SET parent_id='cat_bills'
WHERE taxonomy_id='spending_categories'
  AND id IN ('cat_bills_phone', 'cat_bills_internet')
  AND parent_id='finance_v1_cat_connectivity';
UPDATE taxonomy_categories SET parent_id='cat_bills'
WHERE taxonomy_id='spending_categories' AND id='cat_bills_subscriptions'
  AND parent_id IS NULL;
UPDATE taxonomy_categories SET parent_id='cat_food'
WHERE taxonomy_id='spending_categories' AND id='cat_food_delivery'
  AND parent_id IS NULL;
UPDATE taxonomy_categories SET parent_id='cat_fees'
WHERE taxonomy_id='spending_categories'
  AND id IN ('cat_fees_bank', 'cat_fees_atm', 'cat_fees_interest', 'cat_fees_late')
  AND parent_id='cat_bills';
UPDATE taxonomy_categories SET parent_id='cat_income_employment'
WHERE taxonomy_id='income_sources' AND id='cat_income_salary'
  AND parent_id IS NULL;

-- An activity has at most one assignment per taxonomy. If both an old and clean
-- assignment exist, retain the already-clean row and remove only the duplicate
-- old reference; otherwise update the old row in place.
DELETE FROM activity_taxonomy_assignments
WHERE id IN (
    SELECT old.id
    FROM activity_taxonomy_assignments old
    JOIN native_category_remap remap
      ON remap.old_id=old.category_id
    WHERE old.taxonomy_id='spending_categories'
      AND EXISTS (
          SELECT 1 FROM activity_taxonomy_assignments clean
          WHERE clean.activity_id=old.activity_id
            AND clean.taxonomy_id=old.taxonomy_id
            AND clean.category_id=remap.new_id
      )
);
UPDATE activity_taxonomy_assignments
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=activity_taxonomy_assignments.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);

-- Remap every persisted classification sidecar without changing activities or
-- provider facts.
UPDATE spending_activity_splits
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=spending_activity_splits.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);
UPDATE spending_categorization_evidence
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=spending_categorization_evidence.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);
UPDATE spending_categorization_rules
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=spending_categorization_rules.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);

-- Remap any user-owned budget references left behind by the previous migration.
-- Seeded Finance-v1 rows are removed by the preceding budget cleanup migration.
DELETE FROM budget_group_assignments
WHERE id IN (
    SELECT old.id
    FROM budget_group_assignments old
    JOIN native_category_remap remap
      ON remap.old_id=old.category_id
    WHERE old.taxonomy_id='spending_categories'
      AND EXISTS (
          SELECT 1 FROM budget_group_assignments clean
          WHERE clean.taxonomy_id=old.taxonomy_id
            AND clean.category_id=remap.new_id
      )
);
UPDATE budget_group_assignments
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=budget_group_assignments.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);

-- Category targets have a unique period/category key. When both the legacy and
-- native category had a user target, merge their Decimal strings instead of
-- silently dropping one of the user's planned amounts. The digit-wise CTE
-- avoids binary floating-point rounding while adding the persisted values.
CREATE TEMP TABLE native_budget_target_merges AS
WITH RECURSIVE
pairs AS (
    SELECT old.id AS old_id, clean.id AS clean_id,
           old.amount AS old_amount, clean.amount AS clean_amount
    FROM budget_targets old
    JOIN native_category_remap remap ON remap.old_id=old.category_id
    JOIN budget_targets clean
      ON clean.period_key=old.period_key
     AND clean.target_type=old.target_type
     AND clean.taxonomy_id=old.taxonomy_id
     AND clean.category_id=remap.new_id
    WHERE old.taxonomy_id='spending_categories'
),
parts AS (
    SELECT old_id, clean_id,
           CASE WHEN instr(old_amount, '.')=0 THEN old_amount
                ELSE substr(old_amount, 1, instr(old_amount, '.')-1) END AS old_int,
           CASE WHEN instr(old_amount, '.')=0 THEN ''
                ELSE substr(old_amount, instr(old_amount, '.')+1) END AS old_frac,
           CASE WHEN instr(clean_amount, '.')=0 THEN clean_amount
                ELSE substr(clean_amount, 1, instr(clean_amount, '.')-1) END AS clean_int,
           CASE WHEN instr(clean_amount, '.')=0 THEN ''
                ELSE substr(clean_amount, instr(clean_amount, '.')+1) END AS clean_frac
    FROM pairs
),
dimensions AS (
    SELECT *, max(length(old_int), length(clean_int)) AS integer_width,
           max(length(old_frac), length(clean_frac)) AS scale
    FROM parts
),
padded AS (
    SELECT *,
           replace(printf('%*s', integer_width-length(old_int), ''), ' ', '0')
             || old_int || old_frac
             || replace(printf('%*s', scale-length(old_frac), ''), ' ', '0') AS old_digits,
           replace(printf('%*s', integer_width-length(clean_int), ''), ' ', '0')
             || clean_int || clean_frac
             || replace(printf('%*s', scale-length(clean_frac), ''), ' ', '0') AS clean_digits
    FROM dimensions
),
digits(old_id, clean_id, old_digits, clean_digits, scale, pos, carry, result) AS (
    SELECT old_id, clean_id, old_digits, clean_digits, scale,
           length(old_digits), 0, ''
    FROM padded
    UNION ALL
    SELECT old_id, clean_id, old_digits, clean_digits, scale, pos-1,
           (CAST(substr(old_digits, pos, 1) AS INTEGER)
            + CAST(substr(clean_digits, pos, 1) AS INTEGER) + carry) / 10,
           printf('%d', (CAST(substr(old_digits, pos, 1) AS INTEGER)
            + CAST(substr(clean_digits, pos, 1) AS INTEGER) + carry) % 10) || result
    FROM digits
    WHERE pos > 0
),
raw_sums AS (
    SELECT old_id, clean_id, scale,
           CASE WHEN carry > 0 THEN CAST(carry AS TEXT) || result ELSE result END AS summed
    FROM digits
    WHERE pos=0
)
SELECT old_id, clean_id,
       CASE
         WHEN scale=0 THEN COALESCE(NULLIF(ltrim(summed, '0'), ''), '0')
         WHEN rtrim(substr(summed, 1, length(summed)-scale), '0')='' THEN
           '0' || CASE
             WHEN rtrim(substr(summed, length(summed)-scale+1), '0')='' THEN ''
             ELSE '.' || rtrim(substr(summed, length(summed)-scale+1), '0')
           END
         ELSE
           COALESCE(NULLIF(ltrim(substr(summed, 1, length(summed)-scale), '0'), ''), '0')
           || CASE
             WHEN rtrim(substr(summed, length(summed)-scale+1), '0')='' THEN ''
             ELSE '.' || rtrim(substr(summed, length(summed)-scale+1), '0')
           END
       END AS amount
FROM raw_sums;

UPDATE budget_targets
SET amount=(
    SELECT amount FROM native_budget_target_merges
    WHERE clean_id=budget_targets.id
)
WHERE id IN (SELECT clean_id FROM native_budget_target_merges);
DELETE FROM budget_targets
WHERE id IN (SELECT old_id FROM native_budget_target_merges);
UPDATE budget_targets
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=budget_targets.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);
DROP TABLE native_budget_target_merges;

DELETE FROM budget_rollover_settings
WHERE id IN (
    SELECT old.id
    FROM budget_rollover_settings old
    JOIN native_category_remap remap
      ON remap.old_id=old.category_id
    WHERE old.taxonomy_id='spending_categories'
      AND EXISTS (
          SELECT 1 FROM budget_rollover_settings clean
          WHERE clean.taxonomy_id=old.taxonomy_id
            AND clean.category_id=remap.new_id
      )
);
UPDATE budget_rollover_settings
SET category_id=(
    SELECT new_id FROM native_category_remap
    WHERE old_id=budget_rollover_settings.category_id
)
WHERE taxonomy_id='spending_categories'
  AND category_id IN (SELECT old_id FROM native_category_remap);

-- Remove only duplicate Finance-v1 taxonomy nodes after all references have
-- moved. Native categories, child categories, activities, and classifications
-- remain intact.
DELETE FROM taxonomy_categories
WHERE taxonomy_id='spending_categories'
  AND id IN (SELECT old_id FROM native_category_remap);

DROP TABLE native_category_remap;
