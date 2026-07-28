-- Correct the published Finance-v1 seed without rewriting its migration. This is a
-- one-way data correction for the reserved profile; all dependent category references
-- move before duplicate categories are removed.
DROP TRIGGER IF EXISTS finance_v1_lock_target_update;
DROP TRIGGER IF EXISTS finance_v1_lock_target_delete;
DROP TRIGGER IF EXISTS finance_v1_lock_category_update;
DROP TRIGGER IF EXISTS finance_v1_lock_category_delete;
DROP TRIGGER IF EXISTS finance_v1_lock_assignment_update;
DROP TRIGGER IF EXISTS finance_v1_lock_assignment_delete;

-- Reuse native category identities where one already exists. Finance-specific
-- Insurance and Connectivity remain because the native taxonomy has no equivalent
-- top-level identity.
UPDATE activity_taxonomy_assignments SET category_id='cat_bills'
 WHERE taxonomy_id='spending_categories' AND category_id IN ('finance_v1_cat_fees_bills','cat_fees');
UPDATE activity_taxonomy_assignments SET category_id='cat_gifts'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_giving';
UPDATE activity_taxonomy_assignments SET category_id='cat_food'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_dining';
UPDATE activity_taxonomy_assignments SET category_id='cat_other_expense'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_miscellaneous';
UPDATE spending_categorization_evidence SET category_id='cat_bills'
 WHERE taxonomy_id='spending_categories' AND category_id IN ('finance_v1_cat_fees_bills','cat_fees');
UPDATE spending_categorization_evidence SET category_id='cat_gifts'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_giving';
UPDATE spending_categorization_evidence SET category_id='cat_food'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_dining';
UPDATE spending_categorization_evidence SET category_id='cat_other_expense'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_miscellaneous';
UPDATE spending_categorization_rules SET category_id='cat_bills'
 WHERE taxonomy_id='spending_categories' AND category_id IN ('finance_v1_cat_fees_bills','cat_fees');
UPDATE spending_categorization_rules SET category_id='cat_gifts'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_giving';
UPDATE spending_categorization_rules SET category_id='cat_food'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_dining';
UPDATE spending_categorization_rules SET category_id='cat_other_expense'
 WHERE taxonomy_id='spending_categories' AND category_id='finance_v1_cat_miscellaneous';

UPDATE taxonomy_categories SET name='Transport' WHERE taxonomy_id='spending_categories' AND id='cat_transport';
UPDATE taxonomy_categories SET name='Fees & Bills' WHERE taxonomy_id='spending_categories' AND id='cat_bills';
UPDATE taxonomy_categories SET name='Giving' WHERE taxonomy_id='spending_categories' AND id='cat_gifts';
UPDATE taxonomy_categories SET name='Dining' WHERE taxonomy_id='spending_categories' AND id='cat_food';
UPDATE taxonomy_categories SET name='Miscellaneous' WHERE taxonomy_id='spending_categories' AND id='cat_other_expense';
UPDATE taxonomy_categories SET parent_id='finance_v1_cat_insurance'
 WHERE taxonomy_id='spending_categories' AND id IN ('cat_housing_insurance','cat_transport_insurance','cat_health_insurance');
UPDATE taxonomy_categories SET parent_id='finance_v1_cat_connectivity'
 WHERE taxonomy_id='spending_categories' AND id IN ('cat_bills_phone','cat_bills_internet');
UPDATE taxonomy_categories SET parent_id='cat_bills'
 WHERE taxonomy_id='spending_categories' AND id IN ('cat_fees_bank','cat_fees_atm','cat_fees_interest','cat_fees_late','cat_bills_software');
UPDATE taxonomy_categories SET parent_id=NULL WHERE taxonomy_id='spending_categories' AND id IN ('cat_bills_subscriptions','cat_food_delivery');
UPDATE taxonomy_categories SET parent_id=NULL WHERE taxonomy_id='income_sources' AND id='cat_income_salary';

-- Remove duplicate Finance top levels and the now-empty duplicate fee parent.
DELETE FROM budget_group_assignments WHERE category_id IN ('finance_v1_cat_fees_bills','finance_v1_cat_giving','finance_v1_cat_dining','finance_v1_cat_miscellaneous','cat_fees');
DELETE FROM budget_targets WHERE category_id IN ('finance_v1_cat_fees_bills','finance_v1_cat_giving','finance_v1_cat_dining','finance_v1_cat_miscellaneous','cat_fees');
DELETE FROM budget_rollover_settings WHERE category_id IN ('finance_v1_cat_fees_bills','finance_v1_cat_giving','finance_v1_cat_dining','finance_v1_cat_miscellaneous','cat_fees');
DELETE FROM taxonomy_categories WHERE taxonomy_id='spending_categories' AND id IN ('finance_v1_cat_fees_bills','finance_v1_cat_giving','finance_v1_cat_dining','finance_v1_cat_miscellaneous','cat_fees');

-- Clear stale rollover state. The service exposes only Needs, Wants, and Other
-- for this profile while retaining old system rows for sync/foreign-key compatibility.
DELETE FROM budget_rollover_settings;

-- Reset only the Finance-v1 managed targets, then point them at renderable top-level
-- categories. This makes the native service's totals exactly 30k Needs + 10k Wants.
DELETE FROM budget_targets WHERE id LIKE 'finance_v1_target_%';
INSERT INTO budget_targets (id,period_key,target_type,taxonomy_id,category_id,group_id,amount) VALUES
 ('finance_v1_target_salary','default','category','income_sources','cat_income_salary',NULL,'60000'),
 ('finance_v1_target_other_income','default','category','income_sources','cat_income_other',NULL,'0'),
 ('finance_v1_target_housing','default','category','spending_categories','cat_housing',NULL,'10000'),
 ('finance_v1_target_groceries','default','category','spending_categories','cat_groceries',NULL,'6000'),
 ('finance_v1_target_transport','default','category','spending_categories','cat_transport',NULL,'10000'),
 ('finance_v1_target_insurance','default','category','spending_categories','finance_v1_cat_insurance',NULL,'5000'),
 ('finance_v1_target_fees_bills','default','category','spending_categories','cat_bills',NULL,'1000'),
 ('finance_v1_target_giving','default','category','spending_categories','cat_gifts',NULL,'2000'),
 ('finance_v1_target_connectivity','default','category','spending_categories','finance_v1_cat_connectivity',NULL,'3000'),
 ('finance_v1_target_dining','default','category','spending_categories','cat_food',NULL,'4000'),
 ('finance_v1_target_subscriptions','default','category','spending_categories','cat_bills_subscriptions',NULL,'2000'),
 ('finance_v1_target_entertainment','default','category','spending_categories','cat_entertainment',NULL,'1000'),
 ('finance_v1_target_miscellaneous','default','category','spending_categories','cat_other_expense',NULL,'4000'),
 ('finance_v1_target_food_delivery','default','category','spending_categories','cat_food_delivery',NULL,'2000');

-- Assign the final planned categories to Needs/Wants; every remaining top-level
-- spending category is actual-only Other.
INSERT INTO budget_group_assignments (id,group_id,taxonomy_id,category_id,is_system) VALUES
 ('finance_v1_assign_insurance','032ecb02-5912-42e8-9724-2cd566fc08d5','spending_categories','finance_v1_cat_insurance',1),
 ('finance_v1_assign_connectivity','032ecb02-5912-42e8-9724-2cd566fc08d5','spending_categories','finance_v1_cat_connectivity',1),
 ('finance_v1_assign_subscriptions','a409e0d6-9152-49c8-a5b4-a147a8ac636e','spending_categories','cat_bills_subscriptions',1),
 ('finance_v1_assign_food_delivery','a409e0d6-9152-49c8-a5b4-a147a8ac636e','spending_categories','cat_food_delivery',1)
ON CONFLICT(taxonomy_id,category_id) DO UPDATE SET group_id=excluded.group_id,is_system=1;
UPDATE budget_group_assignments SET group_id='032ecb02-5912-42e8-9724-2cd566fc08d5',is_system=1
 WHERE taxonomy_id='spending_categories' AND category_id IN ('cat_housing','cat_groceries','cat_transport','cat_bills','cat_gifts');
UPDATE budget_group_assignments SET group_id='a409e0d6-9152-49c8-a5b4-a147a8ac636e',is_system=1
 WHERE taxonomy_id='spending_categories' AND category_id IN ('cat_food','cat_entertainment','cat_other_expense');
UPDATE budget_group_assignments SET group_id='6e25d097-0c73-4521-9407-d47e8dfb73e2',is_system=1
 WHERE taxonomy_id='spending_categories' AND category_id IN (
   SELECT id FROM taxonomy_categories WHERE taxonomy_id='spending_categories' AND parent_id IS NULL
 ) AND category_id NOT IN ('cat_housing','cat_groceries','cat_transport','finance_v1_cat_insurance','cat_bills','cat_gifts','finance_v1_cat_connectivity','cat_food','cat_bills_subscriptions','cat_entertainment','cat_other_expense','cat_food_delivery');

DELETE FROM finance_v1_locked_budget_objects;
INSERT INTO finance_v1_locked_budget_objects(object_type,object_id)
 SELECT 'category',id FROM taxonomy_categories WHERE taxonomy_id='spending_categories' AND id IN ('finance_v1_cat_insurance','finance_v1_cat_connectivity','cat_housing','cat_groceries','cat_transport','cat_bills','cat_gifts','cat_food','cat_bills_subscriptions','cat_entertainment','cat_other_expense','cat_food_delivery')
 UNION ALL SELECT 'target',id FROM budget_targets WHERE id LIKE 'finance_v1_target_%'
 UNION ALL SELECT 'assignment',id FROM budget_group_assignments WHERE taxonomy_id='spending_categories' AND category_id IN ('finance_v1_cat_insurance','finance_v1_cat_connectivity','cat_housing','cat_groceries','cat_transport','cat_bills','cat_gifts','cat_food','cat_bills_subscriptions','cat_entertainment','cat_other_expense','cat_food_delivery');

CREATE TRIGGER finance_v1_no_rollover_insert BEFORE INSERT ON budget_rollover_settings BEGIN SELECT RAISE(ABORT,'Finance v1 budget rollover is disabled'); END;
CREATE TRIGGER finance_v1_no_rollover_update BEFORE UPDATE ON budget_rollover_settings BEGIN SELECT RAISE(ABORT,'Finance v1 budget rollover is disabled'); END;
CREATE TRIGGER finance_v1_lock_target_update BEFORE UPDATE ON budget_targets WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='target' AND object_id=OLD.id) BEGIN SELECT RAISE(ABORT,'Finance v1 budget target is locked'); END;
CREATE TRIGGER finance_v1_lock_target_delete BEFORE DELETE ON budget_targets WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='target' AND object_id=OLD.id) BEGIN SELECT RAISE(ABORT,'Finance v1 budget target is locked'); END;
CREATE TRIGGER finance_v1_lock_category_update BEFORE UPDATE ON taxonomy_categories WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='category' AND object_id=OLD.id) BEGIN SELECT RAISE(ABORT,'Finance v1 budget category is locked'); END;
CREATE TRIGGER finance_v1_lock_category_delete BEFORE DELETE ON taxonomy_categories WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='category' AND object_id=OLD.id) BEGIN SELECT RAISE(ABORT,'Finance v1 budget category is locked'); END;
CREATE TRIGGER finance_v1_lock_assignment_update BEFORE UPDATE ON budget_group_assignments WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='assignment' AND object_id=OLD.id) BEGIN SELECT RAISE(ABORT,'Finance v1 budget assignment is locked'); END;
CREATE TRIGGER finance_v1_lock_assignment_delete BEFORE DELETE ON budget_group_assignments WHEN EXISTS (SELECT 1 FROM finance_v1_locked_budget_objects WHERE object_type='assignment' AND object_id=OLD.id) BEGIN SELECT RAISE(ABORT,'Finance v1 budget assignment is locked'); END;
