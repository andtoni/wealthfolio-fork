//! Storage adapter for spending::activity_budget_periods — Diesel impl over
//! the `spending_activity_budget_periods` overlay table.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use diesel::prelude::*;

use crate::db::{get_connection, DbPool, WriteHandle};
use crate::errors::StorageError;
use crate::schema::spending_activity_budget_periods;
use wealthfolio_spending::activity_budget_periods::{
    ActivityBudgetPeriodRepositoryTrait, BudgetPeriodsByActivity,
};

pub struct ActivityBudgetPeriodRepository {
    pool: Arc<DbPool>,
    writer: WriteHandle,
}

impl ActivityBudgetPeriodRepository {
    pub fn new(pool: Arc<DbPool>, writer: WriteHandle) -> Self {
        Self { pool, writer }
    }
}

#[async_trait]
impl ActivityBudgetPeriodRepositoryTrait for ActivityBudgetPeriodRepository {
    async fn list_all(&self) -> Result<BudgetPeriodsByActivity> {
        let mut conn = get_connection(&self.pool).map_err(|e| anyhow::anyhow!(e))?;
        let rows: Vec<(String, String)> = spending_activity_budget_periods::table
            .select((
                spending_activity_budget_periods::activity_id,
                spending_activity_budget_periods::period_start,
            ))
            .load(&mut conn)
            .map_err(StorageError::from)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(rows.into_iter().collect())
    }

    async fn list_for_activities(&self, ids: &[String]) -> Result<BudgetPeriodsByActivity> {
        let mut out = BudgetPeriodsByActivity::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let mut conn = get_connection(&self.pool).map_err(|e| anyhow::anyhow!(e))?;
        const CHUNK: usize = 500;
        for chunk in ids.chunks(CHUNK) {
            let rows: Vec<(String, String)> = spending_activity_budget_periods::table
                .filter(spending_activity_budget_periods::activity_id.eq_any(chunk))
                .select((
                    spending_activity_budget_periods::activity_id,
                    spending_activity_budget_periods::period_start,
                ))
                .load(&mut conn)
                .map_err(StorageError::from)
                .map_err(|e| anyhow::anyhow!(e))?;
            out.extend(rows);
        }
        Ok(out)
    }

    async fn set_many(&self, activity_ids: &[String], period_start: Option<String>) -> Result<()> {
        let activity_ids = activity_ids.to_vec();
        self.writer
            .exec(move |conn| {
                let now = chrono::Utc::now().to_rfc3339();
                for activity_id in &activity_ids {
                    match &period_start {
                        Some(period_start) => {
                            diesel::insert_into(spending_activity_budget_periods::table)
                                .values((
                                    spending_activity_budget_periods::activity_id.eq(activity_id),
                                    spending_activity_budget_periods::period_start.eq(period_start),
                                    spending_activity_budget_periods::created_at.eq(&now),
                                    spending_activity_budget_periods::updated_at.eq(&now),
                                ))
                                .on_conflict(spending_activity_budget_periods::activity_id)
                                .do_update()
                                .set((
                                    spending_activity_budget_periods::period_start.eq(period_start),
                                    spending_activity_budget_periods::updated_at.eq(&now),
                                ))
                                .execute(conn)
                                .map_err(StorageError::from)?;
                        }
                        None => {
                            diesel::delete(spending_activity_budget_periods::table.filter(
                                spending_activity_budget_periods::activity_id.eq(activity_id),
                            ))
                            .execute(conn)
                            .map_err(StorageError::from)?;
                        }
                    }
                }
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_pool, get_connection, init, run_migrations, write_actor::spawn_writer};
    use tempfile::tempdir;

    fn setup_db() -> (Arc<DbPool>, WriteHandle) {
        std::env::set_var("CONNECT_API_URL", "http://test.local");
        let app_data = tempdir()
            .expect("tempdir")
            .keep()
            .to_string_lossy()
            .to_string();
        let db_path = init(&app_data).expect("init db");
        run_migrations(&db_path).expect("migrate db");
        let pool = create_pool(&db_path).expect("create pool");
        let writer = spawn_writer(pool.as_ref().clone()).expect("spawn writer");
        (pool, writer)
    }

    fn insert_activity(conn: &mut SqliteConnection, id: &str) {
        diesel::sql_query(
            "INSERT OR IGNORE INTO accounts \
             (id, name, account_type, `group`, currency, is_default, is_active, created_at, updated_at, \
              platform_id, account_number, meta, provider, provider_account_id, is_archived, tracking_mode) \
             VALUES ('account-1', 'Account', 'cash', NULL, 'ZAR', 0, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, \
                     NULL, NULL, NULL, NULL, NULL, 0, 'portfolio')"
        )
        .execute(conn)
        .expect("insert account");
        diesel::sql_query(format!(
            "INSERT INTO activities \
             (id, account_id, activity_type, status, activity_date, amount, currency, created_at, updated_at) \
             VALUES ('{id}', 'account-1', 'WITHDRAWAL', 'POSTED', '2026-09-23T16:30:00Z', '100', 'ZAR', 'now', 'now')"
        ))
        .execute(conn)
        .expect("insert activity");
    }

    #[tokio::test]
    async fn sets_replaces_and_clears_the_chosen_period() {
        let (pool, writer) = setup_db();
        {
            let mut conn = get_connection(&pool).expect("conn");
            insert_activity(&mut conn, "a1");
            insert_activity(&mut conn, "a2");
        }
        let repo = ActivityBudgetPeriodRepository::new(pool.clone(), writer);
        let ids = vec!["a1".to_string(), "a2".to_string()];

        repo.set_many(&ids, Some("2026-09-25".to_string()))
            .await
            .expect("set");
        repo.set_many(&ids[..1], Some("2026-08-25".to_string()))
            .await
            .expect("replace");
        let all = repo.list_all().await.expect("list all");
        assert_eq!(all.get("a1").map(String::as_str), Some("2026-08-25"));
        assert_eq!(all.get("a2").map(String::as_str), Some("2026-09-25"));

        repo.set_many(&ids[..1], None).await.expect("clear");
        let remaining = repo.list_for_activities(&ids).await.expect("list");
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains_key("a2"));
    }

    #[tokio::test]
    async fn deleting_the_activity_removes_its_chosen_period() {
        let (pool, writer) = setup_db();
        {
            let mut conn = get_connection(&pool).expect("conn");
            insert_activity(&mut conn, "a1");
        }
        let repo = ActivityBudgetPeriodRepository::new(pool.clone(), writer);
        repo.set_many(&["a1".to_string()], Some("2026-09-25".to_string()))
            .await
            .expect("set");
        {
            let mut conn = get_connection(&pool).expect("conn");
            diesel::sql_query("DELETE FROM activities WHERE id = 'a1'")
                .execute(&mut conn)
                .expect("delete activity");
        }
        assert!(repo.list_all().await.expect("list all").is_empty());
    }
}
