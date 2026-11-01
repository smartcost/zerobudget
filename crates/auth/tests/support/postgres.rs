//! Disposable databases only. Never migrate or drop the configured base database.
use sqlx::PgPool;

pub struct TestDatabase {
    admin: PgPool,
    name: String,
    pub owner_url: String,
    pub runtime_url: String,
    pub runtime_role: String,
}
impl TestDatabase {
    pub async fn create() -> Self {
        let owner_url = std::env::var("AUTH_TEST_DATABASE_URL")
            .expect("set AUTH_TEST_DATABASE_URL to the test PostgreSQL owner URL");
        let runtime_url = std::env::var("AUTH_TEST_RUNTIME_DATABASE_URL")
            .expect("set AUTH_TEST_RUNTIME_DATABASE_URL to the test runtime role URL");
        let admin = PgPool::connect(&owner_url).await.unwrap();
        let name = format!("zb_auth_test_{}", uuid::Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .unwrap();
        let mut owner = url::Url::parse(&owner_url).unwrap();
        owner.set_path(&name);
        let mut runtime = url::Url::parse(&runtime_url).unwrap();
        let runtime_role = runtime.username().to_owned();
        runtime.set_path(&name);
        Self {
            admin,
            name,
            owner_url: owner.to_string(),
            runtime_url: runtime.to_string(),
            runtime_role,
        }
    }
    pub async fn destroy(self) {
        assert!(self.name.starts_with("zb_auth_test_"));
        // The name is generated internally from UUID hex, never supplied by env.
        sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
