use crate::Database;
use async_trait::async_trait;
use obz_cycle::{BudgetCycle, CycleError, CycleRepository};
use obz_domain::envelope::Allocation;
use obz_domain::money::Money;
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresCycleRepository {
    database: Database,
}
impl PostgresCycleRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn check_schema(&self) -> Result<(), CycleError> {
        self.bounded(async {
            sqlx::query("SELECT id, owner_id, period_start, period_end, income_cents, currency, status FROM budgeting.cycles LIMIT 0").execute(self.database.pool()).await?;
            Ok(())
        }).await
    }
    pub async fn grant_runtime_role(&self, role: &str) -> Result<(), CycleError> {
        if role.is_empty()
            || role.len() > 63
            || !role.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(CycleError::Invalid);
        }
        self.bounded(async {
            let mut tx=self.database.pool().begin().await?;
            for statement in [format!("GRANT USAGE ON SCHEMA budgeting TO \"{role}\""),format!("GRANT SELECT, INSERT (id, owner_id, period_start, period_end, income_cents) ON budgeting.cycles TO \"{role}\"")] {
                sqlx::query(&statement).execute(&mut *tx).await?;
            }
            tx.commit().await
        }).await
    }
    async fn bounded<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, sqlx::Error>>,
    ) -> Result<T, CycleError> {
        tokio::time::timeout(self.database.timeout(), work)
            .await
            .map_err(|_| CycleError::Unavailable)?
            .map_err(crate::postgres_envelope::sql_error)
    }
}
#[async_trait]
impl CycleRepository for PostgresCycleRepository {
    async fn insert(&self, owner: Uuid, record: BudgetCycle) -> Result<BudgetCycle, CycleError> {
        self.bounded(async {
            let mut tx=self.database.pool().begin().await?;
            sqlx::query("SELECT set_config('zerobudget.user_id', $1, true)").bind(owner.to_string()).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO budgeting.cycles(id,owner_id,period_start,period_end,income_cents) VALUES($1,$2,$3,$4,$5)").bind(record.id).bind(owner).bind(record.period_start).bind(record.period_end).bind(record.income_cents).execute(&mut *tx).await?;
            tx.commit().await
        }).await?;
        Ok(record)
    }
    async fn read(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetCycle>, CycleError> {
        self.bounded(async {
            let mut tx=self.database.pool().begin().await?;
            sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)").bind(owner.to_string()).execute(&mut *tx).await?;
            let rows=sqlx::query("SELECT c.id,c.period_start,c.period_end,c.income_cents, (SELECT COALESCE(sum(e.allocated_cents),0)::bigint FROM budgeting.envelopes e WHERE e.cycle_id=c.id AND e.owner_id=c.owner_id) AS allocated_cents FROM budgeting.cycles c WHERE c.owner_id=$1 AND ($2::uuid IS NULL OR id=$2) ORDER BY period_start DESC,id DESC LIMIT $3 OFFSET $4").bind(owner).bind(id).bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&mut *tx).await?;
            let records=rows.into_iter().map(|row| {
                let income:i64=row.try_get("income_cents")?;
                let allocated:i64=row.try_get("allocated_cents")?;
                let balance=Allocation::new(Money::from_cents(income).map_err(|_|sqlx::Error::Protocol("invalid cycle income".into()))?,Money::from_cents(allocated).map_err(|_|sqlx::Error::Protocol("invalid allocation".into()))?).map_err(|_|sqlx::Error::Protocol("invalid cycle balance".into()))?;
                Ok(BudgetCycle{id:row.try_get("id")?,period_start:row.try_get("period_start")?,period_end:row.try_get("period_end")?,income_cents:income,allocated_cents:balance.allocated().to_cents(),available_cents:balance.available().to_cents()})
            }).collect::<Result<Vec<_>,sqlx::Error>>()?;
            tx.commit().await?;
            Ok(records)
        }).await
    }
}
