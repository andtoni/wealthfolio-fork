use std::sync::Arc;

use rusqlite::Connection;
use tempfile::tempdir;
use wealthfolio_spending::settings::{
    BudgetCadence, BudgetCycleSettings, SpendingSettingsRepositoryTrait, SpendingSettingsService,
    SpendingSettingsUpdate, SETTING_KEY_BUDGET_CYCLE,
};
use wealthfolio_storage_sqlite::db::{
    create_pool, init, run_migrations, write_actor::spawn_writer, DbPool,
};
use wealthfolio_storage_sqlite::spending::settings::SpendingSettingsRepository;

fn setup() -> (String, SpendingSettingsRepository, Arc<DbPool>) {
    let app_data = tempdir().unwrap().keep().to_string_lossy().to_string();
    let db_path = init(&app_data).unwrap();
    run_migrations(&db_path).unwrap();
    let pool = create_pool(&db_path).unwrap();
    let writer = spawn_writer(pool.as_ref().clone()).unwrap();
    (
        db_path,
        SpendingSettingsRepository::new(pool.clone(), writer),
        pool,
    )
}

fn insert_rebase_rows(db_path: &str, invalid_rollover_start: Option<&str>) {
    let connection = Connection::open(db_path).unwrap();
    connection
        .execute(
            "INSERT INTO budget_targets
                (id, period_key, target_type, taxonomy_id, category_id, amount, created_at, updated_at)
             VALUES
                ('legacy-july-target', '2026-07-25', 'category', 'spending_categories', 'cat_groceries', '100', 'created-july', 'updated-july'),
                ('legacy-aug-target', '2026-08-25', 'category', 'spending_categories', 'cat_groceries', '125', 'created-aug', 'updated-aug'),
                ('default-target', 'default', 'category', 'spending_categories', 'cat_groceries', '50', 'created-default', 'updated-default')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO budget_rollover_settings
                (id, target_type, taxonomy_id, category_id, enabled, start_month, starting_balance, created_at, updated_at)
             VALUES ('rollover', 'category', 'spending_categories', 'cat_food', 1, ?1, '10', 'created-rollover', 'updated-rollover')",
            [invalid_rollover_start.unwrap_or("2026-07-25")],
        )
        .unwrap();
}

fn expected_target_id(period_key: &str) -> String {
    format!(
        "budget_target:category:{}:{}:{}:{}:{}:{}",
        period_key.len(),
        period_key,
        "spending_categories".len(),
        "spending_categories",
        "cat_groceries".len(),
        "cat_groceries",
    )
}

fn insert_collision_rows(db_path: &str) {
    let connection = Connection::open(db_path).unwrap();
    connection
        .execute(
            "INSERT INTO budget_targets
                (id, period_key, target_type, taxonomy_id, category_id, amount)
             VALUES
                ('collision-one', '2026-02-28', 'category', 'spending_categories', 'cat_groceries', '100'),
                ('collision-two', '2026-03-30', 'category', 'spending_categories', 'cat_groceries', '125')",
            [],
        )
        .unwrap();
}

#[tokio::test]
async fn rebasing_preserves_budget_data_and_projects_identity_changes_atomically() {
    let (db_path, repository, _pool) = setup();
    insert_rebase_rows(&db_path, None);

    let new_cycle = BudgetCycleSettings {
        cadence: BudgetCadence::Monthly,
        anchor_date: "2026-08-01".to_string(),
    };
    let service = SpendingSettingsService::new(Arc::new(repository));
    service
        .update(SpendingSettingsUpdate {
            budget_cycle: Some(new_cycle.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let connection = Connection::open(&db_path).unwrap();
    let cycle: String = connection
        .query_row(
            "SELECT setting_value FROM app_settings WHERE setting_key = ?1",
            [SETTING_KEY_BUDGET_CYCLE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&cycle).unwrap(),
        serde_json::json!({"cadence": "monthly", "anchorDate": "2026-08-01"})
    );

    let targets: Vec<(String, String, String, String)> = connection
        .prepare(
            "SELECT id, period_key, amount, created_at
             FROM budget_targets ORDER BY period_key, id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        targets,
        vec![
            (
                expected_target_id("2026-07-01"),
                "2026-07-01".to_string(),
                "100".to_string(),
                "created-july".to_string(),
            ),
            (
                expected_target_id("2026-08-01"),
                "2026-08-01".to_string(),
                "125".to_string(),
                "created-aug".to_string(),
            ),
            (
                "default-target".to_string(),
                "default".to_string(),
                "50".to_string(),
                "created-default".to_string(),
            ),
        ]
    );

    let rollover: (String, String, String) = connection
        .query_row(
            "SELECT id, start_month, starting_balance FROM budget_rollover_settings",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        rollover,
        ("rollover".into(), "2026-07-01".into(), "10".into())
    );

    let outbox: Vec<(String, String, String)> = connection
        .prepare("SELECT entity, entity_id, op FROM sync_outbox ORDER BY rowid")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        outbox,
        vec![
            (
                "budget_target".to_string(),
                "legacy-aug-target".to_string(),
                "delete".to_string(),
            ),
            (
                "budget_target".to_string(),
                "legacy-july-target".to_string(),
                "delete".to_string(),
            ),
            (
                "budget_target".to_string(),
                expected_target_id("2026-07-01"),
                "create".to_string(),
            ),
            (
                "budget_target".to_string(),
                expected_target_id("2026-08-01"),
                "create".to_string(),
            ),
            (
                "budget_rollover_setting".to_string(),
                "rollover".to_string(),
                "update".to_string(),
            ),
            (
                "spending_setting".to_string(),
                SETTING_KEY_BUDGET_CYCLE.to_string(),
                "update".to_string(),
            ),
        ]
    );
}

#[tokio::test]
async fn rebasing_combines_targets_that_share_a_new_period() {
    let (db_path, repository, _pool) = setup();
    insert_collision_rows(&db_path);
    Connection::open(&db_path)
        .unwrap()
        .execute(
            "UPDATE app_settings SET setting_value = '{\"cadence\":\"monthly\",\"anchorDate\":\"2026-01-30\"}' WHERE setting_key = ?1",
            [SETTING_KEY_BUDGET_CYCLE],
        )
        .unwrap();

    repository
        .update_budget_cycle(
            BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: "2026-01-30".to_string(),
            },
            BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: "2026-01-31".to_string(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let connection = Connection::open(&db_path).unwrap();
    let target: (String, String, String) = connection
        .query_row(
            "SELECT id, period_key, amount FROM budget_targets WHERE period_key = '2026-02-28'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        target,
        (
            expected_target_id("2026-02-28"),
            "2026-02-28".to_string(),
            "225".to_string(),
        )
    );
    let target_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM budget_targets WHERE period_key <> 'default'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(target_count, 1);
}

#[tokio::test]
async fn rebasing_rejects_a_stale_cycle_without_mutating_data() {
    let (db_path, repository, _pool) = setup();
    insert_rebase_rows(&db_path, None);
    Connection::open(&db_path)
        .unwrap()
        .execute(
            "UPDATE app_settings SET setting_value = '{\"cadence\":\"monthly\",\"anchorDate\":\"2026-08-01\"}' WHERE setting_key = ?1",
            [SETTING_KEY_BUDGET_CYCLE],
        )
        .unwrap();

    let result = repository
        .update_budget_cycle(
            BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: "2026-07-25".to_string(),
            },
            BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: "2026-09-01".to_string(),
            },
            Vec::new(),
        )
        .await;
    assert!(result.is_err());

    let connection = Connection::open(&db_path).unwrap();
    let target_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM budget_targets WHERE id IN ('legacy-july-target', 'legacy-aug-target')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(target_count, 2);
    let outbox_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
        .unwrap();
    assert_eq!(outbox_count, 0);
}

#[tokio::test]
async fn rebasing_rolls_back_targets_and_cycle_when_a_later_row_is_invalid() {
    let (db_path, repository, _pool) = setup();
    insert_rebase_rows(&db_path, Some("2026-07-26"));

    let result = repository
        .update_budget_cycle(
            BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: "2026-07-25".to_string(),
            },
            BudgetCycleSettings {
                cadence: BudgetCadence::Monthly,
                anchor_date: "2026-08-01".to_string(),
            },
            Vec::new(),
        )
        .await;
    assert!(result.is_err());

    let connection = Connection::open(&db_path).unwrap();
    let cycle: String = connection
        .query_row(
            "SELECT setting_value FROM app_settings WHERE setting_key = ?1",
            [SETTING_KEY_BUDGET_CYCLE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&cycle).unwrap(),
        serde_json::json!({"cadence": "monthly", "anchorDate": "2026-07-25"})
    );
    let target_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM budget_targets WHERE id IN ('legacy-july-target', 'legacy-aug-target')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(target_count, 2);
    let outbox_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
        .unwrap();
    assert_eq!(outbox_count, 0);
}
