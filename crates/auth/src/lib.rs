#![doc = include_str!("../README.md")]
//! Authentication through OIDC OIDC and deny-by-default application authorization.
//! This crate does not interpret OIDC access tokens as API JWTs.
mod authorization;
mod identity;
mod oidc;
mod records;
mod redis_store;
mod session;

pub use authorization::{Permission, Policy};
pub use oidc::{AuthConfig, AuthError, Identity, LoginAttempt, OidcAuth};

pub use session::{MemorySessionStore, Session, SessionError, SessionStore};

pub use redis_store::{RedisSessionStore, RedisStoreConfig};

#[cfg(test)]
mod storage_contract;

pub use identity::{IdentityError, IdentityRepository, LocalUser, MemoryIdentityRepository};
#[cfg(test)]
mod identity_tests;
