#![allow(clippy::type_complexity)]

use rusqlite::{params, Connection};
use tempfile::TempDir;
use wealthfolio_storage_sqlite::{bridge_policy::*, db};

const D1: &str = "deployment-1";
const D2: &str = "deployment-2";

fn current_state_operation(
    key: &str,
    generation: i64,
    cash: &str,
    liability: &str,
) -> DisclosureOperation {
    current_state_operation_for_account(key, generation, "a1", cash, liability)
}

fn current_state_operation_for_account(
    key: &str,
    generation: i64,
    account_id: &str,
    cash: &str,
    liability: &str,
) -> DisclosureOperation {
    let state = CurrentStatePayload {
        account_id: account_id.into(),
        provider_connection_id: "deployment-1".into(),
        provider_incarnation_id: "incarnation".into(),
        provider_generation: 1,
        observed_at: "2026-07-18T12:00:00Z".into(),
        revision: 1,
        as_of_date: "2026-07-18".into(),
        currency: "USD".into(),
        cash_balance: cash.into(),
        liability_balance: liability.into(),
        positions: vec![],
    };
    let semantic_hash = current_state_semantic_hash(&state);
    let mut operation = DisclosureOperation::new(
        key,
        "alice",
        generation,
        DisclosureKind::CurrentStateUpsert,
        format!("deployment-1:incarnation:current-state:{account_id}"),
    );
    operation.semantic_hash = Some(semantic_hash);
    operation.current_state = Some(state);
    operation
}

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
            c.execute(
                "INSERT INTO household_membership_epochs(deployment_id,household_id,person_subject_id,role,start_epoch) VALUES(?,?,?,?,1)",
                params![d, format!("{d}-household"), s, "owner"],
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
        let arrangement = format!("{owner_deployment}-{a}-ownership");
        c.execute(
            "INSERT INTO account_ownership_arrangements(deployment_id,id,household_id,account_id,kind,start_epoch) VALUES(?,?,?,?,?,1)",
            params![owner_deployment, arrangement, format!("{owner_deployment}-household"), a, "owned"],
        )
        .unwrap();
        for subject in subjects {
            c.execute(
                "INSERT INTO account_ownership_participants(deployment_id,arrangement_id,person_subject_id) VALUES(?,?,?)",
                params![owner_deployment, arrangement, subject],
            )
            .unwrap();
        }
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
    c.execute("INSERT INTO account_ownership_arrangements(deployment_id,id,household_id,account_id,kind,start_epoch) VALUES(?, 'd2-b1-ownership', ?, 'b1', 'owned', 1)", params![D2, format!("{D2}-household")]).unwrap();
    c.execute("INSERT INTO account_ownership_participants(deployment_id,arrangement_id,person_subject_id) VALUES(?, 'd2-b1-ownership', 'alice')", [D2]).unwrap();
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
async fn current_state_cash_and_positive_liability_are_atomic_and_retry_bound() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let operation = current_state_operation("current-state", generation, "125", "25");

    let first = store.commit(operation.clone()).await.unwrap();
    let retry = store.commit(operation.clone()).await.unwrap();
    assert_eq!(first.receipt_binding, retry.receipt_binding);

    let connection = Connection::open(&path).unwrap();
    let persisted: (String, String, String, String, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT cash_total_account_currency, liability_balance, position_market_values, source, (SELECT count(*) FROM provenance_records WHERE entity_type='snapshot' AND entity_id=holdings_snapshots.id AND role='live_canonical'), (SELECT count(*) FROM provenance_records WHERE entity_type='account' AND entity_id='a1' AND role='live_canonical'), (SELECT count(*) FROM verified_receipts WHERE account_id='a1' AND verification_state='verified'), (SELECT include_in_net_worth FROM accounts WHERE id='a1') FROM holdings_snapshots WHERE account_id='a1' AND snapshot_date='2026-07-18'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        )
        .unwrap();
    assert_eq!(
        persisted,
        (
            "125".into(),
            "25".into(),
            "{}".into(),
            "BROKER_IMPORTED".into(),
            1,
            1,
            1,
            1
        )
    );

    let mut relink = current_state_operation("current-state-relink", generation, "130", "25");
    let relink_state = relink.current_state.as_mut().unwrap();
    relink_state.provider_generation = 2;
    relink_state.observed_at = "2026-07-18T13:00:00Z".into();
    relink_state.revision = 1;
    relink.semantic_hash = Some(current_state_semantic_hash(relink_state));
    store.commit(relink).await.unwrap();
    let provider_generation: i64 = connection
        .query_row(
            "SELECT json_extract(source_record_id,'$.providerGeneration') FROM provenance_records WHERE entity_type='snapshot' AND entity_id=(SELECT id FROM holdings_snapshots WHERE account_id='a1' AND snapshot_date='2026-07-18') AND role='live_canonical'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider_generation, 2);

    let mut stale_provider =
        current_state_operation("current-state-stale-provider", generation, "140", "25");
    let stale_state = stale_provider.current_state.as_mut().unwrap();
    stale_state.provider_generation = 1;
    stale_state.observed_at = "2026-07-18T14:00:00Z".into();
    stale_state.revision = 2;
    stale_provider.semantic_hash = Some(current_state_semantic_hash(stale_state));
    assert!(store.commit(stale_provider).await.is_err());

    let mut superseded =
        current_state_operation("current-state-superseded", generation, "140", "25");
    let superseded_state = superseded.current_state.as_mut().unwrap();
    superseded_state.provider_generation = 2;
    superseded_state.observed_at = "2026-07-18T12:30:00Z".into();
    superseded_state.revision = 1;
    superseded.semantic_hash = Some(current_state_semantic_hash(superseded_state));
    let superseded_receipt = store.commit(superseded).await.unwrap();
    assert_eq!(superseded_receipt.outcome, ReceiptOutcome::Accepted);
    let persisted_cash: String = connection
        .query_row(
            "SELECT cash_total_account_currency FROM holdings_snapshots WHERE account_id='a1' AND snapshot_date='2026-07-18'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted_cash, "130");

    let altered = current_state_operation("current-state", generation, "126", "25");
    assert!(matches!(
        store.commit(altered).await,
        Err(BridgePolicyError::IdempotencyConflict)
    ));

    let mut mismatched = current_state_operation("semantic-mismatch", generation, "125", "25");
    mismatched.semantic_hash = Some("not-the-payload-hash".into());
    assert!(matches!(
        store.commit(mismatched).await,
        Err(BridgePolicyError::SemanticHashMismatch)
    ));
}

#[tokio::test]
async fn current_state_preserves_canonical_liability_and_market_value_precision() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    connection.execute("INSERT INTO assets(id,kind,name,is_active,quote_mode,quote_ccy,instrument_type,instrument_symbol,provider_config) VALUES('provider-vti','INVESTMENT','VTI',1,'MARKET','USD','EQUITY','VTI',?)", [serde_json::json!({"providerConnectionId":"deployment-1","providerIncarnationId":"incarnation","providerSymbol":"VTI","providerCurrency":"USD"}).to_string()]).unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let mut operation = current_state_operation(
        "precise-current-state",
        generation,
        "0.0000000000000000000000000001",
        "123.4567890123456789012345678",
    );
    let state = operation.current_state.as_mut().unwrap();
    state.positions.push(CurrentStatePosition {
        symbol: "VTI".into(),
        quantity: "0.1234567890123456789012345678".into(),
        price: "1.0000000000000000000000000001".into(),
        market_value: "0.1234567890123456789012345679".into(),
        currency: "USD".into(),
        cost_basis: Some("0.1000000000000000000000000001".into()),
    });
    operation.semantic_hash = Some(current_state_semantic_hash(state));
    store.commit(operation).await.unwrap();
    let (liability, market_values): (String, String) = connection
        .query_row(
            "SELECT liability_balance,position_market_values FROM holdings_snapshots WHERE account_id='a1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(liability, "123.4567890123456789012345678");
    assert_eq!(
        market_values,
        "{\"provider-vti\":\"0.1234567890123456789012345679\"}"
    );
    let decoded: std::collections::HashMap<String, rust_decimal::Decimal> =
        serde_json::from_str(&market_values).unwrap();
    assert_eq!(
        decoded["provider-vti"].to_string(),
        "0.1234567890123456789012345679"
    );
}

#[tokio::test]
async fn current_state_rejects_stale_generation_and_unauthorized_account_without_writes() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();

    assert!(matches!(
        store
            .commit(current_state_operation(
                "current-state-stale",
                generation + 1,
                "50",
                "10"
            ))
            .await,
        Err(BridgePolicyError::StaleGeneration { .. })
    ));
    assert!(matches!(
        store
            .commit(current_state_operation_for_account(
                "current-state-unauthorized",
                generation,
                "a2",
                "50",
                "10"
            ))
            .await,
        Err(BridgePolicyError::AccessDenied)
    ));
    let connection = Connection::open(path).unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM holdings_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn current_state_fault_rolls_back_snapshot_provenance_and_receipt() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let error = store
        .commit_with_fault(
            current_state_operation("current-state-fault", generation, "50", "10"),
            CommitFault::AfterSidecar,
        )
        .await;
    assert!(error.is_err());
    let connection = Connection::open(path).unwrap();
    for table in [
        "holdings_snapshots",
        "provenance_records",
        "verified_receipts",
        "disclosure_receipts",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} must roll back");
    }
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
        merchant_context: None,
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
async fn repeated_provider_upsert_preserves_user_owned_overlays() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("UPDATE activities SET notes='user note',activity_type_override='FEE',subtype='USER_SUBTYPE',is_user_modified=1,needs_review=1 WHERE id='x1'", []).unwrap();

    store
        .commit(
            DisclosureOperation::new(
                "provider-revision",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "x1",
            )
            .with_activity(ActivityPayload {
                account_id: "a1".into(),
                activity_type: "WITHDRAWAL".into(),
                status: "POSTED".into(),
                activity_date: "2026-02-01".into(),
                amount: "25".into(),
                currency: "USD".into(),
                notes: Some("provider note".into()),
                activity_type_override: Some("PROVIDER_OVERRIDE".into()),
                needs_review: Some(false),
                merchant_context: None,
            }),
        )
        .await
        .unwrap();

    let overlays: (Option<String>, Option<String>, Option<String>, i64, i64, String) = c.query_row(
        "SELECT notes,activity_type_override,subtype,is_user_modified,needs_review,amount FROM activities WHERE id='x1'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap();
    assert_eq!(
        overlays,
        (
            Some("user note".into()),
            Some("FEE".into()),
            Some("USER_SUBTYPE".into()),
            1,
            1,
            "25".into()
        )
    );
}

#[tokio::test]
async fn provider_can_seed_description_but_not_user_owned_classification_overlays() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    store
        .commit(
            DisclosureOperation::new(
                "new-provider-row",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "provider-new",
            )
            .with_activity(ActivityPayload {
                account_id: "a1".into(),
                activity_type: "WITHDRAWAL".into(),
                status: "POSTED".into(),
                activity_date: "2026-02-01".into(),
                amount: "25".into(),
                currency: "USD".into(),
                notes: Some("provider note".into()),
                activity_type_override: Some("PROVIDER_OVERRIDE".into()),
                needs_review: Some(true),
                merchant_context: None,
            }),
        )
        .await
        .unwrap();
    let c = Connection::open(&path).unwrap();
    let overlays: (Option<String>, Option<String>, i64, String, String, i64) = c.query_row(
        "SELECT notes,activity_type_override,needs_review,created_at,updated_at,(SELECT count(*) FROM provenance_records WHERE entity_type='activity' AND entity_id=activities.id AND role='live_canonical') FROM activities WHERE id='provider-new'", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap();
    assert_eq!(overlays.0, Some("provider note".into()));
    assert_eq!(overlays.1, None);
    assert_eq!(overlays.2, 0);
    chrono::DateTime::parse_from_rfc3339(&overlays.3).unwrap();
    chrono::DateTime::parse_from_rfc3339(&overlays.4).unwrap();
    assert_eq!(overlays.5, 1);
}

#[tokio::test]
async fn merchant_context_is_provider_owned_preserves_user_state_and_binds_retries() {
    use wealthfolio_storage_sqlite::bridge_policy::MerchantContextV1;

    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE activities SET metadata='{\"user\":{\"memo\":\"keep\"},\"providerOwned\":{\"other\":true}}',notes='user note',activity_type_override='FEE',is_user_modified=1,needs_review=1 WHERE id='x1'",
        [],
    )
    .unwrap();

    let op = DisclosureOperation::new(
        "merchant-context",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "x1",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-02-01T00:00:00Z".into(),
        amount: "25".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: Some(MerchantContextV1 {
            merchant_key: "merchant:coffee".into(),
            merchant_key_version: 1,
            normalization_version: 1,
        }),
    });
    let first = store.commit(op.clone()).await.unwrap();
    assert_eq!(store.commit(op.clone()).await.unwrap(), first);

    let (metadata, notes, override_type, modified, review): (String, String, String, i64, i64) = c
        .query_row(
            "SELECT metadata,notes,activity_type_override,is_user_modified,needs_review FROM activities WHERE id='x1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["user"]["memo"], "keep");
    assert_eq!(metadata["providerOwned"]["other"], true);
    assert_eq!(
        metadata["providerOwned"]["merchantContext"],
        serde_json::json!({
            "merchantKey": "merchant:coffee",
            "merchantKeyVersion": 1,
            "normalizationVersion": 1
        })
    );
    assert_eq!(
        (notes, override_type, modified, review),
        ("user note".into(), "FEE".into(), 1, 1)
    );

    let mut altered = op;
    altered
        .activity
        .as_mut()
        .unwrap()
        .merchant_context
        .as_mut()
        .unwrap()
        .merchant_key = "merchant:tea".into();
    assert!(matches!(
        store.commit(altered).await,
        Err(BridgePolicyError::IdempotencyConflict)
    ));
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
    let binding = "v3;12:deployment-15:alice9:lifecycle1:115:activity_upsert14:pending-commit2:a11:113:posted-commit8:accepted1:00:";
    let payload_digest = disclosure_payload_digest(
        &DisclosureOperation::new(
            "lifecycle",
            "alice",
            generation,
            DisclosureKind::ActivityUpsert,
            "pending-commit",
        )
        .with_lifecycle("posted-commit"),
    );
    oracle_c.execute("INSERT INTO disclosure_receipts(deployment_id,person_subject_id,idempotency_key,account_id,access_generation,operation,resource_id,lifecycle_successor_id,binding_version,receipt_binding,payload_digest,outcome) VALUES('deployment-1','alice','lifecycle','a1',1,'activity_upsert','pending-commit','posted-commit',3,?,?,'accepted')", params![binding, payload_digest]).unwrap();
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
    .with_lifecycle("n-replay-identity")
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        amount: "1".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
    });
    store.commit(first.clone()).await.unwrap();
    assert!(matches!(
        store
            .commit(first.clone().with_lifecycle("other-successor"))
            .await,
        Err(BridgePolicyError::IdempotencyConflict)
    ));
    let changed_payload = first.clone().with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        amount: "2".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
    });
    assert!(matches!(
        store.commit(changed_payload).await,
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
        merchant_context: None,
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

#[tokio::test]
async fn grant_requires_current_owner_membership_and_account_participation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO person_subjects(deployment_id,id) VALUES(?, 'mallory')",
            [D1],
        )
        .unwrap();
    let error = store.grant("mallory", "a1").await.unwrap_err();
    assert!(matches!(error, BridgePolicyError::Storage(_)));
}

#[tokio::test]
async fn replay_rejects_changed_provider_payload() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let operation = DisclosureOperation::new(
        "payload-bound",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "payload-bound-activity",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-07-18".into(),
        amount: "10.00".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
    });
    store.commit(operation.clone()).await.unwrap();
    let mut changed = operation;
    changed.activity.as_mut().unwrap().amount = "11.00".into();
    assert!(matches!(
        store.commit(changed).await.unwrap_err(),
        BridgePolicyError::IdempotencyConflict
    ));
}
