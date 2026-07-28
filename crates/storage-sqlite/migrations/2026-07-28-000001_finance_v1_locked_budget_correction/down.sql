-- This data correction remaps durable classifications and cannot safely be reversed.
-- Diesel executes migrations transactionally; the abort rolls back this guard trigger too.
CREATE TRIGGER finance_v1_locked_budget_correction_down_abort
BEFORE INSERT ON finance_v1_locked_budget_objects
BEGIN
  SELECT RAISE(ABORT, 'Finance v1 locked budget correction is irreversible');
END;
INSERT INTO finance_v1_locked_budget_objects(object_type, object_id)
VALUES ('migration', 'finance_v1_locked_budget_correction');
