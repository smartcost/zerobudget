use crate::{
    postgres_banking::{sql_error, Error, PostgresBankingRepository},
    Database,
};
use async_trait::async_trait;
use obz_banking::{BankingError, Card, CardPayment, CreditRepository, Invoice, InvoicePurchase};
use obz_domain::{
    banking::CashMovement,
    credit::{CreditCard, InvoiceBalance, InvoicePeriod},
    money::Money,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
const CARDS:&str="SELECT c.id,c.name,(c.opened_on-DATE '0001-01-01'+1) AS day,c.credit_limit_cents,
 (COALESCE((SELECT sum(x.amount_cents::numeric) FROM budgeting.expenses x JOIN banking.card_invoices i ON i.id=x.invoice_id AND i.owner_id=x.owner_id WHERE i.card_id=c.id AND x.owner_id=c.owner_id AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id)),0)-COALESCE((SELECT sum(p.amount_cents::numeric) FROM banking.card_payments p JOIN banking.card_invoices i ON i.id=p.invoice_id AND i.owner_id=p.owner_id WHERE i.card_id=c.id AND p.owner_id=c.owner_id),0))::bigint AS outstanding
 FROM banking.credit_cards c WHERE c.owner_id=$1 AND ($2::uuid IS NULL OR c.id=$2) ORDER BY c.id LIMIT $3 OFFSET $4";
const INVOICES:&str="SELECT i.id,i.card_id,i.status,(i.period_start-DATE '0001-01-01'+1) AS start_day,(i.period_end-DATE '0001-01-01'+1) AS end_day,(i.due_on-DATE '0001-01-01'+1) AS due_day,
 (SELECT COALESCE(sum(x.amount_cents),0)::bigint FROM budgeting.expenses x WHERE x.invoice_id=i.id AND x.owner_id=i.owner_id AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id)) AS purchases,
 (SELECT COALESCE(sum(p.amount_cents),0)::bigint FROM banking.card_payments p WHERE p.invoice_id=i.id AND p.owner_id=i.owner_id) AS paid
 FROM banking.card_invoices i WHERE i.owner_id=$1 AND i.card_id=$2 AND ($3::uuid IS NULL OR i.id=$3) ORDER BY i.period_start DESC,i.id DESC LIMIT $4 OFFSET $5";
fn card(row: sqlx::postgres::PgRow) -> Result<Card, Error> {
    let name: String = row.try_get("name")?;
    let c = CreditCard::new(
        &name,
        row.try_get("day")?,
        row.try_get("credit_limit_cents")?,
    )
    .map_err(|_| BankingError::Unavailable)?;
    let outstanding = money(row.try_get("outstanding")?)?;
    if outstanding.to_cents() > c.limit().to_cents() {
        return Err(BankingError::Unavailable.into());
    }
    Ok(Card {
        id: row.try_get("id")?,
        card: c,
        outstanding,
    })
}
fn money(n: i64) -> Result<Money, Error> {
    Money::from_cents(n).map_err(|_| BankingError::Unavailable.into())
}
fn invoice(row: sqlx::postgres::PgRow) -> Result<Invoice, Error> {
    let purchases = money(row.try_get("purchases")?)?;
    let paid = money(row.try_get("paid")?)?;
    InvoiceBalance::new(purchases, paid).map_err(|_| BankingError::Unavailable)?;
    Ok(Invoice {
        id: row.try_get("id")?,
        card_id: row.try_get("card_id")?,
        period: InvoicePeriod::new(
            row.try_get("start_day")?,
            row.try_get("end_day")?,
            row.try_get("due_day")?,
        )
        .map_err(|_| BankingError::Unavailable)?,
        closed: row.try_get::<&str, _>("status")? == "closed",
        purchases,
        paid,
    })
}
fn payment(row: sqlx::postgres::PgRow) -> Result<CardPayment, Error> {
    let description: String = row.try_get("description")?;
    Ok(CardPayment {
        id: row.try_get("id")?,
        invoice_id: row.try_get("invoice_id")?,
        account_id: row.try_get("account_id")?,
        movement: CashMovement::new(
            &description,
            row.try_get("day")?,
            row.try_get("amount_cents")?,
        )
        .map_err(|_| BankingError::Unavailable)?,
    })
}
fn date(day: i32) -> Result<sqlx::types::chrono::NaiveDate, Error> {
    sqlx::types::chrono::NaiveDate::from_num_days_from_ce_opt(day)
        .ok_or(BankingError::Invalid.into())
}
#[derive(Clone)]
pub struct PostgresCreditRepository {
    database: Database,
}
impl PostgresCreditRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn check_schema(&self) -> Result<(), BankingError> {
        self.bounded(async{for sql in ["SELECT id,owner_id,name,opened_on,credit_limit_cents FROM banking.credit_cards LIMIT 0","SELECT id,owner_id,card_id,period_start,period_end,due_on,status FROM banking.card_invoices LIMIT 0","SELECT id,owner_id,invoice_id,account_id,occurred_on,amount_cents FROM banking.card_payments LIMIT 0"]{sqlx::query(sql).execute(self.database.pool()).await?;}Ok(())}).await
    }
    pub async fn grant_runtime_role(&self, role: &str) -> Result<(), BankingError> {
        if role.is_empty()
            || role.len() > 63
            || !role.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(BankingError::Invalid);
        }
        self.bounded(async{let mut tx=self.database.pool().begin().await?;for sql in [format!("GRANT USAGE ON SCHEMA banking TO \"{role}\""),format!("GRANT SELECT,INSERT(id,owner_id,name,opened_on,credit_limit_cents) ON banking.credit_cards TO \"{role}\""),format!("GRANT SELECT,INSERT(id,owner_id,card_id,period_start,period_end,due_on),UPDATE(status) ON banking.card_invoices TO \"{role}\""),format!("GRANT SELECT,INSERT(id,owner_id,invoice_id,account_id,description,occurred_on,amount_cents) ON banking.card_payments TO \"{role}\""),format!("GRANT INSERT(invoice_id) ON budgeting.expenses TO \"{role}\""),format!("GRANT INSERT(card_payment_id) ON banking.movements TO \"{role}\"")]{sqlx::query(&sql).execute(&mut *tx).await?;}tx.commit().await?;Ok(())}).await
    }
    async fn bounded<T>(
        &self,
        work: impl std::future::Future<Output = Result<T, Error>>,
    ) -> Result<T, BankingError> {
        tokio::time::timeout(self.database.timeout(), work)
            .await
            .map_err(|_| BankingError::Unavailable)?
            .map_err(|e| match e {
                Error::Sql(e) => sql_error(e),
                Error::Application(e) => e,
            })
    }
    async fn tx(&self, owner: Uuid, lock: bool) -> Result<Transaction<'static, Postgres>, Error> {
        let mut tx = self.database.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        if lock {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||$1::uuid::text,0))").bind(owner).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    async fn card(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        id: Uuid,
    ) -> Result<Card, Error> {
        card(
            sqlx::query(CARDS)
                .bind(owner)
                .bind(Some(id))
                .bind(1_i64)
                .bind(0_i64)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BankingError::NotFound)?,
        )
    }
    async fn invoice(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        card: Uuid,
        id: Uuid,
    ) -> Result<Invoice, Error> {
        Self::card(tx, owner, card).await?;
        invoice(
            sqlx::query(INVOICES)
                .bind(owner)
                .bind(card)
                .bind(Some(id))
                .bind(1_i64)
                .bind(0_i64)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BankingError::NotFound)?,
        )
    }
    pub(crate) async fn funding(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        id: Uuid,
    ) -> Result<(Card, Invoice), Error> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||$1::uuid::text,0))").bind(owner).execute(&mut **tx).await?;
        let card_id: Uuid = sqlx::query_scalar(
            "SELECT card_id FROM banking.card_invoices WHERE id=$1 AND owner_id=$2",
        )
        .bind(id)
        .bind(owner)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(BankingError::NotFound)?;
        let card = Self::card(tx, owner, card_id).await?;
        let invoice = invoice(
            sqlx::query(INVOICES)
                .bind(owner)
                .bind(card_id)
                .bind(Some(id))
                .bind(1_i64)
                .bind(0_i64)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BankingError::NotFound)?,
        )?;
        Ok((card, invoice))
    }
}
#[async_trait]
impl CreditRepository for PostgresCreditRepository {
    async fn create(&self, owner: Uuid, id: Uuid, c: CreditCard) -> Result<Card, BankingError> {
        self.bounded(async{let mut tx=self.tx(owner,false).await?;sqlx::query("INSERT INTO banking.credit_cards(id,owner_id,name,opened_on,credit_limit_cents) VALUES($1,$2,$3,$4,$5)").bind(id).bind(owner).bind(c.name()).bind(date(c.opened_day())?).bind(c.limit().to_cents()).execute(&mut *tx).await?;tx.commit().await?;Ok(Card{id,card:c,outstanding:Money::zero()})}).await
    }
    async fn cards(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Card>, BankingError> {
        self.bounded(async {
            let mut tx = self.tx(owner, false).await?;
            let rows = sqlx::query(CARDS)
                .bind(owner)
                .bind(id)
                .bind(i64::from(limit))
                .bind(i64::from(offset))
                .fetch_all(&mut *tx)
                .await?;
            let records = rows.into_iter().map(card).collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            Ok(records)
        })
        .await
    }
    async fn open(
        &self,
        owner: Uuid,
        id: Uuid,
        card: Uuid,
        p: InvoicePeriod,
    ) -> Result<Invoice, BankingError> {
        self.bounded(async{let mut tx=self.tx(owner,true).await?;Self::card(&mut tx,owner,card).await?;let prior=sqlx::query("SELECT (period_start-DATE '0001-01-01'+1) AS start,(period_end-DATE '0001-01-01'+1) AS end,(due_on-DATE '0001-01-01'+1) AS due FROM banking.card_invoices WHERE owner_id=$1 AND card_id=$2 AND period_start<$3 AND period_end>$4 LIMIT 1").bind(owner).bind(card).bind(date(p.end())?).bind(date(p.start())?).fetch_optional(&mut *tx).await?;if let Some(row)=prior{let other=InvoicePeriod::new(row.try_get("start")?,row.try_get("end")?,row.try_get("due")?).map_err(|_|BankingError::Unavailable)?;if p.overlaps(other){return Err(BankingError::CreditConflict.into());}}sqlx::query("INSERT INTO banking.card_invoices(id,owner_id,card_id,period_start,period_end,due_on) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(owner).bind(card).bind(date(p.start())?).bind(date(p.end())?).bind(date(p.due())?).execute(&mut *tx).await?;tx.commit().await?;Ok(Invoice{id,card_id:card,period:p,closed:false,purchases:Money::zero(),paid:Money::zero()})}).await
    }
    async fn invoices(
        &self,
        owner: Uuid,
        card: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Invoice>, BankingError> {
        self.bounded(async {
            let mut tx = self.tx(owner, false).await?;
            Self::card(&mut tx, owner, card).await?;
            let rows = sqlx::query(INVOICES)
                .bind(owner)
                .bind(card)
                .bind(id)
                .bind(i64::from(limit))
                .bind(i64::from(offset))
                .fetch_all(&mut *tx)
                .await?;
            let records = rows
                .into_iter()
                .map(invoice)
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            Ok(records)
        })
        .await
    }
    async fn close(&self, owner: Uuid, card: Uuid, id: Uuid) -> Result<Invoice, BankingError> {
        self.bounded(async {
            let mut tx = self.tx(owner, true).await?;
            let mut i = Self::invoice(&mut tx, owner, card, id).await?;
            sqlx::query(
                "UPDATE banking.card_invoices SET status='closed' WHERE id=$1 AND owner_id=$2",
            )
            .bind(id)
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            i.closed = true;
            tx.commit().await?;
            Ok(i)
        })
        .await
    }
    async fn pay(
        &self,
        owner: Uuid,
        card: Uuid,
        id: Uuid,
        p: CardPayment,
    ) -> Result<CardPayment, BankingError> {
        self.bounded(async{let mut tx=self.tx(owner,true).await?;let i=Self::invoice(&mut tx,owner,card,id).await?;
 let prior=sqlx::query("SELECT id,invoice_id,account_id,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.card_payments WHERE owner_id=$1 AND id=$2").bind(owner).bind(p.id).fetch_optional(&mut *tx).await?;
 if let Some(row)=prior{let existing=payment(row)?;if existing.invoice_id!=id||existing.account_id!=p.account_id||existing.movement!=p.movement{return Err(BankingError::CreditConflict.into());}tx.commit().await?;return Ok(existing);}
 let account=PostgresBankingRepository::load(&mut tx,owner,p.account_id).await?;let balance=InvoiceBalance::new(i.purchases,i.paid).map_err(|_|BankingError::Unavailable)?;balance.pay(i.closed,p.movement.amount()).map_err(|e|match e{obz_domain::DomainError::CreditOverpayment=>BankingError::Overpayment,obz_domain::DomainError::InvalidInvoiceStatus=>BankingError::InvalidState,_=>BankingError::Invalid})?;if p.movement.day()<i.period.end(){return Err(BankingError::Invalid.into());}p.movement.validate_opening(&account.account).map_err(|_|BankingError::Invalid)?;obz_domain::banking::debit(account.balance,p.movement.amount()).map_err(|_|BankingError::InsufficientFunds)?;sqlx::query("INSERT INTO banking.card_payments(id,owner_id,invoice_id,account_id,description,occurred_on,amount_cents) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(p.id).bind(owner).bind(id).bind(p.account_id).bind(p.movement.description()).bind(date(p.movement.day())?).bind(p.movement.amount().to_cents()).execute(&mut *tx).await?;tx.commit().await?;Ok(p)}).await
    }
    async fn purchases(
        &self,
        owner: Uuid,
        card: Uuid,
        id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<InvoicePurchase>, BankingError> {
        self.bounded(async{let mut tx=self.tx(owner,false).await?;Self::invoice(&mut tx,owner,card,id).await?;let rows=sqlx::query("SELECT x.id,x.cycle_id,x.envelope_id,x.description,x.occurred_on,x.amount_cents,EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id) AS reversed FROM budgeting.expenses x WHERE x.owner_id=$1 AND x.invoice_id=$2 ORDER BY x.occurred_on DESC,x.id DESC LIMIT $3 OFFSET $4").bind(owner).bind(id).bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&mut *tx).await?;let records=rows.into_iter().map(|r|Ok(InvoicePurchase{id:r.try_get("id")?,cycle_id:r.try_get("cycle_id")?,envelope_id:r.try_get("envelope_id")?,description:r.try_get("description")?,occurred_on:r.try_get("occurred_on")?,amount_cents:r.try_get("amount_cents")?,reversed:r.try_get("reversed")?})).collect::<Result<Vec<_>,Error>>()?;tx.commit().await?;Ok(records)}).await
    }
    async fn payments(
        &self,
        owner: Uuid,
        card: Uuid,
        id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<CardPayment>, BankingError> {
        self.bounded(async{let mut tx=self.tx(owner,false).await?;Self::invoice(&mut tx,owner,card,id).await?;let rows=sqlx::query("SELECT id,invoice_id,account_id,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.card_payments WHERE owner_id=$1 AND invoice_id=$2 ORDER BY occurred_on DESC,id DESC LIMIT $3 OFFSET $4").bind(owner).bind(id).bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&mut *tx).await?;let records=rows.into_iter().map(payment).collect::<Result<Vec<_>,_>>()?;tx.commit().await?;Ok(records)}).await
    }
}
