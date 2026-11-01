use crate::{ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use obz_banking::{
    AccountInput, BankAccount, BankMovement, BankingError, BankingService, MovementInput,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/accounts", get(list).post(create))
        .route("/api/accounts/{id}", get(find))
        .route("/api/accounts/{id}/receipts", post(receipt))
        .route("/api/accounts/{id}/movements", get(statement))
        .route("/api/transfers", post(transfer))
        .layer(DefaultBodyLimit::max(4096))
}
impl From<BankingError> for ApiError {
    fn from(e: BankingError) -> Self {
        match e {
            BankingError::Invalid => {
                Self(StatusCode::UNPROCESSABLE_ENTITY, "invalid banking input")
            }
            BankingError::Forbidden => Self(StatusCode::FORBIDDEN, "permission denied"),
            BankingError::NotFound => Self(StatusCode::NOT_FOUND, "banking resource not found"),
            BankingError::Duplicate => Self(StatusCode::CONFLICT, "account name already exists"),
            BankingError::InsufficientFunds => {
                Self(StatusCode::CONFLICT, "insufficient account balance")
            }
            BankingError::Overflow => Self(
                StatusCode::CONFLICT,
                "account balance exceeds integer limit",
            ),
            BankingError::CreditConflict => Self(StatusCode::CONFLICT, "credit resource conflict"),
            BankingError::InvalidState => Self(
                StatusCode::CONFLICT,
                "operation not allowed in invoice state",
            ),
            BankingError::CreditLimit => Self(StatusCode::CONFLICT, "credit limit exceeded"),
            BankingError::Overpayment => {
                Self(StatusCode::CONFLICT, "payment exceeds invoice balance")
            }
            BankingError::Unavailable => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "banking storage unavailable",
            ),
        }
    }
}
fn service(state: &AppState) -> Result<&BankingService, ApiError> {
    state
        .banking
        .as_ref()
        .ok_or(BankingError::Unavailable.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    kind: String,
    opened_on: String,
    opening_balance_cents: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Movement {
    request_id: Uuid,
    description: String,
    occurred_on: String,
    amount_cents: i64,
}
impl Movement {
    fn input(self) -> Result<MovementInput, ApiError> {
        Ok(MovementInput {
            description: self.description,
            occurred_on: crate::http_cycles::date(&self.occurred_on)?,
            amount_cents: self.amount_cents,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transfer {
    request_id: Uuid,
    from_account_id: Uuid,
    to_account_id: Uuid,
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
struct AccountResponse {
    id: Uuid,
    name: String,
    kind: &'static str,
    opened_on: chrono::NaiveDate,
    opening_balance_cents: i64,
    balance_cents: i64,
    currency: &'static str,
}
impl From<BankAccount> for AccountResponse {
    fn from(a: BankAccount) -> Self {
        Self {
            id: a.id,
            name: a.account.name().into(),
            kind: a.account.kind().as_str(),
            opened_on: chrono::NaiveDate::from_num_days_from_ce_opt(a.account.opened_day())
                .expect("validated account date"),
            opening_balance_cents: a.account.opening().to_cents(),
            balance_cents: a.balance.to_cents(),
            currency: "BRL",
        }
    }
}
#[derive(Serialize)]
struct MovementResponse {
    id: Uuid,
    kind: String,
    from_account_id: Option<Uuid>,
    to_account_id: Option<Uuid>,
    expense_id: Option<Uuid>,
    card_payment_id: Option<Uuid>,
    adjustment_id: Option<Uuid>,
    status: &'static str,
    description: String,
    occurred_on: chrono::NaiveDate,
    amount_cents: i64,
}
impl From<BankMovement> for MovementResponse {
    fn from(m: BankMovement) -> Self {
        Self {
            id: m.id,
            kind: m.kind,
            from_account_id: m.from_account_id,
            to_account_id: m.to_account_id,
            expense_id: m.expense_id,
            card_payment_id: m.card_payment_id,
            adjustment_id: m.adjustment_id,
            status: if m.reversed { "reversed" } else { "posted" },
            description: m.movement.description().into(),
            occurred_on: chrono::NaiveDate::from_num_days_from_ce_opt(m.movement.day())
                .expect("validated movement date"),
            amount_cents: m.movement.amount().to_cents(),
        }
    }
}
#[derive(Serialize)]
struct StatementResponse {
    #[serde(flatten)]
    movement: MovementResponse,
    delta_cents: i64,
}
async fn create(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<AccountResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    let input = AccountInput {
        name: body.name,
        kind: body.kind,
        opened_on: crate::http_cycles::date(&body.opened_on)?,
        opening_balance_cents: body.opening_balance_cents,
    };
    Ok((
        StatusCode::CREATED,
        Json(service(&state)?.create(&auth.user, input).await?.into()),
    ))
}
async fn list(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .list(
                &auth.user,
                page.limit.unwrap_or(20),
                page.offset.unwrap_or(0),
            )
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
) -> Result<Json<AccountResponse>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(service(&state)?.get(&auth.user, id).await?.into()))
}
async fn receipt(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Movement>,
) -> Result<(StatusCode, Json<MovementResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok((
        StatusCode::CREATED,
        Json(
            service(&state)?
                .post_with_id(&auth.user, body.request_id, None, id, body.input()?)
                .await?
                .into(),
        ),
    ))
}
async fn transfer(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(body): Json<Transfer>,
) -> Result<(StatusCode, Json<MovementResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    let input = MovementInput {
        description: body.description,
        occurred_on: crate::http_cycles::date(&body.occurred_on)?,
        amount_cents: body.amount_cents,
    };
    Ok((
        StatusCode::CREATED,
        Json(
            service(&state)?
                .post_with_id(
                    &auth.user,
                    body.request_id,
                    Some(body.from_account_id),
                    body.to_account_id,
                    input,
                )
                .await?
                .into(),
        ),
    ))
}
async fn statement(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<StatementResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .statement(
                &auth.user,
                id,
                page.limit.unwrap_or(20),
                page.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(|e| StatementResponse {
                movement: e.movement.into(),
                delta_cents: e.delta_cents,
            })
            .collect(),
    ))
}
