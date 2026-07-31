use rusqlite::Connection;
use wealthfolio_storage_sqlite::db;

#[test]
fn migrations_seed_configurable_cycle_without_weakening_locked_policy() {
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

    let policy_columns = connection
        .prepare("PRAGMA table_info(finance_v1_budget_policy)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!policy_columns.contains(&"cycle_start_day".to_string()));
    assert!(!policy_columns.contains(&"cycle_end_day".to_string()));
    assert!(policy_columns.contains(&"transfers_excluded".to_string()));

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

    assert!(connection
        .execute(
            "INSERT INTO budget_rollover_settings(id,target_type,taxonomy_id,category_id,enabled,start_month,starting_balance) VALUES('rollover','category','spending_categories','cat_groceries',1,'2026-07-25','0')",
            [],
        )
        .is_err());
}
