use crate::{
    budget_storage_error::{sql_error, StorageError},
    Database,
};
use async_trait::async_trait;
use obz_cycle::{BudgetCycle, CycleError, CycleRepository, CycleStatus, CycleTransition};
use obz_domain::{envelope::Allocation, money::Money};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

pub(crate) const READ: &str = "SELECT c.id,c.period_start,c.period_end,c.income_cents,c.status,c.carried_cents,c.source_cycle_id,
    (SELECT COALESCE(sum(e.allocated_cents),0)::bigint FROM budgeting.envelopes e
     WHERE e.cycle_id=c.id AND e.owner_id=c.owner_id) AS allocated_cents
    FROM budgeting.cycles c WHERE c.owner_id=$1 AND ($2::uuid IS NULL OR c.id=$2)
    ORDER BY c.period_start DESC,c.id DESC LIMIT $3 OFFSET $4";

pub(crate) fn decode(row: sqlx::postgres::PgRow) -> Result<BudgetCycle, StorageError> {
    let income =
        Money::from_cents(row.try_get("income_cents")?).map_err(|_| CycleError::Unavailable)?;
    let allocated =
        Money::from_cents(row.try_get("allocated_cents")?).map_err(|_| CycleError::Unavailable)?;
    let balance = Allocation::new(income, allocated).map_err(|_| CycleError::Unavailable)?;
    let status = CycleStatus::try_from(row.try_get::<&str, _>("status")?)
        .map_err(|_| CycleError::Unavailable)?;
    if status != CycleStatus::Planned && balance.available().to_cents() != 0 {
        return Err(CycleError::Unavailable.into());
    }
    Ok(BudgetCycle {
        id: row.try_get("id")?,
        period_start: row.try_get("period_start")?,
        period_end: row.try_get("period_end")?,
        income_cents: income.to_cents(),
        allocated_cents: allocated.to_cents(),
        available_cents: balance.available().to_cents(),
        status,
        carried_cents: row.try_get("carried_cents")?,
        source_cycle_id: row.try_get("source_cycle_id")?,
    })
}

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
            sqlx::query("SELECT id,owner_id,period_start,period_end,income_cents,currency,status,carried_cents,source_cycle_id,copy_request_id,copy_mode FROM budgeting.cycles LIMIT 0")
                .execute(self.database.pool()).await?;
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
            for statement in [
                format!("GRANT USAGE ON SCHEMA budgeting TO \"{role}\""),
                format!("GRANT SELECT, INSERT (id,owner_id,period_start,period_end,income_cents,carried_cents,source_cycle_id,copy_request_id,copy_mode), UPDATE (status) ON budgeting.cycles TO \"{role}\""),
                format!("GRANT SELECT,INSERT(id,owner_id,cycle_id,from_envelope_id,to_envelope_id,amount_cents) ON budgeting.reallocations TO \"{role}\""),
                format!("GRANT SELECT ON budgeting.envelope_balances TO \"{role}\""),
            ] { sqlx::query(&statement).execute(&mut *tx).await?; }
            tx.commit().await?;
            Ok(())
        }).await
    }
    async fn bounded<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, StorageError>>,
    ) -> Result<T, CycleError> {
        tokio::time::timeout(self.database.timeout(), work)
            .await
            .map_err(|_| CycleError::Unavailable)?
            .map_err(|e| match e {
                StorageError::Sql(e) => sql_error(e),
                StorageError::Application(e) => e,
            })
    }
    async fn transaction(
        &self,
        owner: Uuid,
    ) -> Result<Transaction<'static, Postgres>, StorageError> {
        let mut tx = self.database.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
}
#[async_trait]
impl CycleRepository for PostgresCycleRepository {
    async fn insert(&self, owner: Uuid, record: BudgetCycle) -> Result<BudgetCycle, CycleError> {
        self.bounded(async {
            let mut tx=self.transaction(owner).await?;
            sqlx::query("INSERT INTO budgeting.cycles(id,owner_id,period_start,period_end,income_cents) VALUES($1,$2,$3,$4,$5)")
                .bind(record.id).bind(owner).bind(record.period_start).bind(record.period_end).bind(record.income_cents).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(record)
        }).await
    }
    async fn transition(
        &self,
        owner: Uuid,
        id: Uuid,
        action: CycleTransition,
    ) -> Result<BudgetCycle, CycleError> {
        self.bounded(async {
            let mut tx=self.transaction(owner).await?;
            // Same namespace/order as envelope mutations; validate only after acquiring it.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||$1::uuid::text,0))")
                .bind(id).execute(&mut *tx).await?;
            let row=sqlx::query(READ).bind(owner).bind(Some(id)).bind(1_i64).bind(0_i64)
                .fetch_optional(&mut *tx).await?.ok_or(CycleError::NotFound)?;
            let mut record=decode(row)?;
            record.apply_transition(action)?;
            let changed=sqlx::query("UPDATE budgeting.cycles SET status=$3 WHERE owner_id=$1 AND id=$2")
                .bind(owner).bind(id).bind(record.status.as_str()).execute(&mut *tx).await?.rows_affected();
            if changed!=1 { return Err(CycleError::NotFound.into()); }
            tx.commit().await?;
            Ok(record)
        }).await
    }
    async fn read(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetCycle>, CycleError> {
        self.bounded(async {
            let mut tx = self.transaction(owner).await?;
            let rows = sqlx::query(READ)
                .bind(owner)
                .bind(id)
                .bind(i64::from(limit))
                .bind(i64::from(offset))
                .fetch_all(&mut *tx)
                .await?;
            let records = rows
                .into_iter()
                .map(decode)
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            Ok(records)
        })
        .await
    }
}
