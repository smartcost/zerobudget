use crate::CycleError;
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
use obz_domain::expense::Expense;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct BudgetExpense {
    pub id: Uuid,
    pub cycle_id: Uuid,
    pub envelope_id: Uuid,
    pub expense: Expense,
    pub reversed: bool,
    pub account_id: Option<Uuid>,
    pub invoice_id: Option<Uuid>,
}
#[derive(Debug, Clone, Copy)]
pub enum ExpenseFunding {
    Account(Uuid),
    Invoice(Uuid),
}
impl ExpenseFunding {
    pub fn ids(self) -> (Option<Uuid>, Option<Uuid>) {
        match self {
            Self::Account(id) => (Some(id), None),
            Self::Invoice(id) => (None, Some(id)),
        }
    }
}
#[derive(Debug, Clone)]
pub struct ExpenseInput {
    pub funding: ExpenseFunding,
    pub description: String,
    pub date: NaiveDate,
    pub amount_cents: i64,
}
#[derive(Debug, Clone)]
pub struct FundedExpense {
    pub funding: ExpenseFunding,
    pub expense: Expense,
}
/// Mutations must acquire the cycle allocation/lifecycle lock before loading the
/// parent, balance and reversal. Preserve the immutable original expense and scope
/// every lookup to owner, cycle and envelope. Reads remain available after closure.
#[async_trait]
pub trait ExpenseRepository: Send + Sync {
    async fn create(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
        id: Uuid,
        funded: FundedExpense,
    ) -> Result<BudgetExpense, CycleError>;
    async fn list(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetExpense>, CycleError>;
    async fn reverse(
        &self,
        owner: Uuid,
        cycle: Uuid,
        envelope: Uuid,
        id: Uuid,
    ) -> Result<BudgetExpense, CycleError>;
}
#[derive(Clone)]
pub struct ExpenseService {
    repository: Arc<dyn ExpenseRepository>,
}
impl ExpenseService {
    pub fn new(repository: Arc<dyn ExpenseRepository>) -> Self {
        Self { repository }
    }
    pub async fn create(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        envelope: Uuid,
        input: ExpenseInput,
    ) -> Result<BudgetExpense, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        let expense = Expense::new(
            &input.description,
            input.date.num_days_from_ce(),
            input.amount_cents,
        )
        .map_err(|_| CycleError::Invalid)?;
        self.repository
            .create(
                user.id(),
                cycle,
                envelope,
                Uuid::new_v4(),
                FundedExpense {
                    funding: input.funding,
                    expense,
                },
            )
            .await
    }
    pub async fn list(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        envelope: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetExpense>, CycleError> {
        user.require(Permission::ReadBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if !(1..=100).contains(&limit) || offset > 100_000 {
            return Err(CycleError::Invalid);
        }
        self.repository
            .list(user.id(), cycle, envelope, limit, offset)
            .await
    }
    pub async fn reverse(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        envelope: Uuid,
        id: Uuid,
    ) -> Result<BudgetExpense, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        self.repository
            .reverse(user.id(), cycle, envelope, id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Repository(std::sync::Mutex<Vec<(Uuid, Uuid, Uuid)>>);
    #[async_trait]
    impl ExpenseRepository for Repository {
        async fn create(
            &self,
            owner: Uuid,
            cycle: Uuid,
            envelope: Uuid,
            id: Uuid,
            funded: FundedExpense,
        ) -> Result<BudgetExpense, CycleError> {
            self.0.lock().unwrap().push((owner, cycle, envelope));
            Ok(BudgetExpense {
                id,
                cycle_id: cycle,
                envelope_id: envelope,
                expense: funded.expense,
                account_id: funded.funding.ids().0,
                invoice_id: funded.funding.ids().1,
                reversed: false,
            })
        }
        async fn list(
            &self,
            owner: Uuid,
            cycle: Uuid,
            envelope: Uuid,
            _: u32,
            _: u32,
        ) -> Result<Vec<BudgetExpense>, CycleError> {
            self.0.lock().unwrap().push((owner, cycle, envelope));
            Ok(Vec::new())
        }
        async fn reverse(
            &self,
            owner: Uuid,
            cycle: Uuid,
            envelope: Uuid,
            _: Uuid,
        ) -> Result<BudgetExpense, CycleError> {
            self.0.lock().unwrap().push((owner, cycle, envelope));
            Err(CycleError::NotFound)
        }
    }
    #[tokio::test]
    async fn permissions_and_input_precede_storage_and_owner_comes_from_user() {
        let repo = Arc::new(Repository::default());
        let service = ExpenseService::new(repo.clone());
        let owner = Uuid::new_v4();
        let cycle = Uuid::new_v4();
        let envelope = Uuid::new_v4();
        let id = Uuid::new_v4();
        let date = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let denied = LocalUser::from_persisted(owner, Default::default());
        assert!(matches!(
            service
                .create(
                    &denied,
                    cycle,
                    envelope,
                    ExpenseInput {
                        funding: ExpenseFunding::Account(Uuid::new_v4()),
                        description: "A".into(),
                        date,
                        amount_cents: 1
                    }
                )
                .await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service.list(&denied, cycle, envelope, 20, 0).await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service.reverse(&denied, cycle, envelope, id).await,
            Err(CycleError::Forbidden)
        ));
        let writer =
            LocalUser::from_persisted(owner, [Permission::WriteBudget].into_iter().collect());
        for (description, amount) in [("", 1), ("A", 0), ("A", -1)] {
            assert!(matches!(
                service
                    .create(
                        &writer,
                        cycle,
                        envelope,
                        ExpenseInput {
                            funding: ExpenseFunding::Account(Uuid::new_v4()),
                            description: description.into(),
                            date,
                            amount_cents: amount
                        }
                    )
                    .await,
                Err(CycleError::Invalid)
            ));
        }
        assert!(matches!(
            service.list(&writer, cycle, envelope, 20, 0).await,
            Err(CycleError::Forbidden)
        ));
        let reader =
            LocalUser::from_persisted(owner, [Permission::ReadBudget].into_iter().collect());
        assert!(matches!(
            service.reverse(&reader, cycle, envelope, id).await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service
                .create(
                    &reader,
                    cycle,
                    envelope,
                    ExpenseInput {
                        funding: ExpenseFunding::Account(Uuid::new_v4()),
                        description: "A".into(),
                        date,
                        amount_cents: 1
                    }
                )
                .await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service.list(&reader, cycle, envelope, 101, 0).await,
            Err(CycleError::Invalid)
        ));
        assert!(repo.0.lock().unwrap().is_empty());
        service
            .create(
                &writer,
                cycle,
                envelope,
                ExpenseInput {
                    funding: ExpenseFunding::Account(Uuid::new_v4()),
                    description: "A".into(),
                    date,
                    amount_cents: 1,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            service.reverse(&writer, cycle, envelope, id).await,
            Err(CycleError::NotFound)
        ));
        service.list(&reader, cycle, envelope, 20, 0).await.unwrap();
        assert_eq!(*repo.0.lock().unwrap(), vec![(owner, cycle, envelope); 3]);
    }
}
