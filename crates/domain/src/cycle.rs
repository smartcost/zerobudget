//! Budget lifecycle rules; dates are ordinal days in the proleptic Gregorian calendar.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleStatus {
    Planned,
    Active,
    Closed,
}
impl CycleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Active => "active",
            Self::Closed => "closed",
        }
    }
}
impl TryFrom<&str> for CycleStatus {
    type Error = DomainError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "planned" => Ok(Self::Planned),
            "active" => Ok(Self::Active),
            "closed" => Ok(Self::Closed),
            _ => Err(DomainError::InvalidCycleStatus),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    period: BudgetPeriod,
    status: CycleStatus,
    income: Money,
    allocation: Allocation,
}
impl Cycle {
    pub fn plan(period: BudgetPeriod, income_cents: i64) -> Result<Self, DomainError> {
        Self::restore(period, income_cents, 0, CycleStatus::Planned)
    }
    pub fn restore(
        period: BudgetPeriod,
        income_cents: i64,
        allocated_cents: i64,
        status: CycleStatus,
    ) -> Result<Self, DomainError> {
        let income = Money::from_cents(income_cents)?;
        let allocation = Allocation::new(income, Money::from_cents(allocated_cents)?)?;
        if status != CycleStatus::Planned && allocation.available().to_cents() != 0 {
            return Err(DomainError::UnallocatedIncome);
        }
        Ok(Self {
            period,
            status,
            income,
            allocation,
        })
    }
    pub fn status(&self) -> CycleStatus {
        self.status
    }
    /// Repeating the same transition is idempotent; reopening is not supported.
    pub fn activate(&mut self) -> Result<(), DomainError> {
        if self.status == CycleStatus::Closed {
            return Err(DomainError::InvalidCycleStatus);
        }
        if self.available_cents() != 0 {
            return Err(DomainError::UnallocatedIncome);
        }
        self.status = CycleStatus::Active;
        Ok(())
    }
    pub fn close(&mut self) -> Result<(), DomainError> {
        if self.status == CycleStatus::Planned {
            return Err(DomainError::InvalidCycleStatus);
        }
        self.status = CycleStatus::Closed;
        Ok(())
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
        let cycle = Cycle::restore(period, 1000, 600, CycleStatus::Planned).unwrap();
        assert_eq!(cycle.allocated_cents(), 600);
        assert_eq!(cycle.available_cents(), 400);
        assert!(Cycle::restore(period, 1000, 1001, CycleStatus::Planned).is_err());
        assert!(Cycle::restore(period, 1000, -1, CycleStatus::Planned).is_err());
    }
    #[test]
    fn period_requires_ordered_supported_dates() {
        for (start, end) in [(0, 1), (2, 2), (3, 2), (1, 3_652_060)] {
            assert!(BudgetPeriod::new(start, end).is_err());
        }
        assert!(BudgetPeriod::new(1, 3_652_059).is_ok());
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn transitions_require_balance_and_never_reopen() {
        let period = BudgetPeriod::new(1, 32).unwrap();
        let mut incomplete = Cycle::plan(period, 1).unwrap();
        assert_eq!(incomplete.activate(), Err(DomainError::UnallocatedIncome));
        assert_eq!(incomplete.close(), Err(DomainError::InvalidCycleStatus));
        assert_eq!(incomplete.status(), CycleStatus::Planned);
        for income in [0, 100, i64::MAX] {
            let mut cycle = Cycle::restore(period, income, income, CycleStatus::Planned).unwrap();
            cycle.activate().unwrap();
            cycle.activate().unwrap();
            assert_eq!(cycle.status(), CycleStatus::Active);
            cycle.close().unwrap();
            cycle.close().unwrap();
            assert_eq!(cycle.activate(), Err(DomainError::InvalidCycleStatus));
            assert_eq!(cycle.status(), CycleStatus::Closed);
            assert_eq!(cycle.available_cents(), 0);
        }
    }
    #[test]
    fn restoration_rejects_unbalanced_confirmed_budgets_and_unknown_status() {
        let period = BudgetPeriod::new(1, 32).unwrap();
        for status in [CycleStatus::Active, CycleStatus::Closed] {
            assert_eq!(
                Cycle::restore(period, 100, 99, status),
                Err(DomainError::UnallocatedIncome)
            );
        }
        for status in [
            CycleStatus::Planned,
            CycleStatus::Active,
            CycleStatus::Closed,
        ] {
            assert_eq!(CycleStatus::try_from(status.as_str()), Ok(status));
        }
        assert!(CycleStatus::try_from("unknown").is_err());
    }
}
