//! PostgreSQL adapters, connection configuration and trusted administration.
mod database;
mod postgres_cycle;
mod postgres_identity;

pub use database::{Database, DatabaseConfig, DatabaseError, MIGRATOR};
pub use postgres_cycle::PostgresCycleRepository;
pub use postgres_identity::PostgresIdentityRepository;

mod postgres_envelope;
pub use postgres_envelope::PostgresEnvelopeRepository;
