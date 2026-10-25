use crate::DomainError;

/// Non-negative integer cents. Construction and arithmetic preserve this invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Money {
    amount: i64,
}
impl Money {
    pub fn from_cents(amount: i64) -> Result<Self, DomainError> {
        if amount < 0 {
            return Err(DomainError::InvalidAmount);
        }
        Ok(Self { amount })
    }
    pub fn to_cents(self) -> i64 {
        self.amount
    }
    pub fn zero() -> Self {
        Self { amount: 0 }
    }
    /// Presentation only; never use this conversion for financial arithmetic.
    pub fn to_real(self) -> f64 {
        self.amount as f64 / 100.0
    }
    pub fn checked_add(self, other: Self) -> Result<Self, DomainError> {
        Self::from_cents(
            self.amount
                .checked_add(other.amount)
                .ok_or(DomainError::AmountOverflow)?,
        )
    }
    pub fn checked_sub(self, other: Self) -> Result<Self, DomainError> {
        Self::from_cents(
            self.amount
                .checked_sub(other.amount)
                .ok_or(DomainError::AmountOverflow)?,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn construction_and_checked_arithmetic() {
        let a = Money::from_cents(1000).unwrap();
        let b = Money::from_cents(500).unwrap();
        assert_eq!(a.checked_add(b).unwrap().to_cents(), 1500);
        assert_eq!(a.checked_sub(b).unwrap().to_cents(), 500);
        assert_eq!(a.checked_sub(a).unwrap(), Money::zero());
        assert_eq!(b.checked_sub(a), Err(DomainError::InvalidAmount));
        assert_eq!(Money::from_cents(-1), Err(DomainError::InvalidAmount));
        assert_eq!(
            Money::from_cents(i64::MAX)
                .unwrap()
                .checked_add(Money::from_cents(1).unwrap()),
            Err(DomainError::AmountOverflow)
        );
        assert_eq!(a.to_real(), 10.0);
    }
}
