use crate::{http_cycles::date, ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use obz_banking::{
    BankingError, CardCreditInput, CorrectionInput, CreditEntry, FinanceService, OpeningDebtInput,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/movements/{movement}/reverse", post(reverse))
        .route(
            "/api/cards/{card}/opening-debts",
            get(openings).post(opening),
        )
        .route("/api/cards/{card}/credits", get(credits).post(credit))
        .route("/api/overview", get(overview))
        .layer(DefaultBodyLimit::max(4096))
}
fn service(s: &AppState) -> Result<&FinanceService, ApiError> {
    s.finance.as_ref().ok_or(BankingError::Unavailable.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Correction {
    request_id: Uuid,
    description: String,
    occurred_on: String,
}
#[derive(Serialize)]
struct CorrectionResponse {
    id: Uuid,
    movement_id: Uuid,
    description: String,
    occurred_on: chrono::NaiveDate,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Opening {
    request_id: Uuid,
    invoice_id: Uuid,
    description: String,
    occurred_on: String,
    amount_cents: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credit {
    request_id: Uuid,
    invoice_id: Uuid,
    expense_id: Option<Uuid>,
    opening_debt_id: Option<Uuid>,
    description: String,
    occurred_on: String,
}
#[derive(Serialize)]
struct EntryResponse {
    id: Uuid,
    card_id: Uuid,
    invoice_id: Uuid,
    expense_id: Option<Uuid>,
    opening_debt_id: Option<Uuid>,
    description: String,
    occurred_on: chrono::NaiveDate,
    amount_cents: i64,
}
fn day(day: i32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_num_days_from_ce_opt(day).expect("validated finance date")
}
impl From<CreditEntry> for EntryResponse {
    fn from(r: CreditEntry) -> Self {
        Self {
            id: r.id,
            card_id: r.card_id,
            invoice_id: r.invoice_id,
            expense_id: r.expense_id,
            opening_debt_id: r.opening_debt_id,
            description: r.note.description().into(),
            occurred_on: day(r.note.day()),
            amount_cents: r.amount_cents,
        }
    }
}
async fn reverse(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(movement): Path<Uuid>,
    Json(b): Json<Correction>,
) -> Result<(StatusCode, Json<CorrectionResponse>), ApiError> {
    let r = service(&s)?
        .reverse(
            &auth.user,
            movement,
            CorrectionInput {
                request_id: b.request_id,
                description: b.description,
                occurred_on: date(&b.occurred_on)?,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CorrectionResponse {
            id: r.id,
            movement_id: r.movement_id,
            description: r.note.description().into(),
            occurred_on: day(r.note.day()),
        }),
    ))
}
async fn opening(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(card): Path<Uuid>,
    Json(b): Json<Opening>,
) -> Result<(StatusCode, Json<EntryResponse>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            service(&s)?
                .opening(
                    &auth.user,
                    card,
                    OpeningDebtInput {
                        request_id: b.request_id,
                        invoice_id: b.invoice_id,
                        description: b.description,
                        occurred_on: date(&b.occurred_on)?,
                        amount_cents: b.amount_cents,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
async fn credit(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(card): Path<Uuid>,
    Json(b): Json<Credit>,
) -> Result<(StatusCode, Json<EntryResponse>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            service(&s)?
                .credit(
                    &auth.user,
                    card,
                    CardCreditInput {
                        request_id: b.request_id,
                        invoice_id: b.invoice_id,
                        expense_id: b.expense_id,
                        opening_debt_id: b.opening_debt_id,
                        description: b.description,
                        occurred_on: date(&b.occurred_on)?,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
}
async fn entries(
    auth: Authenticated,
    s: AppState,
    card: Uuid,
    p: Page,
    opening: bool,
) -> Result<Json<Vec<EntryResponse>>, ApiError> {
    Ok(Json(
        service(&s)?
            .entries(
                &auth.user,
                card,
                opening,
                p.limit.unwrap_or(20),
                p.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn openings(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(card): Path<Uuid>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<EntryResponse>>, ApiError> {
    entries(auth, s, card, p, true).await
}
async fn credits(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(card): Path<Uuid>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<EntryResponse>>, ApiError> {
    entries(auth, s, card, p, false).await
}
#[derive(Serialize)]
struct Overview {
    cash_cents: i64,
    card_debt_cents: i64,
    card_credit_cents: i64,
    active_budget_remaining_cents: i64,
    funding_gap_cents: i64,
    uncommitted_cash_cents: i64,
    currency: &'static str,
}
async fn overview(
    auth: Authenticated,
    State(s): State<AppState>,
) -> Result<Json<Overview>, ApiError> {
    let r = service(&s)?.overview(&auth.user).await?;
    Ok(Json(Overview {
        cash_cents: r.cash_cents,
        card_debt_cents: r.card_debt_cents,
        card_credit_cents: r.card_credit_cents,
        active_budget_remaining_cents: r.active_budget_remaining_cents,
        funding_gap_cents: r.funding_gap_cents,
        uncommitted_cash_cents: r.uncommitted_cash_cents,
        currency: "BRL",
    }))
}
