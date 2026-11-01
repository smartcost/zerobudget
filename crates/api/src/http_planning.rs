use crate::{
    http_cycles::{date, CycleResponse},
    ApiError, AppState, Authenticated,
};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use obz_cycle::{CopyInput, CycleError, PlanningService, Reallocation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/cycles/{cycle}/reallocations",
            get(list).post(move_money),
        )
        .route("/api/cycles/{cycle}/next", post(next))
        .route("/api/cycles/{cycle}/summary", get(summary))
        .layer(DefaultBodyLimit::max(4096))
}
fn service(s: &AppState) -> Result<&PlanningService, ApiError> {
    s.planning.as_ref().ok_or(CycleError::Unavailable.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    request_id: Uuid,
    from_envelope_id: Uuid,
    to_envelope_id: Uuid,
    amount_cents: i64,
}
#[derive(Serialize)]
struct MoveResponse {
    id: Uuid,
    cycle_id: Uuid,
    from_envelope_id: Uuid,
    to_envelope_id: Uuid,
    amount_cents: i64,
}
impl From<Reallocation> for MoveResponse {
    fn from(r: Reallocation) -> Self {
        Self {
            id: r.id,
            cycle_id: r.cycle_id,
            from_envelope_id: r.from_envelope_id,
            to_envelope_id: r.to_envelope_id,
            amount_cents: r.amount_cents,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Copy {
    request_id: Uuid,
    period_start: String,
    period_end: String,
    new_income_cents: i64,
    mode: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
}
async fn move_money(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(cycle): Path<Uuid>,
    Json(b): Json<Move>,
) -> Result<(StatusCode, Json<MoveResponse>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            service(&s)?
                .reallocate(
                    &auth.user,
                    Reallocation {
                        id: b.request_id,
                        cycle_id: cycle,
                        from_envelope_id: b.from_envelope_id,
                        to_envelope_id: b.to_envelope_id,
                        amount_cents: b.amount_cents,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
async fn list(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(cycle): Path<Uuid>,
    Query(p): Query<Page>,
) -> Result<Json<Vec<MoveResponse>>, ApiError> {
    Ok(Json(
        service(&s)?
            .reallocations(
                &auth.user,
                cycle,
                p.limit.unwrap_or(20),
                p.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn next(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(cycle): Path<Uuid>,
    Json(b): Json<Copy>,
) -> Result<(StatusCode, Json<CycleResponse>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            service(&s)?
                .copy(
                    &auth.user,
                    cycle,
                    CopyInput {
                        request_id: b.request_id,
                        period_start: date(&b.period_start)?,
                        period_end: date(&b.period_end)?,
                        new_income_cents: b.new_income_cents,
                        mode: b.mode,
                    },
                )
                .await?
                .into(),
        ),
    ))
}
#[derive(Serialize)]
struct EnvelopeSummary {
    id: Uuid,
    name: String,
    allocated_cents: i64,
    spent_cents: i64,
    remaining_cents: i64,
}
#[derive(Serialize)]
struct Summary {
    cycle: CycleResponse,
    spent_cents: i64,
    remaining_cents: i64,
    received_cents: i64,
    projected_income_gap_cents: i64,
    envelopes: Vec<EnvelopeSummary>,
}
async fn summary(
    auth: Authenticated,
    State(s): State<AppState>,
    Path(cycle): Path<Uuid>,
) -> Result<Json<Summary>, ApiError> {
    let r = service(&s)?.summary(&auth.user, cycle).await?;
    Ok(Json(Summary {
        cycle: r.cycle.into(),
        spent_cents: r.spent_cents,
        remaining_cents: r.remaining_cents,
        received_cents: r.received_cents,
        projected_income_gap_cents: r.projected_income_gap_cents,
        envelopes: r
            .envelopes
            .into_iter()
            .map(|e| EnvelopeSummary {
                id: e.id,
                name: e.name,
                allocated_cents: e.allocated_cents,
                spent_cents: e.spent_cents,
                remaining_cents: e.remaining_cents,
            })
            .collect(),
    }))
}
