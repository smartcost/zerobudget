//! Server-side storage contract. Backends never expose serializable identities.
use crate::{Identity, LoginAttempt};
use async_trait::async_trait;
use openidconnect::CsrfToken;
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;

pub(crate) const LOGIN_TTL: u64 = 900;
pub(crate) const SESSION_TTL: u64 = 1800;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("session storage capacity reached")]
    Capacity,
    #[error("invalid or expired session")]
    Invalid,
    #[error("invalid CSRF token")]
    Csrf,
    #[error("session storage unavailable")]
    Unavailable,
    #[error("invalid session storage configuration")]
    Configuration,
}

#[derive(Clone)]
pub struct Session {
    pub(crate) identity: Identity,
    pub(crate) csrf: String,
    pub(crate) expires_at: u64,
}
impl Session {
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn csrf_token(&self) -> &str {
        &self.csrf
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub fn verify_csrf(&self, token: &str) -> Result<(), SessionError> {
        if bool::from(self.csrf.as_bytes().ct_eq(token.as_bytes())) {
            Ok(())
        } else {
            Err(SessionError::Csrf)
        }
    }
    pub(crate) fn new(identity: Identity) -> Result<Self, SessionError> {
        let now = unix_now();
        let expires_at = u64::try_from(identity.expires_at())
            .map_err(|_| SessionError::Invalid)?
            .min(now.saturating_add(SESSION_TTL));
        if expires_at <= now {
            return Err(SessionError::Invalid);
        }
        Ok(Self {
            identity,
            csrf: random_secret(),
            expires_at,
        })
    }
}

/// Every backend must consume attempts once, rotate IDs, enforce absolute
/// expiration and capacity, and fail closed. No provider I/O belongs in a lock.
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn start_login(
        &self,
        attempt: LoginAttempt,
        previous: Option<&str>,
    ) -> Result<String, SessionError>;
    async fn take_login(&self, id: &str) -> Result<LoginAttempt, SessionError>;
    async fn authenticate(
        &self,
        identity: Identity,
        previous: Option<&str>,
    ) -> Result<(String, u64), SessionError>;
    async fn get(&self, id: &str) -> Result<Session, SessionError>;
    async fn logout(&self, id: &str, csrf: &str) -> Result<(), SessionError>;
    async fn cleanup(&self) -> Result<(), SessionError>;
    async fn probe(&self) -> Result<(), SessionError> {
        Err(SessionError::Unavailable)
    }
}

struct MemoryState {
    pending: HashMap<String, LoginAttempt>,
    sessions: HashMap<String, Session>,
}
impl MemoryState {
    fn cleanup(&mut self) {
        let now = unix_now();
        self.pending.retain(|_, pending| pending.expires_at > now);
        self.sessions.retain(|_, session| session.expires_at > now);
    }
}
/// Explicit single-process backend for tests and development.
pub struct MemorySessionStore {
    state: Mutex<MemoryState>,
    max_pending: usize,
    max_sessions: usize,
}
impl Default for MemorySessionStore {
    fn default() -> Self {
        Self::new(1024, 4096)
    }
}
impl MemorySessionStore {
    pub fn new(max_pending: usize, max_sessions: usize) -> Self {
        Self {
            state: Mutex::new(MemoryState {
                pending: HashMap::new(),
                sessions: HashMap::new(),
            }),
            max_pending,
            max_sessions,
        }
    }
}
#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn probe(&self) -> Result<(), SessionError> {
        Ok(())
    }
    async fn start_login(
        &self,
        attempt: LoginAttempt,
        previous: Option<&str>,
    ) -> Result<String, SessionError> {
        if attempt.expires_at <= unix_now() {
            return Err(SessionError::Invalid);
        }
        let mut state = self.state.lock().await;
        state.cleanup();
        let replacing = previous.is_some_and(|id| state.pending.contains_key(id));
        if state.pending.len() >= self.max_pending && !replacing {
            return Err(SessionError::Capacity);
        }
        let id = random_secret();
        state.pending.insert(id.clone(), attempt);
        if let Some(previous) = previous {
            state.pending.remove(previous);
        }
        Ok(id)
    }
    async fn take_login(&self, id: &str) -> Result<LoginAttempt, SessionError> {
        let mut state = self.state.lock().await;
        state.cleanup();
        state.pending.remove(id).ok_or(SessionError::Invalid)
    }
    async fn authenticate(
        &self,
        identity: Identity,
        previous: Option<&str>,
    ) -> Result<(String, u64), SessionError> {
        let session = Session::new(identity)?;
        let mut state = self.state.lock().await;
        state.cleanup();
        let replacing = previous.is_some_and(|id| state.sessions.contains_key(id));
        if state.sessions.len() >= self.max_sessions && !replacing {
            return Err(SessionError::Capacity);
        }
        let id = random_secret();
        let remaining = session.expires_at.saturating_sub(unix_now());
        if remaining == 0 {
            return Err(SessionError::Invalid);
        }
        state.sessions.insert(id.clone(), session);
        if let Some(previous) = previous {
            state.sessions.remove(previous);
        }
        Ok((id, remaining))
    }
    async fn get(&self, id: &str) -> Result<Session, SessionError> {
        let mut state = self.state.lock().await;
        state.cleanup();
        state.sessions.get(id).cloned().ok_or(SessionError::Invalid)
    }
    async fn logout(&self, id: &str, csrf: &str) -> Result<(), SessionError> {
        let mut state = self.state.lock().await;
        state.cleanup();
        state
            .sessions
            .get(id)
            .ok_or(SessionError::Invalid)?
            .verify_csrf(csrf)?;
        state.sessions.remove(id);
        Ok(())
    }
    async fn cleanup(&self) -> Result<(), SessionError> {
        self.state.lock().await.cleanup();
        Ok(())
    }
}
pub(crate) fn random_secret() -> String {
    CsrfToken::new_random().secret().to_owned()
}
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(u64::MAX, |time| time.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expiration_releases_memory_capacity() {
        let store = MemorySessionStore::new(1, 1);
        let (id, _) = store
            .authenticate(Identity::for_test("https://dex.example", "alice"), None)
            .await
            .unwrap();
        store
            .state
            .lock()
            .await
            .sessions
            .get_mut(&id)
            .unwrap()
            .expires_at = 1;
        assert!(store.get(&id).await.is_err());
        assert!(store
            .authenticate(Identity::for_test("https://dex.example", "alice"), None)
            .await
            .is_ok());
        let id = store
            .start_login(LoginAttempt::for_test(), None)
            .await
            .unwrap();
        store
            .state
            .lock()
            .await
            .pending
            .get_mut(&id)
            .unwrap()
            .expires_at = 1;
        assert!(store.take_login(&id).await.is_err());
        assert!(store
            .start_login(LoginAttempt::for_test(), None)
            .await
            .is_ok());
    }
}
