use crate::DbPool;
use chrono::NaiveDate;
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{Bool, Nullable, Text},
};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use wealthfolio_core::accounts::{ProvenanceRecord, ProvenanceRepositoryTrait, ProvenanceRole};
use wealthfolio_core::errors::{Result, ValidationError};

pub struct ProvenanceRepository {
    pool: Arc<DbPool>,
}

impl ProvenanceRepository {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ProvenanceRepositoryTrait for ProvenanceRepository {
    async fn get(&self, entity_type: &str, entity_id: &str) -> Result<Option<ProvenanceRecord>> {
        let mut connection = self.pool.get().map_err(to_core_err)?;
        let row = sql_query(
            "SELECT entity_type, entity_id, role, source_system, source_record_id, import_run_id,
                    actor_id, coverage_start_date, manual_current_approved
             FROM provenance_records WHERE entity_type = ? AND entity_id = ?",
        )
        .bind::<Text, _>(entity_type)
        .bind::<Text, _>(entity_id)
        .get_result::<ProvenanceRow>(&mut connection)
        .optional()
        .map_err(to_core_err)?;
        Ok(row.map(ProvenanceRow::into_domain))
    }

    async fn upsert(&self, record: ProvenanceRecord) -> Result<ProvenanceRecord> {
        let mut connection = self.pool.get().map_err(to_core_err)?;
        sql_query(
            "INSERT INTO provenance_records
             (entity_type, entity_id, role, source_system, source_record_id, import_run_id,
              actor_id, coverage_start_date, manual_current_approved)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(entity_type, entity_id) DO UPDATE SET
               role=excluded.role, source_system=excluded.source_system,
               source_record_id=excluded.source_record_id, import_run_id=excluded.import_run_id,
               actor_id=excluded.actor_id, coverage_start_date=excluded.coverage_start_date,
               manual_current_approved=excluded.manual_current_approved,
               updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')",
        )
        .bind::<Text, _>(&record.entity_type)
        .bind::<Text, _>(&record.entity_id)
        .bind::<Text, _>(role_to_str(record.role))
        .bind::<Nullable<Text>, _>(record.source_system.as_deref())
        .bind::<Nullable<Text>, _>(record.source_record_id.as_deref())
        .bind::<Nullable<Text>, _>(record.import_run_id.as_deref())
        .bind::<Nullable<Text>, _>(record.actor_id.as_deref())
        .bind::<Nullable<Text>, _>(record.coverage_start_date.as_deref())
        .bind::<Bool, _>(record.manual_current_approved)
        .execute(&mut connection)
        .map_err(to_core_err)?;
        Ok(record)
    }

    async fn eligible_entity_ids(&self, entity_type: &str, row_date: &str) -> Result<Vec<String>> {
        let mut connection = self.pool.get().map_err(to_core_err)?;
        let rows = sql_query(
            "SELECT entity_id FROM provenance_records
             WHERE entity_type = ? AND
               (role = 'live_canonical' OR
                (role = 'opening_baseline' AND coverage_start_date IS NOT NULL AND coverage_start_date <= ?) OR
                (role = 'manual_current' AND manual_current_approved = 1))
             ORDER BY entity_id",
        )
        .bind::<Text, _>(entity_type)
        .bind::<Text, _>(row_date)
        .load::<EntityIdRow>(&mut connection)
        .map_err(to_core_err)?;
        Ok(rows.into_iter().map(|row| row.entity_id).collect())
    }

    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<String>> {
        let requested = validate_and_deduplicate_row_dates(rows)?;
        let mut connection = self.pool.get().map_err(to_core_err)?;
        let records = sql_query(
            "SELECT entity_id, role, coverage_start_date, manual_current_approved
             FROM provenance_records WHERE entity_type = ?",
        )
        .bind::<Text, _>(entity_type)
        .load::<EligibilityRow>(&mut connection)
        .map_err(to_core_err)?;
        Ok(records
            .into_iter()
            .filter_map(|record| {
                let date = requested.get(&record.entity_id)?;
                role_from_str(&record.role)
                    .eligible_for_aggregate(
                        &date.to_string(),
                        record.coverage_start_date.as_deref(),
                        record.manual_current_approved,
                    )
                    .then_some(record.entity_id)
            })
            .collect())
    }

    async fn eligible_entity_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<(String, NaiveDate)>> {
        let requested = rows.iter().cloned().collect::<HashSet<_>>();
        let mut connection = self.pool.get().map_err(to_core_err)?;
        let records = sql_query(
            "SELECT entity_id, role, coverage_start_date, manual_current_approved
             FROM provenance_records WHERE entity_type = ?",
        )
        .bind::<Text, _>(entity_type)
        .load::<EligibilityRow>(&mut connection)
        .map_err(to_core_err)?;
        Ok(records
            .into_iter()
            .flat_map(|record| {
                let role = role_from_str(&record.role);
                requested.iter().filter_map(move |(id, date)| {
                    (id == &record.entity_id
                        && role.eligible_for_aggregate(
                            &date.to_string(),
                            record.coverage_start_date.as_deref(),
                            record.manual_current_approved,
                        ))
                    .then_some((id.clone(), *date))
                })
            })
            .collect())
    }
}

fn validate_and_deduplicate_row_dates(
    rows: &[(String, NaiveDate)],
) -> Result<BTreeMap<String, NaiveDate>> {
    let mut requested = BTreeMap::new();
    for (entity_id, row_date) in rows {
        match requested.get(entity_id) {
            Some(existing_date) if existing_date != row_date => {
                return Err(wealthfolio_core::Error::Validation(
                    ValidationError::InvalidInput(format!(
                        "Conflicting provenance row dates for entity '{}': {} and {}",
                        entity_id, existing_date, row_date
                    )),
                ));
            }
            Some(_) => {}
            None => {
                requested.insert(entity_id.clone(), *row_date);
            }
        }
    }
    Ok(requested)
}

#[derive(QueryableByName)]
struct ProvenanceRow {
    #[diesel(sql_type = Text)]
    entity_type: String,
    #[diesel(sql_type = Text)]
    entity_id: String,
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_system: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source_record_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    import_run_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    actor_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    coverage_start_date: Option<String>,
    #[diesel(sql_type = Bool)]
    manual_current_approved: bool,
}

#[derive(QueryableByName)]
struct EntityIdRow {
    #[diesel(sql_type = Text)]
    entity_id: String,
}

#[derive(QueryableByName)]
struct EligibilityRow {
    #[diesel(sql_type = Text)]
    entity_id: String,
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = Nullable<Text>)]
    coverage_start_date: Option<String>,
    #[diesel(sql_type = Bool)]
    manual_current_approved: bool,
}

impl ProvenanceRow {
    fn into_domain(self) -> ProvenanceRecord {
        ProvenanceRecord {
            entity_type: self.entity_type,
            entity_id: self.entity_id,
            role: role_from_str(&self.role),
            source_system: self.source_system,
            source_record_id: self.source_record_id,
            import_run_id: self.import_run_id,
            actor_id: self.actor_id,
            coverage_start_date: self.coverage_start_date,
            manual_current_approved: self.manual_current_approved,
        }
    }
}

fn role_to_str(role: ProvenanceRole) -> &'static str {
    match role {
        ProvenanceRole::LiveCanonical => "live_canonical",
        ProvenanceRole::OpeningBaseline => "opening_baseline",
        ProvenanceRole::ManualCurrent => "manual_current",
        ProvenanceRole::HistoricalContext => "historical_context",
        ProvenanceRole::ContextualReference => "contextual_reference",
    }
}

fn role_from_str(role: &str) -> ProvenanceRole {
    match role {
        "live_canonical" => ProvenanceRole::LiveCanonical,
        "opening_baseline" => ProvenanceRole::OpeningBaseline,
        "manual_current" => ProvenanceRole::ManualCurrent,
        "historical_context" => ProvenanceRole::HistoricalContext,
        "contextual_reference" => ProvenanceRole::ContextualReference,
        _ => ProvenanceRole::ContextualReference,
    }
}

fn to_core_err(error: impl std::fmt::Display) -> wealthfolio_core::Error {
    wealthfolio_core::Error::Database(wealthfolio_core::errors::DatabaseError::QueryFailed(
        error.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_pool, run_migrations};
    use tempfile::tempdir;
    use wealthfolio_core::accounts::ProvenanceRepositoryTrait;

    fn setup() -> (ProvenanceRepository, tempfile::TempDir) {
        let directory = tempdir().unwrap();
        let path = directory.path().join("provenance.db");
        run_migrations(path.to_str().unwrap()).unwrap();
        let pool = create_pool(path.to_str().unwrap()).unwrap();
        (ProvenanceRepository::new(pool), directory)
    }

    #[tokio::test]
    async fn round_trips_metadata_and_filters_by_verified_coverage_start() {
        let (repository, _directory) = setup();
        let baseline = ProvenanceRecord {
            entity_type: "account".into(),
            entity_id: "baseline".into(),
            role: ProvenanceRole::OpeningBaseline,
            source_system: Some("synthetic".into()),
            source_record_id: Some("record-1".into()),
            import_run_id: Some("run-1".into()),
            actor_id: Some("actor-1".into()),
            coverage_start_date: Some("2024-01-01".into()),
            manual_current_approved: false,
        };
        repository.upsert(baseline.clone()).await.unwrap();
        assert_eq!(
            repository.get("account", "baseline").await.unwrap(),
            Some(baseline)
        );
        assert_eq!(
            repository
                .eligible_entity_ids("account", "2023-12-31")
                .await
                .unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            repository
                .eligible_entity_ids("account", "2024-01-01")
                .await
                .unwrap(),
            vec!["baseline"]
        );
    }

    #[tokio::test]
    async fn unknown_legacy_role_is_default_denied() {
        let (repository, _directory) = setup();
        let mut connection = repository.pool.get().unwrap();
        let error = diesel::sql_query("INSERT INTO provenance_records(entity_type, entity_id, role) VALUES ('account', 'unknown', 'unrecognized_persisted_role')")
            .execute(&mut connection)
            .expect_err("the storage CHECK constraint must reject unrecognized roles");
        assert!(error.to_string().contains("CHECK constraint failed"));
        assert!(repository
            .get("account", "unknown")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn row_eligibility_collapses_identical_duplicate_dates() {
        let (repository, _directory) = setup();
        repository
            .upsert(ProvenanceRecord {
                entity_type: "daily_valuation".into(),
                entity_id: "valuation-1".into(),
                role: ProvenanceRole::LiveCanonical,
                source_system: None,
                source_record_id: None,
                import_run_id: None,
                actor_id: None,
                coverage_start_date: None,
                manual_current_approved: false,
            })
            .await
            .unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let eligible = repository
            .eligible_entity_ids_for_rows(
                "daily_valuation",
                &[
                    ("valuation-1".to_string(), date),
                    ("valuation-1".to_string(), date),
                ],
            )
            .await
            .unwrap();

        assert_eq!(eligible, HashSet::from(["valuation-1".to_string()]));
    }

    #[tokio::test]
    async fn sqlite_exact_account_date_pairs_distinguish_same_account_mixed_dates() {
        let (repository, _directory) = setup();
        repository
            .upsert(ProvenanceRecord {
                entity_type: "account".into(),
                entity_id: "account-1".into(),
                role: ProvenanceRole::OpeningBaseline,
                source_system: None,
                source_record_id: None,
                import_run_id: None,
                actor_id: None,
                coverage_start_date: Some("2026-07-15".into()),
                manual_current_approved: false,
            })
            .await
            .unwrap();

        let eligible_date = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let denied_date = NaiveDate::from_ymd_opt(2026, 7, 14).unwrap();
        let eligible = repository
            .eligible_entity_rows(
                "account",
                &[
                    ("account-1".to_string(), denied_date),
                    ("account-1".to_string(), eligible_date),
                    ("account-1".to_string(), eligible_date),
                ],
            )
            .await
            .unwrap();

        assert_eq!(
            eligible,
            HashSet::from([("account-1".to_string(), eligible_date)])
        );
    }

    #[tokio::test]
    async fn row_eligibility_rejects_conflicting_duplicate_dates() {
        let (repository, _directory) = setup();
        let error = repository
            .eligible_entity_ids_for_rows(
                "daily_valuation",
                &[
                    (
                        "valuation-1".to_string(),
                        NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                    ),
                    (
                        "valuation-1".to_string(),
                        NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
                    ),
                ],
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            wealthfolio_core::Error::Validation(ValidationError::InvalidInput(_))
        ));
        assert!(error
            .to_string()
            .contains("Conflicting provenance row dates for entity 'valuation-1'"));
    }
}
