//! Application use cases and persistence port for budgets.
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
pub use obz_domain::cycle::CycleStatus;
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
    #[error("cycle income is not fully allocated")]
    Unallocated,
    #[error("operation not allowed in the current cycle status")]
    InvalidState,
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
    pub status: CycleStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleTransition {
    Activate,
    Close,
}

impl BudgetCycle {
    /// Called by persistence adapters while holding the allocation lock.
    pub fn apply_transition(&mut self, action: CycleTransition) -> Result<(), CycleError> {
        let period = BudgetPeriod::new(
            self.period_start.num_days_from_ce(),
            self.period_end.num_days_from_ce(),
        )
        .map_err(|_| CycleError::Unavailable)?;
        let mut cycle =
            Cycle::restore(period, self.income_cents, self.allocated_cents, self.status)
                .map_err(|_| CycleError::Unavailable)?;
        if cycle.available_cents() != self.available_cents {
            return Err(CycleError::Unavailable);
        }
        match action {
            CycleTransition::Activate => cycle.activate(),
            CycleTransition::Close => cycle.close(),
        }
        .map_err(|e| match e {
            obz_domain::DomainError::UnallocatedIncome => CycleError::Unallocated,
            obz_domain::DomainError::InvalidCycleStatus => CycleError::InvalidState,
            _ => CycleError::Unavailable,
        })?;
        self.status = cycle.status();
        Ok(())
    }
}

/// Persistence port. Implementations must scope every operation to the supplied owner.
#[async_trait]
pub trait CycleRepository: Send + Sync {
    async fn insert(&self, owner: Uuid, cycle: BudgetCycle) -> Result<BudgetCycle, CycleError>;
    /// Atomically load, validate the domain transition, and persist under the
    /// same per-cycle lock used by envelope writes. Foreign IDs return NotFound.
    async fn transition(
        &self,
        owner: Uuid,
        id: Uuid,
        action: CycleTransition,
    ) -> Result<BudgetCycle, CycleError>;
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
                    status: cycle.status(),
                },
            )
            .await
    }
    pub async fn activate(&self, user: &LocalUser, id: Uuid) -> Result<BudgetCycle, CycleError> {
        self.transition(user, id, CycleTransition::Activate).await
    }
    pub async fn close(&self, user: &LocalUser, id: Uuid) -> Result<BudgetCycle, CycleError> {
        self.transition(user, id, CycleTransition::Close).await
    }
    async fn transition(
        &self,
        user: &LocalUser,
        id: Uuid,
        action: CycleTransition,
    ) -> Result<BudgetCycle, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        self.repository.transition(user.id(), id, action).await
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

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[derive(Default)]
    struct Repository(std::sync::Mutex<Vec<(Uuid, Uuid, CycleTransition)>>);
    #[async_trait]
    impl CycleRepository for Repository {
        async fn insert(&self, _: Uuid, _: BudgetCycle) -> Result<BudgetCycle, CycleError> {
            unreachable!()
        }
        async fn read(
            &self,
            _: Uuid,
            _: Option<Uuid>,
            _: u32,
            _: u32,
        ) -> Result<Vec<BudgetCycle>, CycleError> {
            unreachable!()
        }
        async fn transition(
            &self,
            owner: Uuid,
            id: Uuid,
            action: CycleTransition,
        ) -> Result<BudgetCycle, CycleError> {
            self.0.lock().unwrap().push((owner, id, action));
            Err(CycleError::NotFound)
        }
    }
    #[tokio::test]
    async fn transitions_require_write_and_forward_only_the_authenticated_owner() {
        let repository = Arc::new(Repository::default());
        let service = CycleService::new(repository.clone());
        let owner = Uuid::new_v4();
        let id = Uuid::new_v4();
        for permissions in [
            Default::default(),
            [Permission::ReadBudget].into_iter().collect(),
        ] {
            let user = LocalUser::from_persisted(owner, permissions);
            assert!(matches!(
                service.activate(&user, id).await,
                Err(CycleError::Forbidden)
            ));
            assert!(matches!(
                service.close(&user, id).await,
                Err(CycleError::Forbidden)
            ));
        }
        assert!(repository.0.lock().unwrap().is_empty());
        let user =
            LocalUser::from_persisted(owner, [Permission::WriteBudget].into_iter().collect());
        assert!(matches!(
            service.activate(&user, id).await,
            Err(CycleError::NotFound)
        ));
        assert!(matches!(
            service.close(&user, id).await,
            Err(CycleError::NotFound)
        ));
        assert_eq!(
            *repository.0.lock().unwrap(),
            vec![
                (owner, id, CycleTransition::Activate),
                (owner, id, CycleTransition::Close)
            ]
        );
    }
}
