#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    InvalidAmount,
    InvalidAccount,
    InvalidExpense,
    InsufficientFunds,
    AmountOverflow,
    InvalidEnvelopeName,
    InvalidPeriod,
    InvalidCycleStatus,
    UnallocatedIncome,
    EnvelopeNotFound,
    EnvelopeOverbudget,
    InvalidUserId,
}

impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomainError::InvalidAccount => write!(f, "Invalid account or cash movement"),
            DomainError::InvalidExpense => write!(f, "Invalid expense description or date"),
            DomainError::InsufficientFunds => write!(f, "Insufficient balance"),
            DomainError::AmountOverflow => write!(f, "Amount overflow"),
            DomainError::InvalidEnvelopeName => write!(f, "Invalid envelope name"),
            DomainError::InvalidPeriod => write!(f, "Invalid budget period"),
            DomainError::InvalidAmount => write!(f, "Amount must be non-negative"),
            DomainError::UnallocatedIncome => write!(f, "Cycle income is not fully allocated"),
            DomainError::InvalidCycleStatus => write!(f, "Invalid cycle status"),
            DomainError::EnvelopeNotFound => write!(f, "Envelope not found"),
            DomainError::EnvelopeOverbudget => write!(f, "Envelope would exceed budget"),
            DomainError::InvalidUserId => write!(f, "Invalid user ID"),
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_error_display() {
        let err = DomainError::InvalidAmount;
        assert_eq!(err.to_string(), "Amount must be non-negative");
    }

    #[test]
    fn test_domain_error_display_invalid_cycle_status() {
        let err = DomainError::InvalidCycleStatus;
        assert_eq!(err.to_string(), "Invalid cycle status");
    }

    #[test]
    fn test_domain_error_display_envelope_not_found() {
        let err = DomainError::EnvelopeNotFound;
        assert_eq!(err.to_string(), "Envelope not found");
    }

    #[test]
    fn test_domain_error_display_envelope_overbudget() {
        let err = DomainError::EnvelopeOverbudget;
        assert_eq!(err.to_string(), "Envelope would exceed budget");
    }

    #[test]
    fn test_domain_error_display_invalid_user_id() {
        let err = DomainError::InvalidUserId;
        assert_eq!(err.to_string(), "Invalid user ID");
    }
}
