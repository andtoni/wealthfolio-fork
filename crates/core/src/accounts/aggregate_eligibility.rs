use async_trait::async_trait;
use chrono::NaiveDate;
use std::{collections::HashSet, sync::Arc};

use super::{
    AccountRepositoryTrait, ProvenanceRepositoryTrait, VerifiedReceiptRepositoryTrait,
    ENTITY_ACCOUNT,
};
use crate::errors::Result;

/// Shared fail-closed policy boundary for portfolio aggregate inputs.
#[async_trait]
pub trait AggregateEligibilityServiceTrait: Send + Sync {
    async fn eligible_account_ids(
        &self,
        requested_account_ids: &[String],
        as_of: NaiveDate,
    ) -> Result<HashSet<String>>;

    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<String>>;

    async fn eligible_account_rows(
        &self,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<(String, NaiveDate)>> {
        let mut requested_ids: Vec<String> = rows
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        requested_ids.sort();
        let as_of = rows
            .iter()
            .map(|(_, date)| *date)
            .max()
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
        let eligible_ids = self.eligible_account_ids(&requested_ids, as_of).await?;
        Ok(rows
            .iter()
            .filter(|(id, _)| eligible_ids.contains(id))
            .cloned()
            .collect())
    }
}

pub struct AggregateEligibilityService {
    account_repository: Arc<dyn AccountRepositoryTrait>,
    receipt_repository: Arc<dyn VerifiedReceiptRepositoryTrait>,
    provenance_repository: Arc<dyn ProvenanceRepositoryTrait>,
}

impl AggregateEligibilityService {
    pub fn new(
        account_repository: Arc<dyn AccountRepositoryTrait>,
        receipt_repository: Arc<dyn VerifiedReceiptRepositoryTrait>,
        provenance_repository: Arc<dyn ProvenanceRepositoryTrait>,
    ) -> Self {
        Self {
            account_repository,
            receipt_repository,
            provenance_repository,
        }
    }
}

#[async_trait]
impl AggregateEligibilityServiceTrait for AggregateEligibilityService {
    async fn eligible_account_ids(
        &self,
        requested_account_ids: &[String],
        as_of: NaiveDate,
    ) -> Result<HashSet<String>> {
        let requested: HashSet<&str> = requested_account_ids.iter().map(String::as_str).collect();
        let receipt_ids: HashSet<String> = self
            .receipt_repository
            .get_eligible_account_ids()
            .await?
            .into_iter()
            .filter(|id| requested.contains(id.as_str()))
            .collect();
        if receipt_ids.is_empty() {
            return Ok(HashSet::new());
        }

        let accounts = self
            .account_repository
            .list(None, None, Some(requested_account_ids))?;
        let account_ids: Vec<String> = accounts
            .into_iter()
            .filter(|account| {
                account.is_active && !account.is_archived && account.include_in_net_worth
            })
            .filter(|account| receipt_ids.contains(&account.id))
            .map(|account| account.id)
            .collect();
        let rows: Vec<(String, NaiveDate)> =
            account_ids.into_iter().map(|id| (id, as_of)).collect();
        self.provenance_repository
            .eligible_entity_ids_for_rows(ENTITY_ACCOUNT, &rows)
            .await
    }

    async fn eligible_entity_ids_for_rows(
        &self,
        entity_type: &str,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<String>> {
        self.provenance_repository
            .eligible_entity_ids_for_rows(entity_type, rows)
            .await
    }

    async fn eligible_account_rows(
        &self,
        rows: &[(String, NaiveDate)],
    ) -> Result<HashSet<(String, NaiveDate)>> {
        let requested: HashSet<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
        let receipt_ids: HashSet<String> = self
            .receipt_repository
            .get_eligible_account_ids()
            .await?
            .into_iter()
            .filter(|id| requested.contains(id.as_str()))
            .collect();
        if receipt_ids.is_empty() {
            return Ok(HashSet::new());
        }
        let requested_ids: Vec<String> = requested.into_iter().map(str::to_string).collect();
        let accounts = self
            .account_repository
            .list(None, None, Some(&requested_ids))?;
        let account_ids: HashSet<String> = accounts
            .into_iter()
            .filter(|account| {
                account.is_active && !account.is_archived && account.include_in_net_worth
            })
            .filter(|account| receipt_ids.contains(&account.id))
            .map(|account| account.id)
            .collect();
        let provenance_rows: Vec<_> = rows
            .iter()
            .filter(|(id, _)| account_ids.contains(id))
            .cloned()
            .collect();
        self.provenance_repository
            .eligible_entity_rows(ENTITY_ACCOUNT, &provenance_rows)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingEligibility {
        requested: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl AggregateEligibilityServiceTrait for RecordingEligibility {
        async fn eligible_account_ids(
            &self,
            requested_account_ids: &[String],
            _as_of: NaiveDate,
        ) -> Result<HashSet<String>> {
            self.requested
                .lock()
                .unwrap()
                .push(requested_account_ids.to_vec());
            Ok(requested_account_ids.iter().cloned().collect())
        }

        async fn eligible_entity_ids_for_rows(
            &self,
            _entity_type: &str,
            _rows: &[(String, NaiveDate)],
        ) -> Result<HashSet<String>> {
            Ok(HashSet::new())
        }
    }

    #[tokio::test]
    async fn compatibility_account_row_selector_uses_sorted_unique_account_ids() {
        let service = RecordingEligibility {
            requested: Mutex::new(Vec::new()),
        };
        let date = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();

        let eligible = service
            .eligible_account_rows(&[
                ("account-b".to_string(), date),
                ("account-a".to_string(), date),
                ("account-b".to_string(), date),
            ])
            .await
            .unwrap();

        assert_eq!(
            service.requested.lock().unwrap().as_slice(),
            &[vec!["account-a".to_string(), "account-b".to_string()]]
        );
        assert_eq!(eligible.len(), 2);
    }
}
