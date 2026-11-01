use crate::{BudgetCycle, CycleError};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
use std::sync::Arc;
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct Reallocation {
    pub id: Uuid,
    pub cycle_id: Uuid,
    pub from_envelope_id: Uuid,
    pub to_envelope_id: Uuid,
    pub amount_cents: i64,
}
#[derive(Debug, Clone)]
pub struct CopyInput {
    pub request_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub new_income_cents: i64,
    pub mode: String,
}
#[derive(Debug, Clone)]
pub struct EnvelopeSummary {
    pub id: Uuid,
    pub name: String,
    pub allocated_cents: i64,
    pub spent_cents: i64,
    pub remaining_cents: i64,
}
#[derive(Debug, Clone)]
pub struct CycleSummary {
    pub cycle: BudgetCycle,
    pub spent_cents: i64,
    pub remaining_cents: i64,
    pub received_cents: i64,
    pub projected_income_gap_cents: i64,
    pub envelopes: Vec<EnvelopeSummary>,
}
#[async_trait]
pub trait PlanningRepository: Send + Sync {
    async fn reallocate(
        &self,
        owner: Uuid,
        record: Reallocation,
    ) -> Result<Reallocation, CycleError>;
    async fn reallocations(
        &self,
        owner: Uuid,
        cycle: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Reallocation>, CycleError>;
    async fn copy(
        &self,
        owner: Uuid,
        source: Uuid,
        input: CopyInput,
    ) -> Result<BudgetCycle, CycleError>;
    async fn summary(&self, owner: Uuid, cycle: Uuid) -> Result<CycleSummary, CycleError>;
}
#[derive(Clone)]
pub struct PlanningService {
    repository: Arc<dyn PlanningRepository>,
}
impl PlanningService {
    pub fn new(repository: Arc<dyn PlanningRepository>) -> Self {
        Self { repository }
    }
    pub async fn reallocate(
        &self,
        user: &LocalUser,
        record: Reallocation,
    ) -> Result<Reallocation, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if record.id.get_version_num() != 7
            || record.id.get_variant() != uuid::Variant::RFC4122
            || record.from_envelope_id == record.to_envelope_id
            || record.amount_cents <= 0
        {
            return Err(CycleError::Invalid);
        }
        self.repository.reallocate(user.id(), record).await
    }
    pub async fn reallocations(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Reallocation>, CycleError> {
        user.require(Permission::ReadBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if !(1..=100).contains(&limit) || offset > 100000 {
            return Err(CycleError::Invalid);
        }
        self.repository
            .reallocations(user.id(), cycle, limit, offset)
            .await
    }
    pub async fn copy(
        &self,
        user: &LocalUser,
        source: Uuid,
        input: CopyInput,
    ) -> Result<BudgetCycle, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if input.request_id.get_version_num() != 7
            || input.request_id.get_variant() != uuid::Variant::RFC4122
            || input.request_id == source
            || input.period_end <= input.period_start
            || input.new_income_cents < 0
            || !matches!(input.mode.as_str(), "empty" | "copy" | "carry_remaining")
        {
            return Err(CycleError::Invalid);
        }
        obz_domain::cycle::BudgetPeriod::new(
            input.period_start.num_days_from_ce(),
            input.period_end.num_days_from_ce(),
        )
        .map_err(|_| CycleError::Invalid)?;
        self.repository.copy(user.id(), source, input).await
    }
    pub async fn summary(&self, user: &LocalUser, cycle: Uuid) -> Result<CycleSummary, CycleError> {
        user.require(Permission::ReadBudget)
            .map_err(|_| CycleError::Forbidden)?;
        self.repository.summary(user.id(), cycle).await
    }
}
