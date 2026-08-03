use rusqlite::Connection;
use wealthfolio_storage_sqlite::db;

#[test]
fn migrations_seed_configurable_cycle_and_restore_native_editing() {
    let dir = tempfile::tempdir().unwrap();
    let path = db::init(dir.path().to_str().unwrap()).unwrap();
    db::run_migrations(&path).unwrap();
    let connection = Connection::open(path).unwrap();

    let cycle: String = connection
        .query_row(
            "SELECT setting_value FROM app_settings WHERE setting_key = 'spending.budget_cycle'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&cycle).unwrap(),
        serde_json::json!({"cadence": "monthly", "anchorDate": "2026-07-25"})
    );

    let finance_policy_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('finance_v1_budget_policy', 'finance_v1_locked_budget_objects')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(finance_policy_count, 0);

    connection
        .execute(
            "INSERT INTO budget_targets(id,period_key,target_type,taxonomy_id,category_id,amount) VALUES('test-period','2026-07-25','category','spending_categories','cat_groceries','1')",
            [],
        )
        .unwrap();
    assert!(connection
        .execute(
            "INSERT OR REPLACE INTO app_settings(setting_key,setting_value) VALUES('spending.budget_cycle','{\"cadence\":\"weekly\",\"anchorDate\":\"2026-07-25\"}')",
            [],
        )
        .is_err());
    for (id, period_key) in [("test-month", "2026-07"), ("test-date", "2026-02-30")] {
        assert!(connection
            .execute(
                "INSERT INTO budget_targets(id,period_key,target_type,taxonomy_id,category_id,amount) VALUES(?1,?2,'category','spending_categories','cat_food','1')",
                [id, period_key],
            )
            .is_err());
    }

    connection
        .execute(
            "INSERT INTO budget_rollover_settings(id,target_type,taxonomy_id,category_id,enabled,start_month,starting_balance) VALUES('rollover','category','spending_categories','cat_groceries',1,'2026-07-25','0')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE budget_targets SET amount='2' WHERE id='finance_v1_target_groceries'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE taxonomy_categories SET name='Insurance edited' WHERE id='finance_v1_cat_insurance'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE budget_groups SET name='Needs edited' WHERE id='032ecb02-5912-42e8-9724-2cd566fc08d5'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE budget_group_assignments SET group_id='8cbd26c8-e3b2-4176-8c61-e5c11e10b808' WHERE id='finance_v1_assign_insurance'",
            [],
        )
        .unwrap();
}
