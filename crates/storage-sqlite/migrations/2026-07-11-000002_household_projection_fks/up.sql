-- Add composite foreign keys on activity_disclosure_projections for
-- (deployment_id, person_subject_id) and (deployment_id, account_id).
-- SQLite cannot ALTER TABLE ADD CONSTRAINT, so we recreate the table.

CREATE TABLE activity_disclosure_projections_new (
 deployment_id TEXT NOT NULL,
 person_subject_id TEXT NOT NULL,
 activity_id TEXT NOT NULL,
 account_id TEXT NOT NULL,
 access_generation BIGINT NOT NULL,
 PRIMARY KEY(deployment_id,person_subject_id,activity_id),
 FOREIGN KEY(activity_id) REFERENCES activities(id) ON DELETE CASCADE,
 FOREIGN KEY(deployment_id,person_subject_id) REFERENCES person_subjects(deployment_id,id),
 FOREIGN KEY(deployment_id,account_id) REFERENCES native_account_deployments(deployment_id,account_id)
);

INSERT INTO activity_disclosure_projections_new
 SELECT * FROM activity_disclosure_projections;

DROP TABLE activity_disclosure_projections;

ALTER TABLE activity_disclosure_projections_new RENAME TO activity_disclosure_projections;
