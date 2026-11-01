//! Local users and current application authorization, independent of HTTP.
use crate::{session::unix_now, Identity, Permission};
use async_trait::async_trait;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("application identity storage unavailable or schema not prepared")]
    Unavailable,
    #[error("invalid application identity configuration")]
    Configuration,
    #[error("local user not found; sign in again")]
    NotFound,
    #[error("user account disabled")]
    Disabled,
    #[error("permission denied")]
    Forbidden,
    #[error("invalid or expired identity")]
    Invalid,
}
#[derive(Debug, Clone)]
pub struct LocalUser {
    pub(crate) id: Uuid,
    pub(crate) permissions: BTreeSet<Permission>,
}
impl LocalUser {
    /// Reconstitute an account from trusted repository data, never request input.
    /// IdentityRepository implementations must validate the identity, account status
    /// and current grants before returning this value.
    pub fn from_persisted(id: Uuid, permissions: BTreeSet<Permission>) -> Self {
        Self { id, permissions }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn permissions(&self) -> &BTreeSet<Permission> {
        &self.permissions
    }
    pub fn require(&self, permission: Permission) -> Result<(), IdentityError> {
        if self.permissions.contains(&permission) {
            Ok(())
        } else {
            Err(IdentityError::Forbidden)
        }
    }
    /// Obtain the owner from trusted persistence, never a request's owner field.
    pub fn require_owner(&self, permission: Permission, owner: Uuid) -> Result<(), IdentityError> {
        self.require(permission)?;
        if self.id == owner {
            Ok(())
        } else {
            Err(IdentityError::Forbidden)
        }
    }
}

pub(crate) fn validate_identity(identity: &Identity) -> Result<(), IdentityError> {
    if identity.issuer().is_empty()
        || identity.issuer().len() > 2048
        || identity.subject().is_empty()
        || identity.subject().len() > 255
        || identity.expires_at() <= 0
        || identity.expires_at() as u64 <= unix_now()
    {
        return Err(IdentityError::Invalid);
    }
    Ok(())
}
#[async_trait]
pub trait IdentityRepository: Send + Sync {
    /// Called only after successful OIDC verification; creates no grants.
    async fn register_login(&self, identity: &Identity) -> Result<LocalUser, IdentityError>;
    /// Read current account state and permissions, not a snapshot in Redis.
    async fn find(&self, identity: &Identity) -> Result<LocalUser, IdentityError>;
}
#[derive(Clone)]
struct MemoryUser {
    user: LocalUser,
    enabled: bool,
}
/// Tests and explicit local development only. Never the automatic fallback.
#[derive(Default, Clone)]
pub struct MemoryIdentityRepository {
    users: Arc<RwLock<BTreeMap<(String, String), MemoryUser>>>,
}
impl MemoryIdentityRepository {
    pub async fn grant(&self, id: Uuid, permission: Permission) -> Result<(), IdentityError> {
        let mut users = self.users.write().await;
        let entry = users
            .values_mut()
            .find(|entry| entry.user.id == id)
            .ok_or(IdentityError::NotFound)?;
        entry.user.permissions.insert(permission);
        Ok(())
    }
    pub async fn revoke(&self, id: Uuid, permission: Permission) -> Result<(), IdentityError> {
        let mut users = self.users.write().await;
        let entry = users
            .values_mut()
            .find(|entry| entry.user.id == id)
            .ok_or(IdentityError::NotFound)?;
        entry.user.permissions.remove(&permission);
        Ok(())
    }
    pub async fn set_enabled(&self, id: Uuid, enabled: bool) -> Result<(), IdentityError> {
        let mut users = self.users.write().await;
        let entry = users
            .values_mut()
            .find(|entry| entry.user.id == id)
            .ok_or(IdentityError::NotFound)?;
        entry.enabled = enabled;
        Ok(())
    }
}
#[async_trait]
impl IdentityRepository for MemoryIdentityRepository {
    async fn register_login(&self, identity: &Identity) -> Result<LocalUser, IdentityError> {
        validate_identity(identity)?;
        let mut users = self.users.write().await;
        let entry = users
            .entry((identity.issuer().into(), identity.subject().into()))
            .or_insert_with(|| MemoryUser {
                user: LocalUser {
                    id: Uuid::now_v7(),
                    permissions: BTreeSet::new(),
                },
                enabled: true,
            });
        if !entry.enabled {
            return Err(IdentityError::Disabled);
        }
        Ok(entry.user.clone())
    }
    async fn find(&self, identity: &Identity) -> Result<LocalUser, IdentityError> {
        validate_identity(identity)?;
        let users = self.users.read().await;
        let entry = users
            .get(&(identity.issuer().into(), identity.subject().into()))
            .ok_or(IdentityError::NotFound)?;
        if !entry.enabled {
            return Err(IdentityError::Disabled);
        }
        Ok(entry.user.clone())
    }
}
