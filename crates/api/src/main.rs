use obz_api::{config::Config, router, AppState};
use obz_auth::DexAuth;
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let auth = DexAuth::discover(config.auth_config()?).await?;
    let store = config.session_store().await?;
    let database = config.database().await?;
    let users = obz_infrastructure::PostgresIdentityRepository::from_database(&database);
    users.check_schema().await?;
    let envelopes = obz_infrastructure::PostgresEnvelopeRepository::new(database.clone());
    envelopes.check_schema().await?;
    let expenses = obz_infrastructure::PostgresExpenseRepository::new(database.clone());
    expenses.check_schema().await?;
    let cycles = obz_infrastructure::PostgresCycleRepository::new(database);
    cycles.check_schema().await?;
    let state = AppState::new(
        auth,
        config.origin.clone(),
        config.secure_cookies,
        store,
        Arc::new(users),
    )
    .with_cycles(obz_cycle::CycleService::new(Arc::new(cycles)))
    .with_envelopes(obz_cycle::EnvelopeService::new(Arc::new(envelopes)))
    .with_expenses(obz_cycle::ExpenseService::new(Arc::new(expenses)));
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    println!("ZeroBudget API listening on {}", listener.local_addr()?);
    let app = router(state.clone());
    let config = Arc::new(config);
    let background_state = state.clone();
    let refresh = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        interval.tick().await;
        loop {
            interval.tick().await;
            let Ok(auth_config) = config.auth_config() else {
                continue;
            };
            match DexAuth::discover(auth_config).await {
                Ok(auth) => background_state.replace_auth(auth).await,
                Err(_) => eprintln!("Dex metadata refresh failed; retaining previous signing keys"),
            }
        }
    });
    let cleanup_state = state;
    let cleanup = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if cleanup_state.cleanup().await.is_err() {
                eprintln!("Session storage cleanup unavailable");
            }
        }
    });
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    refresh.abort();
    cleanup.abort();
    result?;
    Ok(())
}
