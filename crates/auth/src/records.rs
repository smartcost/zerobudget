//! Private trusted-storage DTOs. Never implement public Deserialize for Identity.
use crate::{
    session::{unix_now, LOGIN_TTL, SESSION_TTL},
    Identity, LoginAttempt, Session, SessionError,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginRecord {
    pub version: u8,
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub expires_at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRecord {
    pub version: u8,
    pub issuer: String,
    pub subject: String,
    pub identity_expires_at: i64,
    pub access_token: Option<String>,
    pub csrf: String,
    pub expires_at: u64,
}
fn secret(value: &str, min: usize) -> bool {
    (min..=128).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, SessionError> {
    if value.len() > 16_384 {
        return Err(SessionError::Invalid);
    }
    serde_json::from_str(value).map_err(|_| SessionError::Invalid)
}
impl LoginRecord {
    pub fn decode(value: &str) -> Result<LoginAttempt, SessionError> {
        let record: Self = decode(value)?;
        let now = unix_now();
        if record.version != 1
            || record.expires_at <= now
            || record.expires_at > now.saturating_add(LOGIN_TTL)
            || !secret(&record.state, 22)
            || !secret(&record.nonce, 22)
            || !secret(&record.verifier, 43)
        {
            return Err(SessionError::Invalid);
        }
        Ok(LoginAttempt::from_record(record))
    }
}
impl SessionRecord {
    pub fn from_session(session: &Session) -> Self {
        Self {
            version: 2,
            issuer: session.identity.issuer().into(),
            subject: session.identity.subject().into(),
            identity_expires_at: session.identity.expires_at(),
            access_token: session
                .identity
                .access_token
                .as_ref()
                .map(|t| t.secret().clone()),
            csrf: session.csrf.clone(),
            expires_at: session.expires_at,
        }
    }
    pub fn decode(value: &str) -> Result<Session, SessionError> {
        let record: Self = decode(value)?;
        let now = unix_now();
        if record.version != 2
            || record.expires_at <= now
            || record.expires_at > now.saturating_add(SESSION_TTL)
            || record.identity_expires_at <= 0
            || record.expires_at > record.identity_expires_at as u64
            || !secret(&record.csrf, 22)
        {
            return Err(SessionError::Invalid);
        }
        if record
            .access_token
            .as_ref()
            .is_some_and(|t| t.is_empty() || t.len() > 8192)
        {
            return Err(SessionError::Invalid);
        }
        let mut identity =
            Identity::from_stored(record.issuer, record.subject, record.identity_expires_at)?;
        identity.access_token = record.access_token.map(openidconnect::AccessToken::new);
        Ok(Session {
            identity,
            csrf: record.csrf,
            expires_at: record.expires_at,
        })
    }
}
