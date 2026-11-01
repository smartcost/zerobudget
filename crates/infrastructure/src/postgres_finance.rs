use crate::{
    postgres_banking::{sql_error, Error},
    Database,
};
use async_trait::async_trait;
use obz_banking::{BankingError, Correction, CreditEntry, FinanceOverview, FinanceRepository};
use obz_domain::finance::FinanceNote;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
#[derive(Clone)]
pub struct PostgresFinanceRepository {
    database: Database,
}
fn date(note: &FinanceNote) -> Result<sqlx::types::chrono::NaiveDate, Error> {
    sqlx::types::chrono::NaiveDate::from_num_days_from_ce_opt(note.day())
        .ok_or(BankingError::Invalid.into())
}
fn note(r: &sqlx::postgres::PgRow) -> Result<FinanceNote, Error> {
    FinanceNote::new(r.try_get("description")?, r.try_get("day")?)
        .map_err(|_| BankingError::Unavailable.into())
}
fn entry(r: sqlx::postgres::PgRow) -> Result<CreditEntry, Error> {
    Ok(CreditEntry {
        id: r.try_get("id")?,
        card_id: r.try_get("card_id")?,
        invoice_id: r.try_get("invoice_id")?,
        expense_id: r.try_get("expense_id")?,
        opening_debt_id: r.try_get("opening_debt_id")?,
        note: note(&r)?,
        amount_cents: r.try_get("amount_cents")?,
    })
}
impl PostgresFinanceRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    async fn bounded<T>(
        &self,
        f: impl std::future::Future<Output = Result<T, Error>>,
    ) -> Result<T, BankingError> {
        tokio::time::timeout(self.database.timeout(), f)
            .await
            .map_err(|_| BankingError::Unavailable)?
            .map_err(|e| match e {
                Error::Sql(e) => sql_error(e),
                Error::Application(e) => e,
            })
    }
    async fn tx(&self, owner: Uuid, read: bool) -> Result<Transaction<'static, Postgres>, Error> {
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
        if !read {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||$1::uuid::text,0))").bind(owner).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    async fn save(
        &self,
        owner: Uuid,
        mut record: CreditEntry,
        opening: bool,
    ) -> Result<CreditEntry, BankingError> {
        let work = async {
            let mut tx = self.tx(owner, false).await?;
            let table = if opening {
                "opening_debts"
            } else {
                "card_credits"
            };
            let select = if opening {
                "SELECT id,card_id,invoice_id,NULL::uuid AS expense_id,NULL::uuid AS opening_debt_id,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.opening_debts WHERE owner_id=$1 AND id=$2"
            } else {
                "SELECT id,card_id,invoice_id,expense_id,opening_debt_id,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.card_credits WHERE owner_id=$1 AND id=$2"
            };
            if let Some(row) = sqlx::query(select)
                .bind(owner)
                .bind(record.id)
                .fetch_optional(&mut *tx)
                .await?
            {
                let old = entry(row)?;
                if old.card_id != record.card_id
                    || old.invoice_id != record.invoice_id
                    || old.expense_id != record.expense_id
                    || old.opening_debt_id != record.opening_debt_id
                    || old.note != record.note
                    || (opening && old.amount_cents != record.amount_cents)
                {
                    return Err(BankingError::CreditConflict.into());
                }
                tx.commit().await?;
                return Ok(old);
            }
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM banking.card_invoices WHERE owner_id=$1 AND id=$2 AND card_id=$3)").bind(owner).bind(record.invoice_id).bind(record.card_id).fetch_one(&mut *tx).await?;
            if !owned {
                return Err(BankingError::NotFound.into());
            }
            if !opening {
                record.amount_cents=if let Some(expense)=record.expense_id{
sqlx::query_scalar("SELECT x.amount_cents FROM budgeting.expenses x JOIN banking.card_invoices i ON i.id=x.invoice_id AND i.owner_id=x.owner_id WHERE x.id=$1 AND x.owner_id=$2 AND i.card_id=$3").bind(expense).bind(owner).bind(record.card_id).fetch_optional(&mut *tx).await?}
else{
sqlx::query_scalar("SELECT amount_cents FROM banking.opening_debts WHERE id=$1 AND owner_id=$2 AND card_id=$3").bind(record.opening_debt_id).bind(owner).bind(record.card_id).fetch_optional(&mut *tx).await?}
.ok_or(BankingError::NotFound)?;
            }
            let sql = if opening {
                format!("INSERT INTO banking.{table}(id,owner_id,card_id,invoice_id,description,occurred_on,amount_cents) VALUES($1,$2,$3,$4,$5,$6,$7)")
            } else {
                format!("INSERT INTO banking.{table}(id,owner_id,card_id,invoice_id,description,occurred_on,amount_cents,expense_id,opening_debt_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            };
            let q = sqlx::query(&sql)
                .bind(record.id)
                .bind(owner)
                .bind(record.card_id)
                .bind(record.invoice_id)
                .bind(record.note.description())
                .bind(date(&record.note)?)
                .bind(record.amount_cents);
            if opening {
                q.execute(&mut *tx).await?;
            } else {
                q.bind(record.expense_id)
                    .bind(record.opening_debt_id)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            Ok(record)
        };
        self.bounded(work).await
    }
}
#[async_trait]
impl FinanceRepository for PostgresFinanceRepository {
    async fn reverse(&self, owner: Uuid, record: Correction) -> Result<Correction, BankingError> {
        let work = async {
            let mut tx = self.tx(owner, false).await?;
            if let Some(r)=sqlx::query("SELECT movement_id,description,(occurred_on-DATE '0001-01-01'+1) AS day FROM banking.adjustments WHERE owner_id=$1 AND id=$2").bind(owner).bind(record.id).fetch_optional(&mut *tx).await?{
if r.try_get::<Uuid,_>("movement_id")?!=record.movement_id||note(&r)?!=record.note{
return Err(BankingError::CreditConflict.into());
}
tx.commit().await?;
return Ok(record);
}
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM banking.movements WHERE id=$1 AND owner_id=$2)",
            )
            .bind(record.movement_id)
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(BankingError::NotFound.into());
            }
            sqlx::query("INSERT INTO banking.adjustments(id,owner_id,movement_id,description,occurred_on) VALUES($1,$2,$3,$4,$5)").bind(record.id).bind(owner).bind(record.movement_id).bind(record.note.description()).bind(date(&record.note)?).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(record)
        };
        self.bounded(work).await
    }
    async fn opening(&self, owner: Uuid, r: CreditEntry) -> Result<CreditEntry, BankingError> {
        self.save(owner, r, true).await
    }
    async fn credit(&self, owner: Uuid, r: CreditEntry) -> Result<CreditEntry, BankingError> {
        self.save(owner, r, false).await
    }
    async fn entries(
        &self,
        owner: Uuid,
        card: Uuid,
        opening: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<CreditEntry>, BankingError> {
        let work = async {
            let mut tx = self.tx(owner, true).await?;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM banking.credit_cards WHERE id=$1 AND owner_id=$2)",
            )
            .bind(card)
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(BankingError::NotFound.into());
            }
            let sql = if opening {
                "SELECT id,card_id,invoice_id,NULL::uuid AS expense_id,NULL::uuid AS opening_debt_id,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.opening_debts WHERE owner_id=$1 AND card_id=$2 ORDER BY created_at DESC,id DESC LIMIT $3 OFFSET $4"
            } else {
                "SELECT id,card_id,invoice_id,expense_id,opening_debt_id,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.card_credits WHERE owner_id=$1 AND card_id=$2 ORDER BY created_at DESC,id DESC LIMIT $3 OFFSET $4"
            };
            let entries = sqlx::query(sql)
                .bind(owner)
                .bind(card)
                .bind(i64::from(limit))
                .bind(i64::from(offset))
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .map(entry)
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            Ok(entries)
        };
        self.bounded(work).await
    }
    async fn overview(&self, owner: Uuid) -> Result<FinanceOverview, BankingError> {
        let work = async {
            let mut tx = self.tx(owner, true).await?;
            let cash:i64=sqlx::query_scalar("SELECT (COALESCE((SELECT sum(opening_balance_cents::numeric) FROM banking.accounts WHERE owner_id=$1),0)+COALESCE((SELECT sum(CASE WHEN from_account_id IS NULL THEN amount_cents::numeric WHEN to_account_id IS NULL THEN -amount_cents::numeric ELSE 0 END) FROM banking.movements WHERE owner_id=$1),0))::bigint").bind(owner).fetch_one(&mut *tx).await?;
            let r=sqlx::query("SELECT COALESCE(sum(greatest(debt,0)),0)::bigint AS debt,COALESCE(sum(greatest(-debt,0)),0)::bigint AS credit FROM banking.invoice_positions WHERE owner_id=$1").bind(owner).fetch_one(&mut *tx).await?;
            let debt: i64 = r.try_get("debt")?;
            let credit: i64 = r.try_get("credit")?;
            let remaining:i64=sqlx::query_scalar("SELECT COALESCE(sum(e.allocated_cents::numeric-e.spent_cents::numeric),0)::bigint FROM budgeting.envelope_balances e JOIN budgeting.cycles c ON c.id=e.cycle_id AND c.owner_id=e.owner_id WHERE c.owner_id=$1 AND c.status='active'").bind(owner).fetch_one(&mut *tx).await?;
            let commitment = i128::from(remaining) + i128::from(debt);
            let funding_gap = i64::try_from((commitment - i128::from(cash)).max(0))
                .map_err(|_| BankingError::Overflow)?;
            let free = i64::try_from((i128::from(cash) - commitment).max(0))
                .map_err(|_| BankingError::Overflow)?;
            tx.commit().await?;
            Ok(FinanceOverview {
                cash_cents: cash,
                card_debt_cents: debt,
                card_credit_cents: credit,
                active_budget_remaining_cents: remaining,
                funding_gap_cents: funding_gap,
                uncommitted_cash_cents: free,
            })
        };
        self.bounded(work).await
    }
}
