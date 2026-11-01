use crate::{
    budget_storage_error::{sql_error, StorageError},
    postgres_cycle::{decode, READ as CYCLES},
    Database,
};
use async_trait::async_trait;
use obz_cycle::{
    BudgetCycle, CopyInput, CycleError, CycleStatus, CycleSummary, EnvelopeSummary,
    PlanningRepository, Reallocation,
};
use obz_domain::money::Money;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
#[derive(Clone)]
pub struct PostgresPlanningRepository {
    database: Database,
}
impl PostgresPlanningRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    async fn bounded<T>(
        &self,
        f: impl std::future::Future<Output = Result<T, StorageError>>,
    ) -> Result<T, CycleError> {
        tokio::time::timeout(self.database.timeout(), f)
            .await
            .map_err(|_| CycleError::Unavailable)?
            .map_err(|e| match e {
                StorageError::Sql(e) => sql_error(e),
                StorageError::Application(e) => e,
            })
    }
    async fn tx(
        &self,
        owner: Uuid,
        read: bool,
    ) -> Result<Transaction<'static, Postgres>, StorageError> {
        let mut tx = self.database.pool().begin().await?;
        sqlx::query(if read {
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"
        } else {
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"
        })
        .execute(&mut *tx)
        .await?;
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    async fn cycle(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        id: Uuid,
    ) -> Result<BudgetCycle, StorageError> {
        decode(
            sqlx::query(CYCLES)
                .bind(owner)
                .bind(Some(id))
                .bind(1_i64)
                .bind(0_i64)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(CycleError::NotFound)?,
        )
    }
    async fn lock(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||$1::uuid::text,0))").bind(id).execute(&mut **tx).await?;
        Ok(())
    }
}
fn allocation(row: sqlx::postgres::PgRow) -> Result<Reallocation, sqlx::Error> {
    Ok(Reallocation {
        id: row.try_get("id")?,
        cycle_id: row.try_get("cycle_id")?,
        from_envelope_id: row.try_get("from_envelope_id")?,
        to_envelope_id: row.try_get("to_envelope_id")?,
        amount_cents: row.try_get("amount_cents")?,
    })
}
#[async_trait]
impl PlanningRepository for PostgresPlanningRepository {
    async fn reallocate(&self, owner: Uuid, r: Reallocation) -> Result<Reallocation, CycleError> {
        let work = async {
            let mut tx = self.tx(owner, false).await?;
            Self::lock(&mut tx, r.cycle_id).await?;
            if let Some(row)=sqlx::query("SELECT id,cycle_id,from_envelope_id,to_envelope_id,amount_cents FROM budgeting.reallocations WHERE owner_id=$1 AND id=$2").bind(owner).bind(r.id).fetch_optional(&mut *tx).await?{
let old=allocation(row)?;
if(old.cycle_id,old.from_envelope_id,old.to_envelope_id,old.amount_cents)!=(r.cycle_id,r.from_envelope_id,r.to_envelope_id,r.amount_cents){
return Err(CycleError::OperationConflict.into());
}
tx.commit().await?;
return Ok(old);
}
            let c = Self::cycle(&mut tx, owner, r.cycle_id).await?;
            if c.status != CycleStatus::Active {
                return Err(CycleError::InvalidState.into());
            }
            let source=sqlx::query("SELECT allocated_cents,spent_cents FROM budgeting.envelope_balances WHERE id=$1 AND cycle_id=$2 AND owner_id=$3").bind(r.from_envelope_id).bind(r.cycle_id).bind(owner).fetch_optional(&mut *tx).await?.ok_or(CycleError::NotFound)?;
            let target:i64=sqlx::query_scalar("SELECT allocated_cents FROM budgeting.envelope_balances WHERE id=$1 AND cycle_id=$2 AND owner_id=$3").bind(r.to_envelope_id).bind(r.cycle_id).bind(owner).fetch_optional(&mut *tx).await?.ok_or(CycleError::NotFound)?;
            obz_domain::planning::reallocate(
                Money::from_cents(source.try_get("allocated_cents")?)
                    .map_err(|_| CycleError::Unavailable)?,
                Money::from_cents(source.try_get("spent_cents")?)
                    .map_err(|_| CycleError::Unavailable)?,
                Money::from_cents(target).map_err(|_| CycleError::Unavailable)?,
                Money::from_cents(r.amount_cents).map_err(|_| CycleError::Invalid)?,
            )
            .map_err(|_| CycleError::InsufficientFunds)?;
            sqlx::query("INSERT INTO budgeting.reallocations(id,owner_id,cycle_id,from_envelope_id,to_envelope_id,amount_cents) VALUES($1,$2,$3,$4,$5,$6)").bind(r.id).bind(owner).bind(r.cycle_id).bind(r.from_envelope_id).bind(r.to_envelope_id).bind(r.amount_cents).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(r)
        };
        self.bounded(work).await
    }
    async fn reallocations(
        &self,
        owner: Uuid,
        id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Reallocation>, CycleError> {
        let work = async {
            let mut tx = self.tx(owner, true).await?;
            Self::cycle(&mut tx, owner, id).await?;
            let rows=sqlx::query("SELECT id,cycle_id,from_envelope_id,to_envelope_id,amount_cents FROM budgeting.reallocations WHERE owner_id=$1 AND cycle_id=$2 ORDER BY created_at DESC,id DESC LIMIT $3 OFFSET $4").bind(owner).bind(id).bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&mut *tx).await?;
            let records = rows
                .into_iter()
                .map(allocation)
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            Ok(records)
        };
        self.bounded(work).await
    }
    async fn copy(
        &self,
        owner: Uuid,
        source: Uuid,
        input: CopyInput,
    ) -> Result<BudgetCycle, CycleError> {
        let work = async {
            let mut tx = self.tx(owner, false).await?;
            Self::lock(&mut tx, source).await?;
            let prior=sqlx::query("SELECT id,source_cycle_id,period_start,period_end,income_cents-carried_cents AS fresh,copy_mode FROM budgeting.cycles WHERE owner_id=$1 AND copy_request_id=$2").bind(owner).bind(input.request_id).fetch_optional(&mut *tx).await?;
            if let Some(r) = prior {
                if r.try_get::<Uuid, _>("source_cycle_id")? != source
                    || r.try_get::<sqlx::types::chrono::NaiveDate, _>("period_start")?
                        != input.period_start
                    || r.try_get::<sqlx::types::chrono::NaiveDate, _>("period_end")?
                        != input.period_end
                    || r.try_get::<i64, _>("fresh")? != input.new_income_cents
                    || r.try_get::<String, _>("copy_mode")? != input.mode
                {
                    return Err(CycleError::OperationConflict.into());
                }
                let record = Self::cycle(&mut tx, owner, r.try_get("id")?).await?;
                tx.commit().await?;
                return Ok(record);
            }
            let old = Self::cycle(&mut tx, owner, source).await?;
            if old.status != CycleStatus::Closed {
                return Err(CycleError::InvalidState.into());
            }
            if input.period_start < old.period_end {
                return Err(CycleError::Invalid.into());
            }
            let rows=sqlx::query("SELECT name,allocated_cents,spent_cents FROM budgeting.envelope_balances WHERE owner_id=$1 AND cycle_id=$2 ORDER BY id").bind(owner).bind(source).fetch_all(&mut *tx).await?;
            let mut copied = Vec::new();
            let mut allocated = Money::zero();
            for row in rows {
                let amount = match input.mode.as_str() {
                    "copy" => row.try_get::<i64, _>("allocated_cents")?,
                    "carry_remaining" => row
                        .try_get::<i64, _>("allocated_cents")?
                        .checked_sub(row.try_get("spent_cents")?)
                        .ok_or(CycleError::Unavailable)?,
                    _ => 0,
                };
                if input.mode != "empty" {
                    allocated = allocated
                        .checked_add(
                            Money::from_cents(amount).map_err(|_| CycleError::Unavailable)?,
                        )
                        .map_err(|_| CycleError::BalanceOverflow)?;
                    copied.push((row.try_get::<String, _>("name")?, amount));
                }
            }
            let carried = if input.mode == "carry_remaining" {
                allocated.to_cents()
            } else {
                0
            };
            let income = Money::from_cents(input.new_income_cents)
                .map_err(|_| CycleError::Invalid)?
                .checked_add(Money::from_cents(carried).map_err(|_| CycleError::Unavailable)?)
                .map_err(|_| CycleError::BalanceOverflow)?
                .to_cents();
            if allocated.to_cents() > income {
                return Err(CycleError::Overallocated.into());
            }
            sqlx::query("INSERT INTO budgeting.cycles(id,owner_id,period_start,period_end,income_cents,carried_cents,source_cycle_id,copy_request_id,copy_mode) VALUES($1,$2,$3,$4,$5,$6,$7,$1,$8)").bind(input.request_id).bind(owner).bind(input.period_start).bind(input.period_end).bind(income).bind(carried).bind(source).bind(&input.mode).execute(&mut *tx).await?;
            for (name, amount) in copied {
                sqlx::query("INSERT INTO budgeting.envelopes(id,owner_id,cycle_id,name,allocated_cents) VALUES(uuidv7(),$1,$2,$3,$4)").bind(owner).bind(input.request_id).bind(name).bind(amount).execute(&mut *tx).await?;
            }
            let record = Self::cycle(&mut tx, owner, input.request_id).await?;
            tx.commit().await?;
            Ok(record)
        };
        self.bounded(work).await
    }
    async fn summary(&self, owner: Uuid, id: Uuid) -> Result<CycleSummary, CycleError> {
        let work = async {
            let mut tx = self.tx(owner, true).await?;
            let cycle = Self::cycle(&mut tx, owner, id).await?;
            let rows=sqlx::query("SELECT id,name,allocated_cents,spent_cents FROM budgeting.envelope_balances WHERE cycle_id=$1 AND owner_id=$2 ORDER BY id").bind(id).bind(owner).fetch_all(&mut *tx).await?;
            let mut spent = 0_i64;
            let mut remaining = 0_i64;
            let mut envelopes = Vec::new();
            for r in rows {
                let a: i64 = r.try_get("allocated_cents")?;
                let s: i64 = r.try_get("spent_cents")?;
                let left = a.checked_sub(s).ok_or(CycleError::Unavailable)?;
                spent = spent.checked_add(s).ok_or(CycleError::BalanceOverflow)?;
                remaining = remaining
                    .checked_add(left)
                    .ok_or(CycleError::BalanceOverflow)?;
                envelopes.push(EnvelopeSummary {
                    id: r.try_get("id")?,
                    name: r.try_get("name")?,
                    allocated_cents: a,
                    spent_cents: s,
                    remaining_cents: left,
                });
            }
            let received:i64=sqlx::query_scalar("SELECT (COALESCE((SELECT sum(amount_cents::numeric) FROM banking.movements WHERE owner_id=$1 AND kind='receipt' AND occurred_on >=$2 AND occurred_on<$3),0)-COALESCE((SELECT sum(m.amount_cents::numeric) FROM banking.adjustments a JOIN banking.movements m ON m.id=a.movement_id AND m.owner_id=a.owner_id WHERE a.owner_id=$1 AND m.kind='receipt' AND a.occurred_on >=$2 AND a.occurred_on<$3),0))::bigint").bind(owner).bind(cycle.period_start).bind(cycle.period_end).fetch_one(&mut *tx).await?;
            let gap = cycle
                .income_cents
                .checked_sub(cycle.carried_cents)
                .and_then(|v| v.checked_sub(received))
                .ok_or(CycleError::BalanceOverflow)?
                .max(0);
            tx.commit().await?;
            Ok(CycleSummary {
                cycle,
                spent_cents: spent,
                remaining_cents: remaining,
                received_cents: received,
                projected_income_gap_cents: gap,
                envelopes,
            })
        };
        self.bounded(work).await
    }
}
