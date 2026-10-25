use crate::{money::Money, DomainError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeName(String);
impl EnvelopeName {
    /// Names are trimmed, case-sensitive, and contain 1..=100 Unicode characters.
    pub fn new(name: &str) -> Result<Self, DomainError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err(DomainError::InvalidEnvelopeName);
        }
        Ok(Self(name.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    name: EnvelopeName,
    allocated: Money,
}
impl Envelope {
    pub fn new(name: EnvelopeName, allocated: Money) -> Self {
        Self { name, allocated }
    }
    pub fn name(&self) -> &EnvelopeName {
        &self.name
    }
    pub fn allocated(&self) -> Money {
        self.allocated
    }
}
/// A cycle's aggregate allocation, including all its envelopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    income: Money,
    allocated: Money,
}
impl Allocation {
    pub fn new(income: Money, allocated: Money) -> Result<Self, DomainError> {
        if allocated.to_cents() > income.to_cents() {
            return Err(DomainError::EnvelopeOverbudget);
        }
        Ok(Self { income, allocated })
    }
    /// Remove the previous envelope value before adding its replacement.
    pub fn replace(self, previous: Money, next: Money) -> Result<Self, DomainError> {
        let rest = self.allocated.checked_sub(previous)?;
        // Compare against remaining capacity before adding, including i64::MAX.
        if next.to_cents() > self.income.checked_sub(rest)?.to_cents() {
            return Err(DomainError::EnvelopeOverbudget);
        }
        Self::new(self.income, rest.checked_add(next)?)
    }
    pub fn allocated(self) -> Money {
        self.allocated
    }
    pub fn available(self) -> Money {
        self.income
            .checked_sub(self.allocated)
            .expect("validated allocation")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn m(value: i64) -> Money {
        Money::from_cents(value).unwrap()
    }
    #[test]
    fn names_and_amounts() {
        assert_eq!(EnvelopeName::new(" Moradia ").unwrap().as_str(), "Moradia");
        for name in ["", " \t ", "a\nb", &"a".repeat(101)] {
            assert!(EnvelopeName::new(name).is_err());
        }
        assert!(EnvelopeName::new(&"á".repeat(100)).is_ok());
        assert_eq!(
            Envelope::new(EnvelopeName::new("Reserva").unwrap(), Money::zero()).allocated(),
            Money::zero()
        );
    }
    #[test]
    fn partial_full_reduction_removal_and_limits() {
        let balance = Allocation::new(m(1000), m(0))
            .unwrap()
            .replace(m(0), m(600))
            .unwrap();
        assert_eq!(balance.available(), m(400));
        let full = balance.replace(m(0), m(400)).unwrap();
        assert_eq!(full.available(), m(0));
        assert_eq!(
            full.replace(m(0), m(1)),
            Err(DomainError::EnvelopeOverbudget)
        );
        assert_eq!(full.replace(m(600), m(200)).unwrap().available(), m(400));
        assert_eq!(full.replace(m(600), m(0)).unwrap().allocated(), m(400));
        assert!(Allocation::new(m(0), m(1)).is_err());
        let max = Allocation::new(m(i64::MAX), m(i64::MAX)).unwrap();
        assert_eq!(
            max.replace(m(0), m(1)),
            Err(DomainError::EnvelopeOverbudget)
        );
        assert_eq!(
            max.replace(m(i64::MAX), m(i64::MAX)).unwrap().available(),
            m(0)
        );
        assert!(balance.replace(m(601), m(0)).is_err());
    }
}
