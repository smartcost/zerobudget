//! Banking use cases and persistence contracts; independent of budget allocation.
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
use obz_domain::{
    banking::{Account, AccountKind, CashMovement},
    money::Money,
};
use std::sync::Arc;
use uuid::Uuid;
#[derive(Debug, thiserror::Error)]
pub enum BankingError {
    #[error("invalid banking input")]
    Invalid,
    #[error("permission denied")]
    Forbidden,
    #[error("banking resource not found")]
    NotFound,
    #[error("account name already exists")]
    Duplicate,
    #[error("insufficient account balance")]
    InsufficientFunds,
    #[error("account balance exceeds integer limit")]
    Overflow,
    #[error("banking storage unavailable")]
    Unavailable,
}
#[derive(Debug, Clone)]
pub struct BankAccount {
    pub id: Uuid,
    pub account: Account,
    pub balance: Money,
}
#[derive(Debug, Clone)]
pub struct BankMovement {
    pub id: Uuid,
    pub kind: String,
    pub from_account_id: Option<Uuid>,
    pub to_account_id: Option<Uuid>,
    pub expense_id: Option<Uuid>,
    pub movement: CashMovement,
}
#[derive(Debug, Clone)]
pub struct StatementEntry {
    pub movement: BankMovement,
    pub delta_cents: i64,
}
#[derive(Debug, Clone)]
pub struct AccountInput {
    pub name: String,
    pub kind: String,
    pub opened_on: NaiveDate,
    pub opening_balance_cents: i64,
}
#[derive(Debug, Clone)]
pub struct MovementInput {
    pub description: String,
    pub occurred_on: NaiveDate,
    pub amount_cents: i64,
}
#[async_trait]
pub trait BankingRepository: Send + Sync {
    async fn create(
        &self,
        owner: Uuid,
        id: Uuid,
        account: Account,
    ) -> Result<BankAccount, BankingError>;
    async fn accounts(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BankAccount>, BankingError>;
    async fn post(
        &self,
        owner: Uuid,
        id: Uuid,
        from: Option<Uuid>,
        to: Uuid,
        movement: CashMovement,
    ) -> Result<BankMovement, BankingError>;
    async fn statement(
        &self,
        owner: Uuid,
        account: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<StatementEntry>, BankingError>;
}
#[derive(Clone)]
pub struct BankingService {
    repository: Arc<dyn BankingRepository>,
}
fn require(user: &LocalUser, permission: Permission) -> Result<(), BankingError> {
    user.require(permission)
        .map_err(|_| BankingError::Forbidden)
}
fn page(limit: u32, offset: u32) -> Result<(), BankingError> {
    if !(1..=100).contains(&limit) || offset > 100_000 {
        Err(BankingError::Invalid)
    } else {
        Ok(())
    }
}
impl BankingService {
    pub fn new(repository: Arc<dyn BankingRepository>) -> Self {
        Self { repository }
    }
    pub async fn create(
        &self,
        user: &LocalUser,
        input: AccountInput,
    ) -> Result<BankAccount, BankingError> {
        require(user, Permission::WriteBudget)?;
        let account = Account::new(
            &input.name,
            AccountKind::try_from(input.kind.as_str()).map_err(|_| BankingError::Invalid)?,
            input.opened_on.num_days_from_ce(),
            input.opening_balance_cents,
        )
        .map_err(|_| BankingError::Invalid)?;
        self.repository
            .create(user.id(), Uuid::new_v4(), account)
            .await
    }
    pub async fn list(
        &self,
        user: &LocalUser,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BankAccount>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository
            .accounts(user.id(), None, limit, offset)
            .await
    }
    pub async fn get(&self, user: &LocalUser, id: Uuid) -> Result<BankAccount, BankingError> {
        require(user, Permission::ReadBudget)?;
        self.repository
            .accounts(user.id(), Some(id), 1, 0)
            .await?
            .into_iter()
            .next()
            .ok_or(BankingError::NotFound)
    }
    pub async fn receipt(
        &self,
        user: &LocalUser,
        account: Uuid,
        input: MovementInput,
    ) -> Result<BankMovement, BankingError> {
        self.post(user, None, account, input).await
    }
    pub async fn transfer(
        &self,
        user: &LocalUser,
        from: Uuid,
        to: Uuid,
        input: MovementInput,
    ) -> Result<BankMovement, BankingError> {
        require(user, Permission::WriteBudget)?;
        if from == to {
            return Err(BankingError::Invalid);
        }
        self.post(user, Some(from), to, input).await
    }
    async fn post(
        &self,
        user: &LocalUser,
        from: Option<Uuid>,
        to: Uuid,
        input: MovementInput,
    ) -> Result<BankMovement, BankingError> {
        require(user, Permission::WriteBudget)?;
        let movement = CashMovement::new(
            &input.description,
            input.occurred_on.num_days_from_ce(),
            input.amount_cents,
        )
        .map_err(|_| BankingError::Invalid)?;
        self.repository
            .post(user.id(), Uuid::new_v4(), from, to, movement)
            .await
    }
    pub async fn statement(
        &self,
        user: &LocalUser,
        account: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<StatementEntry>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository
            .statement(user.id(), account, limit, offset)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Repository(std::sync::Mutex<Vec<Uuid>>);
    #[async_trait]
    impl BankingRepository for Repository {
        async fn create(
            &self,
            owner: Uuid,
            id: Uuid,
            a: Account,
        ) -> Result<BankAccount, BankingError> {
            self.0.lock().unwrap().push(owner);
            Ok(BankAccount {
                id,
                balance: a.opening(),
                account: a,
            })
        }
        async fn accounts(
            &self,
            owner: Uuid,
            _: Option<Uuid>,
            _: u32,
            _: u32,
        ) -> Result<Vec<BankAccount>, BankingError> {
            self.0.lock().unwrap().push(owner);
            Ok(vec![])
        }
        async fn post(
            &self,
            owner: Uuid,
            id: Uuid,
            from: Option<Uuid>,
            to: Uuid,
            m: CashMovement,
        ) -> Result<BankMovement, BankingError> {
            self.0.lock().unwrap().push(owner);
            Ok(BankMovement {
                id,
                from_account_id: from,
                to_account_id: Some(to),
                expense_id: None,
                kind: if from.is_some() {
                    "transfer"
                } else {
                    "receipt"
                }
                .into(),
                movement: m,
            })
        }
        async fn statement(
            &self,
            owner: Uuid,
            _: Uuid,
            _: u32,
            _: u32,
        ) -> Result<Vec<StatementEntry>, BankingError> {
            self.0.lock().unwrap().push(owner);
            Ok(vec![])
        }
    }
    #[tokio::test]
    async fn authorization_validation_and_owner_precede_persistence() {
        let repo = Arc::new(Repository::default());
        let service = BankingService::new(repo.clone());
        let owner = Uuid::new_v4();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let input = MovementInput {
            description: "Receipt".into(),
            occurred_on: date,
            amount_cents: 1,
        };
        let denied = LocalUser::from_persisted(owner, Default::default());
        assert!(matches!(
            service.receipt(&denied, a, input.clone()).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.transfer(&denied, a, b, input.clone()).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.list(&denied, 20, 0).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.statement(&denied, a, 20, 0).await,
            Err(BankingError::Forbidden)
        ));
        let writer =
            LocalUser::from_persisted(owner, [Permission::WriteBudget].into_iter().collect());
        assert!(matches!(
            service.list(&writer, 20, 0).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.transfer(&writer, a, a, input.clone()).await,
            Err(BankingError::Invalid)
        ));
        let mut invalid = input.clone();
        invalid.amount_cents = 0;
        assert!(matches!(
            service.receipt(&writer, a, invalid).await,
            Err(BankingError::Invalid)
        ));
        let reader =
            LocalUser::from_persisted(owner, [Permission::ReadBudget].into_iter().collect());
        assert!(matches!(
            service.receipt(&reader, a, input.clone()).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.list(&reader, 101, 0).await,
            Err(BankingError::Invalid)
        ));
        assert!(repo.0.lock().unwrap().is_empty());
        service.receipt(&writer, a, input.clone()).await.unwrap();
        service.transfer(&writer, a, b, input).await.unwrap();
        service.list(&reader, 20, 0).await.unwrap();
        service.statement(&reader, a, 20, 0).await.unwrap();
        assert_eq!(*repo.0.lock().unwrap(), vec![owner; 4]);
    }
}
