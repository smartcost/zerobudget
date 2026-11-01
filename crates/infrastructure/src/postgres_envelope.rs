use crate::budget_storage_error::{sql_error, StorageError};
use crate::Database;
use async_trait::async_trait;
use obz_cycle::{BudgetEnvelope, CycleError, EnvelopePatch, EnvelopeRepository};
use obz_domain::{
    envelope::{Allocation, Envelope, EnvelopeName},
    money::Money,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

fn domain_error(error: obz_domain::DomainError) -> CycleError {
    match error {
        obz_domain::DomainError::EnvelopeOverbudget => CycleError::Overallocated,
        _ => CycleError::Unavailable,
    }
}
fn decode(row: sqlx::postgres::PgRow) -> Result<BudgetEnvelope, StorageError> {
    let name: String = row.try_get("name")?;
    let amount: i64 = row.try_get("allocated_cents")?;
    let name = EnvelopeName::new(&name).map_err(|_| CycleError::Unavailable)?;
    let amount = Money::from_cents(amount).map_err(|_| CycleError::Unavailable)?;
    Ok(BudgetEnvelope {
        id: row.try_get("id")?,
        cycle_id: row.try_get("cycle_id")?,
        envelope: Envelope::new(name, amount),
        spending: obz_domain::expense::Spending::new(
            amount,
            Money::from_cents(row.try_get("spent_cents")?).map_err(|_| CycleError::Unavailable)?,
        )
        .map_err(|_| CycleError::Unavailable)?,
    })
}
#[derive(Clone)]
pub struct PostgresEnvelopeRepository {
    database: Database,
}
impl PostgresEnvelopeRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn check_schema(&self) -> Result<(), CycleError> {
        self.bounded(async {
            sqlx::query(
                "SELECT id,cycle_id,owner_id,name,allocated_cents FROM budgeting.envelopes LIMIT 0",
            )
            .execute(self.database.pool())
            .await?;
            Ok(())
        })
        .await
    }
    pub async fn grant_runtime_role(&self, role: &str) -> Result<(), CycleError> {
        if role.is_empty()
            || role.len() > 63
            || !role.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(CycleError::Invalid);
        }
        self.bounded(async {
            sqlx::query(&format!("GRANT SELECT, DELETE, INSERT (id,cycle_id,owner_id,name,allocated_cents), UPDATE (name,allocated_cents) ON budgeting.envelopes TO \"{role}\"" )).execute(self.database.pool()).await?;
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
            .map_err(|error| match error {
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
    async fn parent(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        cycle: Uuid,
    ) -> Result<(Money, obz_cycle::CycleStatus), StorageError> {
        let row = sqlx::query(
            "SELECT income_cents,status FROM budgeting.cycles WHERE id=$1 AND owner_id=$2",
        )
        .bind(cycle)
        .bind(owner)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(CycleError::NotFound)?;
        Ok((
            Money::from_cents(row.try_get("income_cents")?).map_err(|_| CycleError::Unavailable)?,
            obz_cycle::CycleStatus::try_from(row.try_get::<&str, _>("status")?)
                .map_err(|_| CycleError::Unavailable)?,
        ))
    }

    async fn locked(
        &self,
        owner: Uuid,
        cycle: Uuid,
    ) -> Result<(Transaction<'static, Postgres>, Allocation), StorageError> {
        let mut tx = self.transaction(owner).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||$1::uuid::text,0))").bind(cycle).execute(&mut *tx).await?;
        let (income, status) = Self::parent(&mut tx, owner, cycle).await?;
        if status != obz_cycle::CycleStatus::Planned {
            return Err(CycleError::InvalidState.into());
        }
        let total:i64=sqlx::query_scalar("SELECT COALESCE(sum(allocated_cents),0)::bigint FROM budgeting.envelopes WHERE owner_id=$1 AND cycle_id=$2").bind(owner).bind(cycle).fetch_one(&mut *tx).await?;
        let allocation = Allocation::new(
            income,
            Money::from_cents(total).map_err(|_| CycleError::Unavailable)?,
        )
        .map_err(|_| CycleError::Unavailable)?;
        Ok((tx, allocation))
    }
    async fn current(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        cycle: Uuid,
        id: Uuid,
    ) -> Result<BudgetEnvelope, StorageError> {
        let row=sqlx::query("SELECT id,cycle_id,name,allocated_cents,0::bigint AS spent_cents FROM budgeting.envelopes WHERE owner_id=$1 AND cycle_id=$2 AND id=$3").bind(owner).bind(cycle).bind(id).fetch_optional(&mut **tx).await?.ok_or(CycleError::NotFound)?;
        decode(row)
    }
}
#[async_trait]
impl EnvelopeRepository for PostgresEnvelopeRepository {
    async fn create(
        &self,
        owner: Uuid,
        cycle: Uuid,
        id: Uuid,
        envelope: Envelope,
    ) -> Result<BudgetEnvelope, CycleError> {
        self.bounded(async {
            let (mut tx,allocation)=self.locked(owner,cycle).await?;
            allocation.replace(Money::zero(),envelope.allocated()).map_err(domain_error)?;
            sqlx::query("INSERT INTO budgeting.envelopes(id,cycle_id,owner_id,name,allocated_cents) VALUES($1,$2,$3,$4,$5)").bind(id).bind(cycle).bind(owner).bind(envelope.name().as_str()).bind(envelope.allocated().to_cents()).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(BudgetEnvelope{id,cycle_id:cycle,spending:obz_domain::expense::Spending::new(envelope.allocated(),Money::zero()).map_err(|_| CycleError::Unavailable)?,envelope})
        }).await
    }
    async fn list(
        &self,
        owner: Uuid,
        cycle: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetEnvelope>, CycleError> {
        self.bounded(async {
            let mut tx=self.transaction(owner).await?;
            Self::parent(&mut tx,owner,cycle).await?;
            let rows=sqlx::query("SELECT id,cycle_id,name,allocated_cents,spent_cents FROM budgeting.envelope_balances WHERE owner_id=$1 AND cycle_id=$2 ORDER BY id LIMIT $3 OFFSET $4").bind(owner).bind(cycle).bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&mut *tx).await?;
            let records=rows.into_iter().map(decode).collect::<Result<Vec<_>,_>>()?;
            tx.commit().await?; Ok(records)
        }).await
    }
    async fn update(
        &self,
        owner: Uuid,
        cycle: Uuid,
        id: Uuid,
        patch: EnvelopePatch,
    ) -> Result<BudgetEnvelope, CycleError> {
        self.bounded(async {
            let (mut tx,allocation)=self.locked(owner,cycle).await?;
            let old=Self::current(&mut tx,owner,cycle,id).await?;
            let envelope=Envelope::new(patch.name.unwrap_or_else(||old.envelope.name().clone()),patch.allocated.unwrap_or(old.envelope.allocated()));
            allocation.replace(old.envelope.allocated(),envelope.allocated()).map_err(domain_error)?;
            let changed=sqlx::query("UPDATE budgeting.envelopes SET name=$4,allocated_cents=$5 WHERE owner_id=$1 AND cycle_id=$2 AND id=$3").bind(owner).bind(cycle).bind(id).bind(envelope.name().as_str()).bind(envelope.allocated().to_cents()).execute(&mut *tx).await?.rows_affected();
            if changed!=1 {return Err(CycleError::NotFound.into());}
            tx.commit().await?; Ok(BudgetEnvelope{id,cycle_id:cycle,spending:obz_domain::expense::Spending::new(envelope.allocated(),Money::zero()).map_err(|_| CycleError::Unavailable)?,envelope})
        }).await
    }
    async fn delete(&self, owner: Uuid, cycle: Uuid, id: Uuid) -> Result<(), CycleError> {
        self.bounded(async {
            let (mut tx, allocation) = self.locked(owner, cycle).await?;
            let old = Self::current(&mut tx, owner, cycle, id).await?;
            allocation
                .replace(old.envelope.allocated(), Money::zero())
                .map_err(domain_error)?;
            let changed = sqlx::query(
                "DELETE FROM budgeting.envelopes WHERE owner_id=$1 AND cycle_id=$2 AND id=$3",
            )
            .bind(owner)
            .bind(cycle)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed != 1 {
                return Err(CycleError::NotFound.into());
            }
            tx.commit().await?;
            Ok(())
        })
        .await
    }
}
