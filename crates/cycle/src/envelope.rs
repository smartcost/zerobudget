use crate::CycleError;
use async_trait::async_trait;
use obz_auth::{LocalUser, Permission};
use obz_domain::{
    envelope::{Envelope, EnvelopeName},
    money::Money,
};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct BudgetEnvelope {
    pub id: Uuid,
    pub cycle_id: Uuid,
    pub envelope: Envelope,
    pub spending: obz_domain::expense::Spending,
}
#[derive(Debug, Clone)]
pub struct EnvelopePatch {
    pub name: Option<EnvelopeName>,
    pub allocated: Option<Money>,
}
/// Implementations must atomically load the current allocation, validate the domain
/// invariant and persist the mutation under a per-cycle lock. Scope by both owner
/// and cycle. Missing and foreign resources must be indistinguishable.
#[async_trait]
pub trait EnvelopeRepository: Send + Sync {
    async fn create(
        &self,
        owner: Uuid,
        cycle: Uuid,
        id: Uuid,
        envelope: Envelope,
    ) -> Result<BudgetEnvelope, CycleError>;
    async fn list(
        &self,
        owner: Uuid,
        cycle: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetEnvelope>, CycleError>;
    async fn update(
        &self,
        owner: Uuid,
        cycle: Uuid,
        id: Uuid,
        patch: EnvelopePatch,
    ) -> Result<BudgetEnvelope, CycleError>;
    async fn delete(&self, owner: Uuid, cycle: Uuid, id: Uuid) -> Result<(), CycleError>;
}
#[derive(Clone)]
pub struct EnvelopeService {
    repository: Arc<dyn EnvelopeRepository>,
}
impl EnvelopeService {
    pub fn new(repository: Arc<dyn EnvelopeRepository>) -> Self {
        Self { repository }
    }
    pub async fn create(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        name: &str,
        allocated: i64,
    ) -> Result<BudgetEnvelope, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        let envelope = Envelope::new(
            EnvelopeName::new(name).map_err(|_| CycleError::Invalid)?,
            Money::from_cents(allocated).map_err(|_| CycleError::Invalid)?,
        );
        self.repository
            .create(user.id(), cycle, Uuid::now_v7(), envelope)
            .await
    }
    pub async fn list(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<BudgetEnvelope>, CycleError> {
        user.require(Permission::ReadBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if !(1..=100).contains(&limit) || offset > 100_000 {
            return Err(CycleError::Invalid);
        }
        self.repository.list(user.id(), cycle, limit, offset).await
    }
    pub async fn update(
        &self,
        user: &LocalUser,
        cycle: Uuid,
        id: Uuid,
        name: Option<&str>,
        allocated: Option<i64>,
    ) -> Result<BudgetEnvelope, CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        if name.is_none() && allocated.is_none() {
            return Err(CycleError::Invalid);
        }
        let patch = EnvelopePatch {
            name: name
                .map(EnvelopeName::new)
                .transpose()
                .map_err(|_| CycleError::Invalid)?,
            allocated: allocated
                .map(Money::from_cents)
                .transpose()
                .map_err(|_| CycleError::Invalid)?,
        };
        self.repository.update(user.id(), cycle, id, patch).await
    }
    pub async fn delete(&self, user: &LocalUser, cycle: Uuid, id: Uuid) -> Result<(), CycleError> {
        user.require(Permission::WriteBudget)
            .map_err(|_| CycleError::Forbidden)?;
        self.repository.delete(user.id(), cycle, id).await
    }
}

#[cfg(test)]
mod envelope_service_tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct FakeRepository(Mutex<Vec<(Uuid, Uuid)>>);
    #[async_trait]
    impl EnvelopeRepository for FakeRepository {
        async fn create(
            &self,
            owner: Uuid,
            cycle: Uuid,
            id: Uuid,
            envelope: Envelope,
        ) -> Result<BudgetEnvelope, CycleError> {
            self.0.lock().unwrap().push((owner, cycle));
            Ok(BudgetEnvelope {
                id,
                cycle_id: cycle,
                spending: obz_domain::expense::Spending::new(envelope.allocated(), Money::zero())
                    .unwrap(),
                envelope,
            })
        }
        async fn list(
            &self,
            owner: Uuid,
            cycle: Uuid,
            _: u32,
            _: u32,
        ) -> Result<Vec<BudgetEnvelope>, CycleError> {
            self.0.lock().unwrap().push((owner, cycle));
            Ok(Vec::new())
        }
        async fn update(
            &self,
            owner: Uuid,
            cycle: Uuid,
            _: Uuid,
            _: EnvelopePatch,
        ) -> Result<BudgetEnvelope, CycleError> {
            self.0.lock().unwrap().push((owner, cycle));
            Err(CycleError::NotFound)
        }
        async fn delete(&self, owner: Uuid, cycle: Uuid, _: Uuid) -> Result<(), CycleError> {
            self.0.lock().unwrap().push((owner, cycle));
            Ok(())
        }
    }
    #[tokio::test]
    async fn permission_and_input_validation_precede_persistence() {
        let fake = Arc::new(FakeRepository::default());
        let service = EnvelopeService::new(fake.clone());
        let cycle = Uuid::now_v7();
        let envelope = Uuid::now_v7();
        let id = Uuid::now_v7();
        let denied = LocalUser::from_persisted(id, Default::default());
        assert!(matches!(
            service.create(&denied, cycle, "A", 1).await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service.list(&denied, cycle, 20, 0).await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service
                .update(&denied, cycle, envelope, Some("A"), None)
                .await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service.delete(&denied, cycle, envelope).await,
            Err(CycleError::Forbidden)
        ));
        let writer = LocalUser::from_persisted(id, [Permission::WriteBudget].into_iter().collect());
        assert!(matches!(
            service.list(&writer, cycle, 20, 0).await,
            Err(CycleError::Forbidden)
        ));
        assert!(matches!(
            service.create(&writer, cycle, " ", 1).await,
            Err(CycleError::Invalid)
        ));
        assert!(matches!(
            service.create(&writer, cycle, "A", -1).await,
            Err(CycleError::Invalid)
        ));
        assert!(matches!(
            service.update(&writer, cycle, envelope, None, None).await,
            Err(CycleError::Invalid)
        ));
        assert!(matches!(
            service
                .update(&writer, cycle, envelope, None, Some(-1))
                .await,
            Err(CycleError::Invalid)
        ));
        let reader = LocalUser::from_persisted(id, [Permission::ReadBudget].into_iter().collect());
        for (limit, offset) in [(0, 0), (101, 0), (20, 100_001)] {
            assert!(matches!(
                service.list(&reader, cycle, limit, offset).await,
                Err(CycleError::Invalid)
            ));
        }
        assert!(matches!(
            service.delete(&reader, cycle, envelope).await,
            Err(CycleError::Forbidden)
        ));
        assert!(fake.0.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn valid_operations_pass_only_the_authenticated_owner_and_target_cycle() {
        let fake = Arc::new(FakeRepository::default());
        let service = EnvelopeService::new(fake.clone());
        let cycle = Uuid::now_v7();
        let id = Uuid::now_v7();
        let user = LocalUser::from_persisted(
            id,
            [Permission::ReadBudget, Permission::WriteBudget]
                .into_iter()
                .collect(),
        );
        let created = service.create(&user, cycle, " Casa ", 0).await.unwrap();
        assert_eq!(created.envelope.name().as_str(), "Casa");
        assert_eq!(created.cycle_id, cycle);
        assert!(service.list(&user, cycle, 20, 0).await.unwrap().is_empty());
        assert!(matches!(
            service
                .update(&user, cycle, created.id, Some("Novo"), Some(1))
                .await,
            Err(CycleError::NotFound)
        ));
        service.delete(&user, cycle, created.id).await.unwrap();
        assert_eq!(*fake.0.lock().unwrap(), vec![(id, cycle); 4]);
    }
}
