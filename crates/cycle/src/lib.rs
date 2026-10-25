//! Application use cases and persistence port for planned budgets.
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
use obz_domain::cycle::{BudgetPeriod, Cycle};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum CycleError {
    #[error("invalid budget input")]
    Invalid,
    #[error("permission denied")]
    Forbidden,
    #[error("budget resource not found")]
    NotFound,
    #[error("cycle already exists for this period")]
    Conflict,
    #[error("envelope name already exists in this cycle")]
    DuplicateEnvelope,
    #[error("allocation exceeds cycle income")]
    Overallocated,
    #[error("budget storage unavailable")]
    Unavailable,
}
#[derive(Debug, Clone)]
pub struct BudgetCycle {
    pub id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub income_cents: i64,
    pub allocated_cents: i64,
    pub available_cents: i64,
}

/// Persistence port. Implementations must scope every operation to the supplied owner.
#[async_trait]
pub trait CycleRepository: Send + Sync {
    async fn insert(&self, owner: Uuid, cycle: BudgetCycle) -> Result<BudgetCycle, CycleError>;
    async fn read(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetCycle>, CycleError>;
}

#[derive(Clone)]
pub struct CycleService {
    repository: Arc<dyn CycleRepository>,
}
impl CycleService {
    pub fn new(repository: Arc<dyn CycleRepository>) -> Self {
        Self { repository }
    }
    pub async fn create(
        &self,
        user: &LocalUser,
        start: NaiveDate,
        end: NaiveDate,
        income: i64,
    ) -> Result<BudgetCycle, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        let period = BudgetPeriod::new(start.num_days_from_ce(), end.num_days_from_ce())
            .map_err(|_| CycleError::Invalid)?;
        let cycle = Cycle::plan(period, income).map_err(|_| CycleError::Invalid)?;
        self.repository
            .insert(
                user.id(),
                BudgetCycle {
                    id: Uuid::new_v4(),
                    period_start: start,
                    period_end: end,
                    income_cents: cycle.income_cents(),
                    allocated_cents: 0,
                    available_cents: cycle.available_cents(),
                },
            )
            .await
    }
    pub async fn list(
        &self,
        user: &LocalUser,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetCycle>, CycleError> {
        user.require(Permission::ReadBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if !(1..=100).contains(&limit) || offset > 100_000 {
            return Err(CycleError::Invalid);
        }
        self.repository.read(user.id(), None, limit, offset).await
    }
    pub async fn get(&self, user: &LocalUser, id: Uuid) -> Result<BudgetCycle, CycleError> {
        user.require(Permission::ReadBudget)
            .map_err(|_| CycleError::Forbidden)?;
        self.repository
            .read(user.id(), Some(id), 1, 0)
            .await?
            .into_iter()
            .next()
            .ok_or(CycleError::NotFound)
    }
}

mod envelope;
pub use envelope::{BudgetEnvelope, EnvelopePatch, EnvelopeRepository, EnvelopeService};
