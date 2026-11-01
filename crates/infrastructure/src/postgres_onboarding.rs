use crate::Database;
use async_trait::async_trait;
use obz_auth::{
    Identity, OnboardingError, OnboardingRepository, OnboardingState, OnboardingStatus, Permission,
    SetupCycle, SetupCycleState, SetupProgress,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
#[derive(Clone)]
pub struct PostgresOnboardingRepository {
    database: Database,
}
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Application(#[from] OnboardingError),
}
impl PostgresOnboardingRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn check_schema(&self) -> Result<(), OnboardingError> {
        let work = async {
            let available:bool=sqlx::query_scalar("SELECT has_function_privilege(current_user,'auth.budget_onboarding_state(uuid,text,text)','EXECUTE') AND has_function_privilege(current_user,'auth.complete_budget_onboarding(uuid,text,text)','EXECUTE')").fetch_one(self.database.pool()).await?;
            if !available {
                return Err(OnboardingError::Unavailable.into());
            }
            Ok(())
        };
        self.bounded(work).await
    }
    async fn bounded<T>(
        &self,
        f: impl std::future::Future<Output = Result<T, Error>>,
    ) -> Result<T, OnboardingError> {
        tokio::time::timeout(self.database.timeout(), f)
            .await
            .map_err(|_| OnboardingError::Unavailable)?
            .map_err(|e| match e {
                Error::Application(e) => e,
                Error::Sql(e) => match e.as_database_error().and_then(|e| e.code()).as_deref() {
                    Some("ZO001") => OnboardingError::Managed,
                    Some("42501") => OnboardingError::Forbidden,
                    _ => OnboardingError::Unavailable,
                },
            })
    }
    async fn tx(&self, user: Uuid, read: bool) -> Result<Transaction<'static, Postgres>, Error> {
        let mut tx = self.database.pool().begin().await?;
        sqlx::query(if read {
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"
        } else {
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"
        })
        .execute(&mut *tx)
        .await?;
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(user.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    async fn load(
        tx: &mut Transaction<'_, Postgres>,
        identity: &Identity,
        user: Uuid,
    ) -> Result<OnboardingStatus, Error> {
        let enabled: bool = sqlx::query_scalar(
            "SELECT enabled FROM auth.users WHERE id=$1 AND issuer=$2 AND subject=$3",
        )
        .bind(user)
        .bind(identity.issuer())
        .bind(identity.subject())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(OnboardingError::InvalidIdentity)?;
        if !enabled {
            return Err(OnboardingError::Disabled.into());
        }
        let row=sqlx::query("SELECT floor(extract(epoch FROM completed_at))::bigint AS completed_at,managed FROM auth.budget_onboarding_state($1,$2,$3)").bind(user).bind(identity.issuer()).bind(identity.subject()).fetch_one(&mut **tx).await?;
        let completed_at: Option<i64> = row.try_get("completed_at")?;
        let managed: bool = row.try_get("managed")?;
        let values: Vec<String> = sqlx::query_scalar(
            "SELECT permission FROM auth.permissions WHERE user_id=$1 ORDER BY permission",
        )
        .bind(user)
        .fetch_all(&mut **tx)
        .await?;
        let permissions = values
            .into_iter()
            .map(|p| {
                p.parse::<Permission>()
                    .map_err(|_| OnboardingError::Unavailable)
            })
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        let setup = if permissions.contains(&Permission::ReadBudget) {
            Some(Self::progress(tx, user).await?)
        } else {
            None
        };
        Ok(OnboardingStatus {
            state: if completed_at.is_some() {
                OnboardingState::Completed
            } else if managed {
                OnboardingState::Managed
            } else {
                OnboardingState::Pending
            },
            completed_at,
            permissions,
            setup,
        })
    }
    async fn progress(
        tx: &mut Transaction<'_, Postgres>,
        user: Uuid,
    ) -> Result<SetupProgress, Error> {
        let row=sqlx::query("SELECT EXISTS(SELECT 1 FROM banking.accounts WHERE owner_id=$1) AS accounts,EXISTS(SELECT 1 FROM budgeting.cycles WHERE owner_id=$1) AS cycles,EXISTS(SELECT 1 FROM budgeting.cycles WHERE owner_id=$1 AND status='active') AS active,EXISTS(SELECT 1 FROM budgeting.expenses WHERE owner_id=$1) AS expenses").bind(user).fetch_one(&mut **tx).await?;
        let selected=sqlx::query("SELECT c.id,c.status,EXISTS(SELECT 1 FROM budgeting.envelopes e WHERE e.cycle_id=c.id AND e.owner_id=c.owner_id) AS envelopes,c.income_cents::numeric>COALESCE((SELECT sum(e.allocated_cents::numeric) FROM budgeting.envelopes e WHERE e.cycle_id=c.id AND e.owner_id=c.owner_id),0) AS unallocated FROM budgeting.cycles c WHERE c.owner_id=$1 ORDER BY CASE c.status WHEN 'active' THEN 0 WHEN 'planned' THEN 1 ELSE 2 END,c.period_start DESC,c.id DESC LIMIT 1").bind(user).fetch_optional(&mut **tx).await?;
        let selected_cycle = selected
            .map(|r| {
                Ok::<_, Error>(SetupCycle {
                    id: r.try_get("id")?,
                    status: match r.try_get::<&str, _>("status")? {
                        "planned" => SetupCycleState::Planned,
                        "active" => SetupCycleState::Active,
                        "closed" => SetupCycleState::Closed,
                        _ => return Err(OnboardingError::Unavailable.into()),
                    },
                    has_envelopes: r.try_get("envelopes")?,
                    has_unallocated_income: r.try_get("unallocated")?,
                })
            })
            .transpose()?;
        Ok(SetupProgress {
            has_accounts: row.try_get("accounts")?,
            has_cycles: row.try_get("cycles")?,
            has_active_cycle: row.try_get("active")?,
            has_expenses: row.try_get("expenses")?,
            selected_cycle,
        })
    }
}
#[async_trait]
impl OnboardingRepository for PostgresOnboardingRepository {
    async fn complete(
        &self,
        identity: &Identity,
        user: Uuid,
    ) -> Result<OnboardingStatus, OnboardingError> {
        let work = async {
            let mut tx = self.tx(user, false).await?;
            sqlx::query("SELECT auth.complete_budget_onboarding($1,$2,$3)")
                .bind(user)
                .bind(identity.issuer())
                .bind(identity.subject())
                .execute(&mut *tx)
                .await?;
            let status = Self::load(&mut tx, identity, user).await?;
            tx.commit().await?;
            Ok(status)
        };
        self.bounded(work).await
    }
    async fn status(
        &self,
        identity: &Identity,
        user: Uuid,
    ) -> Result<OnboardingStatus, OnboardingError> {
        let work = async {
            let mut tx = self.tx(user, true).await?;
            let status = Self::load(&mut tx, identity, user).await?;
            tx.commit().await?;
            Ok(status)
        };
        self.bounded(work).await
    }
}
