use wealthfolio_core::accounts::{
    VerifiedReceipt, VerifiedReceiptBasis, VerifiedReceiptRepositoryTrait,
};
use wealthfolio_storage_sqlite::{accounts::VerifiedReceiptRepository, db};

fn receipt(id: &str, account_id: &str, as_of: &str) -> VerifiedReceipt {
    VerifiedReceipt {
        id: id.into(),
        account_id: account_id.into(),
        basis: VerifiedReceiptBasis::CurrentSnapshot,
        as_of: as_of.into(),
        provider_receipt_ref: Some(format!("provider-{id}")),
        verification_state: "verified".into(),
        coverage_start_date: None,
        created_at: "caller-created-at".into(),
        updated_at: "caller-updated-at".into(),
    }
}

#[tokio::test]
async fn upsert_rejects_account_rebinding_and_returns_persisted_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = db::init(dir.path().to_str().unwrap()).unwrap();
    db::run_migrations(&path).unwrap();
    let repository = VerifiedReceiptRepository::new(db::create_pool(&path).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    for account_id in ["account-a", "account-b"] {
        connection.execute(
            "INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode) VALUES(?,?,'CASH','USD',0,1,0,'TRANSACTIONS')",
            [account_id, account_id],
        ).unwrap();
    }

    let persisted = repository
        .upsert(receipt("receipt-1", "account-a", "2026-07-17"))
        .await
        .unwrap();
    assert_eq!(persisted.id, "receipt-1");
    assert_eq!(persisted.account_id, "account-a");
    assert_ne!(persisted.created_at, "caller-created-at");
    assert_ne!(persisted.updated_at, "caller-updated-at");

    let error = repository
        .upsert(receipt("receipt-1", "account-b", "2026-07-18"))
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("verified receipt account identity mismatch"));

    let row: (String, String) = connection
        .query_row(
            "SELECT account_id,as_of FROM verified_receipts WHERE id='receipt-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("account-a".into(), "2026-07-17".into()));
}
