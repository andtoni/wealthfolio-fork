use rusqlite::Connection;
use wealthfolio_core::household_policy::CandidateAccount;
use wealthfolio_storage_sqlite::{
    db,
    onboarding::{
        ApprovalFault, CredentialContext, DeploymentContext, OnboardingStore, ProviderPrincipal,
    },
};

fn setup() -> (tempfile::TempDir, OnboardingStore, String) {
    let d = tempfile::tempdir().unwrap();
    let p = db::init(d.path().to_str().unwrap()).unwrap();
    db::run_migrations(&p).unwrap();
    (d, OnboardingStore::new(p.clone()), p)
}
fn ctx(d: &str, owner: &str) -> CredentialContext {
    CredentialContext::authenticated(
        DeploymentContext::server_configured(d).unwrap(),
        ProviderPrincipal::verified("snaptrade", "credential-7").unwrap(),
        owner,
    )
    .unwrap()
}
fn candidate(id: &str) -> CandidateAccount {
    CandidateAccount {
        provider: "snaptrade".into(),
        provider_account_id: id.into(),
        name: "Brokerage".into(),
        account_type: "SECURITIES".into(),
        currency: "USD".into(),
    }
}

#[test]
fn substitution_and_cross_deployment_are_rejected() {
    let (_d, s, _) = setup();
    let alice = ctx("prod", "issuer#alice");
    s.bind_credential_owner(&alice).unwrap();
    assert!(s
        .bind_credential_owner(&ctx("prod", "issuer#mallory"))
        .is_err());
    let id = s.discover_candidate(&alice, candidate("external")).unwrap();
    assert!(s
        .approve_candidate(&ctx("stage", "issuer#alice"), &id, None)
        .is_err());
    assert!(s
        .approve_candidate(&ctx("prod", "issuer#mallory"), &id, None)
        .is_err());
}

#[test]
fn every_approval_stage_rolls_back() {
    for stage in [
        ApprovalFault::AfterAccount,
        ApprovalFault::AfterGeneration,
        ApprovalFault::AfterArrangement,
        ApprovalFault::AfterParticipant,
        ApprovalFault::AfterAccess,
        ApprovalFault::AfterCandidate,
    ] {
        let (_d, s, p) = setup();
        let c = ctx("prod", "issuer#alice");
        s.bind_credential_owner(&c).unwrap();
        let id = s.discover_candidate(&c, candidate("external")).unwrap();
        assert!(s.approve_candidate(&c, &id, Some(stage)).is_err());
        let db = Connection::open(p).unwrap();
        for table in [
            "accounts",
            "native_account_deployments",
            "account_ownership_arrangements",
            "account_ownership_participants",
            "account_access_epochs",
        ] {
            let q = format!("SELECT count(*) FROM {table}");
            assert_eq!(
                db.query_row(&q, [], |r| r.get::<_, i64>(0)).unwrap(),
                0,
                "{table} at {stage:?}"
            );
        }
        assert_eq!(
            db.query_row(
                "SELECT access_generation FROM policy_deployments WHERE id='prod'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM discovered_account_candidates WHERE id=?1",
                [id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
    }
}

#[test]
fn approval_is_private_unknown_and_production_relink_is_pending() {
    let (_d, s, p) = setup();
    let c = ctx("prod", "issuer#alice");
    s.bind_credential_owner(&c).unwrap();
    let first = s.discover_candidate(&c, candidate("external")).unwrap();
    let account = s.approve_candidate(&c, &first, None).unwrap();
    let db = Connection::open(&p).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT kind FROM account_ownership_arrangements WHERE account_id=?1",
            [&account],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM account_access_epochs WHERE account_id=?1 AND person_subject_id='issuer#alice' AND end_generation IS NULL",[&account],|r|r.get::<_,i64>(0)).unwrap(),1);
    let second = s.unlink_and_relink(&c, &account).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        db.query_row(
            "SELECT state FROM discovered_account_candidates WHERE id=?1",
            [second],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM account_access_epochs WHERE account_id=?1 AND end_generation IS NULL",[account],|r|r.get::<_,i64>(0)).unwrap(),0);
}
