//! Immutable expenses and the spendable balance of a budget envelope.
use crate::{
    cycle::{BudgetPeriod, CycleStatus},
    money::Money,
    DomainError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expense {
    description: String,
    day: i32,
    amount: Money,
}
impl Expense {
    pub fn new(description: &str, day: i32, amount_cents: i64) -> Result<Self, DomainError> {
        let description = description.trim();
        if description.is_empty()
            || description.chars().count() > 200
            || description.chars().any(char::is_control)
            || !(1..=3_652_059).contains(&day)
        {
            return Err(DomainError::InvalidExpense);
        }
        let amount = Money::from_cents(amount_cents)?;
        if amount == Money::zero() {
            return Err(DomainError::InvalidAmount);
        }
        Ok(Self {
            description: description.into(),
            day,
            amount,
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
    pub fn validate_period(&self, period: BudgetPeriod) -> Result<(), DomainError> {
        if self.day < period.start_day() || self.day >= period.end_day() {
            return Err(DomainError::InvalidExpense);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spending {
    allocated: Money,
    spent: Money,
}
impl Spending {
    pub fn new(allocated: Money, spent: Money) -> Result<Self, DomainError> {
        if spent.to_cents() > allocated.to_cents() {
            return Err(DomainError::InsufficientFunds);
        }
        Ok(Self { allocated, spent })
    }
    pub fn spent(self) -> Money {
        self.spent
    }
    pub fn remaining(self) -> Money {
        self.allocated
            .checked_sub(self.spent)
            .expect("validated spending balance")
    }
    pub fn record(self, status: CycleStatus, amount: Money) -> Result<Self, DomainError> {
        if status != CycleStatus::Active {
            return Err(DomainError::InvalidCycleStatus);
        }
        if amount == Money::zero() {
            return Err(DomainError::InvalidAmount);
        }
        if amount.to_cents() > self.remaining().to_cents() {
            return Err(DomainError::InsufficientFunds);
        }
        Self::new(self.allocated, self.spent.checked_add(amount)?)
    }
    pub fn reverse(self, status: CycleStatus, amount: Money) -> Result<Self, DomainError> {
        if status != CycleStatus::Active {
            return Err(DomainError::InvalidCycleStatus);
        }
        if amount == Money::zero() {
            return Err(DomainError::InvalidAmount);
        }
        Self::new(self.allocated, self.spent.checked_sub(amount)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expense_validates_description_amount_and_exclusive_period() {
        let period = BudgetPeriod::new(10, 20).unwrap();
        for (description, day, amount) in [
            ("", 10, 1),
            ("\n", 10, 1),
            ("a\nb", 10, 1),
            ("A", 0, 1),
            ("A", 10, 0),
            ("A", 10, -1),
        ] {
            assert!(Expense::new(description, day, amount).is_err());
        }
        assert!(Expense::new(&"é".repeat(201), 10, 1).is_err());
        assert!(Expense::new(&"é".repeat(200), 10, 1).is_ok());
        for day in [10, 19] {
            let e = Expense::new(" Food ", day, 1).unwrap();
            assert_eq!(e.description(), "Food");
            e.validate_period(period).unwrap();
        }
        for day in [9, 20] {
            assert!(Expense::new("Food", day, 1)
                .unwrap()
                .validate_period(period)
                .is_err());
        }
    }
    #[test]
    fn spending_and_reversal_preserve_exact_balance_and_integer_limits() {
        for cents in [100, i64::MAX] {
            let amount = Money::from_cents(cents).unwrap();
            let balance = Spending::new(amount, Money::zero()).unwrap();
            let full = balance.record(CycleStatus::Active, amount).unwrap();
            assert_eq!(full.remaining(), Money::zero());
            assert_eq!(
                full.record(CycleStatus::Active, Money::from_cents(1).unwrap()),
                Err(DomainError::InsufficientFunds)
            );
            assert_eq!(full.reverse(CycleStatus::Active, amount).unwrap(), balance);
            for state in [CycleStatus::Planned, CycleStatus::Closed] {
                assert_eq!(
                    balance.record(state, amount),
                    Err(DomainError::InvalidCycleStatus)
                );
                assert_eq!(
                    full.reverse(state, amount),
                    Err(DomainError::InvalidCycleStatus)
                );
            }
        }
    }
}
