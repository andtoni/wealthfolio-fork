use rusqlite::{params, Connection};
use tempfile::TempDir;
use wealthfolio_storage_sqlite::{
    db,
    historical_import::{
        historical_evidence_payload_digest, historical_manifest_hash, EvidenceAuthority,
        HistoricalEvidenceRow, HistoricalImportBatch, HistoricalImportError, HistoricalImportFault,
        HistoricalImportRepository, HistoricalLedgerRow, HistoricalManifestEvidenceRow,
        HistoricalManifestLedgerRow, HistoricalOperation, HistoricalReviewedManifest,
        HistoricalSource, LedgerDisposition,
    },
};

const ALICE: &str = "urn:test#alice";
const BOB: &str = "urn:test#bob";

async fn setup() -> (TempDir, String, HistoricalImportRepository) {
    let dir = tempfile::tempdir().unwrap();
    let path = db::init(dir.path().to_str().unwrap()).unwrap();
    db::run_migrations(&path).unwrap();
    let pool = db::create_pool(&path).unwrap();
    let writer = db::write_actor::spawn_writer((*pool).clone()).unwrap();
    let repository = HistoricalImportRepository::new(writer);
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    connection
        .execute("INSERT INTO policy_deployments(id) VALUES('default')", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO households(deployment_id,id) VALUES('default','h')",
            [],
        )
        .unwrap();
    for subject in [ALICE, BOB] {
        connection
            .execute(
                "INSERT INTO person_subjects(deployment_id,id) VALUES('default',?)",
                [subject],
            )
            .unwrap();
        connection.execute("INSERT INTO household_membership_epochs(deployment_id,household_id,person_subject_id,role,start_epoch) VALUES('default','h',?,'owner',1)", [subject]).unwrap();
    }
    for (account, subject) in [("a1", ALICE), ("a2", BOB)] {
        connection.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode) VALUES(?,?,'CASH','USD',0,1,0,'TRANSACTIONS')", params![account, account]).unwrap();
        connection.execute("INSERT INTO native_account_deployments(account_id,deployment_id) VALUES(?,'default')", [account]).unwrap();
        connection.execute("INSERT INTO account_access_epochs(deployment_id,household_id,account_id,person_subject_id,start_generation) VALUES('default','h',?,?,1)", params![account, subject]).unwrap();
    }
    (dir, path, repository)
}

fn ledger_row(identity: &str, account: &str) -> HistoricalLedgerRow {
    HistoricalLedgerRow {
        idempotency_key: identity.into(),
        historical_identity: identity.into(),
        source: HistoricalSource::Tarn,
        source_id: format!("source-{identity}"),
        account_id: account.into(),
        activity_type: "WITHDRAWAL".into(),
        effective_at: "2025-06-01T12:00:00Z".into(),
        cutover_watermark: "2026-01-01T00:00:00Z".into(),
        amount: "-12.50".into(),
        currency: "USD".into(),
        raw_description: "Cafe".into(),
    }
}

fn evidence_row(path: &str) -> HistoricalEvidenceRow {
    let connection = Connection::open(path).unwrap();
    let (taxonomy_id, category_id) = connection
        .query_row(
            "SELECT taxonomy_id,id FROM taxonomy_categories LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    HistoricalEvidenceRow {
        idempotency_key: "evidence-1".into(),
        merchant_key: "merchant:cafe".into(),
        merchant_key_version: 1,
        normalization_version: 1,
        taxonomy_id,
        category_id,
        direction: "outflow".into(),
        activity_type: "WITHDRAWAL".into(),
        provider_context: None,
        account_id: "a1".into(),
        source: HistoricalSource::Tarn,
        source_category_ref: "food".into(),
        first_label_date: "2025-01-01".into(),
        last_label_date: "2025-06-01".into(),
        label_count: 3,
        agreement_state: "agreement".into(),
        authority: EvidenceAuthority::UserConfirmed,
        manifest_ledger_identity: "l1".into(),
        ledger_disposition: LedgerDisposition::Selected,
        overlap_provenance: vec![],
    }
}

fn manifest(
    account: &str,
    identities: &[&str],
    evidence: &[HistoricalEvidenceRow],
) -> HistoricalReviewedManifest {
    HistoricalReviewedManifest {
        ledger: identities
            .iter()
            .map(|identity| HistoricalManifestLedgerRow {
                identity: (*identity).into(),
                source: HistoricalSource::Tarn,
                account_id: account.into(),
                disposition: LedgerDisposition::Selected,
            })
            .collect(),
        evidence: evidence
            .iter()
            .map(|row| HistoricalManifestEvidenceRow {
                identity: row.idempotency_key.clone(),
                source: row.source,
                account_id: row.account_id.clone(),
                authority: row.authority,
                payload_digest: historical_evidence_payload_digest(row),
            })
            .collect(),
        overlap_resolutions: vec![],
    }
}

fn batch(
    subject: &str,
    run: &str,
    key: &str,
    manifest: Option<HistoricalReviewedManifest>,
    operation: HistoricalOperation,
) -> HistoricalImportBatch {
    let manifest_hash = manifest.as_ref().map(historical_manifest_hash);
    HistoricalImportBatch {
        schema_version: 1,
        subject: subject.into(),
        import_run_id: run.into(),
        idempotency_key: key.into(),
        dry_run: false,
        overlaps_resolved: true,
        manifest_hash,
        manifest,
        operation,
    }
}

fn count(path: &str, table: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[tokio::test]
async fn staging_is_invisible_and_finalize_requires_complete_reviewed_identities() {
    let (_dir, path, repository) = setup().await;
    let reviewed = manifest("a1", &["l1", "l2"], &[]);
    repository
        .apply(batch(
            ALICE,
            "run",
            "stage-1",
            Some(reviewed.clone()),
            HistoricalOperation::StageLedger {
                rows: vec![ledger_row("l1", "a1")],
            },
        ))
        .await
        .unwrap();
    assert_eq!(count(&path, "historical_import_staged_ledger"), 1);
    assert_eq!(count(&path, "activities"), 0);
    assert!(matches!(
        repository
            .apply(batch(
                ALICE,
                "run",
                "finalize-incomplete",
                Some(reviewed),
                HistoricalOperation::Finalize,
            ))
            .await,
        Err(HistoricalImportError::Invalid(_))
    ));
    assert_eq!(count(&path, "activities"), 0);
}

#[tokio::test]
async fn finalize_is_atomic_publishes_all_domains_and_exact_replay_returns_receipt() {
    let (_dir, path, repository) = setup().await;
    let evidence = evidence_row(&path);
    let reviewed = manifest("a1", &["l1", "l2"], std::slice::from_ref(&evidence));
    let ledger = batch(
        ALICE,
        "run",
        "stage-ledger",
        Some(reviewed.clone()),
        HistoricalOperation::StageLedger {
            rows: vec![ledger_row("l1", "a1"), ledger_row("l2", "a1")],
        },
    );
    repository.apply(ledger.clone()).await.unwrap();
    assert_eq!(repository.apply(ledger).await.unwrap().written, 2);
    repository
        .apply(batch(
            ALICE,
            "run",
            "stage-evidence",
            Some(reviewed.clone()),
            HistoricalOperation::StageLearningEvidence {
                rows: vec![evidence],
            },
        ))
        .await
        .unwrap();
    let finalize = batch(
        ALICE,
        "run",
        "finalize",
        Some(reviewed),
        HistoricalOperation::Finalize,
    );
    assert!(repository
        .apply_with_fault(finalize.clone(), HistoricalImportFault::AfterFirstWrite)
        .await
        .is_err());
    assert_eq!(count(&path, "activities"), 0);
    assert_eq!(count(&path, "spending_categorization_evidence"), 0);
    assert_eq!(count(&path, "historical_import_staged_ledger"), 2);
    let receipt = repository.apply(finalize.clone()).await.unwrap();
    assert_eq!(receipt.written, 3);
    assert_eq!(repository.apply(finalize).await.unwrap(), receipt);
    assert_eq!(count(&path, "activities"), 2);
    assert_eq!(count(&path, "provenance_records"), 2);
    assert_eq!(count(&path, "spending_categorization_evidence"), 1);
    assert_eq!(count(&path, "historical_import_staged_ledger"), 0);
    assert_eq!(count(&path, "historical_import_staged_evidence"), 0);
}

#[tokio::test]
async fn evidence_digest_and_cross_subject_accounts_are_refused_before_staging() {
    let (_dir, path, repository) = setup().await;
    let evidence = evidence_row(&path);
    let mut reviewed = manifest("a1", &["l1"], std::slice::from_ref(&evidence));
    reviewed.evidence[0].payload_digest = "0".repeat(64);
    assert!(matches!(
        repository
            .apply(batch(
                ALICE,
                "digest",
                "stage",
                Some(reviewed),
                HistoricalOperation::StageLearningEvidence {
                    rows: vec![evidence]
                },
            ))
            .await,
        Err(HistoricalImportError::Invalid(_))
    ));
    let denied = manifest("a2", &["l1"], &[]);
    assert!(matches!(
        repository
            .apply(batch(
                ALICE,
                "denied",
                "stage",
                Some(denied),
                HistoricalOperation::StageLedger {
                    rows: vec![ledger_row("l1", "a2")]
                },
            ))
            .await,
        Err(HistoricalImportError::AccountAccessDenied)
    ));
    assert_eq!(count(&path, "historical_import_runs"), 0);
}

#[tokio::test]
async fn rollback_is_atomic_for_staged_and_active_runs_and_replays_exact_receipt() {
    let (_dir, path, repository) = setup().await;
    for run in ["staged", "active"] {
        let reviewed = manifest("a1", &["l1"], &[]);
        repository
            .apply(batch(
                ALICE,
                run,
                &format!("stage-{run}"),
                Some(reviewed.clone()),
                HistoricalOperation::StageLedger {
                    rows: vec![ledger_row("l1", "a1")],
                },
            ))
            .await
            .unwrap();
        if run == "active" {
            repository
                .apply(batch(
                    ALICE,
                    run,
                    "finalize-active",
                    Some(reviewed),
                    HistoricalOperation::Finalize,
                ))
                .await
                .unwrap();
        }
        let rollback = batch(
            ALICE,
            run,
            &format!("rollback-{run}"),
            None,
            HistoricalOperation::Rollback,
        );
        if run == "active" {
            assert!(repository
                .apply_with_fault(
                    rollback.clone(),
                    HistoricalImportFault::AfterFirstRollbackDelete
                )
                .await
                .is_err());
            assert_eq!(count(&path, "activities"), 1);
        }
        let receipt = repository.apply(rollback.clone()).await.unwrap();
        assert_eq!(repository.apply(rollback).await.unwrap(), receipt);
    }
    assert_eq!(count(&path, "activities"), 0);
    assert_eq!(count(&path, "historical_import_staged_ledger"), 0);
}
