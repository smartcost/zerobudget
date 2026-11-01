//! Credit purchases consume budget at purchase time; payments settle liabilities.
use crate::{money::Money, DomainError};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreditCard {
    name: String,
    opened_day: i32,
    limit: Money,
}
impl CreditCard {
    pub fn new(name: &str, opened_day: i32, limit_cents: i64) -> Result<Self, DomainError> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > 100
            || name.chars().any(char::is_control)
            || !(1..=3_652_059).contains(&opened_day)
            || limit_cents <= 0
        {
            return Err(DomainError::InvalidCredit);
        }
        Ok(Self {
            name: name.into(),
            opened_day,
            limit: Money::from_cents(limit_cents)?,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn opened_day(&self) -> i32 {
        self.opened_day
    }
    pub fn limit(&self) -> Money {
        self.limit
    }
    pub fn charge(&self, outstanding: Money, amount: Money) -> Result<Money, DomainError> {
        if amount == Money::zero() {
            return Err(DomainError::InvalidAmount);
        }
        let available = self
            .limit
            .checked_sub(outstanding)
            .map_err(|_| DomainError::CreditLimit)?;
        if amount.to_cents() > available.to_cents() {
            return Err(DomainError::CreditLimit);
        }
        outstanding.checked_add(amount)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvoicePeriod {
    start: i32,
    end: i32,
    due: i32,
}
impl InvoicePeriod {
    pub fn new(start: i32, end: i32, due: i32) -> Result<Self, DomainError> {
        if start < 1 || end <= start || due < end || due > 3_652_059 {
            return Err(DomainError::InvalidCredit);
        }
        Ok(Self { start, end, due })
    }
    pub fn start(self) -> i32 {
        self.start
    }
    pub fn end(self) -> i32 {
        self.end
    }
    pub fn due(self) -> i32 {
        self.due
    }
    pub fn contains(self, day: i32) -> bool {
        day >= self.start && day < self.end
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.start < other.end && self.end > other.start
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvoiceBalance {
    purchases: Money,
    paid: Money,
}
impl InvoiceBalance {
    pub fn new(purchases: Money, paid: Money) -> Result<Self, DomainError> {
        if paid.to_cents() > purchases.to_cents() {
            return Err(DomainError::InvalidCredit);
        }
        Ok(Self { purchases, paid })
    }
    pub fn outstanding(self) -> Money {
        self.purchases
            .checked_sub(self.paid)
            .expect("validated invoice balance")
    }
    pub fn pay(self, closed: bool, amount: Money) -> Result<Self, DomainError> {
        if !closed {
            return Err(DomainError::InvalidInvoiceStatus);
        }
        if amount == Money::zero() {
            return Err(DomainError::InvalidAmount);
        }
        if amount.to_cents() > self.outstanding().to_cents() {
            return Err(DomainError::CreditOverpayment);
        }
        Self::new(self.purchases, self.paid.checked_add(amount)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credit_limit_and_settlement_are_exact() {
        let card = CreditCard::new("Card", 1, i64::MAX).unwrap();
        let max = Money::from_cents(i64::MAX).unwrap();
        let one = Money::from_cents(1).unwrap();
        assert_eq!(card.charge(Money::zero(), max).unwrap(), max);
        assert_eq!(card.charge(max, one), Err(DomainError::CreditLimit));
        let invoice = InvoiceBalance::new(max, Money::zero()).unwrap();
        assert_eq!(invoice.pay(true, max).unwrap().outstanding(), Money::zero());
        assert_eq!(
            invoice.pay(false, one),
            Err(DomainError::InvalidInvoiceStatus)
        );
        assert_eq!(
            invoice.pay(true, max).unwrap().pay(true, one),
            Err(DomainError::CreditOverpayment)
        );
    }
    #[test]
    fn invoice_period_has_exclusive_end_and_valid_due_date() {
        let p = InvoicePeriod::new(1, 32, 40).unwrap();
        assert!(p.contains(1));
        assert!(!p.contains(32));
        assert!(!p.overlaps(InvoicePeriod::new(32, 60, 70).unwrap()));
        assert!(p.overlaps(InvoicePeriod::new(31, 60, 70).unwrap()));
        assert!(InvoicePeriod::new(1, 32, 31).is_err());
        assert!(CreditCard::new("", 1, 100).is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditPosition {
    pub outstanding: Money,
    pub credit: Money,
}
impl CreditPosition {
    pub fn from_net(net: i128) -> Result<Self, DomainError> {
        if net > i128::from(i64::MAX) || net < -i128::from(i64::MAX) {
            return Err(DomainError::AmountOverflow);
        }
        Ok(Self {
            outstanding: Money::from_cents(net.max(0) as i64)?,
            credit: Money::from_cents((-net).max(0) as i64)?,
        })
    }
    pub fn invoice(
        purchases: Money,
        opening: Money,
        credits: Money,
        paid: Money,
    ) -> Result<Self, DomainError> {
        let gross = purchases.checked_add(opening)?;
        Self::from_net(
            i128::from(gross.to_cents())
                - i128::from(credits.to_cents())
                - i128::from(paid.to_cents()),
        )
    }
}
impl CreditCard {
    pub fn charge_with_credit(
        &self,
        outstanding: Money,
        credit: Money,
        amount: Money,
    ) -> Result<CreditPosition, DomainError> {
        if amount == Money::zero() {
            return Err(DomainError::InvalidAmount);
        }
        let net = i128::from(outstanding.to_cents()) - i128::from(credit.to_cents())
            + i128::from(amount.to_cents());
        if net > i128::from(self.limit.to_cents()) {
            return Err(DomainError::CreditLimit);
        }
        CreditPosition::from_net(net)
    }
}

#[cfg(test)]
mod position_tests {
    use super::*;
    #[test]
    fn refund_credit_and_debt_use_checked_signed_positions() {
        let m = |v| Money::from_cents(v).unwrap();
        let p = CreditPosition::invoice(m(100), m(10), m(200), m(0)).unwrap();
        assert_eq!(p.credit, m(90));
        assert_eq!(p.outstanding, m(0));
        assert!(CreditPosition::from_net(-i128::from(i64::MAX) - 1).is_err());
        assert!(CreditPosition::invoice(m(i64::MAX), m(1), m(0), m(0)).is_err());
        let c = CreditCard::new("Card", 1, 100).unwrap();
        assert_eq!(
            c.charge_with_credit(p.outstanding, p.credit, m(150))
                .unwrap()
                .outstanding,
            m(60)
        );
        assert!(c
            .charge_with_credit(p.outstanding, p.credit, m(191))
            .is_err());
    }
}
