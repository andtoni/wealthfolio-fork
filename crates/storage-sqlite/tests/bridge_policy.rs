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
