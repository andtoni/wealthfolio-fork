use diesel::prelude::*;
use std::sync::Arc;
use tempfile::tempdir;
use wealthfolio_spending::categorization_evidence::{
    CategorizationEvidenceImport, CategorizationEvidenceRepositoryTrait, NewCategorizationEvidence,
    AGREEMENT,
};
use wealthfolio_storage_sqlite::db::{
    create_pool, get_connection, init, run_migrations, write_actor::spawn_writer, DbPool,
};
use wealthfolio_storage_sqlite::schema::{
    activities, activity_taxonomy_assignments, spending_categorization_evidence,
    taxonomy_categories,
};
use wealthfolio_storage_sqlite::spending::categorization_evidence::CategorizationEvidenceRepository;

fn setup() -> (CategorizationEvidenceRepository, Arc<DbPool>) {
    let app_data = tempdir().unwrap().keep().to_string_lossy().to_string();
    let db_path = init(&app_data).unwrap();
    run_migrations(&db_path).unwrap();
    let pool = create_pool(&db_path).unwrap();
    let writer = spawn_writer(pool.as_ref().clone()).unwrap();
    (
        CategorizationEvidenceRepository::new(pool.clone(), writer),
        pool,
    )
}

fn mapped_evidence(pool: &Arc<DbPool>) -> NewCategorizationEvidence {
    let mut conn = get_connection(pool).unwrap();
    let (taxonomy_id, category_id) = taxonomy_categories::table
        .inner_join(wealthfolio_storage_sqlite::schema::taxonomies::table)
        .filter(wealthfolio_storage_sqlite::schema::taxonomies::scope.eq("activity"))
        .select((taxonomy_categories::taxonomy_id, taxonomy_categories::id))
        .first::<(String, String)>(&mut conn)
        .unwrap();
    NewCategorizationEvidence {
        merchant_key: "merchant:v1:coffee-shop".into(),
        merchant_key_version: 1,
        normalization_version: 1,
        taxonomy_id,
        category_id,
        direction: "outflow".into(),
        activity_type: "WITHDRAWAL".into(),
        provider_context: Some("csv".into()),
        account_context: None,
        label_count: 8,
        agreement_state: AGREEMENT.into(),
        first_label_date: "2025-01-02".into(),
        last_label_date: "2026-06-03".into(),
        provenance: "trusted-manual-export".into(),
    }
}

#[tokio::test]
async fn dry_run_idempotency_collision_and_rollback_are_fail_closed() {
    let (repo, pool) = setup();
    let record = mapped_evidence(&pool);
    let request = |dry_run, record| CategorizationEvidenceImport {
        import_run_id: "manual-labels-2026-07".into(),
        dry_run,
        records: vec![record],
    };

    let dry = repo.import(request(true, record.clone())).await.unwrap();
    assert_eq!((dry.validated, dry.inserted), (1, 1));
    let mut conn = get_connection(&pool).unwrap();
    assert_eq!(
        spending_categorization_evidence::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        0
    );
    drop(conn);

    let imported = repo.import(request(false, record.clone())).await.unwrap();
    assert_eq!((imported.inserted, imported.skipped_identical), (1, 0));
    let repeated = repo.import(request(false, record.clone())).await.unwrap();
    assert_eq!((repeated.inserted, repeated.skipped_identical), (0, 1));

    let mut conflicting = record;
    conflicting.label_count = 9;
    assert!(repo.import(request(false, conflicting)).await.is_err());
    let mut conn = get_connection(&pool).unwrap();
    assert_eq!(
        spending_categorization_evidence::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        1
    );
    assert_eq!(
        activities::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        0
    );
    assert_eq!(
        activity_taxonomy_assignments::table
            .count()
            .get_result::<i64>(&mut conn)
            .unwrap(),
        0
    );
    drop(conn);

    assert_eq!(
        repo.rollback_import("manual-labels-2026-07").await.unwrap(),
        1
    );
    assert!(repo
        .list_for_merchant("merchant:v1:coffee-shop", 1, 1)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn import_requires_an_explicit_live_taxonomy_mapping() {
    let (repo, pool) = setup();
    let mut record = mapped_evidence(&pool);
    record.category_id = "not-a-current-category".into();
    let result = repo
        .import(CategorizationEvidenceImport {
            import_run_id: "bad-map".into(),
            dry_run: true,
            records: vec![record],
        })
        .await;
    assert!(result.is_err());
}
