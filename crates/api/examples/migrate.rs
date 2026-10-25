//! Explicit owner-only migration tool. Never called by the HTTP server.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_ADMIN_URL")?;
    let local = std::env::var("AUTH_ALLOW_LOCAL_HTTP").as_deref() == Ok("true");
    let database =
        obz_infrastructure::Database::connect(obz_infrastructure::DatabaseConfig::new(url, local))
            .await?;
    database.migrate().await?;
    if let Ok(role) = std::env::var("DATABASE_RUNTIME_ROLE") {
        obz_infrastructure::PostgresIdentityRepository::from_database(&database)
            .grant_runtime_role(&role)
            .await?;
        obz_infrastructure::PostgresEnvelopeRepository::new(database.clone())
            .grant_runtime_role(&role)
            .await?;
        obz_infrastructure::PostgresCycleRepository::new(database)
            .grant_runtime_role(&role)
            .await?;
    }
    println!("Application migrations completed.");
    Ok(())
}
