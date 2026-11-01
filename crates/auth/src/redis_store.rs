use crate::{
    records::{LoginRecord, SessionRecord},
    session::{random_secret, unix_now},
    Identity, LoginAttempt, Session, SessionError, SessionStore,
};
use async_trait::async_trait;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// A namespace identifies one application/issuer/client and a shared capacity
/// contract. Credentials and URLs are deliberately excluded from Debug output.
pub struct RedisStoreConfig {
    pub url: String,
    pub namespace: String,
    pub timeout: Duration,
    pub max_pending: usize,
    pub max_sessions: usize,
}
impl RedisStoreConfig {
    pub fn new(url: String, namespace: String) -> Self {
        Self {
            url,
            namespace,
            timeout: Duration::from_secs(3),
            max_pending: 1024,
            max_sessions: 4096,
        }
    }
}

pub struct RedisSessionStore {
    connection: ConnectionManager,
    prefix: String,
    timeout: Duration,
    max_pending: usize,
    max_sessions: usize,
}

// ZSET members are complete record keys (all carry the same cluster hash tag).
// Check/reserve capacity, create, then revoke the previous ID atomically. An
// unsuccessful write never invalidates the previous session. No request retry
// is performed after an ambiguous response: a consumed callback stays consumed.
const CREATE: &str = r#"
local now = tonumber(redis.call('TIME')[1])
local expires = tonumber(ARGV[2])
if expires <= now then return -2 end
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
if redis.call('EXISTS', KEYS[2]) == 1 then return -3 end
local replacing = 0
if ARGV[4] == '1' and redis.call('ZSCORE', KEYS[1], KEYS[3]) and redis.call('EXISTS', KEYS[3]) == 1 then replacing = 1 end
if redis.call('ZCARD', KEYS[1]) - replacing >= tonumber(ARGV[3]) then return 0 end
redis.call('ZADD', KEYS[1], expires, KEYS[2])
local written = redis.pcall('SET', KEYS[2], ARGV[1], 'EXAT', expires, 'NX')
if type(written) == 'table' and written.err then
    redis.call('ZREM', KEYS[1], KEYS[2])
    return written
end
if not written then
    redis.call('ZREM', KEYS[1], KEYS[2])
    return -3
end
if ARGV[4] == '1' then
    redis.call('DEL', KEYS[3])
    redis.call('ZREM', KEYS[1], KEYS[3])
end
return 1
"#;
const CONSUME: &str = r#"
local value = redis.call('GETDEL', KEYS[1])
redis.call('ZREM', KEYS[2], KEYS[1])
return value
"#;
const REVOKE: &str = r#"
if redis.call('GET', KEYS[1]) ~= ARGV[1] then return 0 end
redis.call('DEL', KEYS[1])
redis.call('ZREM', KEYS[2], KEYS[1])
return 1
"#;
const CLEANUP: &str = r#"
local now = tonumber(redis.call('TIME')[1])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
return 1
"#;

fn redis_error(error: redis::RedisError) -> SessionError {
    if error.code() == Some("OOM") {
        SessionError::Capacity
    } else {
        SessionError::Unavailable
    }
}
impl RedisSessionStore {
    pub async fn connect(config: RedisStoreConfig) -> Result<Self, SessionError> {
        if config.namespace.is_empty()
            || config.namespace.len() > 80
            || !config
                .namespace
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || config.timeout.is_zero()
            || config.timeout > Duration::from_secs(30)
            || config.max_pending == 0
            || config.max_sessions == 0
            || config.max_pending > 1_000_000
            || config.max_sessions > 1_000_000
        {
            return Err(SessionError::Configuration);
        }
        let client =
            redis::Client::open(config.url.as_str()).map_err(|_| SessionError::Configuration)?;
        let manager_config = ConnectionManagerConfig::new()
            .set_connection_timeout(config.timeout)
            .set_response_timeout(config.timeout)
            .set_number_of_retries(1);
        let connection = tokio::time::timeout(
            config.timeout,
            ConnectionManager::new_with_config(client, manager_config),
        )
        .await
        .map_err(|_| SessionError::Unavailable)?
        .map_err(redis_error)?;
        let store = Self {
            connection,
            prefix: format!("zb:{{{}}}", config.namespace),
            timeout: config.timeout,
            max_pending: config.max_pending,
            max_sessions: config.max_sessions,
        };
        // Refuse different capacity policies on replicas sharing a namespace.
        let expected = format!("1:{}:{}", store.max_pending, store.max_sessions);
        let config_key = format!("{}:config", store.prefix);
        let script = redis::Script::new(
            "redis.call('SET', KEYS[1], ARGV[1], 'NX'); return redis.call('GET', KEYS[1])",
        );
        let mut invocation = script.prepare_invoke();
        invocation.key(config_key).arg(&expected);
        let actual: String = store.invoke(invocation).await?;
        if actual != expected {
            return Err(SessionError::Configuration);
        }
        Ok(store)
    }
    fn index(&self, kind: &str) -> String {
        format!("{}:{kind}-index", self.prefix)
    }
    fn key(&self, kind: &str, id: &str) -> String {
        format!("{}:{kind}:{:x}", self.prefix, Sha256::digest(id.as_bytes()))
    }
    async fn invoke<T: redis::FromRedisValue>(
        &self,
        invocation: redis::ScriptInvocation<'_>,
    ) -> Result<T, SessionError> {
        let mut connection = self.connection.clone();
        tokio::time::timeout(self.timeout, invocation.invoke_async(&mut connection))
            .await
            .map_err(|_| SessionError::Unavailable)?
            .map_err(redis_error)
    }
    async fn create(
        &self,
        kind: &str,
        payload: String,
        expires_at: u64,
        limit: usize,
        previous: Option<&str>,
    ) -> Result<String, SessionError> {
        if payload.len() > 16_384 {
            return Err(SessionError::Invalid);
        }
        let id = random_secret();
        let script = redis::Script::new(CREATE);
        let mut invocation = script.prepare_invoke();
        invocation
            .key(self.index(kind))
            .key(self.key(kind, &id))
            .key(self.key(kind, previous.unwrap_or_default()))
            .arg(payload)
            .arg(expires_at)
            .arg(limit)
            .arg(if previous.is_some() { "1" } else { "0" });
        match self.invoke::<i64>(invocation).await? {
            1 => Ok(id),
            0 => Err(SessionError::Capacity),
            -2 => Err(SessionError::Invalid),
            _ => Err(SessionError::Unavailable),
        }
    }
    async fn raw_session(&self, id: &str) -> Result<(String, Session), SessionError> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = tokio::time::timeout(
            self.timeout,
            redis::cmd("GET")
                .arg(self.key("session", id))
                .query_async(&mut connection),
        )
        .await
        .map_err(|_| SessionError::Unavailable)?
        .map_err(redis_error)?;
        let raw = raw.ok_or(SessionError::Invalid)?;
        let session = SessionRecord::decode(&raw)?;
        Ok((raw, session))
    }
}
#[async_trait]
impl SessionStore for RedisSessionStore {
    async fn probe(&self) -> Result<(), SessionError> {
        let mut connection = self.connection.clone();
        let pong: String = tokio::time::timeout(
            self.timeout,
            redis::cmd("PING").query_async(&mut connection),
        )
        .await
        .map_err(|_| SessionError::Unavailable)?
        .map_err(|_| SessionError::Unavailable)?;
        if pong == "PONG" {
            Ok(())
        } else {
            Err(SessionError::Unavailable)
        }
    }
    async fn start_login(
        &self,
        attempt: LoginAttempt,
        previous: Option<&str>,
    ) -> Result<String, SessionError> {
        let record = attempt.into_record();
        let expires_at = record.expires_at;
        let payload = serde_json::to_string(&record).map_err(|_| SessionError::Invalid)?;
        self.create("login", payload, expires_at, self.max_pending, previous)
            .await
    }
    async fn take_login(&self, id: &str) -> Result<LoginAttempt, SessionError> {
        let script = redis::Script::new(CONSUME);
        let mut invocation = script.prepare_invoke();
        invocation
            .key(self.key("login", id))
            .key(self.index("login"));
        let raw: Option<String> = self.invoke(invocation).await?;
        LoginRecord::decode(&raw.ok_or(SessionError::Invalid)?)
    }
    async fn authenticate(
        &self,
        identity: Identity,
        previous: Option<&str>,
    ) -> Result<(String, u64), SessionError> {
        let session = Session::new(identity)?;
        let expires_at = session.expires_at;
        let payload = serde_json::to_string(&SessionRecord::from_session(&session))
            .map_err(|_| SessionError::Invalid)?;
        let id = self
            .create("session", payload, expires_at, self.max_sessions, previous)
            .await?;
        Ok((id, expires_at.saturating_sub(unix_now())))
    }
    async fn get(&self, id: &str) -> Result<Session, SessionError> {
        Ok(self.raw_session(id).await?.1)
    }
    async fn logout(&self, id: &str, csrf: &str) -> Result<(), SessionError> {
        let (raw, session) = self.raw_session(id).await?;
        session.verify_csrf(csrf)?;
        let script = redis::Script::new(REVOKE);
        let mut invocation = script.prepare_invoke();
        invocation
            .key(self.key("session", id))
            .key(self.index("session"))
            .arg(raw);
        if self.invoke::<i64>(invocation).await? == 1 {
            Ok(())
        } else {
            Err(SessionError::Invalid)
        }
    }
    async fn cleanup(&self) -> Result<(), SessionError> {
        let script = redis::Script::new(CLEANUP);
        let mut invocation = script.prepare_invoke();
        invocation
            .key(self.index("login"))
            .key(self.index("session"));
        self.invoke::<i64>(invocation).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::AsyncCommands;
    use std::sync::Arc;

    fn config(namespace: &str) -> RedisStoreConfig {
        let url = std::env::var("AUTH_TEST_REDIS_URL")
            .expect("set AUTH_TEST_REDIS_URL to a real test Redis");
        let mut config = RedisStoreConfig::new(url, namespace.into());
        config.max_pending = 2;
        config.max_sessions = 2;
        config
    }
    fn namespace() -> String {
        format!("test-{}", random_secret())
    }
    async fn erase(store: &RedisSessionStore) {
        // Delete only this test's unguessable namespace; never FLUSHDB.
        let mut connection = store.connection.clone();
        let keys: Vec<String> = connection
            .keys(format!("{}:*", store.prefix))
            .await
            .unwrap();
        if !keys.is_empty() {
            let _: usize = connection.del(keys).await.unwrap();
        }
    }
    fn identity() -> Identity {
        Identity::for_test("https://dex.example", "alice")
    }

    #[tokio::test]
    #[ignore = "requires AUTH_TEST_REDIS_URL and a real Redis"]
    async fn redis_obeys_contract_across_independent_connections() {
        let namespace = namespace();
        let a = Arc::new(
            RedisSessionStore::connect(config(&namespace))
                .await
                .unwrap(),
        );
        let b = Arc::new(
            RedisSessionStore::connect(config(&namespace))
                .await
                .unwrap(),
        );
        crate::storage_contract::contract(a.clone(), b).await;
        erase(&a).await;
    }
    #[tokio::test]
    #[ignore = "requires AUTH_TEST_REDIS_URL and a real Redis"]
    async fn reconstruction_preserves_pending_and_session_state_with_hashed_keys() {
        let namespace = namespace();
        let a = RedisSessionStore::connect(config(&namespace))
            .await
            .unwrap();
        let pending = a.start_login(LoginAttempt::for_test(), None).await.unwrap();
        let (session, _) = a.authenticate(identity(), None).await.unwrap();
        let mut connection = a.connection.clone();
        let keys: Vec<String> = connection.keys(format!("{}:*", a.prefix)).await.unwrap();
        assert!(keys
            .iter()
            .all(|key| !key.contains(&pending) && !key.contains(&session)));
        let ttl: i64 = connection.ttl(a.key("login", &pending)).await.unwrap();
        assert!((1..=300).contains(&ttl));
        let ttl: i64 = connection.ttl(a.key("session", &session)).await.unwrap();
        assert!((1..=1800).contains(&ttl));
        drop(a);
        let b = RedisSessionStore::connect(config(&namespace))
            .await
            .unwrap();
        b.take_login(&pending).await.unwrap();
        let csrf = b.get(&session).await.unwrap().csrf_token().to_owned();
        b.logout(&session, &csrf).await.unwrap();
        erase(&b).await;
    }
    #[tokio::test]
    #[ignore = "requires AUTH_TEST_REDIS_URL and a real Redis"]
    async fn concurrent_creation_enforces_shared_capacity() {
        let namespace = namespace();
        let a = Arc::new(
            RedisSessionStore::connect(config(&namespace))
                .await
                .unwrap(),
        );
        let b = Arc::new(
            RedisSessionStore::connect(config(&namespace))
                .await
                .unwrap(),
        );
        let mut jobs = Vec::new();
        for i in 0..16 {
            let store = if i % 2 == 0 { a.clone() } else { b.clone() };
            jobs.push(tokio::spawn(async move {
                store.authenticate(identity(), None).await
            }));
        }
        let mut created = 0;
        for job in jobs {
            match job.await.unwrap() {
                Ok(_) => created += 1,
                Err(SessionError::Capacity) => {}
                _ => panic!("unexpected storage error"),
            }
        }
        assert_eq!(created, 2);
        let mut other_policy = config(&namespace);
        other_policy.max_sessions = 3;
        assert!(matches!(
            RedisSessionStore::connect(other_policy).await,
            Err(SessionError::Configuration)
        ));
        erase(&a).await;
    }
    #[tokio::test]
    #[ignore = "requires AUTH_TEST_REDIS_URL and a real Redis"]
    async fn corrupt_version_and_expiration_are_rejected_even_when_redis_keeps_record() {
        let store = RedisSessionStore::connect(config(&namespace()))
            .await
            .unwrap();
        let (id, _) = store.authenticate(identity(), None).await.unwrap();
        let key = store.key("session", &id);
        let mut connection = store.connection.clone();
        let original: String = connection.get(&key).await.unwrap();
        for (field, value) in [
            ("version", serde_json::json!(99)),
            ("expires_at", serde_json::json!(1)),
            ("identity_expires_at", serde_json::json!(1)),
            ("csrf", serde_json::json!("invalid")),
        ] {
            let mut record: serde_json::Value = serde_json::from_str(&original).unwrap();
            record[field] = value;
            let _: () = connection
                .set_ex(&key, record.to_string(), 60)
                .await
                .unwrap();
            assert!(matches!(store.get(&id).await, Err(SessionError::Invalid)));
        }
        let pending = store
            .start_login(LoginAttempt::for_test(), None)
            .await
            .unwrap();
        let _: () = connection
            .set_ex(store.key("login", &pending), "{broken-json", 60)
            .await
            .unwrap();
        assert!(matches!(
            store.take_login(&pending).await,
            Err(SessionError::Invalid)
        ));
        assert!(matches!(
            store.take_login(&pending).await,
            Err(SessionError::Invalid)
        ));
        erase(&store).await;
    }
    #[tokio::test]
    async fn unavailable_redis_is_not_a_missing_session_or_memory_fallback() {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);
        let mut config = RedisStoreConfig::new(format!("redis://{address}"), namespace());
        config.timeout = Duration::from_millis(100);
        assert!(matches!(
            RedisSessionStore::connect(config).await,
            Err(SessionError::Unavailable)
        ));
    }
    #[tokio::test]
    #[ignore = "requires AUTH_TEST_REDIS_URL and a real Redis"]
    async fn expired_records_release_both_shared_capacity_indexes() {
        let store = RedisSessionStore::connect(config(&namespace()))
            .await
            .unwrap();
        for _ in 0..2 {
            let mut attempt = LoginAttempt::for_test();
            attempt.expires_at = unix_now() + 2;
            store.start_login(attempt, None).await.unwrap();
            let mut identity = identity();
            identity.set_test_expiration((unix_now() + 2) as i64);
            store.authenticate(identity, None).await.unwrap();
        }
        assert!(matches!(
            store.start_login(LoginAttempt::for_test(), None).await,
            Err(SessionError::Capacity)
        ));
        assert!(matches!(
            store.authenticate(identity(), None).await,
            Err(SessionError::Capacity)
        ));
        tokio::time::sleep(Duration::from_secs(2)).await;
        for _ in 0..2 {
            store
                .start_login(LoginAttempt::for_test(), None)
                .await
                .unwrap();
            store.authenticate(identity(), None).await.unwrap();
        }
        assert!(matches!(
            store.start_login(LoginAttempt::for_test(), None).await,
            Err(SessionError::Capacity)
        ));
        assert!(matches!(
            store.authenticate(identity(), None).await,
            Err(SessionError::Capacity)
        ));
        erase(&store).await;
    }
}
