use wealthfolio_core::accounts::{ProviderIdentity, ProviderIdentityRepositoryTrait};
use wealthfolio_storage_sqlite::{accounts::ProviderIdentityRepository, db};

fn identity(account_id: &str, display_name: &str) -> ProviderIdentity {
    ProviderIdentity {
        account_id: account_id.into(),
        provider: "plaid".into(),
        provider_account_id: format!("provider-{account_id}"),
        lifecycle_state: "active".into(),
        provider_display_name: Some(display_name.into()),
        last_sync_at: Some("2026-07-17T00:00:00Z".into()),
    }
}

#[tokio::test]
async fn provider_identity_round_trips_display_name_and_lists_in_stable_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = db::init(dir.path().to_str().unwrap()).unwrap();
    db::run_migrations(&path).unwrap();
    let pool = db::create_pool(&path).unwrap();
    let repository = ProviderIdentityRepository::new(pool);

    let connection = rusqlite::Connection::open(&path).unwrap();
    for id in ["account-b", "account-a"] {
        connection.execute(
            "INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,is_archived,tracking_mode,name_source) VALUES(?,?,'CASH','USD',0,1,0,'TRANSACTIONS','user')",
            [id, id],
        ).unwrap();
    }

    repository
        .upsert(identity("account-b", "Provider B"))
        .await
        .unwrap();
    repository
        .upsert(identity("account-a", "Provider A"))
        .await
        .unwrap();

    let loaded = repository
        .get_by_account("account-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.provider_display_name.as_deref(), Some("Provider A"));
    assert_eq!(loaded.last_sync_at.as_deref(), Some("2026-07-17T00:00:00Z"));

    let updated = ProviderIdentity {
        provider_display_name: Some("Provider A updated".into()),
        lifecycle_state: "closed".into(),
        ..loaded
    };
    repository.upsert(updated).await.unwrap();
    let loaded = repository
        .get_by_account("account-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.provider_display_name.as_deref(),
        Some("Provider A updated")
    );
    assert_eq!(loaded.lifecycle_state, "closed");

    let mismatch = ProviderIdentity {
        provider_account_id: "substituted-provider-id".into(),
        ..loaded
    };
    let error = repository.upsert(mismatch).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("immutable provider identity mismatch"));

    let listed = repository.list_all().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|item| item.account_id.as_str())
            .collect::<Vec<_>>(),
        vec!["account-a", "account-b"]
    );
    assert!(listed
        .iter()
        .all(|item| item.provider_display_name.is_some()));
}
