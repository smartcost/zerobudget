use super::{page, require, BankingError};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use obz_auth::{LocalUser, Permission};
use obz_domain::finance::FinanceNote;
use std::sync::Arc;
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct CorrectionInput {
    pub request_id: Uuid,
    pub description: String,
    pub occurred_on: NaiveDate,
}
#[derive(Debug, Clone)]
pub struct Correction {
    pub id: Uuid,
    pub movement_id: Uuid,
    pub note: FinanceNote,
}
#[derive(Debug, Clone)]
pub struct OpeningDebtInput {
    pub request_id: Uuid,
    pub invoice_id: Uuid,
    pub description: String,
    pub occurred_on: NaiveDate,
    pub amount_cents: i64,
}
#[derive(Debug, Clone)]
pub struct CardCreditInput {
    pub request_id: Uuid,
    pub invoice_id: Uuid,
    pub expense_id: Option<Uuid>,
    pub opening_debt_id: Option<Uuid>,
    pub description: String,
    pub occurred_on: NaiveDate,
}
#[derive(Debug, Clone)]
pub struct CreditEntry {
    pub id: Uuid,
    pub card_id: Uuid,
    pub invoice_id: Uuid,
    pub expense_id: Option<Uuid>,
    pub opening_debt_id: Option<Uuid>,
    pub note: FinanceNote,
    pub amount_cents: i64,
}
#[derive(Debug, Clone)]
pub struct FinanceOverview {
    pub cash_cents: i64,
    pub card_debt_cents: i64,
    pub card_credit_cents: i64,
    pub active_budget_remaining_cents: i64,
    pub funding_gap_cents: i64,
    pub uncommitted_cash_cents: i64,
}
#[async_trait]
pub trait FinanceRepository: Send + Sync {
    async fn reverse(&self, owner: Uuid, record: Correction) -> Result<Correction, BankingError>;
    async fn opening(&self, owner: Uuid, record: CreditEntry) -> Result<CreditEntry, BankingError>;
    async fn credit(&self, owner: Uuid, record: CreditEntry) -> Result<CreditEntry, BankingError>;
    async fn entries(
        &self,
        owner: Uuid,
        card: Uuid,
        opening: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<CreditEntry>, BankingError>;
    async fn overview(&self, owner: Uuid) -> Result<FinanceOverview, BankingError>;
}
#[derive(Clone)]
pub struct FinanceService {
    repository: Arc<dyn FinanceRepository>,
}
impl FinanceService {
    pub fn new(repository: Arc<dyn FinanceRepository>) -> Self {
        Self { repository }
    }
    pub async fn reverse(
        &self,
        user: &LocalUser,
        movement: Uuid,
        input: CorrectionInput,
    ) -> Result<Correction, BankingError> {
        require(user, Permission::WriteBudget)?;
        if input.request_id.is_nil() {
            return Err(BankingError::Invalid);
        }
        let note = FinanceNote::new(&input.description, input.occurred_on.num_days_from_ce())
            .map_err(|_| BankingError::Invalid)?;
        self.repository
            .reverse(
                user.id(),
                Correction {
                    id: input.request_id,
                    movement_id: movement,
                    note,
                },
            )
            .await
    }
    pub async fn opening(
        &self,
        user: &LocalUser,
        card: Uuid,
        input: OpeningDebtInput,
    ) -> Result<CreditEntry, BankingError> {
        require(user, Permission::WriteBudget)?;
        if input.request_id.is_nil() || input.amount_cents <= 0 {
            return Err(BankingError::Invalid);
        }
        let note = FinanceNote::new(&input.description, input.occurred_on.num_days_from_ce())
            .map_err(|_| BankingError::Invalid)?;
        self.repository
            .opening(
                user.id(),
                CreditEntry {
                    id: input.request_id,
                    card_id: card,
                    invoice_id: input.invoice_id,
                    expense_id: None,
                    opening_debt_id: None,
                    note,
                    amount_cents: input.amount_cents,
                },
            )
            .await
    }
    pub async fn credit(
        &self,
        user: &LocalUser,
        card: Uuid,
        input: CardCreditInput,
    ) -> Result<CreditEntry, BankingError> {
        require(user, Permission::WriteBudget)?;
        if input.request_id.is_nil()
            || input.expense_id.is_some() == input.opening_debt_id.is_some()
        {
            return Err(BankingError::Invalid);
        }
        let note = FinanceNote::new(&input.description, input.occurred_on.num_days_from_ce())
            .map_err(|_| BankingError::Invalid)?;
        self.repository
            .credit(
                user.id(),
                CreditEntry {
                    id: input.request_id,
                    card_id: card,
                    invoice_id: input.invoice_id,
                    expense_id: input.expense_id,
                    opening_debt_id: input.opening_debt_id,
                    note,
                    amount_cents: 0,
                },
            )
            .await
    }
    pub async fn entries(
        &self,
        user: &LocalUser,
        card: Uuid,
        opening: bool,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<CreditEntry>, BankingError> {
        require(user, Permission::ReadBudget)?;
        page(limit, offset)?;
        self.repository
            .entries(user.id(), card, opening, limit, offset)
            .await
    }
    pub async fn overview(&self, user: &LocalUser) -> Result<FinanceOverview, BankingError> {
        require(user, Permission::ReadBudget)?;
        self.repository.overview(user.id()).await
    }
}
