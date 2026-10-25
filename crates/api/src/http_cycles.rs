use crate::{ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::NaiveDate;
use obz_cycle::{BudgetCycle, CycleError, CycleService};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/cycles", get(list).post(create))
        .route("/api/cycles/{id}", get(find))
        .layer(DefaultBodyLimit::max(4096))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    period_start: String,
    period_end: String,
    income_cents: i64,
}
#[derive(Serialize)]
struct CycleResponse {
    id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    income_cents: i64,
    allocated_cents: i64,
    available_cents: i64,
    currency: &'static str,
    status: &'static str,
}
impl From<BudgetCycle> for CycleResponse {
    fn from(c: BudgetCycle) -> Self {
        Self {
            id: c.id,
            period_start: c.period_start,
            period_end: c.period_end,
            income_cents: c.income_cents,
            allocated_cents: c.allocated_cents,
            available_cents: c.available_cents,
            currency: "BRL",
            status: "planned",
        }
    }
}
impl From<CycleError> for ApiError {
    fn from(e: CycleError) -> Self {
        match e {
            CycleError::Invalid => Self(StatusCode::UNPROCESSABLE_ENTITY, "invalid budget input"),
            CycleError::DuplicateEnvelope => Self(
                StatusCode::CONFLICT,
                "envelope name already exists in this cycle",
            ),
            CycleError::Overallocated => {
                Self(StatusCode::CONFLICT, "allocation exceeds cycle income")
            }
            CycleError::Forbidden => Self(StatusCode::FORBIDDEN, "permission denied"),
            CycleError::NotFound => Self(StatusCode::NOT_FOUND, "budget resource not found"),
            CycleError::Conflict => {
                Self(StatusCode::CONFLICT, "cycle already exists for this period")
            }
            CycleError::Unavailable => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "budget storage unavailable",
            ),
        }
    }
}
fn repository(state: &AppState) -> Result<&CycleService, ApiError> {
    state.cycles.as_ref().ok_or(CycleError::Unavailable.into())
}
fn date(value: &str) -> Result<NaiveDate, ApiError> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| CycleError::Invalid)?;
    if value.len() != 10 || date.format("%Y-%m-%d").to_string() != value {
        return Err(CycleError::Invalid.into());
    }
    Ok(date)
}
async fn create(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<CycleResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    let cycle = repository(&state)?
        .create(
            &auth.user,
            date(&body.period_start)?,
            date(&body.period_end)?,
            body.income_cents,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(cycle.into())))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
}
async fn list(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<CycleResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        repository(&state)?
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
) -> Result<Json<CycleResponse>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(repository(&state)?.get(&auth.user, id).await?.into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_are_strict_civil_dates() {
        for s in [
            "2026-1-01",
            "2026-02-30",
            "+2026-01-01",
            "2026-01-01T00:00:00Z",
        ] {
            assert!(date(s).is_err());
        }
        assert!(date("2028-02-29").is_ok());
    }
}
