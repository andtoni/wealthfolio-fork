#![allow(dead_code)]

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, Utc};
use wealthfolio_core::{
    accounts::{
        account_supports_purpose, AccountPurpose, AccountRepositoryTrait,
        AggregateEligibilityServiceTrait, ENTITY_ACTIVITY,
    },
    activities::{Activity, ActivityStatus, TransferPairResolution},
};

#[derive(Debug)]
pub(crate) struct EligibleSpendingAccounts {
    pub ids: Vec<String>,
    pub account_types: HashMap<String, String>,
}

pub(crate) async fn resolve_eligible_spending_accounts(
    account_repo: &dyn AccountRepositoryTrait,
    policy: &dyn AggregateEligibilityServiceTrait,
    requested_account_ids: &[String],
    as_of: NaiveDate,
) -> Result<EligibleSpendingAccounts> {
    let accounts = account_repo.list(None, Some(false), Some(requested_account_ids))?;
    let candidate_ids: Vec<String> = accounts
        .iter()
        .filter(|account| account_supports_purpose(&account.account_type, AccountPurpose::Spending))
        .map(|account| account.id.clone())
        .collect();

    if candidate_ids.is_empty() {
        return Ok(EligibleSpendingAccounts {
            ids: Vec::new(),
            account_types: HashMap::new(),
        });
    }

    let allowed_ids = policy.eligible_account_ids(&candidate_ids, as_of).await?;
    let mut account_types = HashMap::new();
    let mut ids = Vec::new();

    for account in accounts {
        if allowed_ids.contains(&account.id)
            && account_supports_purpose(&account.account_type, AccountPurpose::Spending)
        {
            ids.push(account.id.clone());
            account_types.insert(account.id, account.account_type);
        }
    }

    Ok(EligibleSpendingAccounts { ids, account_types })
}

pub(crate) async fn filter_eligible_activity_rows(
    policy: &dyn AggregateEligibilityServiceTrait,
    candidate_rows: Vec<Activity>,
    transfer_context_rows: &[Activity],
) -> Result<Vec<Activity>> {
    let rows = deduplicated_activity_rows(
        &candidate_rows
            .iter()
            .chain(transfer_context_rows.iter())
            .cloned()
            .collect::<Vec<_>>(),
    )?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let allowed_ids = policy
        .eligible_entity_ids_for_rows(ENTITY_ACTIVITY, &rows)
        .await?;
    let transfer_resolution = TransferPairResolution::from_activities(transfer_context_rows);

    Ok(candidate_rows
        .into_iter()
        .filter(|activity| {
            // Pending and void activities remain visible in the transaction ledger,
            // but only posted activities may contribute to spending aggregates.
            if activity.status != ActivityStatus::Posted {
                return false;
            }
            if transfer_resolution
                .invalid_group_for_activity(&activity.id)
                .is_some()
            {
                return false;
            }

            if let Some(pair) = transfer_resolution.pair_for_activity(&activity.id) {
                return pair.transfer_in.status == ActivityStatus::Posted
                    && pair.transfer_out.status == ActivityStatus::Posted
                    && allowed_ids.contains(&pair.transfer_in.id)
                    && allowed_ids.contains(&pair.transfer_out.id);
            }

            allowed_ids.contains(&activity.id)
        })
        .collect())
}

pub(crate) fn aggregate_as_of_or_today(end: Option<DateTime<Utc>>) -> NaiveDate {
    end.unwrap_or_else(Utc::now).date_naive()
}

pub(crate) fn deduplicated_activity_rows(rows: &[Activity]) -> Result<Vec<(String, NaiveDate)>> {
    let mut seen = HashMap::new();
    let mut deduped = Vec::new();

    for row in rows {
        let date = row.effective_date();
        match seen.get(&row.id) {
            Some(existing) if *existing != date => {
                return Err(anyhow!(
                    "Conflicting activity dates for activity {}: {} and {}",
                    row.id,
                    existing,
                    date
                ));
            }
            Some(_) => {}
            None => {
                seen.insert(row.id.clone(), date);
                deduped.push((row.id.clone(), date));
            }
        }
    }

    Ok(deduped)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::type_complexity)]

    use super::*;
    use async_trait::async_trait;
    use chrono::{Datelike, NaiveDateTime, TimeZone};
    use rust_decimal::Decimal;
    use serde_json::Value;
    use std::{collections::HashSet, sync::Mutex};
    use wealthfolio_core::{
        accounts::{
            account_types, Account, AccountAccountingSettings, AccountUpdate, NewAccount,
            TrackingMode,
        },
        activities::{
            ActivityStatus, ACTIVITY_TYPE_DEPOSIT, ACTIVITY_TYPE_TRANSFER_IN,
            ACTIVITY_TYPE_TRANSFER_OUT,
        },
        errors::{Error, ValidationError},
    };

    #[derive(Default)]
    struct MockAggregateEligibilityService {
        allowed_accounts: HashSet<String>,
        allowed_activity_ids: HashSet<String>,
        fail_accounts: bool,
        fail_rows: bool,
        account_calls: Mutex<Vec<(Vec<String>, NaiveDate)>>,
        row_calls: Mutex<Vec<(String, Vec<(String, NaiveDate)>)>>,
    }

    #[async_trait]
    impl AggregateEligibilityServiceTrait for MockAggregateEligibilityService {
        async fn eligible_account_ids(
            &self,
            requested_account_ids: &[String],
            as_of: NaiveDate,
        ) -> wealthfolio_core::Result<HashSet<String>> {
            self.account_calls
                .lock()
                .unwrap()
                .push((requested_account_ids.to_vec(), as_of));
            if self.fail_accounts {
                return Err(Error::Validation(ValidationError::InvalidInput(
                    "account policy failed".to_string(),
                )));
            }
            Ok(requested_account_ids
                .iter()
                .filter(|id| self.allowed_accounts.contains(*id))
                .cloned()
                .collect())
        }

        async fn eligible_entity_ids_for_rows(
            &self,
            entity_type: &str,
            rows: &[(String, NaiveDate)],
        ) -> wealthfolio_core::Result<HashSet<String>> {
            self.row_calls
                .lock()
                .unwrap()
                .push((entity_type.to_string(), rows.to_vec()));
            if self.fail_rows {
                return Err(Error::Validation(ValidationError::InvalidInput(
                    "row policy failed".to_string(),
                )));
            }
            Ok(rows
                .iter()
                .filter(|(id, _)| self.allowed_activity_ids.contains(id))
                .map(|(id, _)| id.clone())
                .collect())
        }
    }

    struct MockAccountRepo {
        accounts: Vec<Account>,
        calls: Mutex<Vec<(Option<bool>, Option<bool>, Option<Vec<String>>)>>,
    }

    impl MockAccountRepo {
        fn new(accounts: Vec<Account>) -> Self {
            Self {
                accounts,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl AccountRepositoryTrait for MockAccountRepo {
        async fn create(&self, _: NewAccount) -> wealthfolio_core::Result<Account> {
            unimplemented!()
        }

        async fn update(&self, _: AccountUpdate) -> wealthfolio_core::Result<Account> {
            unimplemented!()
        }

        async fn delete(&self, _: &str) -> wealthfolio_core::Result<usize> {
            unimplemented!()
        }

        fn get_by_id(&self, _: &str) -> wealthfolio_core::Result<Account> {
            unimplemented!()
        }

        fn list(
            &self,
            is_active_filter: Option<bool>,
            is_archived_filter: Option<bool>,
            account_ids: Option<&[String]>,
        ) -> wealthfolio_core::Result<Vec<Account>> {
            self.calls.lock().unwrap().push((
                is_active_filter,
                is_archived_filter,
                account_ids.map(<[String]>::to_vec),
            ));
            Ok(self
                .accounts
                .iter()
                .filter(|account| {
                    is_active_filter
                        .map(|is_active| account.is_active == is_active)
                        .unwrap_or(true)
                        && is_archived_filter
                            .map(|is_archived| account.is_archived == is_archived)
                            .unwrap_or(true)
                        && account_ids
                            .map(|ids| ids.iter().any(|id| id == &account.id))
                            .unwrap_or(true)
                })
                .cloned()
                .collect())
        }

        fn get_accounting_settings_by_account_ids(
            &self,
            account_ids: &[String],
        ) -> wealthfolio_core::Result<HashMap<String, AccountAccountingSettings>> {
            Ok(account_ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        AccountAccountingSettings::default_for_account(id.clone()),
                    )
                })
                .collect())
        }
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn account(id: &str, account_type: &str) -> Account {
        Account {
            id: id.to_string(),
            name: id.to_string(),
            account_type: account_type.to_string(),
            group: None,
            currency: "USD".to_string(),
            is_default: false,
            is_active: true,
            created_at: NaiveDateTime::default(),
            updated_at: NaiveDateTime::default(),
            platform_id: None,
            account_number: None,
            meta: None,
            provider: None,
            provider_account_id: None,
            is_archived: false,
            tracking_mode: TrackingMode::Transactions,
            include_in_net_worth: true,
            name_source: "provider".to_string(),
        }
    }

    fn activity(id: &str, activity_date: NaiveDate) -> Activity {
        Activity {
            id: id.to_string(),
            account_id: "account-1".to_string(),
            asset_id: None,
            activity_type: ACTIVITY_TYPE_DEPOSIT.to_string(),
            activity_type_override: None,
            source_type: None,
            subtype: None,
            status: ActivityStatus::Posted,
            activity_date: Utc
                .with_ymd_and_hms(
                    activity_date.year(),
                    activity_date.month(),
                    activity_date.day(),
                    12,
                    0,
                    0,
                )
                .unwrap(),
            settlement_date: None,
            quantity: None,
            unit_price: None,
            amount: Some(Decimal::new(100, 0)),
            fee: None,
            tax: None,
            currency: "USD".to_string(),
            fx_rate: None,
            notes: None,
            metadata: None::<Value>,
            source_system: None,
            source_record_id: None,
            source_group_id: None,
            idempotency_key: None,
            import_run_id: None,
            is_user_modified: false,
            needs_review: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn transfer(id: &str, activity_type: &str, group_id: &str) -> Activity {
        let mut activity = activity(id, date(2026, 1, 15));
        activity.activity_type = activity_type.to_string();
        activity.account_id = if activity_type == ACTIVITY_TYPE_TRANSFER_IN {
            "account-in".to_string()
        } else {
            "account-out".to_string()
        };
        activity.source_group_id = Some(group_id.to_string());
        activity
    }

    #[tokio::test]
    async fn aggregate_eligibility_resolve_intersects_spending_candidates_and_manual_policy() {
        let repo = MockAccountRepo::new(vec![
            account("cash", account_types::CASH),
            account("card", account_types::CREDIT_CARD),
            account("securities", account_types::SECURITIES),
        ]);
        let policy = MockAggregateEligibilityService {
            allowed_accounts: HashSet::from(["card".to_string(), "securities".to_string()]),
            ..Default::default()
        };
        let requested = vec![
            "cash".to_string(),
            "card".to_string(),
            "securities".to_string(),
        ];

        let resolved =
            resolve_eligible_spending_accounts(&repo, &policy, &requested, date(2026, 7, 15))
                .await
                .unwrap();

        assert_eq!(resolved.ids, vec!["card"]);
        assert_eq!(
            resolved.account_types,
            HashMap::from([("card".to_string(), account_types::CREDIT_CARD.to_string())])
        );
        assert_eq!(
            *repo.calls.lock().unwrap(),
            vec![(None, Some(false), Some(requested.clone()))]
        );
        assert_eq!(
            *policy.account_calls.lock().unwrap(),
            vec![(
                vec!["cash".to_string(), "card".to_string()],
                date(2026, 7, 15)
            )]
        );
    }

    #[tokio::test]
    async fn aggregate_eligibility_resolve_empty_candidates_is_success_without_policy_call() {
        let repo = MockAccountRepo::new(vec![account("securities", account_types::SECURITIES)]);
        let policy = MockAggregateEligibilityService::default();

        let resolved = resolve_eligible_spending_accounts(
            &repo,
            &policy,
            &["securities".to_string()],
            date(2026, 7, 15),
        )
        .await
        .unwrap();

        assert!(resolved.ids.is_empty());
        assert!(resolved.account_types.is_empty());
        assert!(policy.account_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_resolve_live_and_historical_as_of_dates_are_explicit() {
        let repo = MockAccountRepo::new(vec![account("cash", account_types::CASH)]);
        let policy = MockAggregateEligibilityService {
            allowed_accounts: HashSet::from(["cash".to_string()]),
            ..Default::default()
        };

        resolve_eligible_spending_accounts(
            &repo,
            &policy,
            &["cash".to_string()],
            aggregate_as_of_or_today(Some(
                Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap(),
            )),
        )
        .await
        .unwrap();
        resolve_eligible_spending_accounts(&repo, &policy, &["cash".to_string()], date(2024, 1, 1))
            .await
            .unwrap();

        let calls = policy.account_calls.lock().unwrap();
        assert_eq!(calls[0].1, date(2025, 12, 31));
        assert_eq!(calls[1].1, date(2024, 1, 1));
    }

    #[tokio::test]
    async fn aggregate_eligibility_resolve_keeps_mixed_role_cash_account_when_policy_allows() {
        let repo = MockAccountRepo::new(vec![account("cash", account_types::CASH)]);
        let policy = MockAggregateEligibilityService {
            allowed_accounts: HashSet::from(["cash".to_string()]),
            ..Default::default()
        };

        let resolved = resolve_eligible_spending_accounts(
            &repo,
            &policy,
            &["cash".to_string()],
            date(2026, 7, 15),
        )
        .await
        .unwrap();

        assert_eq!(resolved.ids, vec!["cash"]);
    }

    #[tokio::test]
    async fn aggregate_eligibility_resolve_unknown_or_missing_policy_account_is_excluded() {
        let repo = MockAccountRepo::new(vec![account("cash", account_types::CASH)]);
        let policy = MockAggregateEligibilityService {
            allowed_accounts: HashSet::from(["unknown".to_string()]),
            ..Default::default()
        };

        let resolved = resolve_eligible_spending_accounts(
            &repo,
            &policy,
            &["cash".to_string()],
            date(2026, 7, 15),
        )
        .await
        .unwrap();

        assert!(resolved.ids.is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_resolve_propagates_policy_errors() {
        let repo = MockAccountRepo::new(vec![account("cash", account_types::CASH)]);
        let policy = MockAggregateEligibilityService {
            fail_accounts: true,
            ..Default::default()
        };

        let err = resolve_eligible_spending_accounts(
            &repo,
            &policy,
            &["cash".to_string()],
            date(2026, 7, 15),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("account policy failed"));
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_batches_contextual_rows_once_with_exact_dates() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from(["a1".to_string(), "ctx".to_string()]),
            ..Default::default()
        };
        let candidate = activity("a1", date(2026, 1, 10));
        let context = activity("ctx", date(2026, 1, 11));

        let filtered = filter_eligible_activity_rows(
            &policy,
            vec![candidate.clone()],
            std::slice::from_ref(&context),
        )
        .await
        .unwrap();

        assert_eq!(
            filtered.iter().map(|row| &row.id).collect::<Vec<_>>(),
            ["a1"]
        );
        assert_eq!(
            *policy.row_calls.lock().unwrap(),
            vec![(
                ENTITY_ACTIVITY.to_string(),
                vec![
                    ("a1".to_string(), date(2026, 1, 10)),
                    ("ctx".to_string(), date(2026, 1, 11)),
                ],
            )]
        );
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_excludes_pending_and_void_but_keeps_posted_rows() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from([
                "posted".to_string(),
                "pending".to_string(),
                "void".to_string(),
            ]),
            ..Default::default()
        };
        let posted = activity("posted", date(2026, 1, 1));
        let mut pending = activity("pending", date(2026, 1, 1));
        pending.status = ActivityStatus::Pending;
        let mut void = activity("void", date(2026, 1, 1));
        void.status = ActivityStatus::Void;

        let filtered = filter_eligible_activity_rows(&policy, vec![posted, pending, void], &[])
            .await
            .unwrap();

        assert_eq!(
            filtered
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["posted"]
        );
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_excludes_both_legs_when_transfer_is_pending() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from([
                "transfer-in".to_string(),
                "transfer-out".to_string(),
            ]),
            ..Default::default()
        };
        let transfer_in = transfer("transfer-in", ACTIVITY_TYPE_TRANSFER_IN, "group-1");
        let mut transfer_out = transfer("transfer-out", ACTIVITY_TYPE_TRANSFER_OUT, "group-1");
        transfer_out.status = ActivityStatus::Pending;

        let filtered = filter_eligible_activity_rows(
            &policy,
            vec![transfer_in.clone(), transfer_out.clone()],
            &[transfer_in, transfer_out],
        )
        .await
        .unwrap();

        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_duplicate_identical_rows_collapse_at_opening_boundary() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from(["a1".to_string()]),
            ..Default::default()
        };
        let row = activity("a1", date(2026, 1, 1));

        let filtered =
            filter_eligible_activity_rows(&policy, vec![row.clone(), row.clone()], &[row])
                .await
                .unwrap();

        assert_eq!(filtered.len(), 2);
        assert_eq!(
            policy.row_calls.lock().unwrap()[0].1,
            vec![("a1".to_string(), date(2026, 1, 1))]
        );
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_conflicting_dates_error_before_policy_call() {
        let policy = MockAggregateEligibilityService::default();
        let later = activity("a1", date(2026, 1, 12));

        let err = filter_eligible_activity_rows(
            &policy,
            vec![activity("a1", date(2026, 1, 11))],
            &[later],
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("Conflicting activity dates"));
        assert!(policy.row_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_missing_row_id_is_excluded() {
        let policy = MockAggregateEligibilityService::default();

        let filtered = filter_eligible_activity_rows(
            &policy,
            vec![activity("missing", date(2026, 1, 10))],
            &[],
        )
        .await
        .unwrap();

        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_policy_error_propagates() {
        let policy = MockAggregateEligibilityService {
            fail_rows: true,
            ..Default::default()
        };

        let err =
            filter_eligible_activity_rows(&policy, vec![activity("a1", date(2026, 1, 10))], &[])
                .await
                .unwrap_err();

        assert!(err.to_string().contains("row policy failed"));
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_transfer_pair_is_all_or_none() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from(["transfer-in".to_string()]),
            ..Default::default()
        };
        let transfer_in = transfer("transfer-in", ACTIVITY_TYPE_TRANSFER_IN, "group-1");
        let transfer_out = transfer("transfer-out", ACTIVITY_TYPE_TRANSFER_OUT, "group-1");

        let filtered = filter_eligible_activity_rows(
            &policy,
            vec![transfer_in.clone(), transfer_out.clone()],
            &[transfer_in, transfer_out],
        )
        .await
        .unwrap();

        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_valid_transfer_pair_retains_both_when_allowed() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from([
                "transfer-in".to_string(),
                "transfer-out".to_string(),
            ]),
            ..Default::default()
        };
        let transfer_in = transfer("transfer-in", ACTIVITY_TYPE_TRANSFER_IN, "group-1");
        let transfer_out = transfer("transfer-out", ACTIVITY_TYPE_TRANSFER_OUT, "group-1");

        let filtered = filter_eligible_activity_rows(
            &policy,
            vec![transfer_in.clone(), transfer_out.clone()],
            &[transfer_in, transfer_out],
        )
        .await
        .unwrap();

        assert_eq!(
            filtered
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["transfer-in", "transfer-out"]
        );
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_invalid_transfer_group_excludes_all_members() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from([
                "transfer-in-1".to_string(),
                "transfer-in-2".to_string(),
            ]),
            ..Default::default()
        };
        let transfer_in_1 = transfer("transfer-in-1", ACTIVITY_TYPE_TRANSFER_IN, "bad-group");
        let transfer_in_2 = transfer("transfer-in-2", ACTIVITY_TYPE_TRANSFER_IN, "bad-group");

        let filtered = filter_eligible_activity_rows(
            &policy,
            vec![transfer_in_1.clone(), transfer_in_2.clone()],
            &[transfer_in_1, transfer_in_2],
        )
        .await
        .unwrap();

        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn aggregate_eligibility_filter_ungrouped_transfer_uses_own_row_gate() {
        let policy = MockAggregateEligibilityService {
            allowed_activity_ids: HashSet::from(["transfer-in".to_string()]),
            ..Default::default()
        };
        let mut transfer_in = transfer("transfer-in", ACTIVITY_TYPE_TRANSFER_IN, "group-1");
        transfer_in.source_group_id = None;

        let filtered =
            filter_eligible_activity_rows(&policy, vec![transfer_in.clone()], &[transfer_in])
                .await
                .unwrap();

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "transfer-in");
    }
}
