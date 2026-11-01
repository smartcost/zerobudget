use crate::{money::Money, DomainError};
/// Move only the unspent allocation; no cash movement is created.
pub fn reallocate(
    source: Money,
    spent: Money,
    target: Money,
    amount: Money,
) -> Result<(Money, Money), DomainError> {
    if amount == Money::zero() {
        return Err(DomainError::InvalidAmount);
    }
    let available = source.checked_sub(spent)?;
    if amount.to_cents() > available.to_cents() {
        return Err(DomainError::InsufficientFunds);
    }
    Ok((source.checked_sub(amount)?, target.checked_add(amount)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cannot_move_spent_money() {
        let m = |v| Money::from_cents(v).unwrap();
        assert_eq!(
            reallocate(m(100), m(80), m(0), m(21)),
            Err(DomainError::InsufficientFunds)
        );
        assert_eq!(
            reallocate(m(100), m(80), m(0), m(20)).unwrap(),
            (m(80), m(20))
        );
    }
}
