use crate::Database;
use async_trait::async_trait;
use obz_banking::{BankAccount, BankMovement, BankingError, BankingRepository, StatementEntry};
use obz_domain::{
    banking::{Account, AccountKind, CashMovement},
    money::Money,
};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
const ACCOUNTS:&str="SELECT a.id,a.name,a.kind,(a.opened_on-DATE '0001-01-01'+1) AS day,a.opening_balance_cents,
 (a.opening_balance_cents+COALESCE((SELECT sum(CASE WHEN m.to_account_id=a.id THEN m.amount_cents::numeric ELSE -m.amount_cents::numeric END) FROM banking.movements m WHERE m.owner_id=a.owner_id AND (m.from_account_id=a.id OR m.to_account_id=a.id)),0))::bigint AS balance
 FROM banking.accounts a WHERE a.owner_id=$1 AND ($2::uuid IS NULL OR a.id=$2) ORDER BY a.id LIMIT $3 OFFSET $4";
fn account(row: sqlx::postgres::PgRow) -> Result<BankAccount, Error> {
    let name: String = row.try_get("name")?;
    let kind: String = row.try_get("kind")?;
    Ok(BankAccount {
        id: row.try_get("id")?,
        account: Account::new(
            &name,
            AccountKind::try_from(kind.as_str()).map_err(|_| BankingError::Unavailable)?,
            row.try_get("day")?,
            row.try_get("opening_balance_cents")?,
        )
        .map_err(|_| BankingError::Unavailable)?,
        balance: Money::from_cents(row.try_get("balance")?)
            .map_err(|_| BankingError::Unavailable)?,
    })
}
pub(crate) fn movement(row: &sqlx::postgres::PgRow) -> Result<BankMovement, Error> {
    let description: String = row.try_get("description")?;
    Ok(BankMovement {
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        from_account_id: row.try_get("from_account_id")?,
        to_account_id: row.try_get("to_account_id")?,
        expense_id: row.try_get("expense_id")?,
        card_payment_id: row.try_get("card_payment_id")?,
        adjustment_id: row.try_get("adjustment_id")?,
        reversed: row.try_get("reversed")?,
        movement: CashMovement::new(
            &description,
            row.try_get("day")?,
            row.try_get("amount_cents")?,
        )
        .map_err(|_| BankingError::Unavailable)?,
    })
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Application(#[from] BankingError),
}
pub(crate) fn sql_error(e: sqlx::Error) -> BankingError {
    if e.as_database_error().and_then(|e| e.constraint()) == Some("cards_owner_name_key") {
        return BankingError::CreditConflict;
    }
    match e.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("23505") => BankingError::Duplicate,
        Some("ZC001") => BankingError::CreditLimit,
        Some("ZC002") => BankingError::InvalidState,
        Some("ZC003") => BankingError::CreditConflict,
        Some("ZC004") => BankingError::Overpayment,
        Some("42501") => BankingError::Forbidden,
        Some("ZB004") => BankingError::Invalid,
        Some("ZB005") => BankingError::InsufficientFunds,
        Some("ZB006" | "22003") => BankingError::Overflow,
        _ => BankingError::Unavailable,
    }
}
#[derive(Clone)]
pub struct PostgresBankingRepository {
    database: Database,
}
impl PostgresBankingRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }
    pub async fn check_schema(&self) -> Result<(), BankingError> {
        self.bounded(async{sqlx::query("SELECT id,owner_id,name,kind,opened_on,opening_balance_cents FROM banking.accounts LIMIT 0").execute(self.database.pool()).await?;sqlx::query("SELECT id,owner_id,kind,from_account_id,to_account_id,expense_id,amount_cents FROM banking.movements LIMIT 0").execute(self.database.pool()).await?;Ok(())}).await
    }
    pub async fn grant_runtime_role(&self, role: &str) -> Result<(), BankingError> {
        if role.is_empty()
            || role.len() > 63
            || !role.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(BankingError::Invalid);
        }
        self.bounded(async{let mut tx=self.database.pool().begin().await?;
   for s in [format!("GRANT USAGE ON SCHEMA banking TO \"{role}\""),format!("GRANT SELECT,INSERT(id,owner_id,name,kind,opened_on,opening_balance_cents) ON banking.accounts TO \"{role}\""),format!("GRANT SELECT,INSERT(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents,expense_id) ON banking.movements TO \"{role}\""),format!("GRANT INSERT(account_id) ON budgeting.expenses TO \"{role}\"")]{sqlx::query(&s).execute(&mut *tx).await?;}tx.commit().await?;Ok(())}).await
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
    async fn transaction(&self, owner: Uuid) -> Result<Transaction<'static, Postgres>, Error> {
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
    pub(crate) async fn load(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        id: Uuid,
    ) -> Result<BankAccount, Error> {
        account(
            sqlx::query(ACCOUNTS)
                .bind(owner)
                .bind(Some(id))
                .bind(1_i64)
                .bind(0_i64)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or(BankingError::NotFound)?,
        )
    }
}
#[async_trait]
impl BankingRepository for PostgresBankingRepository {
    async fn create(&self, owner: Uuid, id: Uuid, a: Account) -> Result<BankAccount, BankingError> {
        self.bounded(async{let mut tx=self.transaction(owner).await?;let date=sqlx::types::chrono::NaiveDate::from_num_days_from_ce_opt(a.opened_day()).ok_or(BankingError::Invalid)?;sqlx::query("INSERT INTO banking.accounts(id,owner_id,name,kind,opened_on,opening_balance_cents) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(owner).bind(a.name()).bind(a.kind().as_str()).bind(date).bind(a.opening().to_cents()).execute(&mut *tx).await?;tx.commit().await?;Ok(BankAccount{id,balance:a.opening(),account:a})}).await
    }
    async fn accounts(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BankAccount>, BankingError> {
        self.bounded(async {
            let mut tx = self.transaction(owner).await?;
            let rows = sqlx::query(ACCOUNTS)
                .bind(owner)
                .bind(id)
                .bind(i64::from(limit))
                .bind(i64::from(offset))
                .fetch_all(&mut *tx)
                .await?;
            let records = rows
                .into_iter()
                .map(account)
                .collect::<Result<Vec<_>, _>>()?;
            tx.commit().await?;
            Ok(records)
        })
        .await
    }
    async fn post(
        &self,
        owner: Uuid,
        id: Uuid,
        from: Option<Uuid>,
        to: Uuid,
        m: CashMovement,
    ) -> Result<BankMovement, BankingError> {
        self.bounded(async{
  let mut tx=self.transaction(owner).await?;
  sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||$1::uuid::text,0))").bind(owner).execute(&mut *tx).await?;
  if let Some(r)=sqlx::query("SELECT id,kind,from_account_id,to_account_id,expense_id,card_payment_id,adjustment_id,EXISTS(SELECT 1 FROM banking.adjustments a WHERE a.owner_id=movements.owner_id AND a.movement_id=movements.id) AS reversed,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents FROM banking.movements WHERE owner_id=$1 AND id=$2").bind(owner).bind(id).fetch_optional(&mut *tx).await?{let old=movement(&r)?;if old.from_account_id!=from||old.to_account_id!=Some(to)||old.movement!=m||old.kind!=if from.is_some(){"transfer"}else{"receipt"}{return Err(BankingError::CreditConflict.into());}tx.commit().await?;return Ok(old);}
  let target=Self::load(&mut tx,owner,to).await?;m.validate_opening(&target.account).map_err(|_|BankingError::Invalid)?;
  obz_domain::banking::credit(target.balance,m.amount()).map_err(|_|BankingError::Overflow)?;
  if let Some(source)=from{let a=Self::load(&mut tx,owner,source).await?;m.validate_opening(&a.account).map_err(|_|BankingError::Invalid)?;obz_domain::banking::debit(a.balance,m.amount()).map_err(|_|BankingError::InsufficientFunds)?;}
  let kind=if from.is_some(){"transfer"}else{"receipt"};
  let date=sqlx::types::chrono::NaiveDate::from_num_days_from_ce_opt(m.day()).ok_or(BankingError::Invalid)?;
  sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(id).bind(owner).bind(kind).bind(from).bind(to).bind(m.description()).bind(date).bind(m.amount().to_cents()).execute(&mut *tx).await?;
  tx.commit().await?;Ok(BankMovement{id,kind:kind.into(),from_account_id:from,to_account_id:Some(to),expense_id:None,card_payment_id:None,adjustment_id:None,reversed:false,movement:m})
 }).await
    }
    async fn statement(
        &self,
        owner: Uuid,
        id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<StatementEntry>, BankingError> {
        self.bounded(async{
  let mut tx=self.transaction(owner).await?;Self::load(&mut tx,owner,id).await?;
  let rows=sqlx::query("SELECT id,kind,from_account_id,to_account_id,expense_id,card_payment_id,adjustment_id,EXISTS(SELECT 1 FROM banking.adjustments a WHERE a.owner_id=movements.owner_id AND a.movement_id=movements.id) AS reversed,description,(occurred_on-DATE '0001-01-01'+1) AS day,amount_cents,CASE WHEN to_account_id=$2 THEN amount_cents ELSE -amount_cents END AS delta FROM banking.movements WHERE owner_id=$1 AND (from_account_id=$2 OR to_account_id=$2) ORDER BY occurred_on DESC,id DESC LIMIT $3 OFFSET $4").bind(owner).bind(id).bind(i64::from(limit)).bind(i64::from(offset)).fetch_all(&mut *tx).await?;
  let records=rows.iter().map(|r|Ok(StatementEntry{movement:movement(r)?,delta_cents:r.try_get("delta")?})).collect::<Result<Vec<_>,Error>>()?;tx.commit().await?;Ok(records)
 }).await
    }
}
