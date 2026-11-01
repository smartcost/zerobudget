use super::{page, require, BankingError, MovementInput};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
use obz_domain::{
    banking::CashMovement,
    credit::{CreditCard, InvoicePeriod},
    money::Money,
};
use std::sync::Arc;
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct Card {
    pub id: Uuid,
    pub card: CreditCard,
    pub outstanding: Money,
    pub credit_balance: Money,
}
#[derive(Debug, Clone)]
pub struct Invoice {
    pub id: Uuid,
    pub card_id: Uuid,
    pub period: InvoicePeriod,
    pub closed: bool,
    pub purchases: Money,
    pub paid: Money,
    pub opening_debt: Money,
    pub credits: Money,
}
#[derive(Debug, Clone)]
pub struct CardPayment {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub account_id: Uuid,
    pub movement: CashMovement,
    pub reversed: bool,
}
#[derive(Debug, Clone)]
pub struct InvoicePurchase {
    pub id: Uuid,
    pub cycle_id: Uuid,
    pub envelope_id: Uuid,
    pub description: String,
    pub occurred_on: NaiveDate,
    pub amount_cents: i64,
    pub reversed: bool,
}
#[derive(Debug, Clone)]
pub struct CardInput {
    pub name: String,
    pub opened_on: NaiveDate,
    pub credit_limit_cents: i64,
}
#[derive(Debug, Clone)]
pub struct InvoiceInput {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub due_on: NaiveDate,
}
#[derive(Debug, Clone)]
pub struct PaymentInput {
    pub request_id: Uuid,
    pub account_id: Uuid,
    pub movement: MovementInput,
}
#[async_trait]
pub trait CreditRepository: Send + Sync {
    async fn create(&self, owner: Uuid, id: Uuid, card: CreditCard) -> Result<Card, BankingError>;
    async fn cards(
        &self,
        owner: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Card>, BankingError>;
    async fn open(
        &self,
        owner: Uuid,
        id: Uuid,
        card: Uuid,
        period: InvoicePeriod,
    ) -> Result<Invoice, BankingError>;
    async fn invoices(
        &self,
        owner: Uuid,
        card: Uuid,
        id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Invoice>, BankingError>;
    async fn close(&self, owner: Uuid, card: Uuid, invoice: Uuid) -> Result<Invoice, BankingError>;
    async fn pay(
        &self,
        owner: Uuid,
        card: Uuid,
        invoice: Uuid,
        payment: CardPayment,
    ) -> Result<CardPayment, BankingError>;
    async fn purchases(
        &self,
        owner: Uuid,
        card: Uuid,
        invoice: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<InvoicePurchase>, BankingError>;
    async fn payments(
        &self,
        owner: Uuid,
        card: Uuid,
        invoice: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<CardPayment>, BankingError>;
}
#[derive(Clone)]
pub struct CreditService {
    repository: Arc<dyn CreditRepository>,
}
impl CreditService {
    pub fn new(repository: Arc<dyn CreditRepository>) -> Self {
        Self { repository }
    }
    pub async fn create(&self, user: &LocalUser, input: CardInput) -> Result<Card, BankingError> {
        require(user, Permission::WriteBudget)?;
        let card = CreditCard::new(
            &input.name,
            input.opened_on.num_days_from_ce(),
            input.credit_limit_cents,
        )
        .map_err(|_| BankingError::Invalid)?;
        self.repository
            .create(user.id(), Uuid::now_v7(), card)
            .await
    }
    pub async fn list(
        &self,
        user: &LocalUser,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Card>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository.cards(user.id(), None, limit, offset).await
    }
    pub async fn get(&self, user: &LocalUser, id: Uuid) -> Result<Card, BankingError> {
        require(user, Permission::ReadBudget)?;
        self.repository
            .cards(user.id(), Some(id), 1, 0)
            .await?
            .into_iter()
            .next()
            .ok_or(BankingError::NotFound)
    }
    pub async fn open(
        &self,
        user: &LocalUser,
        card: Uuid,
        input: InvoiceInput,
    ) -> Result<Invoice, BankingError> {
        require(user, Permission::WriteBudget)?;
        let period = InvoicePeriod::new(
            input.period_start.num_days_from_ce(),
            input.period_end.num_days_from_ce(),
            input.due_on.num_days_from_ce(),
        )
        .map_err(|_| BankingError::Invalid)?;
        self.repository
            .open(user.id(), Uuid::now_v7(), card, period)
            .await
    }
    pub async fn invoices(
        &self,
        user: &LocalUser,
        card: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Invoice>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository
            .invoices(user.id(), card, None, limit, offset)
            .await
    }
    pub async fn invoice(
        &self,
        user: &LocalUser,
        card: Uuid,
        id: Uuid,
    ) -> Result<Invoice, BankingError> {
        require(user, Permission::ReadBudget)?;
        self.repository
            .invoices(user.id(), card, Some(id), 1, 0)
            .await?
            .into_iter()
            .next()
            .ok_or(BankingError::NotFound)
    }
    pub async fn close(
        &self,
        user: &LocalUser,
        card: Uuid,
        id: Uuid,
    ) -> Result<Invoice, BankingError> {
        require(user, Permission::WriteBudget)?;
        self.repository.close(user.id(), card, id).await
    }
    pub async fn pay(
        &self,
        user: &LocalUser,
        card: Uuid,
        id: Uuid,
        input: PaymentInput,
    ) -> Result<CardPayment, BankingError> {
        require(user, Permission::WriteBudget)?;
        if input.request_id.get_version_num() != 7
            || input.request_id.get_variant() != uuid::Variant::RFC4122
        {
            return Err(BankingError::Invalid);
        }
        let movement = CashMovement::new(
            &input.movement.description,
            input.movement.occurred_on.num_days_from_ce(),
            input.movement.amount_cents,
        )
        .map_err(|_| BankingError::Invalid)?;
        self.repository
            .pay(
                user.id(),
                card,
                id,
                CardPayment {
                    id: input.request_id,
                    invoice_id: id,
                    account_id: input.account_id,
                    movement,
                    reversed: false,
                },
            )
            .await
    }
    pub async fn purchases(
        &self,
        user: &LocalUser,
        card: Uuid,
        id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<InvoicePurchase>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository
            .purchases(user.id(), card, id, limit, offset)
            .await
    }
    pub async fn payments(
        &self,
        user: &LocalUser,
        card: Uuid,
        id: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<CardPayment>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository
            .payments(user.id(), card, id, limit, offset)
            .await
    }
}

pub fn invoice_position(i: &Invoice) -> (i64, i64) {
    let p =
        obz_domain::credit::CreditPosition::invoice(i.purchases, i.opening_debt, i.credits, i.paid)
            .expect("validated invoice position");
    (p.outstanding.to_cents(), p.credit.to_cents())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Repository(std::sync::Mutex<Vec<Uuid>>);
    #[async_trait]
    impl CreditRepository for Repository {
        async fn create(&self, o: Uuid, id: Uuid, c: CreditCard) -> Result<Card, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(Card {
                id,
                card: c,
                outstanding: Money::zero(),
                credit_balance: Money::zero(),
            })
        }
        async fn cards(
            &self,
            o: Uuid,
            _: Option<Uuid>,
            _: u32,
            _: u32,
        ) -> Result<Vec<Card>, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(vec![])
        }
        async fn open(
            &self,
            o: Uuid,
            id: Uuid,
            c: Uuid,
            p: InvoicePeriod,
        ) -> Result<Invoice, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(Invoice {
                id,
                card_id: c,
                period: p,
                closed: false,
                purchases: Money::zero(),
                paid: Money::zero(),
                opening_debt: Money::zero(),
                credits: Money::zero(),
            })
        }
        async fn invoices(
            &self,
            o: Uuid,
            _: Uuid,
            _: Option<Uuid>,
            _: u32,
            _: u32,
        ) -> Result<Vec<Invoice>, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(vec![])
        }
        async fn close(&self, o: Uuid, _: Uuid, _: Uuid) -> Result<Invoice, BankingError> {
            self.0.lock().unwrap().push(o);
            Err(BankingError::NotFound)
        }
        async fn pay(
            &self,
            o: Uuid,
            _: Uuid,
            _: Uuid,
            p: CardPayment,
        ) -> Result<CardPayment, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(p)
        }
        async fn purchases(
            &self,
            o: Uuid,
            _: Uuid,
            _: Uuid,
            _: u32,
            _: u32,
        ) -> Result<Vec<InvoicePurchase>, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(vec![])
        }
        async fn payments(
            &self,
            o: Uuid,
            _: Uuid,
            _: Uuid,
            _: u32,
            _: u32,
        ) -> Result<Vec<CardPayment>, BankingError> {
            self.0.lock().unwrap().push(o);
            Ok(vec![])
        }
    }
    #[tokio::test]
    async fn credit_authorization_validation_and_operation_id_precede_storage() {
        let repo = Arc::new(Repository::default());
        let service = CreditService::new(repo.clone());
        let owner = Uuid::now_v7();
        let card = Uuid::now_v7();
        let invoice = Uuid::now_v7();
        let account = Uuid::now_v7();
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let denied = LocalUser::from_persisted(owner, Default::default());
        let reader =
            LocalUser::from_persisted(owner, [Permission::ReadBudget].into_iter().collect());
        let writer =
            LocalUser::from_persisted(owner, [Permission::WriteBudget].into_iter().collect());
        let input = CardInput {
            name: "Card".into(),
            opened_on: date,
            credit_limit_cents: 100,
        };
        let period = InvoiceInput {
            period_start: date,
            period_end: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            due_on: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        };
        let payment = PaymentInput {
            request_id: Uuid::now_v7(),
            account_id: account,
            movement: MovementInput {
                description: "Bill".into(),
                occurred_on: period.period_end,
                amount_cents: 1,
            },
        };
        assert!(matches!(
            service.create(&denied, input.clone()).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.list(&denied, 20, 0).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.open(&reader, card, period.clone()).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.close(&reader, card, invoice).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.pay(&reader, card, invoice, payment.clone()).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.payments(&writer, card, invoice, 20, 0).await,
            Err(BankingError::Forbidden)
        ));
        assert!(matches!(
            service.purchases(&denied, card, invoice, 20, 0).await,
            Err(BankingError::Forbidden)
        ));
        let mut invalid = input.clone();
        invalid.credit_limit_cents = 0;
        assert!(matches!(
            service.create(&writer, invalid).await,
            Err(BankingError::Invalid)
        ));
        let mut invalid = payment.clone();
        invalid.request_id = Uuid::nil();
        assert!(matches!(
            service.pay(&writer, card, invoice, invalid).await,
            Err(BankingError::Invalid)
        ));
        assert!(repo.0.lock().unwrap().is_empty());
        service.create(&writer, input).await.unwrap();
        service.open(&writer, card, period).await.unwrap();
        let result = service
            .pay(&writer, card, invoice, payment.clone())
            .await
            .unwrap();
        assert_eq!(result.id, payment.request_id);
        service
            .payments(&reader, card, invoice, 20, 0)
            .await
            .unwrap();
        service
            .purchases(&reader, card, invoice, 20, 0)
            .await
            .unwrap();
        assert_eq!(*repo.0.lock().unwrap(), vec![owner; 5]);
    }
}
