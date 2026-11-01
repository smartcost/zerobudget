use crate::{ApiError, AppState, Authenticated};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use obz_auth::{OnboardingError, OnboardingService, OnboardingStatus, Permission};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/onboarding", get(status).post(complete))
        .layer(DefaultBodyLimit::max(4096))
}
impl From<OnboardingError> for ApiError {
    fn from(e: OnboardingError) -> Self {
        match e {
            OnboardingError::Forbidden => Self(StatusCode::FORBIDDEN, "verified identity required"),
            OnboardingError::InvalidIdentity => Self(StatusCode::UNAUTHORIZED, "sign in again"),
            OnboardingError::Disabled => Self(StatusCode::FORBIDDEN, "user account disabled"),
            OnboardingError::Managed => {
                Self(StatusCode::CONFLICT, "account access managed by operator")
            }
            OnboardingError::Unavailable => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "onboarding storage unavailable",
            ),
        }
    }
}
fn service(s: &AppState) -> Result<&OnboardingService, ApiError> {
    s.onboarding
        .as_ref()
        .ok_or(OnboardingError::Unavailable.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Complete {}
#[derive(Serialize)]
struct SelectedCycle {
    id: Uuid,
    status: &'static str,
    has_envelopes: bool,
    has_unallocated_income: bool,
}
#[derive(Serialize)]
struct Setup {
    has_accounts: bool,
    has_cycles: bool,
    has_active_cycle: bool,
    has_expenses: bool,
    selected_cycle: Option<SelectedCycle>,
}
#[derive(Serialize)]
struct Response {
    status: &'static str,
    completed_at: Option<i64>,
    permissions: Vec<Permission>,
    setup: Option<Setup>,
    next_step: &'static str,
}
impl From<OnboardingStatus> for Response {
    fn from(s: OnboardingStatus) -> Self {
        let next = s.next_step().as_str();
        Self {
            status: s.state.as_str(),
            completed_at: s.completed_at,
            permissions: s.permissions.into_iter().collect(),
            setup: s.setup.map(|p| Setup {
                has_accounts: p.has_accounts,
                has_cycles: p.has_cycles,
                has_active_cycle: p.has_active_cycle,
                has_expenses: p.has_expenses,
                selected_cycle: p.selected_cycle.map(|c| SelectedCycle {
                    id: c.id,
                    status: c.status.as_str(),
                    has_envelopes: c.has_envelopes,
                    has_unallocated_income: c.has_unallocated_income,
                }),
            }),
            next_step: next,
        }
    }
}
async fn status(
    auth: Authenticated,
    State(s): State<AppState>,
) -> Result<Json<Response>, ApiError> {
    Ok(Json(
        service(&s)?
            .status(&auth.identity, &auth.user)
            .await?
            .into(),
    ))
}
async fn complete(
    auth: Authenticated,
    State(s): State<AppState>,
    Json(_body): Json<Complete>,
) -> Result<Json<Response>, ApiError> {
    Ok(Json(
        service(&s)?
            .complete(&auth.identity, &auth.user)
            .await?
            .into(),
    ))
}
