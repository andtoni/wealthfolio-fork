//! Current financial eligibility resolver.
//!
//! An account is eligible for current financial aggregates (current valuations,
//! current net worth, holdings, allocation totals, retirement portfolio resolution)
//! when all of the following hold:
//!
//! 1. The account is not archived.
//! 2. `include_in_net_worth` is `true`.
//! 3. A verified receipt with `verification_state = "verified"` exists for this account.
//!
//! Provider-discovered accounts default to `include_in_net_worth = false` and have
//! no verified receipt until a reconciled snapshot or opening baseline is established.

use crate::accounts::Account;

/// Resolves whether an account is currently eligible for financial aggregates.
///
/// This is a pure domain function — it does not perform I/O. Callers supply the
/// account and the optional verified receipt state.
pub fn account_is_eligible(account: &Account, has_verified_receipt: bool) -> bool {
    if account.is_archived {
        return false;
    }
    if !account.include_in_net_worth {
        return false;
    }
    if !has_verified_receipt {
        return false;
    }
    true
}

/// Returns the subset of accounts that are eligible for current financial aggregates.
pub fn filter_eligible_accounts(
    accounts: Vec<Account>,
    eligible_account_ids: &[String],
) -> Vec<Account> {
    let eligible: std::collections::HashSet<String> =
        eligible_account_ids.iter().cloned().collect();
    accounts
        .into_iter()
        .filter(|a| eligible.contains(&a.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
// TrackingMode, VerifiedReceipt, VerifiedReceiptBasis not needed here

    fn make_account(id: &str, include: bool, archived: bool) -> Account {
        let now = chrono::NaiveDateTime::parse_from_str("2026-07-11 00:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap();
        Account {
            id: id.to_string(),
            name: format!("Account {id}"),
            account_type: "CASH".to_string(),
            currency: "USD".to_string(),
            include_in_net_worth: include,
            is_archived: archived,
            created_at: now,
            updated_at: now,
            ..Default::default()
        }
    }

    #[test]
    fn archived_account_is_not_eligible() {
        let a = make_account("a1", true, true);
        assert!(!account_is_eligible(&a, true));
    }

    #[test]
    fn excluded_account_is_not_eligible() {
        let a = make_account("a1", false, false);
        assert!(!account_is_eligible(&a, true));
    }

    #[test]
    fn account_without_verified_receipt_is_not_eligible() {
        let a = make_account("a1", true, false);
        assert!(!account_is_eligible(&a, false));
    }

    #[test]
    fn fully_eligible_account_is_eligible() {
        let a = make_account("a1", true, false);
        assert!(account_is_eligible(&a, true));
    }

    #[test]
    fn excluded_without_receipt_is_not_eligible() {
        let a = make_account("a1", false, false);
        assert!(!account_is_eligible(&a, false));
    }

    #[test]
    fn filter_eligible_accounts_works() {
        let accounts = vec![
            make_account("a1", true, false),
            make_account("a2", true, false),
            make_account("a3", false, false),
        ];
        let eligible = filter_eligible_accounts(accounts, &["a1".into(), "a2".into()]);
        assert_eq!(eligible.len(), 2);
        assert!(eligible.iter().any(|a| a.id == "a1"));
        assert!(eligible.iter().any(|a| a.id == "a2"));
        assert!(!eligible.iter().any(|a| a.id == "a3"));
    }

    #[test]
    fn empty_eligible_ids_returns_empty() {
        let accounts = vec![make_account("a1", true, false)];
        let eligible = filter_eligible_accounts(accounts, &[]);
        assert!(eligible.is_empty());
    }
}
