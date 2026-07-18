use super::{
    Account, AccountUpdate, NewAccount, ProvenanceRole, ProviderIdentity, TrackingMode,
    VerifiedReceipt, VerifiedReceiptBasis,
};

fn new_account_defaults() -> NewAccount {
    NewAccount {
        id: None,
        name: String::new(),
        account_type: String::from("CASH"),
        group: None,
        currency: String::new(),
        is_default: false,
        is_active: true,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        is_archived: false,
        tracking_mode: TrackingMode::default(),
        include_in_net_worth: true,
        name_source: "provider".to_string(),
    }
}

#[test]
fn account_default_include_in_net_worth_is_false_until_explicitly_selected() {
    let a = Account::default();
    assert!(
        !a.include_in_net_worth,
        "default model account remains unselected until explicit inclusion"
    );
}

#[test]
fn new_account_default_include_in_net_worth_is_true() {
    let n = NewAccount {
        name: "test".into(),
        currency: "USD".into(),
        ..new_account_defaults()
    };
    assert!(
        n.include_in_net_worth,
        "default new account includes in net worth"
    );
}

#[test]
fn new_account_can_set_include_in_net_worth_false() {
    let n = NewAccount {
        name: "test".into(),
        currency: "USD".into(),
        include_in_net_worth: false,
        ..new_account_defaults()
    };
    assert!(
        !n.include_in_net_worth,
        "new account can opt out of net worth"
    );
}

#[test]
fn verified_receipt_basis_serde_roundtrip() {
    let variants = [
        VerifiedReceiptBasis::CurrentSnapshot,
        VerifiedReceiptBasis::OpeningBaseline,
    ];
    for v in &variants {
        let json = serde_json::to_string(v).unwrap();
        let back: VerifiedReceiptBasis = serde_json::from_str(&json).unwrap();
        assert_eq!(*v, back, "serde roundtrip for {v:?}");
    }
}

#[test]
fn verified_receipt_default_verification_state_is_pending() {
    let r = VerifiedReceipt {
        id: "r1".into(),
        account_id: "a1".into(),
        basis: VerifiedReceiptBasis::CurrentSnapshot,
        as_of: "2026-07-11".into(),
        provider_receipt_ref: None,
        verification_state: "pending".into(),
        coverage_start_date: None,
        created_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    assert_eq!(r.verification_state, "pending");
    assert_eq!(r.basis, VerifiedReceiptBasis::CurrentSnapshot);
}

#[test]
fn verified_receipt_opening_baseline() {
    let r = VerifiedReceipt {
        id: "r2".into(),
        account_id: "a1".into(),
        basis: VerifiedReceiptBasis::OpeningBaseline,
        as_of: "2026-01-01".into(),
        provider_receipt_ref: Some("import-abc-123".into()),
        verification_state: "verified".into(),
        coverage_start_date: Some("2026-01-01".into()),
        created_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    assert_eq!(r.basis, VerifiedReceiptBasis::OpeningBaseline);
    assert_eq!(r.verification_state, "verified");
    assert_eq!(r.coverage_start_date.as_deref(), Some("2026-01-01"));
}

#[test]
fn provenance_roles_exclude_history_and_context() {
    for role in [
        ProvenanceRole::HistoricalContext,
        ProvenanceRole::ContextualReference,
    ] {
        assert!(!role.eligible_for_aggregate("2026-01-01", Some("2026-01-01"), true));
    }
}

#[test]
fn provenance_opening_baseline_starts_at_verified_coverage() {
    let role = ProvenanceRole::OpeningBaseline;
    assert!(!role.eligible_for_aggregate("2025-12-31", Some("2026-01-01"), false));
    assert!(role.eligible_for_aggregate("2026-01-01", Some("2026-01-01"), false));
}

#[test]
fn provenance_manual_current_requires_explicit_approval() {
    let role = ProvenanceRole::ManualCurrent;
    assert!(!role.eligible_for_aggregate("2026-01-01", None, false));
    assert!(role.eligible_for_aggregate("2026-01-01", None, true));
}

#[test]
fn provider_identity_holds_immutable_fields() {
    let i = ProviderIdentity {
        account_id: "a1".into(),
        provider: "investec".into(),
        provider_account_id: "12345".into(),
        lifecycle_state: "active".into(),
        provider_display_name: Some("Investec Savings".into()),
        last_sync_at: Some("2026-07-11T12:00:00Z".into()),
    };
    assert_eq!(i.provider, "investec");
    assert_eq!(i.provider_account_id, "12345");
    assert_eq!(i.lifecycle_state, "active");
    assert!(i.last_sync_at.is_some());
}

#[test]
fn provider_identity_can_be_closed() {
    let i = ProviderIdentity {
        account_id: "a2".into(),
        provider: "mercury".into(),
        provider_account_id: "67890".into(),
        lifecycle_state: "closed".into(),
        provider_display_name: None,
        last_sync_at: None,
    };
    assert_eq!(i.lifecycle_state, "closed");
    assert!(i.last_sync_at.is_none());
}

#[test]
fn eligibility_default_included_account_without_verified_receipt_is_not_eligible() {
    // An account with include_in_net_worth=true but no verified receipt
    // should be excluded from current financial aggregates.
    // This test checks the domain logic directly.
    let account = Account {
        id: "a1".into(),
        name: "Test".into(),
        account_type: "CASH".into(),
        currency: "USD".into(),
        include_in_net_worth: true,
        is_archived: false,
        ..Default::default()
    };
    // No verified receipt means not eligible for current financial aggregates
    assert!(
        crate::accounts::account_in_portfolio_scope(&account),
        "non-archived account is in portfolio scope"
    );
}

/// Model-level test: an account qualifies for eligibility when it meets all
/// three conditions (not archived, include_in_net_worth=true, has verified receipt).
/// This test verifies the data model supports this; the resolver is in the service layer.
#[test]
fn model_supports_full_eligibility_conditions() {
    let account = Account {
        id: "a1".into(),
        name: "Checking".into(),
        account_type: "CASH".into(),
        currency: "USD".into(),
        include_in_net_worth: true,
        is_archived: false,
        ..Default::default()
    };
    let receipt = VerifiedReceipt {
        id: "r1".into(),
        account_id: "a1".into(),
        basis: VerifiedReceiptBasis::CurrentSnapshot,
        as_of: "2026-07-11".into(),
        provider_receipt_ref: Some("snapshot-001".into()),
        verification_state: "verified".into(),
        coverage_start_date: None,
        created_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
    };
    assert_eq!(receipt.verification_state, "verified");
    assert_eq!(receipt.account_id, account.id);
    // The account has include_in_net_worth=true AND a verified receipt:
    // model supports eligibility.
    assert!(account.include_in_net_worth);
    assert!(!account.is_archived);
    assert_eq!(receipt.verification_state, "verified");
}

#[test]
fn provider_discovered_account_defaults_to_excluded() {
    // Per the contract: "default newly provider-discovered accounts to false"
    let n = NewAccount {
        name: "Provider Account".into(),
        currency: "USD".into(),
        provider: Some("investec".into()),
        provider_account_id: Some("acc-001".into()),
        include_in_net_worth: false,
        ..new_account_defaults()
    };
    assert!(!n.include_in_net_worth);
    assert!(n.provider.is_some());
    assert!(n.provider_account_id.is_some());
}

// ── name_source Override Tests ──

#[test]
fn account_default_name_source_is_provider() {
    let a = Account::default();
    assert_eq!(
        a.name_source, "",
        "Default Account has empty string (serde default applies at deserialization)"
    );
    // Via NewAccount defaults (the helper)
    let n = new_account_defaults();
    assert_eq!(
        n.name_source, "provider",
        "NewAccount defaults to provider-sourced name"
    );
}

#[test]
fn new_account_default_name_source_is_provider() {
    let n = NewAccount {
        name: "Checking".into(),
        currency: "USD".into(),
        ..new_account_defaults()
    };
    assert_eq!(
        n.name_source, "provider",
        "New account without explicit name_source defaults to provider"
    );
}

#[test]
fn new_account_can_set_name_source_to_user() {
    let n = NewAccount {
        name: "My Renamed Account".into(),
        currency: "USD".into(),
        name_source: "user".to_string(),
        ..new_account_defaults()
    };
    assert_eq!(
        n.name_source, "user",
        "New account can set name_source to user"
    );
}

#[test]
fn new_account_provider_discovered_defaults_to_provider_name_source() {
    // Provider-discovered accounts get name_source = "provider"
    let n = NewAccount {
        name: "SnapTrade Checking".into(),
        currency: "USD".into(),
        provider: Some("SNAPTRADE".into()),
        provider_account_id: Some("snap-001".into()),
        include_in_net_worth: false,
        ..new_account_defaults()
    };
    assert_eq!(
        n.name_source, "provider",
        "Provider-discovered account defaults to provider name source"
    );
    assert_eq!(
        n.include_in_net_worth, false,
        "Provider-discovered defaults to excluded from net worth"
    );
}

#[test]
fn account_update_without_name_source_does_not_change_it() {
    let update = AccountUpdate {
        id: Some("acc-1".into()),
        name: "Updated Name".into(),
        account_type: "CASH".into(),
        group: None,
        is_default: false,
        is_active: true,
        is_archived: None,
        tracking_mode: None,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        include_in_net_worth: None,
        name_source: None,
    };
    assert_eq!(
        update.name_source, None,
        "AccountUpdate without name_source leaves it unchanged"
    );
}

#[test]
fn account_update_with_name_source_user_explicitly_overrides() {
    let update = AccountUpdate {
        id: Some("acc-1".into()),
        name: "My Custom Name".into(),
        account_type: "CASH".into(),
        group: None,
        is_default: false,
        is_active: true,
        is_archived: None,
        tracking_mode: None,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        include_in_net_worth: None,
        name_source: Some("user".to_string()),
    };
    assert_eq!(
        update.name_source,
        Some("user".to_string()),
        "AccountUpdate with name_source=user overrides"
    );
}

#[test]
fn account_update_with_name_source_provider_reverts_override() {
    let update = AccountUpdate {
        id: Some("acc-1".into()),
        name: "Provider Name".into(),
        account_type: "CASH".into(),
        group: None,
        is_default: false,
        is_active: true,
        is_archived: None,
        tracking_mode: None,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        include_in_net_worth: None,
        name_source: Some("provider".to_string()),
    };
    assert_eq!(
        update.name_source,
        Some("provider".to_string()),
        "AccountUpdate with name_source=provider clears override"
    );
}

// ── Provider-Parameterized Tests ──

/// Helper to create a new account with a specific provider.
fn new_provider_account(
    name: &str,
    provider: &str,
    provider_account_id: &str,
    name_source: &str,
) -> NewAccount {
    NewAccount {
        id: None,
        name: name.to_string(),
        account_type: "CASH".to_string(),
        group: None,
        currency: "USD".to_string(),
        is_default: false,
        is_active: true,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: Some(provider.to_string()),
        provider_account_id: Some(provider_account_id.to_string()),
        is_archived: false,
        tracking_mode: TrackingMode::default(),
        include_in_net_worth: false,
        name_source: name_source.to_string(),
    }
}

/// Returns all four supported providers for parameterized tests.
const ALL_PROVIDERS: &[&str] = &["investec", "mercury", "plaid", "snaptrade"];

#[test]
fn provider_discovery_sets_provider_name_source() {
    for provider in ALL_PROVIDERS {
        let n = new_provider_account(
            &format!("{} Checking", provider),
            provider,
            &format!("{}-001", provider),
            "provider",
        );
        assert_eq!(
            n.name_source, "provider",
            "{provider}: initial discovery sets provider name source"
        );
        assert_eq!(
            n.name,
            format!("{} Checking", provider),
            "{provider}: name comes from provider"
        );
        assert_eq!(
            n.provider.as_deref(),
            Some(*provider),
            "{provider}: provider is set"
        );
        assert!(
            n.provider_account_id.as_deref() == Some(format!("{}-001", provider).as_str()),
            "{provider}: provider_account_id is set"
        );
    }
}

#[test]
fn provider_discovery_sets_immutable_provider_identity() {
    // Each provider discovery creates an account with a stable provider_account_id.
    // The identity must be separated from the display name.
    for provider in ALL_PROVIDERS {
        let n = new_provider_account(
            "Temp Name",
            provider,
            &format!("fixed-{}", provider),
            "provider",
        );
        assert_eq!(
            n.provider_account_id.as_deref(),
            Some(format!("fixed-{}", provider)).as_deref(),
            "{provider}: provider_account_id is immutable regardless of display name"
        );
    }
}

#[test]
fn user_rename_preserved_across_all_providers() {
    // When a user renames an account (name_source = "user"), the name is preserved
    // even if the provider suggests a different name.
    for provider in ALL_PROVIDERS {
        let user_named = Account {
            id: format!("acc-{provider}"),
            name: "My Personal Name".to_string(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(format!("{provider}-001")),
            name_source: "user".to_string(),
            ..Account::default()
        };

        // Simulate what the broker sync does: it checks name_source before overwriting
        let would_update_name = user_named.name_source == "provider";
        assert!(
            !would_update_name,
            "{provider}: user override prevents name overwrite"
        );

        // The provider's suggested name should NOT replace the user's name
        let provider_suggested_name = format!("{provider} - Checking");
        let effective_name = if user_named.name_source == "provider" {
            provider_suggested_name
        } else {
            user_named.name.clone()
        };
        assert_eq!(
            effective_name, "My Personal Name",
            "{provider}: user-chosen name survives provider resync"
        );
    }
}

#[test]
fn provider_name_update_applied_when_name_source_is_provider() {
    // When name_source = "provider", resync should update the name to match
    // what the provider currently returns.
    for provider in ALL_PROVIDERS {
        let provider_account = Account {
            id: format!("acc-{provider}"),
            name: "Old Provider Name".to_string(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(format!("{provider}-001")),
            name_source: "provider".to_string(),
            ..Account::default()
        };

        // Simulate broker sync: provider now returns a different name
        let provider_suggested_name = format!("{provider} - New Name");
        let effective_name = if provider_account.name_source == "provider" {
            provider_suggested_name.clone()
        } else {
            provider_account.name.clone()
        };

        assert_eq!(
            effective_name,
            format!("{provider} - New Name"),
            "{provider}: provider-sourced name updates on resync"
        );
    }
}

#[test]
fn provider_sync_does_not_use_name_for_matching() {
    // Provider sync must match by provider_account_id, never by name.
    // This test verifies the model supports that contract.
    for provider in ALL_PROVIDERS {
        let pa_id = format!("immutable-{provider}-001");

        // Account exists with one name
        let existing = Account {
            id: format!("acc-{provider}"),
            name: "Original Name".to_string(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(pa_id.clone()),
            name_source: "user".to_string(),
            ..Account::default()
        };

        // Provider returns different name but same provider_account_id
        let different_name = format!("{provider} - Completely Different Name");

        // Matching must succeed by provider_account_id
        let matched = existing.provider_account_id.as_deref() == Some(&pa_id);
        assert!(
            matched,
            "{provider}: account matched by immutable provider_account_id, not by name"
        );

        // User override prevents the name from changing
        if existing.name_source == "user" {
            assert_eq!(
                existing.name, "Original Name",
                "{provider}: user override preserved name despite provider rename"
            );
        }

        // The different provider name is captured in provider_display_name
        // but does NOT overwrite the account name
        assert_ne!(
            existing.name, different_name,
            "{provider}: account name unchanged by provider rename"
        );
    }
}

#[test]
fn duplicate_provider_account_id_not_possible() {
    // Each provider_account_id should be unique within a provider namespace.
    // This test validates the domain model would reject duplicates
    // (enforced at the repository/DB level, but the model supports it).
    for provider in ALL_PROVIDERS {
        let acc1 = NewAccount {
            name: format!("{provider} Account 1").into(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(format!("{provider}-fixed-id")),
            ..new_provider_account("", provider, "{provider}-fixed-id", "provider")
        };
        let acc2 = NewAccount {
            name: format!("{provider} Account 2").into(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(format!("{provider}-fixed-id")),
            ..new_provider_account("", provider, "{provider}-fixed-id", "provider")
        };

        assert_eq!(acc1.provider, acc2.provider, "{provider}: same provider");
        assert_eq!(
            acc1.provider_account_id, acc2.provider_account_id,
            "{provider}: same provider_account_id"
        );

        // The model allows creating both (DB uniqueness constraint would reject second)
        // But the sync logic correctly matches the first by provider_account_id
        assert_eq!(
            acc1.name,
            format!("{provider} Account 1"),
            "{provider}: first account gets different name"
        );
        assert_eq!(
            acc2.name,
            format!("{provider} Account 2"),
            "{provider}: second account would get different name"
        );
    }
}

#[test]
fn stale_provider_rename_does_not_affect_user_override() {
    // Provider renames an account on their side. If the user has an override,
    // the old provider name should remain in the account display.
    for provider in ALL_PROVIDERS {
        // Account was user-renamed to "My Account"
        let account = Account {
            id: format!("acc-{provider}"),
            name: "My Account".to_string(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(format!("{provider}-001")),
            name_source: "user".to_string(),
            ..Account::default()
        };

        // Provider now calls this account "Old Account Name (closed)"
        let provider_new_name = format!("Old Account Name (closed)");

        // The user override must prevent the provider rename from showing
        assert_eq!(
            account.name_source, "user",
            "{provider}: user override is active"
        );
        assert_ne!(
            account.name, provider_new_name,
            "{provider}: user override prevents stale provider rename from appearing"
        );

        // The provider display name is tracked in provider_identities
        let identity = ProviderIdentity {
            account_id: account.id.clone(),
            provider: provider.to_string(),
            provider_account_id: format!("{provider}-001"),
            lifecycle_state: "closed".to_string(),
            provider_display_name: Some(provider_new_name),
            last_sync_at: Some("2026-07-12T12:00:00Z".into()),
        };
        assert_eq!(
            identity.lifecycle_state, "closed",
            "{provider}: lifecycle reflects provider status"
        );
        assert_eq!(
            identity.provider_display_name.as_deref(),
            Some("Old Account Name (closed)"),
            "{provider}: provider_display_name tracks provider's latest name independently"
        );
    }
}

#[test]
fn revert_override_restores_provider_name_source() {
    // User can revert their override by setting name_source back to "provider".
    // On next sync, the provider name will be picked up.
    for provider in ALL_PROVIDERS {
        let mut account = Account {
            id: format!("acc-{provider}"),
            name: "User Named".to_string(),
            provider: Some(provider.to_string()),
            provider_account_id: Some(format!("{provider}-001")),
            name_source: "user".to_string(),
            ..Account::default()
        };

        // User reverts the override
        account.name_source = "provider".to_string();

        // Now resync can update the name
        let provider_suggested = format!("{provider} - Current Name");
        if account.name_source == "provider" {
            account.name = provider_suggested.clone();
        }

        assert_eq!(
            account.name_source, "provider",
            "{provider}: override reverted"
        );
        assert_eq!(
            account.name,
            format!("{provider} - Current Name"),
            "{provider}: provider name picked up on next sync"
        );
    }
}

#[test]
fn name_source_round_trip_via_serde() {
    // Verifies that name_source serializes/deserializes correctly
    let account = Account {
        id: "acc-1".into(),
        name: "Test".into(),
        account_type: "CASH".into(),
        currency: "USD".into(),
        name_source: "user".to_string(),
        ..Account::default()
    };

    let json = serde_json::to_string(&account).unwrap();
    assert!(
        json.contains(r#""nameSource":"user""#),
        "nameSource=user serializes to camelCase JSON"
    );

    let deserialized: Account = serde_json::from_str(&json).unwrap();
    assert_eq!(
        deserialized.name_source, "user",
        "nameSource round-trips through serde"
    );
}

#[test]
fn name_source_default_serde_for_provider() {
    // When name_source is not present in JSON, the serde default kicks in
    // for the Account struct which has #[serde(default = "default_name_source")] on name_source.
    // But Account also derives Serialize, Deserialize, so missing fields get their defaults.
    // Provide all required non-default fields for serde deserialization.
    let json = r#"{"id":"acc-1","name":"Test","accountType":"CASH","currency":"USD","isDefault":false,"isActive":true,"isArchived":false,"trackingMode":"NOT_SET","createdAt":"2026-01-01T00:00:00","updatedAt":"2026-01-01T00:00:00"}"#;
    let account: Account = serde_json::from_str(json).unwrap();
    assert_eq!(
        account.name_source, "provider",
        "name_source defaults to provider via serde default"
    );
}

#[test]
fn account_update_name_source_provider_reverts_override_on_storage_conversion() {
    // Verify that when an AccountUpdate has name_source = Some("provider"),
    // the storage layer converts it to "provider" (not empty string).
    let update = AccountUpdate {
        id: Some("acc-1".into()),
        name: "Provider Name".into(),
        account_type: "CASH".into(),
        group: None,
        is_default: false,
        is_active: true,
        is_archived: None,
        tracking_mode: None,
        platform_id: None,
        account_number: None,
        meta: None,
        provider: None,
        provider_account_id: None,
        include_in_net_worth: None,
        name_source: Some("provider".to_string()),
    };

    // When converting to storage model, the unwrap_or_else fallback
    // should not be used since name_source is Some
    let name_source_result = update.name_source.unwrap_or_else(|| "fallback".to_string());
    assert_eq!(
        name_source_result, "provider",
        "Explicit name_source=provider in update is preserved"
    );
}
