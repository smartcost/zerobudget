use async_trait::async_trait;
use obz_auth::{Identity, IdentityError, IdentityRepository, LocalUser, Permission};
use sqlx::{PgPool, Row};
use std::{collections::BTreeSet, time::Duration};
use uuid::Uuid;

pub use crate::DatabaseConfig;

#[derive(Clone)]
pub struct PostgresIdentityRepository {
    pool: PgPool,
    timeout: Duration,
}
const USER_SELECT: &str = "SELECT u.id, u.enabled, COALESCE(array_agg(p.permission ORDER BY p.permission) FILTER (WHERE p.permission IS NOT NULL), ARRAY[]::text[]) AS permissions FROM auth.users u LEFT JOIN auth.permissions p ON p.user_id = u.id WHERE u.issuer = $1 AND u.subject = $2 GROUP BY u.id";

impl PostgresIdentityRepository {
    pub async fn connect(config: DatabaseConfig) -> Result<Self, IdentityError> {
        let database = crate::Database::connect(config)
            .await
            .map_err(|error| match error {
                crate::DatabaseError::Configuration => IdentityError::Configuration,
                crate::DatabaseError::Unavailable => IdentityError::Unavailable,
            })?;
        Ok(Self::from_database(&database))
    }
    pub fn from_database(database: &crate::Database) -> Self {
        Self {
            pool: database.pool().clone(),
            timeout: database.timeout(),
        }
    }
    pub async fn check_schema(&self) -> Result<(), IdentityError> {
        self.bounded(async {
            sqlx::query("SELECT u.id, u.enabled, p.permission FROM auth.users u LEFT JOIN auth.permissions p ON p.user_id = u.id WHERE false").execute(&self.pool).await?;
            Ok(())
        }).await
    }
    pub async fn migrate(&self) -> Result<(), IdentityError> {
        tokio::time::timeout(Duration::from_secs(30), crate::MIGRATOR.run(&self.pool))
            .await
            .map_err(|_| IdentityError::Unavailable)?
            .map_err(|_| IdentityError::Unavailable)
    }
    async fn bounded<T>(
        &self,
        operation: impl std::future::Future<Output = Result<T, sqlx::Error>>,
    ) -> Result<T, IdentityError> {
        tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| IdentityError::Unavailable)?
            .map_err(|_| IdentityError::Unavailable)
    }
    /// Administrative provisioning only. The API never calls this method.
    pub async fn grant_runtime_role(&self, role: &str) -> Result<(), IdentityError> {
        if role.is_empty()
            || role.len() > 63
            || !role.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(IdentityError::Configuration);
        }
        self.bounded(async {
            let mut transaction = self.pool.begin().await?;
            for statement in [
                format!("GRANT USAGE ON SCHEMA auth TO \"{role}\""),
                format!("GRANT EXECUTE ON FUNCTION auth.budget_onboarding_state(uuid,text,text),auth.complete_budget_onboarding(uuid,text,text) TO \"{role}\""),
                format!("GRANT SELECT ON auth.users, auth.permissions TO \"{role}\""),
                format!("GRANT INSERT (id, issuer, subject, last_login_at), UPDATE (last_login_at) ON auth.users TO \"{role}\""),
            ] { sqlx::query(&statement).execute(&mut *transaction).await?; }
            transaction.commit().await
        }).await
    }
    pub async fn grant(&self, user_id: Uuid, permission: Permission) -> Result<(), IdentityError> {
        self.change_permission(user_id, permission, true).await
    }
    pub async fn revoke(&self, user_id: Uuid, permission: Permission) -> Result<(), IdentityError> {
        self.change_permission(user_id, permission, false).await
    }
    async fn change_permission(
        &self,
        user_id: Uuid,
        permission: Permission,
        grant: bool,
    ) -> Result<(), IdentityError> {
        // Serialize administrative changes per user, with audit in the same transaction.
        let mut transaction = tokio::time::timeout(self.timeout, self.pool.begin())
            .await
            .map_err(|_| IdentityError::Unavailable)?
            .map_err(|_| IdentityError::Unavailable)?;
        let action = if grant { "grant" } else { "revoke" };
        let result = tokio::time::timeout(self.timeout, async {
            let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM auth.users WHERE id = $1 FOR UPDATE").bind(user_id).fetch_optional(&mut *transaction).await?;
            if exists.is_none() { return Ok::<_, sqlx::Error>(false); }
            let changed = if grant {
                sqlx::query("INSERT INTO auth.permissions (user_id, permission) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(user_id).bind(permission.as_str()).execute(&mut *transaction).await?.rows_affected()
            } else {
                sqlx::query("DELETE FROM auth.permissions WHERE user_id = $1 AND permission = $2").bind(user_id).bind(permission.as_str()).execute(&mut *transaction).await?.rows_affected()
            };
            if changed > 0 {
                sqlx::query("INSERT INTO auth.audit_events (id, user_id, action, permission) VALUES ($1, $2, $3, $4)").bind(Uuid::new_v4()).bind(user_id).bind(action).bind(permission.as_str()).execute(&mut *transaction).await?;
            }
            transaction.commit().await?;
            Ok(true)
        }).await.map_err(|_| IdentityError::Unavailable)?.map_err(|_| IdentityError::Unavailable)?;
        if result {
            Ok(())
        } else {
            Err(IdentityError::NotFound)
        }
    }
    pub async fn set_enabled(&self, user_id: Uuid, enabled: bool) -> Result<(), IdentityError> {
        self.bounded(async {
            let mut transaction = self.pool.begin().await?;
            let changed = sqlx::query(
                "UPDATE auth.users SET enabled = $2 WHERE id = $1 AND enabled IS DISTINCT FROM $2",
            )
            .bind(user_id)
            .bind(enabled)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if changed > 0 {
                sqlx::query(
                    "INSERT INTO auth.audit_events (id, user_id, action) VALUES ($1, $2, $3)",
                )
                .bind(Uuid::new_v4())
                .bind(user_id)
                .bind(if enabled { "enable" } else { "disable" })
                .execute(&mut *transaction)
                .await?;
            }
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth.users WHERE id = $1)")
                    .bind(user_id)
                    .fetch_one(&mut *transaction)
                    .await?;
            transaction.commit().await?;
            Ok(exists)
        })
        .await
        .and_then(|exists| {
            if exists {
                Ok(())
            } else {
                Err(IdentityError::NotFound)
            }
        })
    }
}
#[async_trait]
impl IdentityRepository for PostgresIdentityRepository {
    async fn register_login(&self, identity: &Identity) -> Result<LocalUser, IdentityError> {
        identity.validate_current()?;
        self.bounded(async {
            sqlx::query("INSERT INTO auth.users (id, issuer, subject, last_login_at) VALUES ($1, $2, $3, now()) ON CONFLICT (issuer, subject) DO UPDATE SET last_login_at = EXCLUDED.last_login_at WHERE auth.users.enabled")
                .bind(Uuid::new_v4()).bind(identity.issuer()).bind(identity.subject()).execute(&self.pool).await?;
            Ok(())
        }).await?;
        self.find(identity).await
    }
    async fn find(&self, identity: &Identity) -> Result<LocalUser, IdentityError> {
        identity.validate_current()?;
        let row = self
            .bounded(
                sqlx::query(USER_SELECT)
                    .bind(identity.issuer())
                    .bind(identity.subject())
                    .fetch_optional(&self.pool),
            )
            .await?
            .ok_or(IdentityError::NotFound)?;
        let enabled: bool = row
            .try_get("enabled")
            .map_err(|_| IdentityError::Unavailable)?;
        if !enabled {
            return Err(IdentityError::Disabled);
        }
        let permissions: Vec<String> = row
            .try_get("permissions")
            .map_err(|_| IdentityError::Unavailable)?;
        let permissions: BTreeSet<Permission> = permissions
            .into_iter()
            .map(|permission| permission.parse().map_err(|_| IdentityError::Unavailable))
            .collect::<Result<_, _>>()?;
        Ok(LocalUser::from_persisted(
            row.try_get("id").map_err(|_| IdentityError::Unavailable)?,
            permissions,
        ))
    }
}
