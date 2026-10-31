//! Domain crate - Core business logic
//! Zero external dependencies
pub mod errors;
pub mod money;

pub use errors::DomainError;
pub mod cycle;

pub mod envelope;

pub mod expense;

pub mod banking;
