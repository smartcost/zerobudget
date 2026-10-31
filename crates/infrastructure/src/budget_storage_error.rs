use obz_cycle::CycleError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum StorageError {
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Application(#[from] CycleError),
}
pub(crate) fn sql_error(error: sqlx::Error) -> CycleError {
    let Some(db) = error.as_database_error() else {
        return CycleError::Unavailable;
    };
    match db.code().as_deref() {
        Some("23505") if db.constraint() == Some("envelopes_cycle_name_key") => {
            CycleError::DuplicateEnvelope
        }
        Some("23505") => CycleError::Conflict,
        Some("42501") => CycleError::Forbidden,
        Some("P0001") => CycleError::Overallocated,
        Some("ZB001") => CycleError::Unallocated,
        Some("ZB002") => CycleError::InvalidState,
        Some("ZB003") => CycleError::InsufficientFunds,
        Some("ZB004") => CycleError::Invalid,
        Some("ZB005") => CycleError::AccountInsufficientFunds,
        Some("ZB006") => CycleError::BalanceOverflow,
        Some("ZC001") => CycleError::CreditLimit,
        Some("ZC002") => CycleError::InvoiceClosed,
        _ => CycleError::Unavailable,
    }
}
