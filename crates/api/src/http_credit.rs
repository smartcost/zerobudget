use crate::{ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use obz_banking::{
    BankingError, Card, CardInput, CardPayment, CreditService, Invoice, InvoiceInput,
    InvoicePurchase, MovementInput, PaymentInput,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/cards", get(list).post(create))
        .route("/api/cards/{card}", get(find))
        .route("/api/cards/{card}/invoices", get(invoices).post(open))
        .route("/api/cards/{card}/invoices/{invoice}", get(invoice))
        .route("/api/cards/{card}/invoices/{invoice}/close", post(close))
        .route(
            "/api/cards/{card}/invoices/{invoice}/payments",
            get(payments).post(pay),
        )
        .route(
            "/api/cards/{card}/invoices/{invoice}/purchases",
            get(purchases),
        )
        .layer(DefaultBodyLimit::max(4096))
}
fn service(state: &AppState) -> Result<&CreditService, ApiError> {
    state
        .credit
        .as_ref()
        .ok_or(BankingError::Unavailable.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    opened_on: String,
    credit_limit_cents: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Open {
    period_start: String,
    period_end: String,
    due_on: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pay {
    request_id: Uuid,
    account_id: Uuid,
    description: String,
    occurred_on: String,
    amount_cents: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
}
#[derive(Serialize)]
struct CardResponse {
    id: Uuid,
    name: String,
    opened_on: chrono::NaiveDate,
    credit_limit_cents: i64,
    outstanding_cents: i64,
    credit_balance_cents: i64,
    available_credit_cents: i64,
    currency: &'static str,
}
fn date(day: i32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_num_days_from_ce_opt(day).expect("validated credit date")
}
impl From<Card> for CardResponse {
    fn from(c: Card) -> Self {
        Self {
            id: c.id,
            name: c.card.name().into(),
            opened_on: date(c.card.opened_day()),
            credit_limit_cents: c.card.limit().to_cents(),
            outstanding_cents: c.outstanding.to_cents(),
            credit_balance_cents: c.credit_balance.to_cents(),
            available_credit_cents: c
                .card
                .limit()
                .checked_sub(c.outstanding)
                .expect("validated credit limit")
                .to_cents(),
            currency: "BRL",
        }
    }
}
#[derive(Serialize)]
struct InvoiceResponse {
    id: Uuid,
    card_id: Uuid,
    period_start: chrono::NaiveDate,
    period_end: chrono::NaiveDate,
    due_on: chrono::NaiveDate,
    purchases_cents: i64,
    paid_cents: i64,
    opening_debt_cents: i64,
    credits_cents: i64,
    credit_balance_cents: i64,
    outstanding_cents: i64,
    status: &'static str,
}
impl From<Invoice> for InvoiceResponse {
    fn from(i: Invoice) -> Self {
        let position = obz_banking::invoice_position(&i);
        let remaining = position.0;
        Self {
            id: i.id,
            card_id: i.card_id,
            period_start: date(i.period.start()),
            period_end: date(i.period.end()),
            due_on: date(i.period.due()),
            purchases_cents: i.purchases.to_cents(),
            paid_cents: i.paid.to_cents(),
            opening_debt_cents: i.opening_debt.to_cents(),
            credits_cents: i.credits.to_cents(),
            credit_balance_cents: position.1,
            outstanding_cents: remaining,
            status: if !i.closed {
                "open"
            } else if remaining == 0 {
                "paid"
            } else {
                "closed"
            },
        }
    }
}
#[derive(Serialize)]
struct PaymentResponse {
    id: Uuid,
    invoice_id: Uuid,
    account_id: Uuid,
    description: String,
    occurred_on: chrono::NaiveDate,
    amount_cents: i64,
    status: &'static str,
}
impl From<CardPayment> for PaymentResponse {
    fn from(p: CardPayment) -> Self {
        Self {
            id: p.id,
            invoice_id: p.invoice_id,
            account_id: p.account_id,
            description: p.movement.description().into(),
            occurred_on: date(p.movement.day()),
            amount_cents: p.movement.amount().to_cents(),
            status: if p.reversed { "reversed" } else { "posted" },
        }
    }
}
#[derive(Serialize)]
struct PurchaseResponse {
    id: Uuid,
    cycle_id: Uuid,
    envelope_id: Uuid,
    description: String,
    occurred_on: chrono::NaiveDate,
    amount_cents: i64,
    status: &'static str,
}
impl From<InvoicePurchase> for PurchaseResponse {
    fn from(p: InvoicePurchase) -> Self {
        Self {
            id: p.id,
            cycle_id: p.cycle_id,
            envelope_id: p.envelope_id,
            description: p.description,
            occurred_on: p.occurred_on,
            amount_cents: p.amount_cents,
            status: if p.reversed { "reversed" } else { "posted" },
        }
    }
}
async fn create(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<CardResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok((
        StatusCode::CREATED,
        Json(
            service(&state)?
                .create(
                    &auth.user,
                    CardInput {
                        name: body.name,
                        opened_on: crate::http_cycles::date(&body.opened_on)?,
                        credit_limit_cents: body.credit_limit_cents,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
async fn list(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<CardResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .list(&auth.user, p.limit.unwrap_or(20), p.offset.unwrap_or(0))
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn find(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CardResponse>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(service(&state)?.get(&auth.user, id).await?.into()))
}
async fn open(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(card): Path<Uuid>,
    Json(body): Json<Open>,
) -> Result<(StatusCode, Json<InvoiceResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok((
        StatusCode::CREATED,
        Json(
            service(&state)?
                .open(
                    &auth.user,
                    card,
                    InvoiceInput {
                        period_start: crate::http_cycles::date(&body.period_start)?,
                        period_end: crate::http_cycles::date(&body.period_end)?,
                        due_on: crate::http_cycles::date(&body.due_on)?,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
async fn invoices(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(card): Path<Uuid>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<InvoiceResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .invoices(
                &auth.user,
                card,
                p.limit.unwrap_or(20),
                p.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn invoice(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((card, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<InvoiceResponse>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?.invoice(&auth.user, card, id).await?.into(),
    ))
}
async fn close(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((card, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<InvoiceResponse>, ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok(Json(
        service(&state)?.close(&auth.user, card, id).await?.into(),
    ))
}
async fn pay(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((card, id)): Path<(Uuid, Uuid)>,
    Json(body): Json<Pay>,
) -> Result<(StatusCode, Json<PaymentResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok((
        StatusCode::CREATED,
        Json(
            service(&state)?
                .pay(
                    &auth.user,
                    card,
                    id,
                    PaymentInput {
                        request_id: body.request_id,
                        account_id: body.account_id,
                        movement: MovementInput {
                            description: body.description,
                            occurred_on: crate::http_cycles::date(&body.occurred_on)?,
                            amount_cents: body.amount_cents,
                        },
                    },
                )
                .await?
                .into(),
        ),
    ))
}
async fn payments(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((card, id)): Path<(Uuid, Uuid)>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<PaymentResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .payments(
                &auth.user,
                card,
                id,
                p.limit.unwrap_or(20),
                p.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn purchases(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((card, id)): Path<(Uuid, Uuid)>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<PurchaseResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .purchases(
                &auth.user,
                card,
                id,
                p.limit.unwrap_or(20),
                p.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
