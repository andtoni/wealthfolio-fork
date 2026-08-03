use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;

use super::model::{ActivityTaxonomyAssignment, NewActivityTaxonomyAssignment};
use super::traits::ActivityTaxonomyAssignmentRepositoryTrait;
use crate::budget::{BudgetRepositoryTrait, BudgetService};
use crate::cash_activities::INCOME_TAXONOMY;
use crate::error::SpendingError;
use wealthfolio_core::taxonomies::TaxonomyServiceTrait;

pub struct ActivityTaxonomyAssignmentService {
    repo: Arc<dyn ActivityTaxonomyAssignmentRepositoryTrait>,
    budget_repo: Option<Arc<dyn BudgetRepositoryTrait>>,
    taxonomy_service: Option<Arc<dyn TaxonomyServiceTrait>>,
}

impl ActivityTaxonomyAssignmentService {
    pub fn new(repo: Arc<dyn ActivityTaxonomyAssignmentRepositoryTrait>) -> Self {
        Self {
            repo,
            budget_repo: None,
            taxonomy_service: None,
        }
    }

    pub fn with_budget_repository(mut self, budget_repo: Arc<dyn BudgetRepositoryTrait>) -> Self {
        self.budget_repo = Some(budget_repo);
        self
    }

    pub fn with_taxonomy_service(
        mut self,
        taxonomy_service: Arc<dyn TaxonomyServiceTrait>,
    ) -> Self {
        self.taxonomy_service = Some(taxonomy_service);
        self
    }

    pub async fn list_for_activity(
        &self,
        activity_id: &str,
    ) -> Result<Vec<ActivityTaxonomyAssignment>> {
        self.repo.list_for_activity(activity_id).await
    }

    pub async fn list_for_activities(
        &self,
        activity_ids: &[String],
    ) -> Result<Vec<ActivityTaxonomyAssignment>> {
        self.repo.list_for_activities(activity_ids).await
    }

    pub(crate) async fn ensure_category_pairs(&self, pairs: &[(String, String)]) -> Result<()> {
        let mut by_taxonomy: HashMap<&str, Vec<String>> = HashMap::new();
        for (taxonomy_id, category_id) in pairs {
            by_taxonomy
                .entry(taxonomy_id.as_str())
                .or_default()
                .push(category_id.clone());
        }
        for (taxonomy_id, category_ids) in by_taxonomy {
            if !self
                .repo
                .categories_belong_to_taxonomy(taxonomy_id, &category_ids)
                .await?
            {
                return Err(SpendingError::InvalidInput {
                    message: "Category does not belong to the selected taxonomy".to_string(),
                }
                .into());
            }
        }

        let Some(configured) = self.configured_category_pairs().await? else {
            return Ok(());
        };
        if pairs.iter().any(|(taxonomy_id, category_id)| {
            taxonomy_id != INCOME_TAXONOMY
                && !configured.contains(&(taxonomy_id.clone(), category_id.clone()))
        }) {
            return Err(SpendingError::InvalidInput {
                message: "Category is not configured in the budget plan".to_string(),
            }
            .into());
        }
        Ok(())
    }

    /// Return category pairs that are valid for a rule or proposal.
    ///
    /// Unlike [`ensure_category_pairs`], invalid pairs are omitted instead of
    /// failing the whole operation. This lets reruns skip stale rules while
    /// still applying lower-priority compatible rules and unrelated valid rows.
    pub(crate) async fn filter_configured_category_pairs(
        &self,
        pairs: &[(String, String)],
    ) -> Result<HashSet<(String, String)>> {
        let configured = self.configured_category_pairs().await?;
        let mut valid = HashSet::new();
        for (taxonomy_id, category_id) in pairs {
            if !self
                .repo
                .categories_belong_to_taxonomy(taxonomy_id, &[category_id.clone()])
                .await?
            {
                continue;
            }
            if configured.as_ref().is_some_and(|configured| {
                taxonomy_id != INCOME_TAXONOMY
                    && !configured.contains(&(taxonomy_id.clone(), category_id.clone()))
            }) {
                continue;
            }
            valid.insert((taxonomy_id.clone(), category_id.clone()));
        }
        Ok(valid)
    }

    async fn configured_category_pairs(&self) -> Result<Option<HashSet<(String, String)>>> {
        let Some(budget_repo) = &self.budget_repo else {
            return Ok(None);
        };

        Ok(Some(
            BudgetService::configured_category_pairs_from_repository(
                budget_repo.as_ref(),
                self.taxonomy_service.as_deref(),
            )
            .await?,
        ))
    }

    /// Set the (single) category for `activity_id` in `taxonomy_id`.
    /// Clears any prior assignments tying that activity to that taxonomy.
    pub async fn assign_single(
        &self,
        activity_id: &str,
        taxonomy_id: &str,
        category_id: &str,
    ) -> Result<ActivityTaxonomyAssignment> {
        let mut assigned = self
            .assign_many_single_select(&[BulkCategoryAssignment {
                activity_id: activity_id.to_string(),
                taxonomy_id: taxonomy_id.to_string(),
                category_id: category_id.to_string(),
            }])
            .await?;
        assigned
            .pop()
            .ok_or_else(|| anyhow::anyhow!("assignment write returned no row"))
    }

    pub async fn assign_single_clearing_splits(
        &self,
        activity_id: &str,
        taxonomy_id: &str,
        category_id: &str,
    ) -> Result<ActivityTaxonomyAssignment> {
        let mut assigned = self
            .assign_many_single_select_clearing_splits(&[BulkCategoryAssignment {
                activity_id: activity_id.to_string(),
                taxonomy_id: taxonomy_id.to_string(),
                category_id: category_id.to_string(),
            }])
            .await?;
        assigned
            .pop()
            .ok_or_else(|| anyhow::anyhow!("assignment write returned no row"))
    }

    pub async fn unassign(&self, activity_id: &str, taxonomy_id: &str) -> Result<()> {
        self.repo.clear_for_taxonomy(activity_id, taxonomy_id).await
    }

    /// Direct upsert (used by Activity Rules to apply rule-sourced assignments).
    pub async fn upsert(
        &self,
        new_assignment: NewActivityTaxonomyAssignment,
    ) -> Result<ActivityTaxonomyAssignment> {
        self.ensure_category_pairs(&[(
            new_assignment.taxonomy_id.clone(),
            new_assignment.category_id.clone(),
        )])
        .await?;
        self.repo.upsert(new_assignment).await
    }

    /// Bulk single-select assignment. Each item replaces any existing assignment
    /// for its (`activity_id`, `taxonomy_id`) pair. Atomic across the batch.
    /// Source is hardcoded to `"manual"` — caller intent for v1 is "user explicitly
    /// confirmed these" (whether via bulk-select on the transactions page or via
    /// the AI proposal widget).
    pub async fn assign_many_single_select(
        &self,
        items: &[BulkCategoryAssignment],
    ) -> Result<Vec<ActivityTaxonomyAssignment>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_category_pairs(
            &items
                .iter()
                .map(|item| (item.taxonomy_id.clone(), item.category_id.clone()))
                .collect::<Vec<_>>(),
        )
        .await?;
        let news: Vec<NewActivityTaxonomyAssignment> = items
            .iter()
            .map(|item| NewActivityTaxonomyAssignment {
                id: None,
                activity_id: item.activity_id.clone(),
                taxonomy_id: item.taxonomy_id.clone(),
                category_id: item.category_id.clone(),
                weight: 10_000,
                source: "manual".to_string(),
            })
            .collect();
        self.repo.assign_many_single_select(news).await
    }

    pub async fn assign_many_single_select_clearing_splits(
        &self,
        items: &[BulkCategoryAssignment],
    ) -> Result<Vec<ActivityTaxonomyAssignment>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_category_pairs(
            &items
                .iter()
                .map(|item| (item.taxonomy_id.clone(), item.category_id.clone()))
                .collect::<Vec<_>>(),
        )
        .await?;
        let news: Vec<NewActivityTaxonomyAssignment> = items
            .iter()
            .map(|item| NewActivityTaxonomyAssignment {
                id: None,
                activity_id: item.activity_id.clone(),
                taxonomy_id: item.taxonomy_id.clone(),
                category_id: item.category_id.clone(),
                weight: 10_000,
                source: "manual".to_string(),
            })
            .collect();
        self.repo
            .assign_many_single_select_clearing_splits(news)
            .await
    }

    /// Bulk single-select with full control over `source` / `weight`. Used by
    /// the categorization rules "re-run" path so it can preserve
    /// `source = "rule"` while still benefiting from the single-transaction
    /// delete-then-insert semantics of `assign_many_single_select`.
    pub async fn bulk_apply(
        &self,
        items: Vec<NewActivityTaxonomyAssignment>,
    ) -> Result<Vec<ActivityTaxonomyAssignment>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_category_pairs(
            &items
                .iter()
                .map(|item| (item.taxonomy_id.clone(), item.category_id.clone()))
                .collect::<Vec<_>>(),
        )
        .await?;
        self.repo.assign_many_single_select(items).await
    }

    pub async fn bulk_apply_rule_assignments(
        &self,
        items: Vec<NewActivityTaxonomyAssignment>,
        only_uncategorized: bool,
    ) -> Result<Vec<ActivityTaxonomyAssignment>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_category_pairs(
            &items
                .iter()
                .map(|item| (item.taxonomy_id.clone(), item.category_id.clone()))
                .collect::<Vec<_>>(),
        )
        .await?;
        self.repo
            .assign_rule_many_single_select(items, only_uncategorized)
            .await
    }
}

/// Lightweight input for the bulk-assign service method. Doesn't carry weight
/// or source — those default to `10_000` and `"manual"`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkCategoryAssignment {
    pub activity_id: String,
    pub taxonomy_id: String,
    pub category_id: String,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;

    use super::*;

    struct MockRepository {
        categories_valid: bool,
        wrote: AtomicBool,
    }

    #[async_trait]
    impl ActivityTaxonomyAssignmentRepositoryTrait for MockRepository {
        async fn list_for_activity(&self, _: &str) -> Result<Vec<ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn list_for_activities(
            &self,
            _: &[String],
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            unimplemented!()
        }

        async fn categories_belong_to_taxonomy(&self, _: &str, _: &[String]) -> Result<bool> {
            Ok(self.categories_valid)
        }

        async fn upsert(
            &self,
            _: NewActivityTaxonomyAssignment,
        ) -> Result<ActivityTaxonomyAssignment> {
            unimplemented!()
        }

        async fn assign_many_single_select(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            self.wrote.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        async fn assign_many_single_select_clearing_splits(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            self.wrote.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        async fn assign_rule_many_single_select(
            &self,
            _: Vec<NewActivityTaxonomyAssignment>,
            _: bool,
        ) -> Result<Vec<ActivityTaxonomyAssignment>> {
            self.wrote.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        async fn delete(&self, _: &str) -> Result<()> {
            unimplemented!()
        }

        async fn clear_for_taxonomy(&self, _: &str, _: &str) -> Result<()> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn rejects_category_from_another_taxonomy_before_writing() {
        let repo = Arc::new(MockRepository {
            categories_valid: false,
            wrote: AtomicBool::new(false),
        });
        let service = ActivityTaxonomyAssignmentService::new(repo.clone());

        let error = service
            .assign_many_single_select(&[BulkCategoryAssignment {
                activity_id: "activity-1".to_string(),
                taxonomy_id: "income_sources".to_string(),
                category_id: "spending-groceries".to_string(),
            }])
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("Category does not belong to the selected taxonomy"));
        assert!(!repo.wrote.load(Ordering::SeqCst));
    }
}
