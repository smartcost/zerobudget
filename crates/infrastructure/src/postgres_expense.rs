use crate::{
    budget_storage_error::{sql_error, StorageError},
    Database,
};
use async_trait::async_trait;
use obz_cycle::{BudgetExpense, CycleError, CycleStatus, ExpenseRepository, FundedExpense};
use obz_domain::{
    cycle::BudgetPeriod,
    expense::{Expense, Spending},
    money::Money,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

const READ: &str = "SELECT x.id,x.cycle_id,x.envelope_id,x.description,x.amount_cents,x.account_id,
    (x.occurred_on-DATE '0001-01-01'+1) AS day,
    EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id) AS reversed
    FROM budgeting.expenses x WHERE x.owner_id=$1 AND x.cycle_id=$2 AND x.envelope_id=$3
    AND ($4::uuid IS NULL OR x.id=$4) ORDER BY x.occurred_on DESC,x.id DESC LIMIT $5 OFFSET $6";
fn decode(row: sqlx::postgres::PgRow) -> Result<BudgetExpense, StorageError> {
    let description: String = row.try_get("description")?;
    let expense = Expense::new(
        &description,
        row.try_get("day")?,
        row.try_get("amount_cents")?,
    )
    .map_err(|_| CycleError::Unavailable)?;
    Ok(BudgetExpense {
        id: row.try_get("id")?,
        account_id: row.try_get("account_id")?,
        cycle_id: row.try_get("cycle_id")?,
        envelope_id: row.try_get("envelope_id")?,
        expense,
        reversed: row.try_get("reversed")?,
    })
}
fn domain(error: obz_domain::DomainError) -> CycleError {
    match error {
        obz_domain::DomainError::InsufficientFunds => CycleError::InsufficientFunds,
        obz_domain::DomainError::InvalidCycleStatus => CycleError::InvalidState,
        obz_domain::DomainError::InvalidExpense => CycleError::Invalid,
        _ => CycleError::Unavailable,
    }
}
#[derive(Clone)]
pub struct PostgresExpenseRepository {
    database: Database,
}
impl PostgresExpenseRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn check_schema(&self) -> Result<(), CycleError> {
        self.bounded(async {
            sqlx::query("SELECT id,cycle_id,envelope_id,owner_id,description,occurred_on,amount_cents FROM budgeting.expenses LIMIT 0").execute(self.database.pool()).await?;
            sqlx::query("SELECT expense_id,cycle_id,envelope_id,owner_id FROM budgeting.expense_reversals LIMIT 0").execute(self.database.pool()).await?;
            Ok(())
        }).await
    }
    pub async fn grant_runtime_role(&self, role: &str) -> Result<(), CycleError> {
        if role.is_empty()
            || role.len() > 63
            || !role.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(CycleError::Invalid);
        }
        self.bounded(async {
            let mut tx=self.database.pool().begin().await?;
            sqlx::query(&format!("GRANT SELECT, INSERT(id,cycle_id,envelope_id,owner_id,description,occurred_on,amount_cents) ON budgeting.expenses TO \"{role}\"" )).execute(&mut *tx).await?;
            sqlx::query(&format!("GRANT SELECT, INSERT(expense_id,cycle_id,envelope_id,owner_id) ON budgeting.expense_reversals TO \"{role}\"" )).execute(&mut *tx).await?;
            tx.commit().await?; Ok(())
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
    async fn parent(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
    ) -> Result<(CycleStatus, BudgetPeriod, Money), StorageError> {
        let row=sqlx::query("SELECT c.status,(c.period_start-DATE '0001-01-01'+1) AS start_day,(c.period_end-DATE '0001-01-01'+1) AS end_day,e.allocated_cents FROM budgeting.cycles c JOIN budgeting.envelopes e ON e.cycle_id=c.id AND e.owner_id=c.owner_id WHERE c.id=$1 AND c.owner_id=$2 AND e.id=$3")
            .bind(cycle).bind(owner).bind(envelope).fetch_optional(&mut **tx).await?.ok_or(CycleError::NotFound)?;
        Ok((
            CycleStatus::try_from(row.try_get::<&str, _>("status")?)
                .map_err(|_| CycleError::Unavailable)?,
            BudgetPeriod::new(row.try_get("start_day")?, row.try_get("end_day")?)
                .map_err(|_| CycleError::Unavailable)?,
            Money::from_cents(row.try_get("allocated_cents")?)
                .map_err(|_| CycleError::Unavailable)?,
        ))
    }
    async fn locked(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
    ) -> Result<(Transaction<'static, Postgres>, BudgetPeriod, Spending), StorageError> {
        let mut tx = self.transaction(owner).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||$1::uuid::text,0))").bind(cycle).execute(&mut *tx).await?;
        let (status, period, allocated) = Self::parent(&mut tx, owner, cycle, envelope).await?;
        if status != CycleStatus::Active {
            return Err(CycleError::InvalidState.into());
        }
        let spent:i64=sqlx::query_scalar("SELECT COALESCE(sum(x.amount_cents),0)::bigint FROM budgeting.expenses x WHERE x.owner_id=$1 AND x.cycle_id=$2 AND x.envelope_id=$3 AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id)")
            .bind(owner).bind(cycle).bind(envelope).fetch_one(&mut *tx).await?;
        let balance = Spending::new(
            allocated,
            Money::from_cents(spent).map_err(|_| CycleError::Unavailable)?,
        )
        .map_err(|_| CycleError::Unavailable)?;
        Ok((tx, period, balance))
    }
}
#[async_trait]
impl ExpenseRepository for PostgresExpenseRepository {
    async fn create(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
        id: Uuid,
        funded: FundedExpense,
    ) -> Result<BudgetExpense, CycleError> {
        self.bounded(async {
            let (mut tx,period,balance)=self.locked(owner,cycle,envelope).await?;
            let expense=funded.expense;
            let account_id=funded.account_id;
            let found:Option<Uuid>=sqlx::query_scalar("SELECT id FROM banking.accounts WHERE id=$1 AND owner_id=$2").bind(account_id).bind(owner).fetch_optional(&mut *tx).await?;
            if found.is_none(){return Err(CycleError::NotFound.into());}
            expense.validate_period(period).map_err(domain)?;
            balance.record(CycleStatus::Active,expense.amount()).map_err(domain)?;
            let date=sqlx::types::chrono::NaiveDate::from_num_days_from_ce_opt(expense.day()).ok_or(CycleError::Invalid)?;
            sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,envelope_id,owner_id,description,occurred_on,amount_cents,account_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(id).bind(cycle).bind(envelope).bind(owner).bind(expense.description()).bind(date).bind(expense.amount().to_cents()).bind(account_id).execute(&mut *tx).await?;
            tx.commit().await?; Ok(BudgetExpense { id,cycle_id:cycle,envelope_id:envelope,expense,reversed:false,account_id:Some(account_id) })
        }).await
    }
    async fn list(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetExpense>, CycleError> {
        self.bounded(async {
            let mut tx = self.transaction(owner).await?;
            Self::parent(&mut tx, owner, cycle, envelope).await?;
            let rows = sqlx::query(READ)
                .bind(owner)
                .bind(cycle)
                .bind(envelope)
                .bind(None::<Uuid>)
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
    async fn reverse(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
        id: Uuid,
    ) -> Result<BudgetExpense, CycleError> {
        self.bounded(async {
            let (mut tx,_,balance)=self.locked(owner,cycle,envelope).await?;
            let row=sqlx::query(READ).bind(owner).bind(cycle).bind(envelope).bind(Some(id)).bind(1_i64).bind(0_i64).fetch_optional(&mut *tx).await?.ok_or(CycleError::NotFound)?;
            let mut record=decode(row)?;
            if !record.reversed {
                balance.reverse(CycleStatus::Active,record.expense.amount()).map_err(domain)?;
                sqlx::query("INSERT INTO budgeting.expense_reversals(expense_id,cycle_id,envelope_id,owner_id) VALUES($1,$2,$3,$4)")
                    .bind(id).bind(cycle).bind(envelope).bind(owner).execute(&mut *tx).await?;
                record.reversed=true;
            }
            tx.commit().await?; Ok(record)
        }).await
    }
}
