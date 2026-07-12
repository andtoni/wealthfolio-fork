CREATE TABLE policy_deployments (
 id TEXT PRIMARY KEY NOT NULL,
 access_generation BIGINT NOT NULL DEFAULT 0 CHECK(access_generation >= 0)
);
CREATE TABLE person_subjects (
 deployment_id TEXT NOT NULL, id TEXT NOT NULL, display_name TEXT,
 PRIMARY KEY(deployment_id,id), FOREIGN KEY(deployment_id) REFERENCES policy_deployments(id)
);
-- A provider principal's owner is write-once within a deployment.
CREATE TABLE credential_owner_subjects (deployment_id TEXT NOT NULL,provider TEXT NOT NULL,provider_principal TEXT NOT NULL,credential_owner_subject TEXT NOT NULL,PRIMARY KEY(deployment_id,provider,provider_principal),FOREIGN KEY(deployment_id) REFERENCES policy_deployments(id),FOREIGN KEY(deployment_id,credential_owner_subject) REFERENCES person_subjects(deployment_id,id));
CREATE TRIGGER credential_owner_immutable_update BEFORE UPDATE ON credential_owner_subjects BEGIN SELECT RAISE(ABORT,'credential owner is immutable'); END;
CREATE TRIGGER credential_owner_immutable_delete BEFORE DELETE ON credential_owner_subjects BEGIN SELECT RAISE(ABORT,'credential owner is immutable'); END;
CREATE TABLE discovered_account_candidates (id TEXT PRIMARY KEY NOT NULL,deployment_id TEXT NOT NULL,provider TEXT NOT NULL,provider_principal TEXT NOT NULL,provider_account_id TEXT NOT NULL,name TEXT NOT NULL,account_type TEXT NOT NULL,currency TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','approved','relinked','rejected')),approved_account_id TEXT,FOREIGN KEY(deployment_id,provider,provider_principal) REFERENCES credential_owner_subjects(deployment_id,provider,provider_principal));
CREATE UNIQUE INDEX discovered_account_pending ON discovered_account_candidates(deployment_id,provider,provider_principal,provider_account_id) WHERE state='pending';

-- A native account belongs to exactly one deployment. Policy rows reference
-- this composite key so an account id can never be substituted cross-tenant.
CREATE TABLE native_account_deployments (account_id TEXT PRIMARY KEY NOT NULL,deployment_id TEXT NOT NULL,UNIQUE(deployment_id,account_id),FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,FOREIGN KEY(deployment_id) REFERENCES policy_deployments(id));

CREATE TABLE households (
 deployment_id TEXT NOT NULL, id TEXT NOT NULL, closed_generation BIGINT,
 PRIMARY KEY(deployment_id,id), FOREIGN KEY(deployment_id) REFERENCES policy_deployments(id)
);
CREATE TABLE household_membership_epochs (
 deployment_id TEXT NOT NULL, household_id TEXT NOT NULL, person_subject_id TEXT NOT NULL,
 role TEXT NOT NULL CHECK(role IN ('owner','member')), start_epoch BIGINT NOT NULL, end_epoch BIGINT,
 PRIMARY KEY(deployment_id,household_id,person_subject_id,start_epoch),
 FOREIGN KEY(deployment_id,household_id) REFERENCES households(deployment_id,id),
 FOREIGN KEY(deployment_id,person_subject_id) REFERENCES person_subjects(deployment_id,id),
 CHECK(end_epoch IS NULL OR end_epoch > start_epoch)
);
CREATE UNIQUE INDEX household_membership_current ON household_membership_epochs(deployment_id,household_id,person_subject_id) WHERE end_epoch IS NULL;
CREATE TRIGGER household_membership_no_overlap BEFORE INSERT ON household_membership_epochs BEGIN
 SELECT RAISE(ABORT,'overlapping membership epoch') WHERE EXISTS (SELECT 1 FROM household_membership_epochs e WHERE e.deployment_id=NEW.deployment_id AND e.household_id=NEW.household_id AND e.person_subject_id=NEW.person_subject_id AND COALESCE(e.end_epoch,9223372036854775807)>NEW.start_epoch AND COALESCE(NEW.end_epoch,9223372036854775807)>e.start_epoch);
END;
CREATE TABLE account_ownership_arrangements (
 deployment_id TEXT NOT NULL,id TEXT NOT NULL,household_id TEXT NOT NULL,account_id TEXT NOT NULL,kind TEXT NOT NULL,start_epoch BIGINT NOT NULL,end_epoch BIGINT,
 PRIMARY KEY(deployment_id,id), UNIQUE(deployment_id,id,household_id), FOREIGN KEY(deployment_id,household_id) REFERENCES households(deployment_id,id), FOREIGN KEY(deployment_id,account_id) REFERENCES native_account_deployments(deployment_id,account_id), CHECK(end_epoch IS NULL OR end_epoch>start_epoch)
);
CREATE UNIQUE INDEX account_ownership_current ON account_ownership_arrangements(deployment_id,account_id) WHERE end_epoch IS NULL;
CREATE TABLE account_ownership_participants (
 deployment_id TEXT NOT NULL,arrangement_id TEXT NOT NULL,person_subject_id TEXT NOT NULL,share_basis_points INTEGER,
 PRIMARY KEY(deployment_id,arrangement_id,person_subject_id), FOREIGN KEY(deployment_id,arrangement_id) REFERENCES account_ownership_arrangements(deployment_id,id), FOREIGN KEY(deployment_id,person_subject_id) REFERENCES person_subjects(deployment_id,id), CHECK(share_basis_points IS NULL OR share_basis_points BETWEEN 0 AND 10000)
);
CREATE TABLE account_access_epochs (
 deployment_id TEXT NOT NULL,household_id TEXT NOT NULL,account_id TEXT NOT NULL,person_subject_id TEXT NOT NULL,start_generation BIGINT NOT NULL,end_generation BIGINT,
 PRIMARY KEY(deployment_id,account_id,person_subject_id,start_generation), FOREIGN KEY(deployment_id,household_id) REFERENCES households(deployment_id,id), FOREIGN KEY(deployment_id,person_subject_id) REFERENCES person_subjects(deployment_id,id), FOREIGN KEY(deployment_id,account_id) REFERENCES native_account_deployments(deployment_id,account_id), CHECK(end_generation IS NULL OR end_generation>start_generation)
);
CREATE UNIQUE INDEX account_access_current ON account_access_epochs(deployment_id,account_id,person_subject_id) WHERE end_generation IS NULL;
CREATE TRIGGER account_access_no_overlap BEFORE INSERT ON account_access_epochs BEGIN SELECT RAISE(ABORT,'overlapping access epoch') WHERE EXISTS(SELECT 1 FROM account_access_epochs e WHERE e.deployment_id=NEW.deployment_id AND e.account_id=NEW.account_id AND e.person_subject_id=NEW.person_subject_id AND COALESCE(e.end_generation,9223372036854775807)>NEW.start_generation AND COALESCE(NEW.end_generation,9223372036854775807)>e.start_generation); END;
CREATE TABLE activity_attribution_overlays (
 deployment_id TEXT NOT NULL,activity_id TEXT NOT NULL,account_id TEXT NOT NULL,attributed_person_subject_id TEXT,attribution_kind TEXT NOT NULL DEFAULT 'person' CHECK(attribution_kind IN ('person','household','unassigned')),provider_actor TEXT,revision BIGINT NOT NULL CHECK(revision>0),
 PRIMARY KEY(deployment_id,activity_id), FOREIGN KEY(deployment_id) REFERENCES policy_deployments(id), FOREIGN KEY(deployment_id,attributed_person_subject_id) REFERENCES person_subjects(deployment_id,id), CHECK((attribution_kind='person')=(attributed_person_subject_id IS NOT NULL))
);
CREATE TABLE disclosure_receipts (
 deployment_id TEXT NOT NULL, person_subject_id TEXT NOT NULL, idempotency_key TEXT NOT NULL,
 account_id TEXT NOT NULL, access_generation BIGINT NOT NULL, operation TEXT NOT NULL,
 resource_id TEXT NOT NULL, outcome TEXT NOT NULL,
 PRIMARY KEY(deployment_id,person_subject_id,idempotency_key),
 FOREIGN KEY(deployment_id) REFERENCES policy_deployments(id),
 FOREIGN KEY(deployment_id,person_subject_id) REFERENCES person_subjects(deployment_id,id)
);
CREATE TABLE activity_disclosure_projections (
 deployment_id TEXT NOT NULL, person_subject_id TEXT NOT NULL, activity_id TEXT NOT NULL,
 account_id TEXT NOT NULL, access_generation BIGINT NOT NULL,
 PRIMARY KEY(deployment_id,person_subject_id,activity_id),
 FOREIGN KEY(activity_id) REFERENCES activities(id) ON DELETE CASCADE
);
