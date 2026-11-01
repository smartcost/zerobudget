use obz_api::{config::Config, router, AppState};
use obz_auth::OidcAuth;
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    let auth = OidcAuth::discover(config.auth_config()?).await?;
    let store = config.session_store().await?;
    let database = config.database().await?;
    let users = obz_infrastructure::PostgresIdentityRepository::from_database(&database);
    users.check_schema().await?;
    let envelopes = obz_infrastructure::PostgresEnvelopeRepository::new(database.clone());
    envelopes.check_schema().await?;
    let expenses = obz_infrastructure::PostgresExpenseRepository::new(database.clone());
    expenses.check_schema().await?;
    let banking = obz_infrastructure::PostgresBankingRepository::new(database.clone());
    banking.check_schema().await?;
    let credit = obz_infrastructure::PostgresCreditRepository::new(database.clone());
    credit.check_schema().await?;
    let planning = obz_infrastructure::PostgresPlanningRepository::new(database.clone());
    let finance = obz_infrastructure::PostgresFinanceRepository::new(database.clone());
    let cycles = obz_infrastructure::PostgresCycleRepository::new(database.clone());
    cycles.check_schema().await?;
    let state = AppState::new(
        auth,
        config.origin.clone(),
        config.secure_cookies,
        store,
        Arc::new(users),
    )
    .with_database(database)
    .with_planning(obz_cycle::PlanningService::new(Arc::new(planning)))
    .with_finance(obz_banking::FinanceService::new(Arc::new(finance)))
    .with_cycles(obz_cycle::CycleService::new(Arc::new(cycles)))
    .with_envelopes(obz_cycle::EnvelopeService::new(Arc::new(envelopes)))
    .with_expenses(obz_cycle::ExpenseService::new(Arc::new(expenses)))
    .with_banking(obz_banking::BankingService::new(Arc::new(banking)))
    .with_credit(obz_banking::CreditService::new(Arc::new(credit)));
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
            match OidcAuth::discover(auth_config).await {
                Ok(auth) => background_state.replace_auth(auth).await,
                Err(_) => {
                    eprintln!("OIDC metadata refresh failed; retaining previous signing keys")
                }
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
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = term.recv()=>{}}
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    refresh.abort();
    cleanup.abort();
    result?;
    Ok(())
}
