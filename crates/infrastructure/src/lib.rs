//! PostgreSQL adapters, connection configuration and trusted administration.
mod budget_storage_error;
mod database;
mod postgres_cycle;
mod postgres_identity;

pub use database::{Database, DatabaseConfig, DatabaseError, MIGRATOR};
pub use postgres_cycle::PostgresCycleRepository;
pub use postgres_identity::PostgresIdentityRepository;

mod postgres_envelope;
pub use postgres_envelope::PostgresEnvelopeRepository;

mod postgres_expense;
pub use postgres_expense::PostgresExpenseRepository;

mod postgres_banking;
pub use postgres_banking::PostgresBankingRepository;

mod postgres_credit;
pub use postgres_credit::PostgresCreditRepository;
