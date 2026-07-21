-- Finance v1 locked monthly plan. The canonical namespace is reserved and the
-- guard fails closed rather than overwriting colliding user-owned objects.
CREATE TABLE IF NOT EXISTS finance_v1_budget_policy (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  cycle_start_day INTEGER NOT NULL CHECK (cycle_start_day = 25),
  cycle_end_day INTEGER NOT NULL CHECK (cycle_end_day = 24),
  currency TEXT NOT NULL CHECK (currency = 'ZAR'),
  rollover_enabled INTEGER NOT NULL CHECK (rollover_enabled = 0),
  transfers_excluded INTEGER NOT NULL CHECK (transfers_excluded = 1),
  seeded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TEMP TABLE finance_v1_seed_guard (ok INTEGER NOT NULL CHECK (ok = 1));
INSERT INTO finance_v1_seed_guard
SELECT CASE WHEN
  EXISTS (
    SELECT 1 FROM finance_v1_budget_policy
    WHERE id <> 1 OR cycle_start_day <> 25 OR cycle_end_day <> 24
      OR currency <> 'ZAR' OR rollover_enabled <> 0 OR transfers_excluded <> 1
  )
  OR EXISTS (
    SELECT 1 FROM taxonomy_categories
    WHERE taxonomy_id = 'spending_categories'
      AND name IN ('Insurance','Fees & Bills','Giving','Connectivity','Dining','Miscellaneous')
      AND id NOT IN (
        'finance_v1_cat_insurance','finance_v1_cat_fees_bills','finance_v1_cat_giving',
        'finance_v1_cat_connectivity','finance_v1_cat_dining','finance_v1_cat_miscellaneous'
      )
  )
  OR EXISTS (
    SELECT 1 FROM taxonomy_categories
    WHERE id LIKE 'finance_v1_cat_%' AND NOT (
      taxonomy_id = 'spending_categories' AND (
        (id='finance_v1_cat_insurance' AND name='Insurance' AND key='finance_v1_insurance') OR
        (id='finance_v1_cat_fees_bills' AND name='Fees & Bills' AND key='finance_v1_fees_bills') OR
        (id='finance_v1_cat_giving' AND name='Giving' AND key='finance_v1_giving') OR
        (id='finance_v1_cat_connectivity' AND name='Connectivity' AND key='finance_v1_connectivity') OR
        (id='finance_v1_cat_dining' AND name='Dining' AND key='finance_v1_dining') OR
        (id='finance_v1_cat_miscellaneous' AND name='Miscellaneous' AND key='finance_v1_miscellaneous') -- gitleaks:allow deterministic taxonomy key
      )
    )
  )
  OR EXISTS (
    SELECT 1 FROM budget_targets bt
    WHERE bt.period_key='default' AND bt.target_type='category'
      AND bt.category_id IN (
        'cat_housing','cat_groceries','cat_transport','cat_entertainment',
        'cat_food_delivery','cat_bills_subscriptions','cat_income_salary','cat_income_other',
        'finance_v1_cat_insurance','finance_v1_cat_fees_bills','finance_v1_cat_giving',
        'finance_v1_cat_connectivity','finance_v1_cat_dining','finance_v1_cat_miscellaneous'
      ) AND bt.id NOT LIKE 'finance_v1_target_%'
  )
  OR EXISTS (
    SELECT 1 FROM budget_targets
    WHERE id LIKE 'finance_v1_target_%' AND NOT (
      period_key='default' AND target_type='category' AND group_id IS NULL AND (
        (id='finance_v1_target_salary' AND taxonomy_id='income_sources' AND category_id='cat_income_salary' AND amount='60000') OR
        (id='finance_v1_target_other_income' AND taxonomy_id='income_sources' AND category_id='cat_income_other' AND amount='0') OR
        (id='finance_v1_target_housing' AND taxonomy_id='spending_categories' AND category_id='cat_housing' AND amount='10000') OR
        (id='finance_v1_target_groceries' AND taxonomy_id='spending_categories' AND category_id='cat_groceries' AND amount='6000') OR
        (id='finance_v1_target_transport' AND taxonomy_id='spending_categories' AND category_id='cat_transport' AND amount='10000') OR
        (id='finance_v1_target_insurance' AND taxonomy_id='spending_categories' AND category_id='finance_v1_cat_insurance' AND amount='5000') OR
        (id='finance_v1_target_fees_bills' AND taxonomy_id='spending_categories' AND category_id='finance_v1_cat_fees_bills' AND amount='1000') OR
        (id='finance_v1_target_giving' AND taxonomy_id='spending_categories' AND category_id='finance_v1_cat_giving' AND amount='2000') OR
        (id='finance_v1_target_connectivity' AND taxonomy_id='spending_categories' AND category_id='finance_v1_cat_connectivity' AND amount='3000') OR
        (id='finance_v1_target_dining' AND taxonomy_id='spending_categories' AND category_id='finance_v1_cat_dining' AND amount='4000') OR
        (id='finance_v1_target_subscriptions' AND taxonomy_id='spending_categories' AND category_id='cat_bills_subscriptions' AND amount='2000') OR
        (id='finance_v1_target_entertainment' AND taxonomy_id='spending_categories' AND category_id='cat_entertainment' AND amount='1000') OR
        (id='finance_v1_target_miscellaneous' AND taxonomy_id='spending_categories' AND category_id='finance_v1_cat_miscellaneous' AND amount='4000') OR
        (id='finance_v1_target_food_delivery' AND taxonomy_id='spending_categories' AND category_id='cat_food_delivery' AND amount='2000')
      )
    )
  )
  OR EXISTS (
    SELECT 1 FROM budget_group_assignments
    WHERE category_id LIKE 'finance_v1_cat_%' AND id NOT LIKE 'finance_v1_assign_%'
  )
  OR EXISTS (
    SELECT 1 FROM budget_group_assignments
    WHERE id LIKE 'finance_v1_assign_%' AND NOT (
      taxonomy_id='spending_categories' AND is_system=1 AND (
        (id='finance_v1_assign_insurance' AND group_id='032ecb02-5912-42e8-9724-2cd566fc08d5' AND category_id='finance_v1_cat_insurance') OR
        (id='finance_v1_assign_fees_bills' AND group_id='032ecb02-5912-42e8-9724-2cd566fc08d5' AND category_id='finance_v1_cat_fees_bills') OR
        (id='finance_v1_assign_connectivity' AND group_id='032ecb02-5912-42e8-9724-2cd566fc08d5' AND category_id='finance_v1_cat_connectivity') OR
        (id='finance_v1_assign_giving' AND group_id='8cbd26c8-e3b2-4176-8c61-e5c11e10b808' AND category_id='finance_v1_cat_giving') OR
        (id='finance_v1_assign_dining' AND group_id='a409e0d6-9152-49c8-a5b4-a147a8ac636e' AND category_id='finance_v1_cat_dining') OR
        (id='finance_v1_assign_miscellaneous' AND group_id='a409e0d6-9152-49c8-a5b4-a147a8ac636e' AND category_id='finance_v1_cat_miscellaneous')
      )
    )
  )
THEN 0 ELSE 1 END;
DROP TABLE finance_v1_seed_guard;

INSERT INTO finance_v1_budget_policy
  (id, cycle_start_day, cycle_end_day, currency, rollover_enabled, transfers_excluded)
VALUES (1, 25, 24, 'ZAR', 0, 1)
ON CONFLICT(id) DO NOTHING;

INSERT INTO taxonomy_categories
  (id, taxonomy_id, parent_id, name, key, color, icon, sort_order) VALUES
  ('finance_v1_cat_insurance', 'spending_categories', NULL, 'Insurance', 'finance_v1_insurance', '#4F6B92', 'Shield', 15),
  ('finance_v1_cat_fees_bills', 'spending_categories', NULL, 'Fees & Bills', 'finance_v1_fees_bills', '#4F6B92', 'Receipt', 16),
  ('finance_v1_cat_giving', 'spending_categories', NULL, 'Giving', 'finance_v1_giving', '#A35742', 'Gift', 17),
  ('finance_v1_cat_connectivity', 'spending_categories', NULL, 'Connectivity', 'finance_v1_connectivity', '#4F6B92', 'Wifi', 18),
  ('finance_v1_cat_dining', 'spending_categories', NULL, 'Dining', 'finance_v1_dining', '#8E7CB3', 'UtensilsCrossed', 19),
  ('finance_v1_cat_miscellaneous', 'spending_categories', NULL, 'Miscellaneous', 'finance_v1_miscellaneous', '#9C998E', 'MoreHorizontal', 20)
ON CONFLICT(taxonomy_id, id) DO NOTHING;

INSERT INTO budget_group_assignments
  (id, group_id, taxonomy_id, category_id, is_system) VALUES
  ('finance_v1_assign_insurance', '032ecb02-5912-42e8-9724-2cd566fc08d5', 'spending_categories', 'finance_v1_cat_insurance', 1),
  ('finance_v1_assign_fees_bills', '032ecb02-5912-42e8-9724-2cd566fc08d5', 'spending_categories', 'finance_v1_cat_fees_bills', 1),
  ('finance_v1_assign_connectivity', '032ecb02-5912-42e8-9724-2cd566fc08d5', 'spending_categories', 'finance_v1_cat_connectivity', 1),
  ('finance_v1_assign_giving', '8cbd26c8-e3b2-4176-8c61-e5c11e10b808', 'spending_categories', 'finance_v1_cat_giving', 1),
  ('finance_v1_assign_dining', 'a409e0d6-9152-49c8-a5b4-a147a8ac636e', 'spending_categories', 'finance_v1_cat_dining', 1),
  ('finance_v1_assign_miscellaneous', 'a409e0d6-9152-49c8-a5b4-a147a8ac636e', 'spending_categories', 'finance_v1_cat_miscellaneous', 1)
ON CONFLICT(id) DO NOTHING;

INSERT INTO budget_targets
  (id, period_key, target_type, taxonomy_id, category_id, group_id, amount) VALUES
  ('finance_v1_target_salary', 'default', 'category', 'income_sources', 'cat_income_salary', NULL, '60000'),
  ('finance_v1_target_other_income', 'default', 'category', 'income_sources', 'cat_income_other', NULL, '0'),
  ('finance_v1_target_housing', 'default', 'category', 'spending_categories', 'cat_housing', NULL, '10000'),
  ('finance_v1_target_groceries', 'default', 'category', 'spending_categories', 'cat_groceries', NULL, '6000'),
  ('finance_v1_target_transport', 'default', 'category', 'spending_categories', 'cat_transport', NULL, '10000'),
  ('finance_v1_target_insurance', 'default', 'category', 'spending_categories', 'finance_v1_cat_insurance', NULL, '5000'),
  ('finance_v1_target_fees_bills', 'default', 'category', 'spending_categories', 'finance_v1_cat_fees_bills', NULL, '1000'),
  ('finance_v1_target_giving', 'default', 'category', 'spending_categories', 'finance_v1_cat_giving', NULL, '2000'),
  ('finance_v1_target_connectivity', 'default', 'category', 'spending_categories', 'finance_v1_cat_connectivity', NULL, '3000'),
  ('finance_v1_target_dining', 'default', 'category', 'spending_categories', 'finance_v1_cat_dining', NULL, '4000'),
  ('finance_v1_target_subscriptions', 'default', 'category', 'spending_categories', 'cat_bills_subscriptions', NULL, '2000'),
  ('finance_v1_target_entertainment', 'default', 'category', 'spending_categories', 'cat_entertainment', NULL, '1000'),
  ('finance_v1_target_miscellaneous', 'default', 'category', 'spending_categories', 'finance_v1_cat_miscellaneous', NULL, '4000'),
  ('finance_v1_target_food_delivery', 'default', 'category', 'spending_categories', 'cat_food_delivery', NULL, '2000')
ON CONFLICT(id) DO NOTHING;

CREATE TABLE IF NOT EXISTS finance_v1_locked_budget_objects (
  object_type TEXT NOT NULL CHECK (object_type IN ('category','target','assignment')),
  object_id TEXT NOT NULL,
  PRIMARY KEY (object_type, object_id)
);
INSERT OR IGNORE INTO finance_v1_locked_budget_objects(object_type, object_id)
SELECT 'category', id FROM taxonomy_categories WHERE id LIKE 'finance_v1_cat_%'
UNION ALL SELECT 'target', id FROM budget_targets WHERE id LIKE 'finance_v1_target_%'
UNION ALL SELECT 'assignment', id FROM budget_group_assignments WHERE id LIKE 'finance_v1_assign_%';

CREATE TRIGGER IF NOT EXISTS finance_v1_lock_target_update BEFORE UPDATE ON budget_targets
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='target' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget target is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_target_delete BEFORE DELETE ON budget_targets
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='target' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget target is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_category_update BEFORE UPDATE ON taxonomy_categories
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='category' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget category is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_category_delete BEFORE DELETE ON taxonomy_categories
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='category' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget category is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_assignment_update BEFORE UPDATE ON budget_group_assignments
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='assignment' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget assignment is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_assignment_delete BEFORE DELETE ON budget_group_assignments
WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='assignment' AND object_id=OLD.id)
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget assignment is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_policy_update BEFORE UPDATE ON finance_v1_budget_policy
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget policy is locked'); END;
CREATE TRIGGER IF NOT EXISTS finance_v1_lock_policy_delete BEFORE DELETE ON finance_v1_budget_policy
BEGIN SELECT RAISE(ABORT, 'Finance v1 budget policy is locked'); END;
