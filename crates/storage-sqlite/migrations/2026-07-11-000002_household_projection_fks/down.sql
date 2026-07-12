-- Revert to the original table without composite FKs.
CREATE TABLE activity_disclosure_projections_old (
 deployment_id TEXT NOT NULL,
 person_subject_id TEXT NOT NULL,
 activity_id TEXT NOT NULL,
 account_id TEXT NOT NULL,
 access_generation BIGINT NOT NULL,
 PRIMARY KEY(deployment_id,person_subject_id,activity_id),
 FOREIGN KEY(activity_id) REFERENCES activities(id) ON DELETE CASCADE
);

INSERT INTO activity_disclosure_projections_old
 SELECT * FROM activity_disclosure_projections;

DROP TABLE activity_disclosure_projections;

ALTER TABLE activity_disclosure_projections_old RENAME TO activity_disclosure_projections;
