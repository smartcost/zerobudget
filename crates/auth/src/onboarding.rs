//! Explicit, one-time financial onboarding. No grants happen during login.
use crate::{Identity, LocalUser, Permission};
use async_trait::async_trait;
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OnboardingError {
    #[error("verified email required")]
    Forbidden,
    #[error("invalid or expired identity")]
    InvalidIdentity,
    #[error("user account disabled")]
    Disabled,
    #[error("account access managed by operator")]
    Managed,
    #[error("onboarding storage unavailable")]
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingState {
    Pending,
    Completed,
    Managed,
}
impl OnboardingState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Managed => "managed",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupCycleState {
    Planned,
    Active,
    Closed,
}
impl SetupCycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Active => "active",
            Self::Closed => "closed",
        }
    }
}
#[derive(Debug, Clone)]
pub struct SetupCycle {
    pub id: Uuid,
    pub status: SetupCycleState,
    pub has_envelopes: bool,
    pub has_unallocated_income: bool,
}
#[derive(Debug, Clone)]
pub struct SetupProgress {
    pub has_accounts: bool,
    pub has_cycles: bool,
    pub has_active_cycle: bool,
    /// Includes historical reversals: the first expense milestone is not undone.
    pub has_expenses: bool,
    /// Latest active cycle first, otherwise latest planned, otherwise latest closed.
    pub selected_cycle: Option<SetupCycle>,
}
#[derive(Debug, Clone)]
pub struct OnboardingStatus {
    pub state: OnboardingState,
    /// Unix seconds, stable on every repeated completion.
    pub completed_at: Option<i64>,
    pub permissions: BTreeSet<Permission>,
    /// Returned only to users currently allowed to read their budget.
    pub setup: Option<SetupProgress>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextStep {
    CompleteOnboarding,
    ContactOperator,
    CreateAccount,
    CreateCycle,
    AllocateIncome,
    ActivateCycle,
    RecordExpense,
    Ready,
}
impl NextStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CompleteOnboarding => "complete_onboarding",
            Self::ContactOperator => "contact_operator",
            Self::CreateAccount => "create_account",
            Self::CreateCycle => "create_cycle",
            Self::AllocateIncome => "allocate_income",
            Self::ActivateCycle => "activate_cycle",
            Self::RecordExpense => "record_expense",
            Self::Ready => "ready",
        }
    }
}
impl OnboardingStatus {
    pub fn next_step(&self) -> NextStep {
        if self.state == OnboardingState::Pending {
            return NextStep::CompleteOnboarding;
        }
        if !self.permissions.contains(&Permission::ReadBudget)
            || !self.permissions.contains(&Permission::WriteBudget)
        {
            return NextStep::ContactOperator;
        }
        let Some(setup) = &self.setup else {
            return NextStep::ContactOperator;
        };
        if !setup.has_accounts {
            return NextStep::CreateAccount;
        }
        let Some(cycle) = &setup.selected_cycle else {
            return NextStep::CreateCycle;
        };
        match cycle.status {
            SetupCycleState::Closed => NextStep::CreateCycle,
            SetupCycleState::Planned if cycle.has_unallocated_income => NextStep::AllocateIncome,
            SetupCycleState::Planned => NextStep::ActivateCycle,
            SetupCycleState::Active if !setup.has_expenses => NextStep::RecordExpense,
            SetupCycleState::Active => NextStep::Ready,
        }
    }
}
#[async_trait]
pub trait OnboardingRepository: Send + Sync {
    /// Implementations must recheck the identity binding, enabled state and current
    /// permissions, and serialize completion with administrative changes.
    async fn complete(
        &self,
        identity: &Identity,
        user: Uuid,
    ) -> Result<OnboardingStatus, OnboardingError>;
    /// Read-only snapshot. Never grants or repairs permissions.
    async fn status(
        &self,
        identity: &Identity,
        user: Uuid,
    ) -> Result<OnboardingStatus, OnboardingError>;
}
#[derive(Clone)]
pub struct OnboardingService {
    repository: Arc<dyn OnboardingRepository>,
}
impl OnboardingService {
    pub fn new(repository: Arc<dyn OnboardingRepository>) -> Self {
        Self { repository }
    }
    fn validate(identity: &Identity, user: &LocalUser) -> Result<(), OnboardingError> {
        identity
            .validate_current()
            .map_err(|_| OnboardingError::InvalidIdentity)?;
        if user.id().is_nil() || !identity.email_verified() {
            return Err(OnboardingError::Forbidden);
        }
        Ok(())
    }
    pub async fn complete(
        &self,
        identity: &Identity,
        user: &LocalUser,
    ) -> Result<OnboardingStatus, OnboardingError> {
        Self::validate(identity, user)?;
        self.repository.complete(identity, user.id()).await
    }
    pub async fn status(
        &self,
        identity: &Identity,
        user: &LocalUser,
    ) -> Result<OnboardingStatus, OnboardingError> {
        Self::validate(identity, user)?;
        self.repository.status(identity, user.id()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Repository(Mutex<Vec<Uuid>>);
    fn complete_status() -> OnboardingStatus {
        OnboardingStatus {
            state: OnboardingState::Completed,
            completed_at: Some(10),
            permissions: [Permission::ReadBudget, Permission::WriteBudget]
                .into_iter()
                .collect(),
            setup: Some(SetupProgress {
                has_accounts: false,
                has_cycles: false,
                has_active_cycle: false,
                has_expenses: false,
                selected_cycle: None,
            }),
        }
    }
    #[async_trait]
    impl OnboardingRepository for Repository {
        async fn complete(
            &self,
            _: &Identity,
            id: Uuid,
        ) -> Result<OnboardingStatus, OnboardingError> {
            self.0.lock().unwrap().push(id);
            Ok(complete_status())
        }
        async fn status(
            &self,
            _: &Identity,
            id: Uuid,
        ) -> Result<OnboardingStatus, OnboardingError> {
            self.0.lock().unwrap().push(id);
            Ok(complete_status())
        }
    }
    #[tokio::test]
    async fn verified_current_identity_precedes_storage_and_only_authenticated_id_is_forwarded() {
        let repository = Arc::new(Repository::default());
        let service = OnboardingService::new(repository.clone());
        let user = LocalUser::from_persisted(Uuid::now_v7(), BTreeSet::new());
        let mut identity = Identity::for_test("https://id.example/realms/test", "user");
        assert_eq!(
            service.complete(&identity, &user).await.unwrap_err(),
            OnboardingError::Forbidden
        );
        assert_eq!(
            service.status(&identity, &user).await.unwrap_err(),
            OnboardingError::Forbidden
        );
        assert!(repository.0.lock().unwrap().is_empty());
        identity.email_verified = true;
        service.complete(&identity, &user).await.unwrap();
        service.status(&identity, &user).await.unwrap();
        assert_eq!(*repository.0.lock().unwrap(), vec![user.id(), user.id()]);
        identity.set_test_expiration(1);
        assert_eq!(
            service.complete(&identity, &user).await.unwrap_err(),
            OnboardingError::InvalidIdentity
        );
    }
    #[test]
    fn revoked_rights_never_become_an_onboarding_instruction() {
        let mut status = complete_status();
        status.permissions.remove(&Permission::WriteBudget);
        assert_eq!(status.next_step(), NextStep::ContactOperator);
        status.state = OnboardingState::Managed;
        status.permissions.clear();
        assert_eq!(status.next_step(), NextStep::ContactOperator);
        status.state = OnboardingState::Pending;
        assert_eq!(status.next_step(), NextStep::CompleteOnboarding);
    }
    #[test]
    fn setup_guidance_handles_zero_income_and_the_full_journey() {
        let mut s = complete_status();
        assert_eq!(s.next_step(), NextStep::CreateAccount);
        s.setup.as_mut().unwrap().has_accounts = true;
        assert_eq!(s.next_step(), NextStep::CreateCycle);
        s.setup.as_mut().unwrap().selected_cycle = Some(SetupCycle {
            id: Uuid::now_v7(),
            status: SetupCycleState::Planned,
            has_envelopes: false,
            has_unallocated_income: true,
        });
        assert_eq!(s.next_step(), NextStep::AllocateIncome);
        s.setup
            .as_mut()
            .unwrap()
            .selected_cycle
            .as_mut()
            .unwrap()
            .has_unallocated_income = false;
        assert_eq!(s.next_step(), NextStep::ActivateCycle);
        s.setup
            .as_mut()
            .unwrap()
            .selected_cycle
            .as_mut()
            .unwrap()
            .status = SetupCycleState::Active;
        assert_eq!(s.next_step(), NextStep::RecordExpense);
        s.setup.as_mut().unwrap().has_expenses = true;
        assert_eq!(s.next_step(), NextStep::Ready);
        s.setup
            .as_mut()
            .unwrap()
            .selected_cycle
            .as_mut()
            .unwrap()
            .status = SetupCycleState::Closed;
        assert_eq!(s.next_step(), NextStep::CreateCycle);
    }
}
