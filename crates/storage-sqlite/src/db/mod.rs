use chrono::Local;
use log::{error, info, warn};
use rusqlite::Connection as RusqliteConnection;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use diesel::connection::{Connection, SimpleConnection};
use diesel::r2d2;
use diesel::r2d2::{ConnectionManager, Pool, PooledConnection};
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use wealthfolio_core::errors::{DatabaseError, Error, Result};

use crate::errors::StorageError;

// Keep this invocation in sync with the on-disk migrations directory.
const MIGRATIONS: EmbeddedMigrations = embed_migrations!();
const BACKUP_FILENAME_PREFIX: &str = "wealthfolio_backup_";
const BACKUP_FILENAME_SUFFIX: &str = ".db";
const BACKUP_FILENAME_TIMESTAMP_FORMAT: &str = "%Y%m%d_%H%M%S";

pub type DbPool = r2d2::Pool<ConnectionManager<SqliteConnection>>;
pub type DbConnection = PooledConnection<ConnectionManager<SqliteConnection>>;

pub mod write_actor;
pub use write_actor::WriteHandle;

pub fn init(app_data_dir: &str) -> Result<String> {
    let db_path = get_db_path(app_data_dir);

    // 1. Ensure directory exists
    let db_dir = Path::new(&db_path).parent().unwrap();
    if !db_dir.exists() {
        fs::create_dir_all(db_dir)?;
    }

    {
        let mut conn = SqliteConnection::establish(&db_path).map_err(StorageError::from)?;
        conn.batch_execute(
            "\n            PRAGMA journal_mode = WAL;\n            PRAGMA foreign_keys = ON;\n            PRAGMA busy_timeout = 30000;\n            PRAGMA synchronous  = NORMAL;\n        ",
        ).map_err(StorageError::from)?;
    }

    Ok(db_path)
}

pub fn create_pool(db_path: &str) -> Result<Arc<DbPool>> {
    let manager = ConnectionManager::<SqliteConnection>::new(db_path);
    let pool = r2d2::Pool::builder()
        .max_size(8)
        .min_idle(Some(1)) // Keep at least one connection ready
        .connection_timeout(std::time::Duration::from_secs(30))
        .connection_customizer(Box::new(ConnectionCustomizer {}))
        .build(manager)
        .map_err(|e| DatabaseError::PoolCreationFailed(e.to_string()))?;
    Ok(Arc::new(pool))
}

pub fn run_migrations(db_path: &str) -> Result<()> {
    info!("Running database migrations");
    let mut connection = SqliteConnection::establish(db_path).map_err(StorageError::from)?;

    connection
        .batch_execute(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA foreign_keys = OFF;
            PRAGMA synchronous = OFF;
            PRAGMA cache_size = -64000;
            PRAGMA temp_store = MEMORY;
        ",
        )
        .map_err(StorageError::from)?;

    let migration_result: Result<Vec<String>> = connection
        .run_pending_migrations(MIGRATIONS)
        .map(|versions| {
            versions
                .into_iter()
                .map(|version| version.to_string())
                .collect()
        })
        .map_err(|e| {
            error!("Database migration failed: {}", e);
            Error::Database(DatabaseError::MigrationFailed(e.to_string()))
        });

    // Always attempt to restore connection pragmas, even if migration fails.
    if let Err(e) = connection.batch_execute(
        "
            PRAGMA temp_store = DEFAULT;
            PRAGMA cache_size = -2000;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
        ",
    ) {
        error!("Failed to restore migration PRAGMAs: {}", e);
        if migration_result.is_ok() {
            return Err(Error::Database(DatabaseError::QueryFailed(e.to_string())));
        }
    }

    // Flush WAL to main DB file before pool creation
    connection
        .batch_execute("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap_or_else(|e| warn!("WAL checkpoint after migration failed: {}", e));
    drop(connection);

    let result = migration_result?;

    if result.is_empty() {
        info!("No pending migrations to apply.");
    } else {
        info!("Applied the following migrations:");
        for migration_version in &result {
            info!("  - {}", migration_version);
        }
    }

    Ok(())
}

pub fn get_db_path(input: &str) -> String {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        // On mobile (iOS/Android), always keep the database inside the app's sandbox
        // to avoid permission issues. Ignore DATABASE_URL entirely.
        return Path::new(input)
            .join("app.db")
            .to_str()
            .unwrap()
            .to_string();
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Desktop/server behavior:
        // Prefer DATABASE_URL if provided and non-empty; otherwise, always
        // treat `input` as the app data directory and append `app.db`.
        if let Ok(url) = std::env::var("DATABASE_URL") {
            if !url.trim().is_empty() {
                return url;
            }
        }

        Path::new(input)
            .join("app.db")
            .to_str()
            .unwrap()
            .to_string()
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Text};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn count(conn: &mut SqliteConnection, sql: &str) -> i64 {
        diesel::sql_query(sql)
            .get_result::<CountRow>(conn)
            .unwrap()
            .count
    }

    #[derive(QueryableByName)]
    struct VersionRow {
        #[diesel(sql_type = Text)]
        version: String,
    }

    #[derive(QueryableByName)]
    struct SnapshotRow {
        #[diesel(sql_type = Text)]
        value: String,
    }

    fn latest_version(conn: &mut SqliteConnection) -> String {
        if count(conn, "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table' AND name = '__diesel_schema_migrations'") == 0 {
            return String::new();
        }
        diesel::sql_query(
            "SELECT version FROM __diesel_schema_migrations ORDER BY version DESC LIMIT 1",
        )
        .get_result::<VersionRow>(conn)
        .optional()
        .unwrap()
        .map(|row| row.version)
        .unwrap_or_default()
    }

    fn migrate_until(conn: &mut SqliteConnection, target: &str) {
        loop {
            let version = conn.run_next_migration(MIGRATIONS).unwrap();
            let version = version.to_string();
            assert!(target >= version.as_str(), "migration order changed");
            if version == target {
                break;
            }
        }
    }

    fn receipt_columns(conn: &mut SqliteConnection) -> Vec<String> {
        diesel::sql_query(
            "SELECT name AS value FROM pragma_table_info('disclosure_receipts') ORDER BY cid",
        )
        .load::<SnapshotRow>(conn)
        .unwrap()
        .into_iter()
        .map(|row| row.value)
        .collect()
    }

    fn receipt_row(conn: &mut SqliteConnection) -> String {
        let columns = receipt_columns(conn);
        let quoted: Vec<String> = columns
            .iter()
            .map(|name| format!("\"{}\"", name.replace('\"', "\"\"")))
            .collect();
        let sql = format!(
            "SELECT json_group_array(json(row_json)) AS value FROM (SELECT json_array({}) AS row_json FROM disclosure_receipts ORDER BY {})",
            quoted.join(","),
            quoted.join(",")
        );
        diesel::sql_query(sql)
            .get_result::<SnapshotRow>(conn)
            .unwrap()
            .value
    }

    #[test]
    fn lot_disposals_migration_clears_generated_data() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE accounts (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL
            );

            CREATE TABLE assets (
                id TEXT PRIMARY KEY NOT NULL
            );

            CREATE TABLE activities (
                id TEXT PRIMARY KEY NOT NULL,
                account_id TEXT,
                activity_date TEXT,
                status TEXT,
                activity_type TEXT,
                activity_type_override TEXT,
                source_group_id TEXT
            );

            CREATE TABLE lots (
                id TEXT PRIMARY KEY NOT NULL,
                account_id TEXT NOT NULL,
                asset_id TEXT NOT NULL,
                open_date TEXT NOT NULL,
                open_activity_id TEXT NULL,
                original_quantity TEXT NOT NULL,
                cost_per_unit TEXT NOT NULL,
                original_cost_basis TEXT NOT NULL,
                remaining_cost_basis TEXT NOT NULL,
                fee_allocated TEXT NOT NULL,
                remaining_quantity TEXT NOT NULL,
                split_ratio TEXT NOT NULL,
                is_closed INTEGER NOT NULL,
                close_date TEXT NULL,
                close_activity_id TEXT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE daily_account_valuation (
                id TEXT PRIMARY KEY NOT NULL
            );

            CREATE TABLE holdings_snapshots (
                id TEXT PRIMARY KEY NOT NULL,
                source TEXT NOT NULL
            );

            INSERT INTO accounts (id, name) VALUES ('acc1', 'Account');
            INSERT INTO assets (id) VALUES ('asset1');
            INSERT INTO activities (id) VALUES ('activity1');
            INSERT INTO lots (
                id, account_id, asset_id, open_date, original_quantity,
                cost_per_unit, original_cost_basis, remaining_cost_basis,
                fee_allocated, remaining_quantity, split_ratio, is_closed,
                created_at, updated_at
            ) VALUES (
                'lot1', 'acc1', 'asset1', '2026-01-01', '1',
                '10', '10', '10', '0', '1', '1', 0,
                '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'
            );
            INSERT INTO daily_account_valuation (id) VALUES ('valuation1');
            INSERT INTO holdings_snapshots (id, source) VALUES ('snapshot1', 'CALCULATED');
            INSERT INTO holdings_snapshots (id, source) VALUES ('snapshot2', 'MANUAL_ENTRY');
            ",
        )
        .unwrap();

        conn.batch_execute(include_str!(
            "../../migrations/2026-05-26-000001_lot_disposals/up.sql"
        ))
        .unwrap();

        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM pragma_table_info('lots')
                 WHERE name = 'cost_basis_method'"
            ),
            1
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM pragma_table_info('lot_disposals')
                 WHERE name = 'cost_basis_method' AND dflt_value = '''FIFO'''"
            ),
            1
        );
        assert_eq!(count(&mut conn, "SELECT COUNT(*) AS count FROM lots"), 0);
        assert_eq!(
            count(&mut conn, "SELECT COUNT(*) AS count FROM lot_disposals"),
            0
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM daily_account_valuation"
            ),
            0
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM holdings_snapshots WHERE source = 'CALCULATED'"
            ),
            0
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM holdings_snapshots"
            ),
            1
        );
    }

    #[test]
    fn legacy_receipts_are_backfilled_with_canonical_binding() {
        #[derive(diesel::QueryableByName)]
        struct BindingRow {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            binding_version: i64,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
            receipt_binding: Option<String>,
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("wealthfolio-binding-{nonce}.db"));
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        migrate_until(&mut conn, "20260717000002");
        conn.batch_execute("PRAGMA foreign_keys = OFF; INSERT INTO disclosure_receipts (deployment_id, person_subject_id, idempotency_key, account_id, access_generation, operation, resource_id, lifecycle_successor_id, outcome, binding_version, receipt_binding) VALUES ('d', 'issuer#p', 'k', 'a', 7, 'activity_upsert', 'r', NULL, 'accepted', 0, NULL);").unwrap();
        conn.run_pending_migrations(MIGRATIONS).unwrap();
        let row = diesel::sql_query("SELECT binding_version, receipt_binding FROM disclosure_receipts WHERE idempotency_key='k'")
            .get_result::<BindingRow>(&mut conn)
            .unwrap();
        assert_eq!(row.binding_version, 2);
        assert_eq!(
            row.receipt_binding.as_deref(),
            Some("v2;1:d8:issuer#p1:k1:715:activity_upsert1:r1:a1:00:8:accepted")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn embedded_receipt_migrations_guard_and_revert_preserve_rows() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("wealthfolio-migration-{nonce}.db"));
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        migrate_until(&mut conn, "20260717000002");
        assert_eq!(latest_version(&mut conn), "20260717000002");
        conn.batch_execute("PRAGMA foreign_keys = OFF; INSERT INTO disclosure_receipts (deployment_id, person_subject_id, idempotency_key, account_id, access_generation, operation, resource_id, outcome, binding_version, receipt_binding) VALUES ('d', 'p', 'k', 'a', 7, 'read', 'r', 'ok', 1, 'bound');").unwrap();
        let row = receipt_row(&mut conn);
        let columns = receipt_columns(&mut conn);
        let guarded = conn
            .revert_last_migration(MIGRATIONS)
            .unwrap_err()
            .to_string();
        assert!(guarded.contains("constraint failed"), "{guarded}");
        assert_eq!(latest_version(&mut conn), "20260717000002");
        assert_eq!(receipt_columns(&mut conn), columns);
        assert_eq!(receipt_row(&mut conn), row);
        conn.batch_execute(
            "UPDATE disclosure_receipts SET binding_version = 0, receipt_binding = NULL;",
        )
        .unwrap();
        conn.revert_last_migration(MIGRATIONS).unwrap();
        assert_eq!(latest_version(&mut conn), "20260717000001");
        conn.batch_execute("UPDATE disclosure_receipts SET lifecycle_successor_id = 'successor';")
            .unwrap();
        let identity_columns = receipt_columns(&mut conn);
        let identity_row = receipt_row(&mut conn);
        let guarded = conn
            .revert_last_migration(MIGRATIONS)
            .unwrap_err()
            .to_string();
        assert!(guarded.contains("constraint failed"), "{guarded}");
        assert_eq!(latest_version(&mut conn), "20260717000001");
        assert_eq!(receipt_columns(&mut conn), identity_columns);
        assert_eq!(receipt_row(&mut conn), identity_row);
        conn.batch_execute("UPDATE disclosure_receipts SET lifecycle_successor_id = NULL;")
            .unwrap();
        conn.revert_last_migration(MIGRATIONS).unwrap();
        assert_eq!(latest_version(&mut conn), "20260715000001");
        drop(conn);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn configurable_budget_cycle_migrates_populated_legacy_period_keys() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        migrate_until(&mut conn, "20260730000002");
        conn.batch_execute(
            "INSERT INTO budget_targets
               (id,period_key,target_type,group_id,amount)
             VALUES
               ('legacy-period-target','2026-08','group_buffer',
                '032ecb02-5912-42e8-9724-2cd566fc08d5','123.45');",
        )
        .unwrap();

        conn.run_pending_migrations(MIGRATIONS).unwrap();

        let period_key = diesel::sql_query(
            "SELECT period_key AS value FROM budget_targets WHERE id='legacy-period-target'",
        )
        .get_result::<SnapshotRow>(&mut conn)
        .unwrap()
        .value;
        assert_eq!(period_key, "2026-07-25");
        let cycle = diesel::sql_query(
            "SELECT setting_value AS value FROM app_settings WHERE setting_key='spending.budget_cycle'",
        )
        .get_result::<SnapshotRow>(&mut conn)
        .unwrap()
        .value;
        assert_eq!(cycle, r#"{"cadence":"monthly","anchorDate":"2026-07-25"}"#);
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM finance_v1_budget_policy
                 WHERE currency='ZAR' AND rollover_enabled=0 AND transfers_excluded=1",
            ),
            1
        );
        let locked = conn
            .batch_execute(
                "UPDATE budget_targets SET amount='0' WHERE id='finance_v1_target_salary';",
            )
            .unwrap_err()
            .to_string();
        assert!(
            locked.contains("Finance v1 budget target is locked"),
            "{locked}"
        );
    }

    #[test]
    fn finance_v1_correction_upgrades_colliding_assignments_to_the_exact_snapshot() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        migrate_until(&mut conn, "20260719000001");
        conn.batch_execute(
            "INSERT INTO budget_group_assignments(id,group_id,taxonomy_id,category_id,is_system) VALUES
             ('legacy-subscriptions','6e25d097-0c73-4521-9407-d47e8dfb73e2','spending_categories','cat_bills_subscriptions',0),
             ('legacy-food-delivery','6e25d097-0c73-4521-9407-d47e8dfb73e2','spending_categories','cat_food_delivery',0);",
        )
        .unwrap();

        migrate_until(&mut conn, "20260728000001");

        assert_eq!(
            count(
                &mut conn,
                "SELECT COALESCE(SUM(CAST(target.amount AS INTEGER)), 0) AS count
                 FROM budget_targets target
                 JOIN budget_group_assignments assignment
                   ON assignment.taxonomy_id=target.taxonomy_id
                  AND assignment.category_id=target.category_id
                 JOIN budget_groups budget_group ON budget_group.id=assignment.group_id
                 WHERE target.id LIKE 'finance_v1_target_%' AND budget_group.key='needs'",
            ),
            37000
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COALESCE(SUM(CAST(target.amount AS INTEGER)), 0) AS count
                 FROM budget_targets target
                 JOIN budget_group_assignments assignment
                   ON assignment.taxonomy_id=target.taxonomy_id
                  AND assignment.category_id=target.category_id
                 JOIN budget_groups budget_group ON budget_group.id=assignment.group_id
                 WHERE target.id LIKE 'finance_v1_target_%' AND budget_group.key='wants'",
            ),
            13000
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COALESCE(SUM(CAST(target.amount AS INTEGER)), 0) AS count
                 FROM budget_targets target
                 JOIN budget_group_assignments assignment
                   ON assignment.taxonomy_id=target.taxonomy_id
                  AND assignment.category_id=target.category_id
                 JOIN budget_groups budget_group ON budget_group.id=assignment.group_id
                 WHERE target.id LIKE 'finance_v1_target_%' AND budget_group.key='other'",
            ),
            0
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS count FROM budget_group_assignments
                 WHERE category_id IN ('cat_bills_subscriptions','cat_food_delivery')
                   AND group_id='a409e0d6-9152-49c8-a5b4-a147a8ac636e' AND is_system=1",
            ),
            2
        );
        let locked = conn
            .batch_execute(
                "UPDATE budget_group_assignments SET is_system=0 WHERE id='legacy-subscriptions';",
            )
            .unwrap_err()
            .to_string();
        assert!(
            locked.contains("Finance v1 budget assignment is locked"),
            "{locked}"
        );

        let version = latest_version(&mut conn);
        let reverted = conn
            .revert_last_migration(MIGRATIONS)
            .unwrap_err()
            .to_string();
        assert!(
            reverted.contains("Finance v1 locked budget correction is irreversible"),
            "{reverted}"
        );
        assert_eq!(latest_version(&mut conn), version);
    }
}

fn create_backup_filename(timestamp: chrono::DateTime<Local>) -> String {
    format!(
        "{}{}{}",
        BACKUP_FILENAME_PREFIX,
        timestamp.format(BACKUP_FILENAME_TIMESTAMP_FORMAT),
        BACKUP_FILENAME_SUFFIX
    )
}

pub fn is_valid_backup_filename(filename: &str) -> bool {
    const EXPECTED_LEN: usize =
        BACKUP_FILENAME_PREFIX.len() + "YYYYMMDD_HHMMSS".len() + BACKUP_FILENAME_SUFFIX.len();

    if filename.len() != EXPECTED_LEN
        || !filename.starts_with(BACKUP_FILENAME_PREFIX)
        || !filename.ends_with(BACKUP_FILENAME_SUFFIX)
    {
        return false;
    }

    let timestamp =
        &filename[BACKUP_FILENAME_PREFIX.len()..filename.len() - BACKUP_FILENAME_SUFFIX.len()];
    if timestamp.as_bytes().get(8) != Some(&b'_') {
        return false;
    }

    let compact = timestamp.replace('_', "");
    if compact.len() != 14 || !compact.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }

    chrono::NaiveDateTime::parse_from_str(timestamp, BACKUP_FILENAME_TIMESTAMP_FORMAT).is_ok()
}

pub fn create_backup_path(app_data_dir: &str) -> Result<String> {
    let backup_dir = Path::new(app_data_dir).join("backups");
    fs::create_dir_all(&backup_dir).map_err(|e| {
        error!("Failed to create backup directory: {}", e);
        Error::Database(DatabaseError::BackupFailed(e.to_string()))
    })?;

    let backup_file = create_backup_filename(Local::now());
    let backup_path = backup_dir.join(backup_file);

    Ok(backup_path.to_str().unwrap().to_string())
}

pub fn backup_database_to_file(app_data_dir: &str, backup_path: &str) -> Result<()> {
    let db_path = get_db_path(app_data_dir);

    info!(
        "Creating database backup from {} to {}",
        db_path, backup_path
    );

    if let Some(parent) = Path::new(backup_path).parent() {
        fs::create_dir_all(parent).map_err(|e| {
            error!("Failed to create backup directory: {}", e);
            Error::Database(DatabaseError::BackupFailed(e.to_string()))
        })?;
    }

    if Path::new(backup_path).exists() {
        fs::remove_file(backup_path).map_err(|e| {
            error!("Failed to remove existing backup file: {}", e);
            Error::Database(DatabaseError::BackupFailed(e.to_string()))
        })?;
    }

    let source_conn = RusqliteConnection::open(&db_path).map_err(|e| {
        error!("Failed to open source database for backup: {}", e);
        Error::Database(DatabaseError::BackupFailed(e.to_string()))
    })?;

    source_conn
        .busy_timeout(Duration::from_secs(30))
        .map_err(|e| Error::Database(DatabaseError::BackupFailed(e.to_string())))?;

    source_conn
        .execute_batch("PRAGMA wal_checkpoint(FULL);")
        .unwrap_or_else(|e| warn!("WAL checkpoint before backup failed: {}", e));

    let escaped_backup_path = backup_path.replace('\'', "''");
    let vacuum_sql = format!("VACUUM INTO '{}';", escaped_backup_path);

    source_conn.execute_batch(&vacuum_sql).map_err(|e| {
        error!(
            "Failed to create self-contained backup via VACUUM INTO: {}",
            e
        );
        Error::Database(DatabaseError::BackupFailed(e.to_string()))
    })?;

    info!("Database backup created successfully (self-contained .db)");
    Ok(())
}

pub fn backup_database(app_data_dir: &str) -> Result<String> {
    let backup_path = create_backup_path(app_data_dir)?;

    backup_database_to_file(app_data_dir, &backup_path)?;
    Ok(backup_path)
}

pub fn restore_database(app_data_dir: &str, backup_file_path: &str) -> Result<()> {
    let db_path = get_db_path(app_data_dir);

    info!(
        "Restoring database from {} to {}",
        backup_file_path, db_path
    );

    // Verify backup file exists
    if !Path::new(backup_file_path).exists() {
        return Err(Error::Database(DatabaseError::BackupFailed(
            "Backup file not found".to_string(),
        )));
    }

    // Create backup of current database before restore
    let restore_backup_path = format!(
        "{}.pre-restore-{}",
        db_path,
        Local::now().format("%Y%m%d_%H%M%S")
    );

    if Path::new(&db_path).exists() {
        // Copy main database file
        fs::copy(&db_path, &restore_backup_path).map_err(|e| {
            error!("Failed to create pre-restore backup: {}", e);
            Error::Database(DatabaseError::BackupFailed(e.to_string()))
        })?;

        // Copy WAL file if it exists
        let current_wal_path = format!("{}-wal", db_path);
        let backup_wal_path = format!("{}-wal", restore_backup_path);
        if Path::new(&current_wal_path).exists() {
            fs::copy(&current_wal_path, &backup_wal_path).map_err(|e| {
                error!("Failed to copy WAL file during pre-restore backup: {}", e);
                Error::Database(DatabaseError::BackupFailed(e.to_string()))
            })?;
        }

        // Copy SHM file if it exists
        let current_shm_path = format!("{}-shm", db_path);
        let backup_shm_path = format!("{}-shm", restore_backup_path);
        if Path::new(&current_shm_path).exists() {
            fs::copy(&current_shm_path, &backup_shm_path).map_err(|e| {
                error!("Failed to copy SHM file during pre-restore backup: {}", e);
                Error::Database(DatabaseError::BackupFailed(e.to_string()))
            })?;
        }

        info!(
            "Created pre-restore backup at: {} (including WAL/SHM files if present)",
            restore_backup_path
        );
    }

    // Remove existing WAL and SHM files to ensure clean state.
    // On Windows, these files might be locked by active connections; tolerate sharing violations.
    let wal_path = format!("{}-wal", db_path);
    let shm_path = format!("{}-shm", db_path);

    if Path::new(&wal_path).exists() {
        if let Err(e) = try_remove_file_best_effort(&wal_path, "WAL") {
            return Err(Error::Database(DatabaseError::BackupFailed(e.to_string())));
        }
    }

    if Path::new(&shm_path).exists() {
        if let Err(e) = try_remove_file_best_effort(&shm_path, "SHM") {
            return Err(Error::Database(DatabaseError::BackupFailed(e.to_string())));
        }
    }

    // Copy the main backup file
    copy_with_retries(
        backup_file_path,
        &db_path,
        5,
        std::time::Duration::from_millis(200),
    )?;

    // Copy WAL file if it exists in backup
    let backup_wal_path = format!("{}-wal", backup_file_path);
    if Path::new(&backup_wal_path).exists() {
        if let Err(e) = copy_with_retries(
            &backup_wal_path,
            &wal_path,
            3,
            std::time::Duration::from_millis(200),
        ) {
            // WAL copy failure is non-fatal; DB will recreate WAL on next write.
            warn!("Failed to restore WAL file (non-fatal): {}", e);
        }
    }

    // Copy SHM file if it exists in backup
    let backup_shm_path = format!("{}-shm", backup_file_path);
    if Path::new(&backup_shm_path).exists() {
        if let Err(e) = copy_with_retries(
            &backup_shm_path,
            &shm_path,
            3,
            std::time::Duration::from_millis(200),
        ) {
            // SHM copy failure is non-fatal; it'll be recreated as needed.
            warn!("Failed to restore SHM file (non-fatal): {}", e);
        }
    }

    // Ensure desired journal mode; recreate WAL after restore for consistency
    if let Ok(mut conn) = SqliteConnection::establish(&db_path) {
        let _ = conn.batch_execute("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;");
    }

    info!("Database restored successfully");
    Ok(())
}

/// Function to safely restore database with connection management
/// This is the main function that should be called from Tauri commands
pub fn restore_database_safe(app_data_dir: &str, backup_file_path: &str) -> Result<()> {
    // First, execute a checkpoint to force WAL content to be written to the main database file
    // This helps reduce the chance of WAL files being locked on Windows
    let db_path = get_db_path(app_data_dir);

    // Try to checkpoint the database before restore
    if let Ok(mut conn) = SqliteConnection::establish(&db_path) {
        use diesel::RunQueryDsl;
        let _ = diesel::sql_query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&mut conn);
        // Try to temporarily switch to DELETE journal mode to minimize WAL interactions
        let _ = diesel::sql_query("PRAGMA journal_mode = DELETE").execute(&mut conn);
        info!("Executed WAL checkpoint before restore");
    }

    // Small delay to allow any pending operations to complete
    std::thread::sleep(std::time::Duration::from_millis(150));

    // Now perform the actual restore
    restore_database(app_data_dir, backup_file_path)
}

/// Gets a connection from the pool
pub fn get_connection(pool: &Pool<ConnectionManager<SqliteConnection>>) -> Result<DbConnection> {
    Ok(pool.get().map_err(StorageError::from)?)
}

#[derive(Debug)]
struct ConnectionCustomizer;

impl r2d2::CustomizeConnection<SqliteConnection, diesel::r2d2::Error> for ConnectionCustomizer {
    fn on_acquire(
        &self,
        conn: &mut SqliteConnection,
    ) -> std::result::Result<(), diesel::r2d2::Error> {
        // IMPORTANT: Use batch_execute (sqlite3_exec) instead of sql_query (sqlite3_prepare_v2).
        // sql_query only executes the FIRST statement; subsequent PRAGMAs are silently ignored.
        conn.batch_execute(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 30000;
             PRAGMA synchronous = NORMAL;",
        )
        .map_err(diesel::r2d2::Error::QueryError)?;

        Ok(())
    }
}

// --- Internal helpers for robust, cross-platform file operations ---

/// Determine if an IO error on Windows is a sharing/lock violation.
#[inline]
#[allow(unused_variables)]
fn is_sharing_violation(e: &io::Error) -> bool {
    #[cfg(target_os = "windows")]
    {
        matches!(e.raw_os_error(), Some(32) | Some(33))
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Try to remove a file; on Windows, tolerate sharing violations and continue.
fn try_remove_file_best_effort(path: &str, label: &str) -> std::result::Result<(), io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) => {
            if is_sharing_violation(&e) {
                warn!(
                    "{} file appears to be in use ({}). Proceeding with restore anyway.",
                    label, e
                );
                Ok(())
            } else if e.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                error!("Failed to remove existing {} file '{}': {}", label, path, e);
                Err(e)
            }
        }
    }
}

/// Copy a file with retry/backoff; maps errors into existing Result type on failure.
fn copy_with_retries(
    src: &str,
    dst: &str,
    attempts: usize,
    backoff: std::time::Duration,
) -> Result<()> {
    let mut last_err: Option<io::Error> = None;
    for i in 0..attempts {
        match fs::copy(src, dst) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                // On Windows, if destination is locked, wait and retry
                if let Some(ref err) = last_err {
                    if is_sharing_violation(err) {
                        warn!(
                            "Attempt {}/{}: destination appears locked when copying to '{}'. Retrying in {:?}...",
                            i + 1,
                            attempts,
                            dst,
                            backoff
                        );
                        std::thread::sleep(backoff);
                        continue;
                    }
                }
                // For other errors, retry a couple times anyway
                warn!(
                    "Attempt {}/{}: failed to copy '{}' -> '{}': {}. Retrying in {:?}...",
                    i + 1,
                    attempts,
                    src,
                    dst,
                    last_err.as_ref().unwrap(),
                    backoff
                );
                std::thread::sleep(backoff);
            }
        }
    }
    let e = last_err.unwrap_or_else(|| io::Error::other("unknown copy error"));
    error!("Failed to copy '{}' -> '{}': {}", src, dst, e);
    Err(Error::Database(DatabaseError::BackupFailed(e.to_string())))
}

/// Trait for executing database transactions
pub trait DbTransactionExecutor {
    /// Execute operations within a transaction and return the result
    fn execute<F, T, E>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut DbConnection) -> std::result::Result<T, E>,
        E: Into<Error>;
}

/// Implementation of DbTransactionExecutor for DbPool
impl DbTransactionExecutor for DbPool {
    fn execute<F, T, E>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut DbConnection) -> std::result::Result<T, E>,
        E: Into<Error>,
    {
        let mut conn = self.get().map_err(StorageError::from)?;

        conn.transaction(|tx_conn| {
            f(tx_conn).map_err(|_| diesel::result::Error::RollbackTransaction)
        })
        .map_err(|e| Error::Database(DatabaseError::QueryFailed(e.to_string())))
    }
}

/// Implementation of DbTransactionExecutor for Arc<DbPool>
impl DbTransactionExecutor for Arc<DbPool> {
    fn execute<F, T, E>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut DbConnection) -> std::result::Result<T, E>,
        E: Into<Error>,
    {
        (**self).execute(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    #[test]
    fn validates_backup_filename_contract() {
        assert!(is_valid_backup_filename(
            "wealthfolio_backup_20260514_150409.db"
        ));

        for filename in [
            "../wealthfolio_backup_20260514_150409.db",
            "wealthfolio_backup_20260514_150409.db\0",
            "wealthfolio_backup_20260514_150409.sqlite",
            "wealthfolio_backup_20260514_150409_123.db",
            "wealthfolio_backup_20260514150409.db",
            "wealthfolio_backup_20260514_15040x.db",
            "wealthfolio_backup_20260231_150409.db",
            "other_backup_20260514_150409.db",
        ] {
            assert!(
                !is_valid_backup_filename(filename),
                "expected {filename} to be rejected"
            );
        }
    }

    #[test]
    fn generated_backup_filename_matches_validator() {
        let timestamp = Local
            .with_ymd_and_hms(2026, 5, 14, 15, 4, 9)
            .single()
            .unwrap();

        assert_eq!(
            create_backup_filename(timestamp),
            "wealthfolio_backup_20260514_150409.db"
        );
        assert!(is_valid_backup_filename(&create_backup_filename(timestamp)));
    }

    #[test]
    fn create_backup_path_uses_valid_backup_filename() {
        let app_data = tempdir().unwrap();
        let backup_path = create_backup_path(app_data.path().to_str().unwrap()).unwrap();
        let filename = Path::new(&backup_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap();

        assert!(is_valid_backup_filename(filename));
    }
}
