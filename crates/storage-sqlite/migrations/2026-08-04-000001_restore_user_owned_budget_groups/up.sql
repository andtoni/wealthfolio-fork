-- The spending module's initial budget seed was a product default, not a
-- permanent user-owned budget. Remove that configuration while preserving all
-- taxonomies, categories, activities, and transaction classifications.
--
-- Keep only the Other catch-all group. It is an ordinary editable row; the
-- budget service recreates it only when no fallback exists.

-- Remove the Finance-v1 hard-coded category targets.
DELETE FROM budget_targets
WHERE id LIKE 'finance_v1_target_%'
  AND updated_at = created_at;

-- A system flag records the group's origin, not whether a user later attached
-- budget state to it. Preserve any such group as an ordinary user-owned group;
-- otherwise its targets, rollover settings, and assignments would be lost.
UPDATE budget_groups
SET is_system = 0
WHERE is_system = 1
  AND key <> 'other'
  AND (
      EXISTS (
          SELECT 1 FROM budget_targets
          WHERE budget_targets.group_id = budget_groups.id
            AND budget_targets.id NOT LIKE 'finance_v1_target_%'
      )
      OR EXISTS (
          SELECT 1 FROM budget_rollover_settings
          WHERE budget_rollover_settings.group_id = budget_groups.id
      )
      OR EXISTS (
          SELECT 1 FROM budget_group_assignments
          WHERE budget_group_assignments.group_id = budget_groups.id
            AND budget_group_assignments.is_system = 0
      )
  );

-- Remove targets and rollover settings belonging only to seeded groups that will
-- be removed below. Migrations run with foreign keys disabled, so do this
-- explicitly rather than relying on ON DELETE CASCADE.
DELETE FROM budget_targets
WHERE group_id IN (
    SELECT id FROM budget_groups WHERE is_system = 1 AND key <> 'other'
);
DELETE FROM budget_rollover_settings
WHERE group_id IN (
    SELECT id FROM budget_groups WHERE is_system = 1 AND key <> 'other'
);

-- Remove only seeded category-to-group placement from groups that are actually
-- being removed. Preserved groups retain their complete editable plan state.
DELETE FROM budget_group_assignments
WHERE group_id IN (
    SELECT id FROM budget_groups WHERE is_system = 1 AND key <> 'other'
);

-- Remove the seeded canonical groups with no user-owned plan state, but retain
-- the Other fallback and any group preserved above.
DELETE FROM budget_groups
WHERE is_system = 1 AND key <> 'other';

-- Existing groups are now ordinary user-owned budget data. Seeded category
-- placements are not user-authored plan state; remove them so the surviving
-- fallback group does not recreate the deleted canonical grouping.
UPDATE budget_groups SET is_system = 0;
DELETE FROM budget_group_assignments WHERE is_system = 1;

-- Databases created by an older/custom migration may not have an Other row.
INSERT INTO budget_groups
    (id, name, key, color, icon, sort_order, is_system)
SELECT
    '6e25d097-0c73-4521-9407-d47e8dfb73e2',
    'Other',
    'other',
    '#9C998E',
    'MoreHorizontal',
    99,
    0
WHERE NOT EXISTS (
    SELECT 1 FROM budget_groups WHERE key = 'other'
);
