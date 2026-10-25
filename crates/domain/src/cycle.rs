//! Planned budget rules; dates are ordinal days in the proleptic Gregorian calendar.
use crate::{envelope::Allocation, errors::DomainError, money::Money};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPeriod {
    start_day: i32,
    end_day: i32,
}
impl BudgetPeriod {
    /// The end is exclusive. Supported civil dates: 0001-01-01 to 9999-12-31.
    pub fn new(start_day: i32, end_day: i32) -> Result<Self, DomainError> {
        if start_day < 1 || end_day > 3_652_059 || end_day <= start_day {
            return Err(DomainError::InvalidPeriod);
        }
        Ok(Self { start_day, end_day })
    }
    pub fn start_day(self) -> i32 {
        self.start_day
    }
    pub fn end_day(self) -> i32 {
        self.end_day
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    period: BudgetPeriod,
    income: Money,
    allocation: Allocation,
}
impl Cycle {
    pub fn plan(period: BudgetPeriod, income_cents: i64) -> Result<Self, DomainError> {
        Self::restore(period, income_cents, 0)
    }
    pub fn restore(
        period: BudgetPeriod,
        income_cents: i64,
        allocated_cents: i64,
    ) -> Result<Self, DomainError> {
        let income = Money::from_cents(income_cents)?;
        Ok(Self {
            period,
            income,
            allocation: Allocation::new(income, Money::from_cents(allocated_cents)?)?,
        })
    }
    pub fn period(&self) -> BudgetPeriod {
        self.period
    }
    pub fn income_cents(&self) -> i64 {
        self.income.to_cents()
    }
    pub fn allocated_cents(&self) -> i64 {
        self.allocation.allocated().to_cents()
    }
    pub fn available_cents(&self) -> i64 {
        self.allocation.available().to_cents()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planned_cycle_starts_with_all_income_unallocated() {
        let period = BudgetPeriod::new(1, 32).unwrap();
        for amount in [0, 100_000, i64::MAX] {
            let cycle = Cycle::plan(period, amount).unwrap();
            assert_eq!(cycle.income_cents(), amount);
            assert_eq!(cycle.allocated_cents(), 0);
            assert_eq!(cycle.available_cents(), amount);
        }
        assert!(Cycle::plan(period, -1).is_err());
    }
    #[test]
    fn restored_totals_preserve_budget_invariants() {
        let period = BudgetPeriod::new(1, 32).unwrap();
        let cycle = Cycle::restore(period, 1000, 600).unwrap();
        assert_eq!(cycle.allocated_cents(), 600);
        assert_eq!(cycle.available_cents(), 400);
        assert!(Cycle::restore(period, 1000, 1001).is_err());
        assert!(Cycle::restore(period, 1000, -1).is_err());
    }
    #[test]
    fn period_requires_ordered_supported_dates() {
        for (start, end) in [(0, 1), (2, 2), (3, 2), (1, 3_652_060)] {
            assert!(BudgetPeriod::new(start, end).is_err());
        }
        assert!(BudgetPeriod::new(1, 3_652_059).is_ok());
    }
}
