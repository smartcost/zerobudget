use crate::{ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use obz_cycle::{BudgetExpense, CycleError, ExpenseService};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/cycles/{cycle}/envelopes/{envelope}/expenses",
            get(list).post(create),
        )
        .route(
            "/api/cycles/{cycle}/envelopes/{envelope}/expenses/{expense}/reverse",
            post(reverse),
        )
        .layer(DefaultBodyLimit::max(4096))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
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
struct ExpenseResponse {
    id: Uuid,
    cycle_id: Uuid,
    envelope_id: Uuid,
    description: String,
    occurred_on: chrono::NaiveDate,
    amount_cents: i64,
    status: &'static str,
}
impl From<BudgetExpense> for ExpenseResponse {
    fn from(r: BudgetExpense) -> Self {
        Self {
            id: r.id,
            cycle_id: r.cycle_id,
            envelope_id: r.envelope_id,
            description: r.expense.description().into(),
            occurred_on: chrono::NaiveDate::from_num_days_from_ce_opt(r.expense.day())
                .expect("validated domain expense date"),
            amount_cents: r.expense.amount().to_cents(),
            status: if r.reversed { "reversed" } else { "posted" },
        }
    }
}
fn service(state: &AppState) -> Result<&ExpenseService, ApiError> {
    state
        .expenses
        .as_ref()
        .ok_or(CycleError::Unavailable.into())
}
async fn create(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((cycle, envelope)): Path<(Uuid, Uuid)>,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<ExpenseResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    let record = service(&state)?
        .create(
            &auth.user,
            cycle,
            envelope,
            &body.description,
            crate::http_cycles::date(&body.occurred_on)?,
            body.amount_cents,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(record.into())))
}
async fn list(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((cycle, envelope)): Path<(Uuid, Uuid)>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<ExpenseResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .list(
                &auth.user,
                cycle,
                envelope,
                page.limit.unwrap_or(20),
                page.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn reverse(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((cycle, envelope, id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Json<ExpenseResponse>, ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok(Json(
        service(&state)?
            .reverse(&auth.user, cycle, envelope, id)
            .await?
            .into(),
    ))
}
