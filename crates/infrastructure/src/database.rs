use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
    ConnectOptions, PgPool,
};
use std::{str::FromStr, time::Duration};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Deliberately lacks Debug: URLs can contain credentials.
#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub allow_loopback_plaintext: bool,
    pub timeout: Duration,
    pub max_connections: u32,
}
impl DatabaseConfig {
    pub fn new(url: String, allow_loopback_plaintext: bool) -> Self {
        Self {
            url,
            allow_loopback_plaintext,
            timeout: Duration::from_secs(3),
            max_connections: 10,
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("invalid application database configuration")]
    Configuration,
    #[error("application database unavailable or migration failed")]
    Unavailable,
}
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
    timeout: Duration,
}
impl Database {
    pub async fn connect(config: DatabaseConfig) -> Result<Self, DatabaseError> {
        if config.timeout.is_zero()
            || config.timeout > Duration::from_secs(30)
            || config.max_connections == 0
            || config.max_connections > 100
        {
            return Err(DatabaseError::Configuration);
        }
        let url = url::Url::parse(&config.url).map_err(|_| DatabaseError::Configuration)?;
        if !matches!(url.scheme(), "postgres" | "postgresql")
            || url.fragment().is_some()
            || url.host_str().is_none()
            || url.password().is_none_or(str::is_empty)
        {
            return Err(DatabaseError::Configuration);
        }
        let local = config.allow_loopback_plaintext
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if url
            .query_pairs()
            .any(|(name, _)| !matches!(name.as_ref(), "sslmode" | "sslrootcert"))
        {
            return Err(DatabaseError::Configuration);
        }
        let options = PgConnectOptions::from_str(&config.url)
            .map_err(|_| DatabaseError::Configuration)?
            .ssl_mode(if local {
                PgSslMode::Disable
            } else {
                PgSslMode::VerifyFull
            })
            .disable_statement_logging();
        let pool = tokio::time::timeout(
            config.timeout,
            PgPoolOptions::new()
                .max_connections(config.max_connections)
                .acquire_timeout(config.timeout)
                .connect_with(options),
        )
        .await
        .map_err(|_| DatabaseError::Unavailable)?
        .map_err(|_| DatabaseError::Unavailable)?;
        Ok(Self {
            pool,
            timeout: config.timeout,
        })
    }
    pub async fn probe(&self) -> Result<(), DatabaseError> {
        tokio::time::timeout(self.timeout, sqlx::query("SELECT 1").execute(&self.pool))
            .await
            .map_err(|_| DatabaseError::Unavailable)?
            .map_err(|_| DatabaseError::Unavailable)?;
        Ok(())
    }
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    pub async fn migrate(&self) -> Result<(), DatabaseError> {
        tokio::time::timeout(Duration::from_secs(30), MIGRATOR.run(&self.pool))
            .await
            .map_err(|_| DatabaseError::Unavailable)?
            .map_err(|_| DatabaseError::Unavailable)
    }
}
