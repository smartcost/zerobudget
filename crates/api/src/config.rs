use obz_auth::{
    AuthConfig, AuthError, IdentityError, IdentityRepository, MemorySessionStore,
    RedisSessionStore, RedisStoreConfig, SessionError, SessionStore,
};
use obz_infrastructure::{DatabaseConfig, PostgresIdentityRepository};
use std::{env, net::SocketAddr};
use std::{sync::Arc, time::Duration};
use url::Url;

pub struct Config {
    pub bind: SocketAddr,
    pub origin: String,
    pub secure_cookies: bool,
    issuer: String,
    client_id: String,
    client_secret: String,
    callback: String,
    local_http: bool,
    storage: StorageConfig,
    database: DatabaseConfig,
}
enum StorageConfig {
    Memory {
        max_pending: usize,
        max_sessions: usize,
    },
    Redis(RedisStoreConfig),
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let origin = env::var("API_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3000".into());
        let url = Url::parse(&origin)?;
        if url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("API_PUBLIC_URL must contain only scheme, host and optional port".into());
        }
        let local_http = match env::var("AUTH_ALLOW_LOCAL_HTTP").as_deref() {
            Ok("true") => true,
            Ok("false") | Err(_) => false,
            _ => return Err("AUTH_ALLOW_LOCAL_HTTP must be true or false".into()),
        };
        let positive =
            |name: &str, default: usize, max: usize| -> Result<usize, Box<dyn std::error::Error>> {
                let value = match env::var(name) {
                    Ok(value) => value
                        .parse::<usize>()
                        .map_err(|_| "invalid session storage limit or timeout")?,
                    Err(_) => default,
                };
                if value == 0 || value > max {
                    return Err("session storage limit or timeout out of range".into());
                }
                Ok(value)
            };
        let max_pending = positive("AUTH_MAX_PENDING", 1024, 1_000_000)?;
        let max_sessions = positive("AUTH_MAX_SESSIONS", 4096, 1_000_000)?;
        let storage = match env::var("AUTH_SESSION_BACKEND").as_deref() {
            Ok("memory") if local_http => StorageConfig::Memory {
                max_pending,
                max_sessions,
            },
            Ok("memory") => {
                return Err(
                    "memory session storage requires explicit local development mode".into(),
                )
            }
            Ok("redis") => {
                let redis_url = env::var("AUTH_REDIS_URL")
                    .map_err(|_| "AUTH_REDIS_URL is required for Redis sessions")?;
                let parsed = Url::parse(&redis_url).map_err(|_| "invalid Redis URL")?;
                let loopback =
                    matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
                if (parsed.scheme() != "rediss"
                    && !(parsed.scheme() == "redis" && local_http && loopback))
                    || parsed.fragment().is_some()
                    || parsed.query().is_some()
                    || (parsed.scheme() == "rediss" && parsed.password().is_none_or(str::is_empty))
                {
                    return Err(
                        "Redis requires authenticated TLS, except explicit loopback development"
                            .into(),
                    );
                }
                let namespace = env::var("AUTH_REDIS_NAMESPACE")
                    .map_err(|_| "AUTH_REDIS_NAMESPACE is required")?;
                let mut redis = RedisStoreConfig::new(redis_url, namespace);
                redis.max_pending = max_pending;
                redis.max_sessions = max_sessions;
                redis.timeout =
                    Duration::from_millis(positive("AUTH_REDIS_TIMEOUT_MS", 3000, 30_000)? as u64);
                StorageConfig::Redis(redis)
            }
            _ => return Err("AUTH_SESSION_BACKEND must explicitly be redis or memory".into()),
        };
        let mut database = DatabaseConfig::new(
            env::var("DATABASE_URL")
                .map_err(|_| "DATABASE_URL is required for application identities")?,
            local_http,
        );
        database.timeout =
            Duration::from_millis(positive("DATABASE_TIMEOUT_MS", 3000, 30_000)? as u64);
        database.max_connections = positive("DATABASE_MAX_CONNECTIONS", 10, 100)? as u32;
        let config = Self {
            bind: env::var("API_BIND")
                .unwrap_or_else(|_| "127.0.0.1:3000".into())
                .parse()?,
            origin: url.origin().ascii_serialization(),
            secure_cookies: url.scheme() == "https",
            issuer: env::var("OIDC_ISSUER")
                .unwrap_or_else(|_| "http://localhost:8080/realms/zerobudget".into()),
            client_id: env::var("OIDC_CLIENT_ID").unwrap_or_else(|_| "zerobudget".into()),
            client_secret: env::var("OIDC_CLIENT_SECRET")
                .map_err(|_| "OIDC_CLIENT_SECRET is required")?,
            callback: url.join("/api/auth/callback")?.to_string(),
            local_http,
            storage,
            database,
        };
        config.auth_config()?;
        Ok(config)
    }
    pub async fn session_store(&self) -> Result<Arc<dyn SessionStore>, SessionError> {
        match &self.storage {
            StorageConfig::Memory {
                max_pending,
                max_sessions,
            } => Ok(Arc::new(MemorySessionStore::new(
                *max_pending,
                *max_sessions,
            ))),
            StorageConfig::Redis(config) => {
                let mut copy = RedisStoreConfig::new(config.url.clone(), config.namespace.clone());
                copy.timeout = config.timeout;
                copy.max_pending = config.max_pending;
                copy.max_sessions = config.max_sessions;
                Ok(Arc::new(RedisSessionStore::connect(copy).await?))
            }
        }
    }
    pub async fn database(
        &self,
    ) -> Result<obz_infrastructure::Database, obz_infrastructure::DatabaseError> {
        obz_infrastructure::Database::connect(self.database.clone()).await
    }
    pub async fn identity_repository(&self) -> Result<Arc<dyn IdentityRepository>, IdentityError> {
        let mut config = DatabaseConfig::new(
            self.database.url.clone(),
            self.database.allow_loopback_plaintext,
        );
        config.timeout = self.database.timeout;
        config.max_connections = self.database.max_connections;
        let repository = PostgresIdentityRepository::connect(config).await?;
        repository.check_schema().await?;
        Ok(Arc::new(repository))
    }
    pub fn auth_config(&self) -> Result<AuthConfig, AuthError> {
        Ok(AuthConfig::new(
            self.issuer.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
            self.callback.clone(),
            self.local_http,
        )?
        .keycloak())
    }
}
