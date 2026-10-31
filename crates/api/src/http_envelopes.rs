use crate::{ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
    Json, Router,
};
use obz_cycle::{BudgetEnvelope, CycleError, EnvelopeService};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/cycles/{cycle}/envelopes", get(list).post(create))
        .route(
            "/api/cycles/{cycle}/envelopes/{envelope}",
            patch(update).delete(delete),
        )
        .layer(DefaultBodyLimit::max(4096))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    allocated_cents: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    #[serde(default, deserialize_with = "present")]
    name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    allocated_cents: Option<i64>,
}
// An omitted field is unchanged. Explicit null is invalid, preventing ambiguous patches.
fn present<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
}
#[derive(Serialize)]
struct EnvelopeResponse {
    id: Uuid,
    cycle_id: Uuid,
    name: String,
    allocated_cents: i64,
    spent_cents: i64,
    remaining_cents: i64,
}
impl From<BudgetEnvelope> for EnvelopeResponse {
    fn from(record: BudgetEnvelope) -> Self {
        Self {
            id: record.id,
            cycle_id: record.cycle_id,
            name: record.envelope.name().as_str().into(),
            allocated_cents: record.envelope.allocated().to_cents(),
            spent_cents: record.spending.spent().to_cents(),
            remaining_cents: record.spending.remaining().to_cents(),
        }
    }
}
fn service(state: &AppState) -> Result<&EnvelopeService, ApiError> {
    state
        .envelopes
        .as_ref()
        .ok_or(CycleError::Unavailable.into())
}
async fn create(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(cycle): Path<Uuid>,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<EnvelopeResponse>), ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    let record = service(&state)?
        .create(&auth.user, cycle, &body.name, body.allocated_cents)
        .await?;
    Ok((StatusCode::CREATED, Json(record.into())))
}
async fn list(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(cycle): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<EnvelopeResponse>>, ApiError> {
    auth.require(obz_auth::Permission::ReadBudget)?;
    Ok(Json(
        service(&state)?
            .list(
                &auth.user,
                cycle,
                page.limit.unwrap_or(20),
                page.offset.unwrap_or(0),
            )
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
async fn update(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((cycle, id)): Path<(Uuid, Uuid)>,
    Json(body): Json<Patch>,
) -> Result<Json<EnvelopeResponse>, ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    Ok(Json(
        service(&state)?
            .update(
                &auth.user,
                cycle,
                id,
                body.name.as_deref(),
                body.allocated_cents,
            )
            .await?
            .into(),
    ))
}
async fn delete(
    auth: Authenticated,
    State(state): State<AppState>,
    Path((cycle, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    auth.require(obz_auth::Permission::WriteBudget)?;
    service(&state)?.delete(&auth.user, cycle, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
