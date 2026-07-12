use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use wealthfolio_core::household_policy::CandidateAccount;

#[derive(Clone, Debug)]
pub struct DeploymentContext(String);
impl DeploymentContext {
    pub fn server_configured(value: impl Into<String>) -> rusqlite::Result<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Self(value))
    }
    fn id(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ProviderPrincipal {
    provider: String,
    principal: String,
}
impl ProviderPrincipal {
    pub fn verified(
        provider: impl Into<String>,
        principal: impl Into<String>,
    ) -> rusqlite::Result<Self> {
        let (provider, principal) = (provider.into(), principal.into());
        if provider.trim().is_empty() || principal.trim().is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Self {
            provider,
            principal,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CredentialContext {
    deployment: DeploymentContext,
    principal: ProviderPrincipal,
    owner: String,
}
impl CredentialContext {
    pub fn authenticated(
        deployment: DeploymentContext,
        principal: ProviderPrincipal,
        authenticated_subject: impl Into<String>,
    ) -> rusqlite::Result<Self> {
        let owner = authenticated_subject.into();
        if owner.trim().is_empty() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(Self {
            deployment,
            principal,
            owner,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalFault {
    AfterAccount,
    AfterGeneration,
    AfterArrangement,
    AfterParticipant,
    AfterAccess,
    AfterCandidate,
}

pub struct OnboardingStore {
    path: String,
}
impl OnboardingStore {
    pub fn new(path: String) -> Self {
        Self { path }
    }
    fn conn(&self) -> rusqlite::Result<Connection> {
        let c = Connection::open(&self.path)?;
        c.execute_batch("PRAGMA foreign_keys=ON")?;
        Ok(c)
    }

    pub fn bind_credential_owner(&self, ctx: &CredentialContext) -> rusqlite::Result<()> {
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO policy_deployments(id) VALUES(?1)",
            [ctx.deployment.id()],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO person_subjects(deployment_id,id) VALUES(?1,?2)",
            params![ctx.deployment.id(), ctx.owner],
        )?;
        let existing: Option<String>=tx.query_row("SELECT credential_owner_subject FROM credential_owner_subjects WHERE deployment_id=?1 AND provider=?2 AND provider_principal=?3",params![ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal],|r|r.get(0)).optional()?;
        if let Some(owner) = existing {
            if owner != ctx.owner {
                return Err(rusqlite::Error::InvalidQuery);
            }
        } else {
            tx.execute(
                "INSERT INTO credential_owner_subjects VALUES(?1,?2,?3,?4)",
                params![
                    ctx.deployment.id(),
                    ctx.principal.provider,
                    ctx.principal.principal,
                    ctx.owner
                ],
            )?;
        }
        tx.commit()
    }

    pub fn discover_candidate(
        &self,
        ctx: &CredentialContext,
        a: CandidateAccount,
    ) -> rusqlite::Result<String> {
        self.require_binding(ctx)?;
        if a.provider != ctx.principal.provider {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.conn()?.execute("INSERT INTO discovered_account_candidates(id,deployment_id,provider,provider_principal,provider_account_id,name,account_type,currency) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal,a.provider_account_id,a.name,a.account_type,a.currency])?;
        Ok(id)
    }

    fn require_binding(&self, ctx: &CredentialContext) -> rusqlite::Result<()> {
        let owner: Option<String>=self.conn()?.query_row("SELECT credential_owner_subject FROM credential_owner_subjects WHERE deployment_id=?1 AND provider=?2 AND provider_principal=?3",params![ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal],|r|r.get(0)).optional()?;
        if owner.as_deref() == Some(ctx.owner.as_str()) {
            Ok(())
        } else {
            Err(rusqlite::Error::QueryReturnedNoRows)
        }
    }

    pub fn approve_candidate(
        &self,
        ctx: &CredentialContext,
        id: &str,
        fault: Option<ApprovalFault>,
    ) -> rusqlite::Result<String> {
        self.require_binding(ctx)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let r:(String,String,String,String)=tx.query_row("SELECT provider_account_id,name,account_type,currency FROM discovered_account_candidates WHERE id=?1 AND deployment_id=?2 AND provider=?3 AND provider_principal=?4 AND state='pending'",params![id,ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal],|x|Ok((x.get(0)?,x.get(1)?,x.get(2)?,x.get(3)?)))?;
        let aid = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO accounts(id,name,account_type,currency,is_default,is_active,provider,provider_account_id,is_archived,tracking_mode) VALUES(?1,?2,?3,?4,0,1,?5,?6,0,'TRANSACTIONS')",params![aid,r.1,r.2,r.3,ctx.principal.provider,r.0])?;
        tx.execute(
            "INSERT INTO native_account_deployments(account_id,deployment_id) VALUES(?1,?2)",
            params![aid, ctx.deployment.id()],
        )?;
        fail(fault, ApprovalFault::AfterAccount)?;
        let h = format!("private:{}", ctx.owner);
        tx.execute(
            "INSERT OR IGNORE INTO households(deployment_id,id) VALUES(?1,?2)",
            params![ctx.deployment.id(), h],
        )?;
        tx.execute(
            "UPDATE policy_deployments SET access_generation=access_generation+1 WHERE id=?1",
            [ctx.deployment.id()],
        )?;
        fail(fault, ApprovalFault::AfterGeneration)?;
        let generation: i64 = tx.query_row(
            "SELECT access_generation FROM policy_deployments WHERE id=?1",
            [ctx.deployment.id()],
            |x| x.get(0),
        )?;
        let arrangement = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO account_ownership_arrangements(deployment_id,id,household_id,account_id,kind,start_epoch) VALUES(?1,?2,?3,?4,'unknown',?5)",params![ctx.deployment.id(),arrangement,h,aid,generation])?;
        fail(fault, ApprovalFault::AfterArrangement)?;
        tx.execute("INSERT INTO account_ownership_participants(deployment_id,arrangement_id,person_subject_id) VALUES(?1,?2,?3)",params![ctx.deployment.id(),arrangement,ctx.owner])?;
        fail(fault, ApprovalFault::AfterParticipant)?;
        tx.execute("INSERT INTO account_access_epochs(deployment_id,household_id,account_id,person_subject_id,start_generation) VALUES(?1,?2,?3,?4,?5)",params![ctx.deployment.id(),h,aid,ctx.owner,generation])?;
        fail(fault, ApprovalFault::AfterAccess)?;
        tx.execute("UPDATE discovered_account_candidates SET state='approved',approved_account_id=?2 WHERE id=?1 AND deployment_id=?3 AND provider=?4 AND provider_principal=?5",params![id,aid,ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal])?;
        fail(fault, ApprovalFault::AfterCandidate)?;
        tx.commit()?;
        Ok(aid)
    }

    pub fn unlink_and_relink(
        &self,
        ctx: &CredentialContext,
        account_id: &str,
    ) -> rusqlite::Result<String> {
        self.require_binding(ctx)?;
        let mut c = self.conn()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old:(String,String,String,String)=tx.query_row("SELECT c.provider_account_id,c.name,c.account_type,c.currency FROM discovered_account_candidates c JOIN native_account_deployments n ON n.account_id=c.approved_account_id AND n.deployment_id=c.deployment_id WHERE c.approved_account_id=?1 AND c.deployment_id=?2 AND c.provider=?3 AND c.provider_principal=?4 AND c.state='approved'",params![account_id,ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        tx.execute(
            "UPDATE policy_deployments SET access_generation=access_generation+1 WHERE id=?1",
            [ctx.deployment.id()],
        )?;
        let g: i64 = tx.query_row(
            "SELECT access_generation FROM policy_deployments WHERE id=?1",
            [ctx.deployment.id()],
            |r| r.get(0),
        )?;
        tx.execute("UPDATE account_access_epochs SET end_generation=?1 WHERE deployment_id=?2 AND account_id=?3 AND end_generation IS NULL",params![g,ctx.deployment.id(),account_id])?;
        tx.execute("UPDATE account_ownership_arrangements SET end_epoch=?1 WHERE deployment_id=?2 AND account_id=?3 AND end_epoch IS NULL",params![g,ctx.deployment.id(),account_id])?;
        tx.execute("UPDATE discovered_account_candidates SET state='relinked' WHERE approved_account_id=?1 AND deployment_id=?2",params![account_id,ctx.deployment.id()])?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO discovered_account_candidates(id,deployment_id,provider,provider_principal,provider_account_id,name,account_type,currency) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,ctx.deployment.id(),ctx.principal.provider,ctx.principal.principal,old.0,old.1,old.2,old.3])?;
        tx.commit()?;
        Ok(id)
    }
}
fn fail(actual: Option<ApprovalFault>, stage: ApprovalFault) -> rusqlite::Result<()> {
    if actual == Some(stage) {
        Err(rusqlite::Error::InvalidQuery)
    } else {
        Ok(())
    }
}
