-- Budget targets are re-keyed to recurring period start dates, so reversing this
-- migration would lose valid weekly and biweekly history.
CREATE TRIGGER configurable_budget_cycles_down_abort
BEFORE INSERT ON finance_v1_locked_budget_objects
BEGIN
  SELECT RAISE(ABORT, 'Configurable budget cycles migration is irreversible');
END;
INSERT INTO finance_v1_locked_budget_objects(object_type, object_id)
VALUES ('migration', 'configurable_budget_cycles');
