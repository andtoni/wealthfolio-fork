#![allow(clippy::type_complexity)]

use chrono::NaiveDate;
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
async fn current_state_repairs_legacy_broker_snapshot_without_provenance() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    let snapshot_id = wealthfolio_core::portfolio::snapshot::AccountStateSnapshot::stable_id(
        "a1",
        NaiveDate::parse_from_str("2026-07-18", "%Y-%m-%d").unwrap(),
    );
    connection
        .execute(
            "INSERT INTO holdings_snapshots(id,account_id,snapshot_date,currency,source) VALUES(?,?,?,?,?)",
            params![snapshot_id, "a1", "2026-07-18", "USD", "BROKER_IMPORTED"],
        )
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();

    let receipt = store
        .commit(current_state_operation(
            "legacy-current-state",
            generation,
            "50",
            "10",
        ))
        .await
        .unwrap();

    assert_eq!(receipt.outcome, ReceiptOutcome::Accepted);
    let snapshot_role: String = connection
        .query_row(
            "SELECT role FROM provenance_records WHERE entity_type='snapshot' AND entity_id=?",
            params![snapshot_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshot_role, "live_canonical");
    let verified_receipt_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM verified_receipts WHERE account_id='a1' AND verification_state='verified'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(verified_receipt_count, 1);
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
async fn bridge_normalizes_date_only_activity_dates_and_preserves_precision() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    store
        .commit(
            DisclosureOperation::new(
                "date-only",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "date-only-row",
            )
            .with_activity(ActivityPayload {
                account_id: "a1".into(),
                activity_type: "WITHDRAWAL".into(),
                status: "POSTED".into(),
                activity_date: "2026-08-14".into(),
                activity_date_precision: Some(
                    wealthfolio_core::activities::ActivityDatePrecision::Date,
                ),
                amount: "1".into(),
                currency: "ZAR".into(),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    let c = Connection::open(path).unwrap();
    let row: (String, String) = c
        .query_row(
            "SELECT activity_date, activity_date_precision FROM activities WHERE id='date-only-row'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("2026-08-14T00:00:00+00:00".into(), "DATE".into()));
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
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "999".into(),
        currency: "EUR".into(),
        notes: Some("must not write".into()),
        activity_type_override: None,
        needs_review: Some(true),
        merchant_context: None,
        ..Default::default()
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
    c.execute("UPDATE activities SET activity_type='TRANSFER_IN',notes='user note',activity_type_override='FEE',subtype='USER_SUBTYPE',is_user_modified=1,needs_review=1 WHERE id='x1'", []).unwrap();

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
                activity_date_precision: Some(
                    wealthfolio_core::activities::ActivityDatePrecision::Date,
                ),
                amount: "25".into(),
                currency: "USD".into(),
                notes: Some("provider note".into()),
                activity_type_override: Some("PROVIDER_OVERRIDE".into()),
                needs_review: Some(false),
                merchant_context: None,
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    let overlays: (String, Option<String>, Option<String>, Option<String>, i64, i64, String) = c.query_row(
        "SELECT activity_type,notes,activity_type_override,subtype,is_user_modified,needs_review,amount FROM activities WHERE id='x1'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).unwrap();
    assert_eq!(
        overlays,
        (
            "TRANSFER_IN".into(),
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
                activity_date_precision: Some(
                    wealthfolio_core::activities::ActivityDatePrecision::Date,
                ),
                amount: "25".into(),
                currency: "USD".into(),
                notes: Some("provider note".into()),
                activity_type_override: Some("PROVIDER_OVERRIDE".into()),
                needs_review: Some(true),
                merchant_context: None,
                ..Default::default()
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
    assert_eq!(overlays.2, 1);
    chrono::DateTime::parse_from_rfc3339(&overlays.3).unwrap();
    chrono::DateTime::parse_from_rfc3339(&overlays.4).unwrap();
    assert_eq!(overlays.5, 1);
}

#[tokio::test]
async fn provider_extended_fields_round_trip_without_touching_user_overlays() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    store
        .commit(
            DisclosureOperation::new(
                "provider-extended",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "provider-extended-row",
            )
            .with_activity(ActivityPayload {
                account_id: "a1".into(),
                activity_type: "WITHDRAWAL".into(),
                status: "POSTED".into(),
                activity_date: "2026-08-05T16:07:00Z".into(),
                activity_date_precision: Some(
                    wealthfolio_core::activities::ActivityDatePrecision::Instant,
                ),
                settlement_date: Some("2026-08-06".into()),
                amount: "193.91".into(),
                currency: "ZAR".into(),
                fee: Some("1.25".into()),
                tax: Some("0.10".into()),
                fx_rate: Some("18.25".into()),
                source_type: Some("CardPurchases".into()),
                subtype: Some("CARD_PURCHASE".into()),
                source_record_id: Some("settled-1".into()),
                source_group_id: None,
                notes: Some("provider note".into()),
                activity_type_override: None,
                needs_review: Some(true),
                metadata: Some(
                    serde_json::json!({"providerOwned":{"evidence":{"reference":"ref-1"}}}),
                ),
                original_amount: Some("11.50".into()),
                original_currency: Some("USD".into()),
                settlement_amount: Some("193.91".into()),
                settlement_currency: Some("ZAR".into()),
                merchant_context: None,
            }),
        )
        .await
        .unwrap();
    let c = Connection::open(&path).unwrap();
    let date_shape: (String, String) = c
        .query_row(
            "SELECT activity_date, activity_date_precision FROM activities WHERE id='provider-extended-row'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        date_shape,
        (
            "2026-08-05T16:07:00+00:00".to_string(),
            "INSTANT".to_string()
        )
    );
    let row: (String, String, String, String, String, String, String, String, String, i64) = c
        .query_row(
            "SELECT settlement_date,source_type,subtype,fee,tax,fx_rate,currency,source_record_id,metadata,needs_review FROM activities WHERE id='provider-extended-row'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?)),
        )
        .unwrap();
    assert_eq!(row.0, "2026-08-06");
    assert_eq!(row.1, "CardPurchases");
    assert_eq!(row.2, "CARD_PURCHASE");
    assert_eq!(row.3, "1.25");
    assert_eq!(row.4, "0.10");
    assert_eq!(row.5, "18.25");
    assert_eq!(row.6, "ZAR");
    assert_eq!(row.7, "settled-1");
    let metadata: serde_json::Value = serde_json::from_str(&row.8).unwrap();
    assert_eq!(metadata["user"], serde_json::Value::Null);
    assert_eq!(metadata["providerOwned"]["evidence"]["reference"], "ref-1");
    assert_eq!(metadata["providerOwned"]["original"]["currency"], "USD");
    assert_eq!(metadata["providerOwned"]["settlement"]["currency"], "ZAR");
    assert_eq!(row.9, 1);
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
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Instant),
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
        ..Default::default()
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
    assert_eq!(metadata["providerOwned"]["other"], serde_json::Value::Null);
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
async fn commit_lifecycle_rebinds_an_older_projection_generation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("pending-generation", "posted-generation")
        .await
        .unwrap();
    let first_generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute(
        "INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)",
        params![D1, "alice", "pending-generation", "a1", first_generation],
    )
    .unwrap();

    // Granting another account advances the deployment-wide generation while
    // the already-projected pending activity remains valid for the same owner.
    let current_generation = store.grant("alice", "a2").await.unwrap();
    assert!(current_generation > first_generation);
    store
        .commit(
            DisclosureOperation::new(
                "lifecycle-generation",
                "alice",
                current_generation,
                DisclosureKind::ActivityUpsert,
                "pending-generation",
            )
            .with_lifecycle("posted-generation"),
        )
        .await
        .unwrap();

    let pending: i64 = c
        .query_row(
            "SELECT count(*) FROM activities WHERE id='pending-generation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let posted: i64 = c
        .query_row(
            "SELECT count(*) FROM activities WHERE id='posted-generation' AND status='POSTED'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let projection_generation: i64 = c
        .query_row(
            "SELECT access_generation FROM activity_disclosure_projections WHERE activity_id='posted-generation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        (pending, posted, projection_generation),
        (0, 1, current_generation)
    );
}

#[tokio::test]
async fn commit_lifecycle_replay_preserves_pending_children_when_posted_children_exist() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-overlays", "n-overlays")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO taxonomy_categories(id,taxonomy_id,name,key,color,sort_order,created_at,updated_at) VALUES('bridge-posted-category','bridge-taxonomy','Posted','posted','#111',1,'now','now')", []).unwrap();
    c.execute("INSERT INTO activity_taxonomy_assignments(id,activity_id,taxonomy_id,category_id) VALUES('bridge-posted-assignment','n-overlays','bridge-taxonomy','bridge-posted-category')", []).unwrap();
    c.execute("INSERT INTO spending_activity_events(activity_id,event_id) VALUES('n-overlays','bridge-event')", []).unwrap();
    c.execute("INSERT INTO activity_attribution_overlays(deployment_id,activity_id,account_id,attributed_person_subject_id,revision) VALUES(?,?,'a1','alice',2)", params![D1, "n-overlays"]).unwrap();
    for activity_id in ["p-overlays", "n-overlays"] {
        c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", activity_id, "a1", generation]).unwrap();
    }

    store
        .commit(
            DisclosureOperation::new(
                "overlay-replay",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "p-overlays",
            )
            .with_lifecycle("n-overlays"),
        )
        .await
        .unwrap();

    let state: (i64, String, String, String, String, i64, i64) = c.query_row(
        "SELECT (SELECT count(*) FROM activities WHERE id='p-overlays'), (SELECT id FROM activity_taxonomy_assignments WHERE activity_id='n-overlays'), (SELECT id FROM spending_activity_splits WHERE activity_id='n-overlays'), (SELECT event_id FROM spending_activity_events WHERE activity_id='n-overlays'), (SELECT attributed_person_subject_id FROM activity_attribution_overlays WHERE deployment_id=? AND activity_id='n-overlays'), (SELECT revision FROM activity_attribution_overlays WHERE deployment_id=? AND activity_id='n-overlays'), (SELECT count(*) FROM activity_disclosure_projections WHERE deployment_id=? AND person_subject_id='alice' AND activity_id='n-overlays')",
        params![D1, D1, D1],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    ).unwrap();
    assert_eq!(
        state,
        (
            0,
            "bridge-assignment".into(),
            "bridge-split".into(),
            "bridge-event".into(),
            "alice".into(),
            1,
            1
        )
    );
}

#[tokio::test]
async fn commit_lifecycle_rejects_two_split_sets_without_mutation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-splits", "n-splits")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("INSERT INTO spending_activity_splits(id,activity_id,taxonomy_id,category_id,amount) VALUES('bridge-posted-split','n-splits','bridge-taxonomy','bridge-category','1')", []).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-splits", "a1", generation]).unwrap();

    assert!(store
        .commit(
            DisclosureOperation::new(
                "split-conflict",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "p-splits"
            )
            .with_lifecycle("n-splits"),
        )
        .await
        .is_err());

    let state: (i64, i64, i64) = c.query_row(
        "SELECT (SELECT count(*) FROM activities WHERE id IN ('p-splits','n-splits')), (SELECT count(*) FROM spending_activity_splits WHERE activity_id IN ('p-splits','n-splits')), (SELECT count(*) FROM disclosure_receipts WHERE idempotency_key='split-conflict')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(state, (2, 2, 0));
}

#[tokio::test]
async fn commit_lifecycle_rejects_conflicting_attribution_without_mutation() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("p-attribution", "n-attribution")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute("UPDATE activity_attribution_overlays SET provider_actor='pending-actor' WHERE deployment_id=? AND activity_id='p-attribution'", [D1]).unwrap();
    c.execute("INSERT INTO activity_attribution_overlays(deployment_id,activity_id,account_id,attributed_person_subject_id,provider_actor,revision) VALUES(?,?,'a1','alice','posted-actor',2)", params![D1, "n-attribution"]).unwrap();
    c.execute("INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)", params![D1, "alice", "p-attribution", "a1", generation]).unwrap();

    assert!(store
        .commit(
            DisclosureOperation::new(
                "attribution-conflict",
                "alice",
                generation,
                DisclosureKind::ActivityUpsert,
                "p-attribution"
            )
            .with_lifecycle("n-attribution"),
        )
        .await
        .is_err());

    let state: (i64, i64, i64) = c.query_row(
        "SELECT (SELECT count(*) FROM activities WHERE id IN ('p-attribution','n-attribution')), (SELECT count(*) FROM activity_attribution_overlays WHERE deployment_id=? AND activity_id IN ('p-attribution','n-attribution')), (SELECT count(*) FROM disclosure_receipts WHERE idempotency_key='attribution-conflict')",
        [D1],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(state, (2, 2, 0));
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
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "1".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
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
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "2".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
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
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "1".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
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
async fn legacy_precision_omission_replays_to_existing_receipt() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    let operation = DisclosureOperation::new(
        "legacy-date-replay",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "legacy-date-activity",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-07-18".into(),
        activity_date_precision: None,
        amount: "10.00".into(),
        currency: "USD".into(),
        ..Default::default()
    });
    let accepted = store.commit(operation.clone()).await.unwrap();
    let replay = store.commit(operation).await.unwrap();
    assert_eq!(replay, accepted);
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
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "10.00".into(),
        currency: "USD".into(),
        notes: None,
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
    });
    store.commit(operation.clone()).await.unwrap();
    let mut changed = operation;
    changed.activity.as_mut().unwrap().amount = "11.00".into();
    assert!(matches!(
        store.commit(changed).await.unwrap_err(),
        BridgePolicyError::IdempotencyConflict
    ));
}

#[tokio::test]
async fn provider_identity_supersession_preserves_user_overlay_and_projection() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    connection.execute(
        "INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity','x1','live_canonical','finance','x1','alice')",
        [],
    ).unwrap();
    connection
        .execute(
            "UPDATE activities SET notes='user note',activity_type_override='TRANSFER_IN',source_group_id='group-1',is_user_modified=1 WHERE id='x1'",
            [],
        )
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    connection
        .execute(
            "INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)",
            params![D1, "alice", "x1", "a1", generation],
        )
        .unwrap();

    let operation = DisclosureOperation::new(
        "prime-saver-supersession",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "stable-prime-resource",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-07-18".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "10.00".into(),
        currency: "USD".into(),
        notes: Some("provider note".into()),
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
    })
    .with_superseded_resources(["x1".to_string()]);
    store.commit(operation.clone()).await.unwrap();

    let result: (i64, i64, String, Option<String>, Option<String>, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM activities WHERE id='x1'), (SELECT count(*) FROM activities WHERE id='stable-prime-resource'), (SELECT notes FROM activities WHERE id='stable-prime-resource'), (SELECT activity_type_override FROM activities WHERE id='stable-prime-resource'), (SELECT source_group_id FROM activities WHERE id='stable-prime-resource'), (SELECT count(*) FROM activity_disclosure_projections WHERE activity_id='stable-prime-resource' AND person_subject_id='alice')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        result,
        (
            0,
            1,
            "user note".into(),
            Some("TRANSFER_IN".into()),
            Some("group-1".into()),
            1
        )
    );
    assert_eq!(
        store.commit(operation).await.unwrap().outcome,
        ReceiptOutcome::Accepted
    );
}

#[tokio::test]
async fn provider_identity_supersession_preserves_effective_activity_type() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    connection.execute(
        "INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity','x1','live_canonical','finance','x1','alice')",
        [],
    ).unwrap();
    connection
        .execute(
            "UPDATE activities SET activity_type='TRANSFER_IN',activity_type_override=NULL,source_group_id='group-1',is_user_modified=1 WHERE id='x1'",
            [],
        )
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let operation = DisclosureOperation::new(
        "prime-saver-type-preservation",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "stable-prime-type-resource",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-07-18".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "10.00".into(),
        currency: "USD".into(),
        notes: Some("provider note".into()),
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
    })
    .with_superseded_resources(["x1".to_string()]);
    store.commit(operation).await.unwrap();

    let result: (String, Option<String>, Option<String>, i32) = connection
        .query_row(
            "SELECT activity_type,activity_type_override,source_group_id,is_user_modified FROM activities WHERE id='stable-prime-type-resource'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        result,
        (
            "DEPOSIT".into(),
            Some("TRANSFER_IN".into()),
            Some("group-1".into()),
            1
        )
    );
}

#[tokio::test]
async fn legacy_prime_saver_supersession_preserves_linked_overlay_and_provider_notes() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    connection.execute(
        "UPDATE activities SET id='wealthfolio-production:incarnation:synthetic-v2:weak' WHERE id='x1'",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity','wealthfolio-production:incarnation:synthetic-v2:weak','live_canonical','finance','x1','alice')",
        [],
    ).unwrap();
    connection.execute(
        "UPDATE activities SET activity_type='TRANSFER_OUT',source_record_id='synthetic-v2:weak',notes='A Example 10000000001',source_group_id='group-1',is_user_modified=1 WHERE id='wealthfolio-production:incarnation:synthetic-v2:weak'",
        [],
    ).unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();

    let target = DisclosureOperation::new(
        "legacy-prime-target",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "wealthfolio-production:incarnation:synthetic-v2:strong",
    ).with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-08-06".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "70000".into(),
        currency: "ZAR".into(),
        notes: Some("Transfer to Card Internet Value Date 06Aug26 MBP0000000001 10000000001 A Example 10000000001".into()),
        source_record_id: Some("synthetic-v2:strong".into()),
        source_group_id: Some("provider-group".into()),
        ..Default::default()
    });
    store.commit(target).await.unwrap();

    let correction = DisclosureOperation::new(
        "legacy-prime-correction",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "wealthfolio-production:incarnation:synthetic-v2:strong",
    ).with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-08-06".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "70000".into(),
        currency: "ZAR".into(),
        notes: Some("Transfer to Card Internet Value Date 06Aug26 MBP0000000001 10000000001 A Example 10000000001".into()),
        source_record_id: Some("synthetic-v2:strong".into()),
        source_group_id: Some("provider-group".into()),
        ..Default::default()
    }).with_superseded_resources(["wealthfolio-production:incarnation:synthetic-v2:weak".to_string()]);
    store.commit(correction).await.unwrap();

    let result: (i64, String, Option<String>, Option<String>, String) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM activities WHERE id='wealthfolio-production:incarnation:synthetic-v2:weak'), notes, activity_type_override, source_group_id, metadata FROM activities WHERE id='wealthfolio-production:incarnation:synthetic-v2:strong'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert_eq!(result.0, 0);
    assert_eq!(result.1, "A Example 10000000001");
    assert_eq!(result.2.as_deref(), Some("TRANSFER_OUT"));
    assert_eq!(result.3.as_deref(), Some("group-1"));
    assert!(result.4.contains("A Example 10000000001"));
}

#[tokio::test]
async fn provider_identity_supersession_rejects_nonlegacy_transfer_group_conflict() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let connection = Connection::open(&path).unwrap();
    connection.execute(
        "INSERT INTO provenance_records(entity_type,entity_id,role,source_system,source_record_id,actor_id) VALUES('activity','x1','live_canonical','finance','x1','alice')",
        [],
    ).unwrap();
    connection
        .execute(
            "UPDATE activities SET source_group_id='user-group',is_user_modified=1 WHERE id='x1'",
            [],
        )
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let operation = DisclosureOperation::new(
        "conflicting-provider-group",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "new-provider-resource",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-07-18".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "10.00".into(),
        currency: "USD".into(),
        source_record_id: Some("new-provider-resource".into()),
        source_group_id: Some("provider-group".into()),
        ..Default::default()
    })
    .with_superseded_resources(["x1".to_string()]);
    assert!(matches!(
        store.commit(operation).await.unwrap_err(),
        BridgePolicyError::Storage(_)
    ));
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM activities WHERE id='x1'", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM activities WHERE id='new-provider-resource'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn provider_identity_supersession_moves_overlay_children_atomically() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    store
        .seed_migration_fixture("old-provider", "new-provider")
        .await
        .unwrap();
    let generation = store.grant("alice", "a1").await.unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO activity_disclosure_projections(deployment_id,person_subject_id,activity_id,account_id,access_generation) VALUES(?,?,?,?,?)",
            params![D1, "alice", "old-provider", "a1", generation],
        )
        .unwrap();
    let operation = DisclosureOperation::new(
        "provider-children",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "new-provider",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "DEPOSIT".into(),
        status: "POSTED".into(),
        activity_date: "2026-07-18".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "1".into(),
        currency: "USD".into(),
        notes: Some("provider correction".into()),
        activity_type_override: None,
        needs_review: None,
        merchant_context: None,
        ..Default::default()
    })
    .with_superseded_resources(["old-provider".to_string()]);
    store.commit(operation).await.unwrap();
    let counts: (i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM activities WHERE id='old-provider'), (SELECT count(*) FROM activity_taxonomy_assignments WHERE activity_id='new-provider'), (SELECT count(*) FROM spending_activity_splits WHERE activity_id='new-provider'), (SELECT count(*) FROM spending_activity_events WHERE activity_id='new-provider'), (SELECT count(*) FROM activity_attribution_overlays WHERE activity_id='new-provider'), (SELECT count(*) FROM activity_disclosure_projections WHERE activity_id='new-provider' AND person_subject_id='alice')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();
    assert_eq!(counts, (0, 1, 1, 1, 1, 1));
}

#[tokio::test]
async fn provider_upsert_omitting_original_preserves_existing_provider_original() {
    let (_dir, path, store) = store(D1).await;
    seed(&path, &[D1], &["alice"]);
    let generation = store.grant("alice", "a1").await.unwrap();
    // First upsert creates original evidence
    let op1 = DisclosureOperation::new(
        "orig-1",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "act-orig",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "100".into(),
        currency: "ZAR".into(),
        original_amount: Some("5".into()),
        original_currency: Some("USD".into()),
        ..Default::default()
    });
    store.commit(op1).await.unwrap();

    // Second upsert omits original fields entirely – original must be preserved
    let op2 = DisclosureOperation::new(
        "orig-2",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "act-orig",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "100".into(),
        currency: "ZAR".into(),
        ..Default::default()
    });
    store.commit(op2).await.unwrap();

    let c = Connection::open(&path).unwrap();
    let metadata: serde_json::Value = c
        .query_row(
            "SELECT metadata FROM activities WHERE id='act-orig'",
            [],
            |row| {
                let s: String = row.get(0)?;
                Ok(serde_json::from_str(&s).unwrap())
            },
        )
        .unwrap();
    assert_eq!(metadata["providerOwned"]["original"]["amount"], "5");
    assert_eq!(metadata["providerOwned"]["original"]["currency"], "USD");

    // Partial incoming pair must NOT create malformed original – preserve existing
    let op3 = DisclosureOperation::new(
        "orig-3",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "act-orig",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "100".into(),
        currency: "ZAR".into(),
        original_amount: Some("6".into()),
        // original_currency omitted – partial
        ..Default::default()
    });
    store.commit(op3).await.unwrap();

    let metadata2: serde_json::Value = c
        .query_row(
            "SELECT metadata FROM activities WHERE id='act-orig'",
            [],
            |row| {
                let s: String = row.get(0)?;
                Ok(serde_json::from_str(&s).unwrap())
            },
        )
        .unwrap();
    // Original must remain the original valid pair, not a partial
    assert_eq!(metadata2["providerOwned"]["original"]["amount"], "5");
    assert_eq!(metadata2["providerOwned"]["original"]["currency"], "USD");

    // Complete incoming pair must replace existing original evidence
    let op4 = DisclosureOperation::new(
        "orig-4",
        "alice",
        generation,
        DisclosureKind::ActivityUpsert,
        "act-orig",
    )
    .with_activity(ActivityPayload {
        account_id: "a1".into(),
        activity_type: "WITHDRAWAL".into(),
        status: "POSTED".into(),
        activity_date: "2026-01-01".into(),
        activity_date_precision: Some(wealthfolio_core::activities::ActivityDatePrecision::Date),
        amount: "100".into(),
        currency: "ZAR".into(),
        original_amount: Some("7".into()),
        original_currency: Some("EUR".into()),
        ..Default::default()
    });
    store.commit(op4).await.unwrap();

    let metadata3: serde_json::Value = c
        .query_row(
            "SELECT metadata FROM activities WHERE id='act-orig'",
            [],
            |row| {
                let s: String = row.get(0)?;
                Ok(serde_json::from_str(&s).unwrap())
            },
        )
        .unwrap();
    assert_eq!(metadata3["providerOwned"]["original"]["amount"], "7");
    assert_eq!(metadata3["providerOwned"]["original"]["currency"], "EUR");
}
