#![allow(clippy::type_complexity)]

use rusqlite::{params, Connection};
use tempfile::TempDir;
use wealthfolio_storage_sqlite::{bridge_policy::*, db};

const D1: &str = "deployment-1";
const D2: &str = "deployment-2";

async fn store(deployment: &str) -> (TempDir, String, BridgePolicyStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = db::init(dir.path().to_str().unwrap()).unwrap();
    db::run_migrations(&path).unwrap();
    let pool = db::create_pool(&path).unwrap();
    let writer = db::write_actor::spawn_writer((*pool).clone()).unwrap();
    let store = BridgePolicyStore::new(deployment, pool, writer);
    (dir, path, store)
}

fn seed(path: &str, deployments: &[&str], subjects: &[&str]) {
    let c = Connection::open(path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for d in deployments {
        c.execute("INSERT INTO policy_deployments(id) VALUES(?)", [d])
            .unwrap();
        c.execute(
            "INSERT INTO households(deployment_id,id) VALUES(?,?)",
            params![d, format!("{d}-household")],
        )
        .unwrap();
        for s in subjects {
            c.execute(
                "INSERT INTO person_subjects(deployment_id,id) VALUES(?,?)",
                params![d, s],
            )
            .unwrap();
        }
    }
    let owner_deployment = deployments[0];
    for a in ["a1", "a2"] {
        c.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode) VALUES(?,?, 'CASH','USD',0,1,0,'TRANSACTIONS')", params![a,a]).unwrap();
        c.execute(
            "INSERT INTO native_account_deployments(account_id,deployment_id) VALUES(?,?)",
            params![a, owner_deployment],
        )
        .unwrap();
    }
    for (id, account) in [("x1", "a1"), ("x2", "a2")] {
        c.execute("INSERT INTO activities(id,account_id,activity_type,status,activity_date,amount,currency,is_user_modified,needs_review,created_at,updated_at) VALUES(?,?, 'DEPOSIT','POSTED','2026-01-01','1','USD',0,0,'2026-01-01','2026-01-01')", params![id,account]).unwrap();
    }
}

/// Deterministic, lossless snapshot of every Gate 5 governed table.
fn canonical_snapshot(c: &Connection) -> Vec<String> {
    canonical_snapshot_with_timestamp_normalization(c, false)
}

fn canonical_snapshot_with_timestamp_normalization(
    c: &Connection,
    normalize_timestamps: bool,
) -> Vec<String> {
    const TABLES: &[&str] = &[
        "activities",
        "activity_taxonomy_assignments",
        "taxonomy_categories",
        "taxonomy_category_assignments",
        "spending_activity_splits",
        "spending_activity_events",
        "spending_events",
        "spending_event_types",
        "activity_attribution_overlays",
        "activity_disclosure_projections",
        "disclosure_receipts",
        "account_access_epochs",
    ];
    let mut out = Vec::new();
    for table in TABLES {
        let mut cols = c.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let names: Vec<String> = cols
            .query_map([], |r| r.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        if names.is_empty() {
            continue;
        }
        let quoted: Vec<String> = names
            .iter()
            .map(|n| format!("\"{}\"", n.replace('\"', "\"\"")))
            .collect();
        let sql = format!(
            "SELECT {} FROM \"{table}\" ORDER BY {}",
            quoted.join(","),
            quoted.join(",")
        );
        let mut stmt = c.prepare(&sql).unwrap();
        let rows = stmt
            .query_map([], |row| {
                let mut values = Vec::with_capacity(names.len());
                for (i, name) in names.iter().enumerate() {
                    let v: rusqlite::types::Value = row.get(i)?;
                    let normalize = normalize_timestamps
                        && matches!(name.as_str(), "created_at" | "updated_at");
                    values.push(if normalize {
                        "<NONDETERMINISTIC_TIMESTAMP>".into()
                    } else {
                        match v {
                            rusqlite::types::Value::Null => "<NULL>".into(),
                            other => format!("{other:?}"),
                        }
                    });
                }
                Ok(format!("{table}:{}", values.join("\u{1f}")))
            })
            .unwrap();
        out.extend(rows.map(|r| r.unwrap()));
    }
    out
}

fn canonical_snapshot_for_lifecycle_oracle(c: &Connection) -> Vec<String> {
    canonical_snapshot_with_timestamp_normalization(c, true)
}

#[test]
fn lifecycle_oracle_preserves_timestamp_looking_activity_notes() {
    let dir = tempfile::tempdir().unwrap();
    let path = db::init(dir.path().to_str().unwrap()).unwrap();
    db::run_migrations(&path).unwrap();
    seed(&path, &[D1], &["alice"]);
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE activities SET notes='2026-07-17Tfirst' WHERE id='x1'",
        [],
    )
    .unwrap();
    let before = canonical_snapshot_for_lifecycle_oracle(&c);
    c.execute(
        "UPDATE activities SET notes='2026-07-17Tsecond' WHERE id='x1'",
        [],
    )
    .unwrap();
    assert_ne!(canonical_snapshot_for_lifecycle_oracle(&c), before);
}

#[tokio::test]
async fn no_grant_generation_zero_is_rejected() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let error = store
        .commit(DisclosureOperation::new(
            "k",
            "alice",
            0,
            DisclosureKind::ActivityUpsert,
            "x1",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        BridgePolicyError::MissingGeneration | BridgePolicyError::AccessDenied
    ));
}

#[tokio::test]
async fn same_subject_and_key_do_not_collide_across_deployments() {
    let (_dir, path, d1) = store(D1).await;
    seed(&path, &[D1, D2], &["alice"]);
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode) VALUES('b1','b1','CASH','USD',0,1,0,'TRANSACTIONS')", []).unwrap();
    c.execute(
        "INSERT INTO native_account_deployments(account_id,deployment_id) VALUES('b1',?)",
        [D2],
    )
    .unwrap();
    c.execute("INSERT INTO activities(id,account_id,activity_type,status,activity_date,amount,currency,is_user_modified,needs_review,created_at,updated_at) VALUES('y1','b1','DEPOSIT','POSTED','2026-01-01','1','USD',0,0,'2026-01-01','2026-01-01')", []).unwrap();
    let pool = db::create_pool(&path).unwrap();
    let writer = db::write_actor::spawn_writer((*pool).clone()).unwrap();
    let d2 = BridgePolicyStore::new(D2, pool, writer);
    let g1 = d1.grant("alice", "a1").await.unwrap();
    let g2 = d2.grant("alice", "b1").await.unwrap();
    d1.commit(DisclosureOperation::new(
        "same",
        "alice",
        g1,
        DisclosureKind::ActivityUpsert,
        "x1",
    ))
    .await
    .unwrap();
    d2.commit(DisclosureOperation::new(
        "same",
        "alice",
        g2,
        DisclosureKind::ActivityUpsert,
        "y1",
    ))
    .await
    .unwrap();
    assert_eq!(d1.receipt_count().await.unwrap(), 1);
    assert_eq!(d2.receipt_count().await.unwrap(), 1);
}

#[tokio::test]
async fn resource_owned_by_wrong_account_is_rejected() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let error = store
        .commit(DisclosureOperation::new(
            "wrong",
            "alice",
            generation,
            DisclosureKind::ActivityUpsert,
            "x2",
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, BridgePolicyError::AccessDenied));
}

#[tokio::test]
async fn commit_mutates_actual_activity_projection() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    store
        .commit(DisclosureOperation::new(
            "upsert",
            "alice",
            generation,
            DisclosureKind::ActivityUpsert,
            "x1",
        ))
        .await
        .unwrap();
    let c = Connection::open(path).unwrap();
    let count: i64 = c.query_row("SELECT count(*) FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id='alice' AND activity_id='x1' AND account_id='a1'", [D1], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
    store
        .commit(DisclosureOperation::new(
            "delete",
            "alice",
            generation,
            DisclosureKind::ActivityDelete,
            "x1",
        ))
        .await
        .unwrap();
    let count: i64 = c.query_row("SELECT count(*) FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id='alice' AND activity_id='x1'", [D1], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn existing_activity_payload_account_mismatch_is_rejected_without_mutation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    let before: (String, String, String) = c
        .query_row(
            "SELECT account_id, amount, currency FROM activities WHERE id='x1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let payload = ActivityPayload {
        account_id: "a2".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        amount: "999".into(),
        currency: "EUR".into(),
        notes: Some("must not write".into()),
        activity_type_override: None,
        needs_review: Some(true),
    };
    let error = store
        .commit(
            DisclosureOperation::new(
                "account-mismatch",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "x1",
            )
            .with_activity(payload),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        BridgePolicyError::Storage(_) | BridgePolicyError::ActivityAccountMismatch
    ));
    let after: (String, String, String) = c
        .query_row(
            "SELECT account_id, amount, currency FROM activities WHERE id='x1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(store.receipt_count().await.unwrap(), 0);
}

#[tokio::test]
async fn revoke_atomically_fences_and_fanout_isolated() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice", "bob"]);
    let _alice_before_fence = store.grant("alice", "a1").await.unwrap();
    let bob = store.grant("bob", "a1").await.unwrap();
    assert!(store
        .revoke_with_fault("bob", "a1", "failed", RevokeFault::BeforeReceipt)
        .await
        .is_err());
    assert!(store.has_access("bob", "a1").await.unwrap());
    store.revoke("bob", "a1", "fence").await.unwrap();
    let alice = store.grant("alice", "a1").await.unwrap();
    let outcomes = store
        .commit_fanout(vec![
            DisclosureOperation::new("fan", "alice", alice, DisclosureKind::ActivityUpsert, "x1"),
            DisclosureOperation::new("fan", "bob", bob, DisclosureKind::ActivityUpsert, "x1"),
        ])
        .await;
    assert!(outcomes[0].is_ok());
    assert!(matches!(
        outcomes[1],
        Err(BridgePolicyError::StaleGeneration { .. })
    ));
}

#[tokio::test]
async fn migration_fixture_can_be_seeded() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("pending-1", "posted-1")
        .await
        .unwrap();
}

#[tokio::test]
async fn pending_to_posted_preserves_overlays_and_is_idempotent() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("pending-1", "posted-1")
        .await
        .unwrap();
    store
        .migrate_pending_to_posted("pending-1", "posted-1")
        .await
        .unwrap();
    let state = store
        .migration_state("pending-1", "posted-1")
        .await
        .unwrap();
    assert!(!state.pending_exists);
    assert!(state.posted_exists);
    assert_eq!(state.attribution.as_deref(), Some("alice"));
    assert_eq!(state.assignments, 1);
    assert_eq!(state.splits, 1);
    assert_eq!(state.event_tags, 1);
    assert!(state.ignored);
    assert!(state.needs_review);
    assert!(store
        .migrate_pending_to_posted("pending-1", "posted-1")
        .await
        .is_err());
    let c = Connection::open(path).unwrap();
    let notes: String = c
        .query_row(
            "SELECT notes FROM activities WHERE id='posted-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(notes, "pending note");
    let modified: i32 = c
        .query_row(
            "SELECT is_user_modified FROM activities WHERE id='posted-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(modified, 1);
    let (account, subject, revision): (String, String, i64) = c.query_row("SELECT account_id,attributed_person_subject_id,revision FROM activity_attribution_overlays WHERE deployment_id=? AND activity_id='posted-1'", [D1], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(
        (account, subject, revision),
        ("a1".into(), "alice".into(), 1)
    );
}

#[tokio::test]
async fn canonical_snapshot_detects_non_count_child_corruption() {
    let (_dir, path, _store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let c = Connection::open(&path).unwrap();
    let before = canonical_snapshot(&c);
    c.execute("UPDATE activities SET notes='changed' WHERE id='x1'", [])
        .unwrap();
    assert_ne!(canonical_snapshot(&c), before);
}

#[tokio::test]
async fn commit_lifecycle_path_migrates_only_projected_candidate() {
    let (_dir, path, lifecycle_store) = store(D1).await;
    let (_oracle_dir, oracle_path, _oracle_store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    seed(&oracle_path, &[D1], &["alice"]);
    lifecycle_store
        .seed_migration_fixture("pending-commit", "posted-commit")
        .await
        .unwrap();
    let oracle_pool = db::create_pool(&oracle_path).unwrap();
    let oracle_writer = db::write_actor::spawn_writer((*oracle_pool).clone()).unwrap();
    let oracle = BridgePolicyStore::new(D1, oracle_pool, oracle_writer);
    oracle
        .seed_migration_fixture("pending-commit", "posted-commit")
        .await
        .unwrap();
    let generation = lifecycle_store.grant("alice", "a1").await.unwrap();
    let oracle_generation = oracle.grant("alice", "a1").await.unwrap();
    assert_eq!(generation, oracle_generation);
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "pending-commit", "a1", generation]).unwrap();
    let oracle_c = Connection::open(&oracle_path).unwrap();
    oracle_c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "pending-commit", "a1", oracle_generation]).unwrap();
    let before = canonical_snapshot(&c);
    assert!(!before.is_empty());
    lifecycle_store
        .commit(
            DisclosureOperation::new(
                "lifecycle",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "pending-commit",
            )
            .with_lifecycle("posted-commit"),
        )
        .await
        .unwrap();
    let pending: i64 = c
        .query_row(
            "SELECT count(*) FROM activities WHERE id='pending-commit'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let posted: i64 = c
        .query_row(
            "SELECT count(*) FROM activities WHERE id='posted-commit' AND status='POSTED'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let projection: i64 = c.query_row("SELECT count(*) FROM activity_disclosure_projections WHERE activity_id='posted-commit'", [], |r| r.get(0)).unwrap();
    assert_eq!((pending, posted, projection), (0, 1, 1));

    // Independent oracle: this is the literal pending->posted transformation,
    // not a projection of values read from the production commit result.
    for table in [
        "activity_taxonomy_assignments",
        "spending_activity_splits",
        "spending_activity_events",
    ] {
        oracle_c
            .execute(&format!("UPDATE {table} SET activity_id='posted-commit' WHERE activity_id='pending-commit'"), [])
            .unwrap();
    }
    oracle_c.execute("UPDATE activities SET notes=coalesce((SELECT notes FROM activities WHERE id='pending-commit'),notes),activity_type_override=coalesce((SELECT activity_type_override FROM activities WHERE id='pending-commit'),activity_type_override),is_user_modified=max(is_user_modified,coalesce((SELECT is_user_modified FROM activities WHERE id='pending-commit'),0)),needs_review=max(needs_review,coalesce((SELECT needs_review FROM activities WHERE id='pending-commit'),0)) WHERE id='posted-commit'", []).unwrap();
    oracle_c.execute("UPDATE activity_disclosure_projections SET activity_id='posted-commit' WHERE deployment_id='deployment-1' AND person_subject_id='alice' AND activity_id='pending-commit' AND account_id='a1' AND access_generation=1", []).unwrap();
    oracle_c.execute("UPDATE activity_attribution_overlays SET activity_id='posted-commit',account_id=(SELECT account_id FROM activities WHERE id='posted-commit') WHERE deployment_id='deployment-1' AND activity_id='pending-commit'", []).unwrap();
    oracle_c
        .execute("DELETE FROM activities WHERE id='pending-commit'", [])
        .unwrap();
    let binding = "v1;12:deployment-15:alice9:lifecycle1:115:activity_upsert14:pending-commit2:a113:posted-commit8:accepted";
    oracle_c.execute("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,outcome) VALUES('deployment-1','alice','lifecycle','a1',1,'activity_upsert','pending-commit','posted-commit',1,?,'accepted')", [binding]).unwrap();
    let actual = canonical_snapshot_for_lifecycle_oracle(&c);
    let expected = canonical_snapshot_for_lifecycle_oracle(&oracle_c);
    assert_eq!(actual.len(), expected.len());
    for (index, (actual_row, expected_row)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            actual_row, expected_row,
            "canonical row mismatch at {index}"
        );
    }
}

#[tokio::test]
async fn commit_lifecycle_rejects_wrong_status_and_missing_projection() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-invalid", "n-invalid")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let op = DisclosureOperation::new(
        "invalid",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "p-invalid",
    )
    .with_lifecycle("n-invalid");
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE activities SET status='POSTED' WHERE id='p-invalid'",
        [],
    )
    .unwrap();
    assert!(store.commit(op.clone()).await.is_err());
    c.execute(
        "UPDATE activities SET status='PENDING' WHERE id='p-invalid'",
        [],
    )
    .unwrap();
    assert!(store.commit(op.clone()).await.is_err());
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-invalid", "a1", generation]).unwrap();
    c.execute(
        "UPDATE activities SET account_id='a2' WHERE id='n-invalid'",
        [],
    )
    .unwrap();
    assert!(store.commit(op).await.is_err());
}

#[tokio::test]
async fn commit_lifecycle_fault_rolls_back_all_sidecars() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-rollback", "n-rollback")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-rollback", "a1", generation]).unwrap();
    let before = canonical_snapshot(&c);
    let op = DisclosureOperation::new(
        "rollback",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "p-rollback",
    )
    .with_lifecycle("n-rollback");
    assert!(store
        .commit_with_fault(op, CommitFault::AfterSidecar)
        .await
        .is_err());
    assert_eq!(canonical_snapshot(&c), before);
    assert_eq!(store.receipt_count().await.unwrap(), 0);
}

#[tokio::test]
async fn commit_lifecycle_exact_replay_uses_durable_receipt_after_pending_removal() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-replay", "n-replay")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-replay", "a1", generation]).unwrap();
    let op = DisclosureOperation::new(
        "replay",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "p-replay",
    )
    .with_lifecycle("n-replay");
    let first = store.commit(op.clone()).await.unwrap();
    let second = store.commit(op).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(store.receipt_count().await.unwrap(), 1);
}

#[tokio::test]
async fn lifecycle_replay_rejects_different_successor_and_account() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-replay-identity", "n-replay-identity")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-replay-identity", "a1", generation]).unwrap();
    let first = DisclosureOperation::new(
        "identity",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "p-replay-identity",
    )
    .with_lifecycle("n-replay-identity");
    store.commit(first.clone()).await.unwrap();
    assert!(matches!(
        store
            .commit(first.clone().with_lifecycle("other-successor"))
            .await,
        Err(BridgePolicyError::IdempotencyConflict)
    ));
    let wrong_account = first.with_activity(ActivityPayload {
        account_id: "a2".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        amount: "1".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
    });
    assert!(matches!(
        store.commit(wrong_account).await,
        Err(BridgePolicyError::IdempotencyConflict)
    ));
    assert_eq!(store.receipt_count().await.unwrap(), 1);
}

#[tokio::test]
async fn commit_identity_and_generation_matrix_fails_closed_without_mutation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice", "bob"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO activities(id,account_id,activity_type,status,activity_date,amount,currency,is_user_modified,needs_review,created_at,updated_at) VALUES('matrix','a1','DEPOSIT','POSTED','2026-01-01','1','USD',0,0,'2026-01-01','2026-01-01')", []).unwrap();
    let snapshot = || {
        c.query_row("SELECT (SELECT count(*) FROM activities WHERE id='matrix'), (SELECT count(*) FROM activity_disclosure_projections), (SELECT count(*) FROM disclosure_receipts)", [], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,i64>(1)?, r.get::<_,i64>(2)?))).unwrap()
    };
    let before = snapshot();
    for (subject, access_generation) in [("bob", generation), ("alice", generation + 1)] {
        assert!(store
            .commit(DisclosureOperation::new(
                "matrix-key",
                subject,
                access_generation,
                DisclosureKind::ActivityUpsert,
                "matrix"
            ))
            .await
            .is_err());
        assert_eq!(snapshot(), before);
    }
}

#[tokio::test]
async fn lifecycle_projection_candidate_zero_fails_closed_without_mutation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-cardinality", "n-cardinality")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let op = DisclosureOperation::new(
        "cardinality",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "p-cardinality",
    )
    .with_lifecycle("n-cardinality");
    let c = Connection::open(&path).unwrap();
    let before: (i64, i64) = c.query_row("SELECT (SELECT count(*) FROM activities WHERE id='p-cardinality'), (SELECT count(*) FROM disclosure_receipts)", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(store.commit(op).await.is_err());
    assert_eq!(c.query_row("SELECT (SELECT count(*) FROM activities WHERE id='p-cardinality'), (SELECT count(*) FROM disclosure_receipts)", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap(), before);
}

#[tokio::test]
async fn legacy_receipt_without_binding_is_rejected_before_mutation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,outcome) VALUES(?,?,?,?,?,?,?,NULL,'accepted')", params![D1, "alice", "legacy", "a1", generation, "activity_upsert", "x1"]).unwrap();
    assert!(store
        .commit(DisclosureOperation::new(
            "legacy",
            "alice",
            generation,
            DisclosureKind::ActivityUpsert,
            "x1"
        ))
        .await
        .is_err());
}

#[tokio::test]
async fn malformed_and_partial_receipt_bindings_are_rejected_on_both_replay_paths() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    store
        .commit(DisclosureOperation::new(
            "bound",
            "alice",
            generation,
            DisclosureKind::ActivityUpsert,
            "x1",
        ))
        .await
        .unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("UPDATE disclosure_receipts SET receipt_binding='v1|deployment-1|alice' WHERE idempotency_key='bound'", []).unwrap();
    assert!(store
        .commit(DisclosureOperation::new(
            "bound",
            "alice",
            generation,
            DisclosureKind::ActivityUpsert,
            "x1"
        ))
        .await
        .is_err());
    c.execute("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,outcome) VALUES(?,?,?,?,?,?,?,NULL,0,NULL,'revoked')", params![D1,"alice","revoke-legacy","a1",generation,"projection_fence","a1"]).unwrap();
    assert!(store.revoke("alice", "a1", "revoke-legacy").await.is_err());
}

#[tokio::test]
async fn lifecycle_commit_preserves_complete_overlay_state_exactly() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-exact", "n-exact")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-exact", "a1", generation]).unwrap();
    let op = DisclosureOperation::new(
        "exact",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "p-exact",
    )
    .with_lifecycle("n-exact");
    store.commit(op).await.unwrap();
    let state: (i64,i64,i64,i64,i64,i64,i64) = c.query_row("SELECT (SELECT count(*) FROM activities WHERE id='p-exact'), (SELECT count(*) FROM activities WHERE id='n-exact'), (SELECT count(*) FROM activity_taxonomy_assignments WHERE activity_id='n-exact'), (SELECT count(*) FROM spending_activity_splits WHERE activity_id='n-exact'), (SELECT count(*) FROM spending_activity_events WHERE activity_id='n-exact'), (SELECT count(*) FROM activity_attribution_overlays WHERE activity_id='n-exact'), (SELECT count(*) FROM activity_disclosure_projections WHERE activity_id='n-exact' AND access_generation=?)", [generation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    assert_eq!(state, (0, 1, 1, 1, 1, 1, 1));
    let posted: (String, String, String, String, String, String, Option<String>, Option<String>, i32, i32) = c.query_row("SELECT id,account_id,activity_type,status,amount,currency,notes,activity_type_override,is_user_modified,needs_review FROM activities WHERE id='n-exact'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).unwrap();
    assert_eq!(
        posted,
        (
            "n-exact".into(),
            "a1".into(),
            "DEPOSIT".into(),
            "POSTED".into(),
            "1".into(),
            "USD".into(),
            Some("pending note".into()),
            Some("IGNORE".into()),
            1,
            1
        )
    );
    let receipt: (String, String, i64, String, String, String, Option<String>, String) = c.query_row("SELECT person_subject_id,idempotency_key,access_generation,operation,resource_id,account_id,lifecycle_successor_id,outcome FROM disclosure_receipts WHERE deployment_id=?", [D1], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).unwrap();
    assert_eq!(
        receipt,
        (
            "alice".into(),
            "exact".into(),
            generation,
            "activity_upsert".into(),
            "p-exact".into(),
            "a1".into(),
            Some("n-exact".into()),
            "accepted".into()
        )
    );
    let source_counts: (i64, i64, i64, i64) = c.query_row("SELECT (SELECT count(*) FROM activities WHERE id='p-exact'), (SELECT count(*) FROM activity_taxonomy_assignments WHERE activity_id='p-exact'), (SELECT count(*) FROM spending_activity_splits WHERE activity_id='p-exact'), (SELECT count(*) FROM spending_activity_events WHERE activity_id='p-exact')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(source_counts, (0, 0, 0, 0));
}
