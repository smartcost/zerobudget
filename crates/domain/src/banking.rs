//! Actual money is independent of its budget allocation.
use crate::{money::Money, DomainError};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
    Bank,
    Cash,
}
impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bank => "bank",
            Self::Cash => "cash",
        }
    }
}
impl TryFrom<&str> for AccountKind {
    type Error = DomainError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "bank" => Ok(Self::Bank),
            "cash" => Ok(Self::Cash),
            _ => Err(DomainError::InvalidAccount),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    name: String,
    kind: AccountKind,
    opened_day: i32,
    opening: Money,
}
impl Account {
    pub fn new(
        name: &str,
        kind: AccountKind,
        opened_day: i32,
        opening: i64,
    ) -> Result<Self, DomainError> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > 100
            || name.chars().any(char::is_control)
            || !(1..=3_652_059).contains(&opened_day)
        {
            return Err(DomainError::InvalidAccount);
        }
        Ok(Self {
            name: name.into(),
            kind,
            opened_day,
            opening: Money::from_cents(opening)?,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn kind(&self) -> AccountKind {
        self.kind
    }
    pub fn opened_day(&self) -> i32 {
        self.opened_day
    }
    pub fn opening(&self) -> Money {
        self.opening
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashMovement {
    description: String,
    day: i32,
    amount: Money,
}
impl CashMovement {
    pub fn new(description: &str, day: i32, amount: i64) -> Result<Self, DomainError> {
        let description = description.trim();
        if description.is_empty()
            || description.chars().count() > 200
            || description.chars().any(char::is_control)
            || !(1..=3_652_059).contains(&day)
            || amount <= 0
        {
            return Err(DomainError::InvalidAccount);
        }
        Ok(Self {
            description: description.into(),
            day,
            amount: Money::from_cents(amount)?,
        })
    }
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn day(&self) -> i32 {
        self.day
    }
    pub fn amount(&self) -> Money {
        self.amount
    }
    pub fn validate_opening(&self, account: &Account) -> Result<(), DomainError> {
        if self.day < account.opened_day {
            return Err(DomainError::InvalidAccount);
        }
        Ok(())
    }
}
pub fn debit(balance: Money, amount: Money) -> Result<Money, DomainError> {
    if amount.to_cents() > balance.to_cents() {
        return Err(DomainError::InsufficientFunds);
    }
    balance.checked_sub(amount)
}
pub fn credit(balance: Money, amount: Money) -> Result<Money, DomainError> {
    balance.checked_add(amount)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn balances_are_exact_and_accounts_validate() {
        assert!(Account::new(" ", AccountKind::Cash, 1, 0).is_err());
        assert!(Account::new("A", AccountKind::Cash, 1, -1).is_err());
        assert!(AccountKind::try_from("credit_card").is_err());
        let max = Money::from_cents(i64::MAX).unwrap();
        let one = Money::from_cents(1).unwrap();
        assert_eq!(credit(max, one), Err(DomainError::AmountOverflow));
        assert_eq!(
            debit(Money::zero(), one),
            Err(DomainError::InsufficientFunds)
        );
        assert_eq!(debit(max, max).unwrap(), Money::zero());
        let account = Account::new(" Bank ", AccountKind::Bank, 10, 0).unwrap();
        assert_eq!(account.name(), "Bank");
        assert!(CashMovement::new("Receipt", 9, 1)
            .unwrap()
            .validate_opening(&account)
            .is_err());
        assert!(CashMovement::new("Receipt", 10, 1)
            .unwrap()
            .validate_opening(&account)
            .is_ok());
        assert!(CashMovement::new("Receipt", 10, 0).is_err());
    }
}
