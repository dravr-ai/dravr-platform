// ABOUTME: HTTP boundary for the onboarding-status check — single cheap call frontends use to gate routing
// ABOUTME: Returns {needs_provider_connection: bool} based on services::onboarding_gate as source of truth
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for onboarding state.
//!
//! - `GET /api/me/onboarding-status` — single cheap call that returns
//!   `{needs_provider_connection: bool}`. Web and mobile clients call this
//!   right after login to decide whether to render the onboarding screen or
//!   route to the main UI.
//!
//! Logic lives in [`pierre_services::onboarding_gate`]; this module is the
//! thin HTTP boundary.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_core::models::{CoverageMap, TenantId};
use pierre_middleware::extract_auth_from_headers;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::{onboarding_gate, parq};

/// Response body for `GET /api/me/onboarding-status`.
///
/// `needs_provider_connection` is `true` when the caller has no *real*
/// (non-`Synthetic`) row in `provider_connections` and should be routed to the
/// connect-a-provider onboarding screen. Synthetic seed/demo connections do not
/// count here — a demo user still needs to link a real provider.
#[derive(Debug, Serialize)]
pub struct OnboardingStatusResponse {
    /// `true` ⇒ frontend should redirect to the onboarding screen.
    pub needs_provider_connection: bool,
    /// How many onboarding topics (North Star + 6 pillars) the user has covered.
    pub pillars_covered: usize,
    /// Total onboarding topics (North Star + 6 pillars = 7).
    pub pillars_total: usize,
    /// `true` once all pillar context + North Star are captured.
    pub onboarding_complete: bool,
}

/// Total onboarding topics: the North Star plus the six pillars.
const ONBOARDING_TOPIC_TOTAL: usize = 7;

/// Best-effort (covered, complete) pillar coverage from the user's dossier.
/// Returns `(0, false)` when there is no active tenant or the dossier cannot be
/// composed — onboarding-status must never fail the routing gate.
async fn pillar_coverage(
    resources: &Arc<ServerContext>,
    tenant: Option<uuid::Uuid>,
    user_id: uuid::Uuid,
) -> (usize, bool) {
    let Some(tenant_uuid) = tenant else {
        return (0, false);
    };
    let tenant = TenantId::from_uuid(tenant_uuid);
    let Ok(dossier) = resources
        .common
        .repos
        .dossier
        .compose_dossier(tenant, user_id)
        .await
    else {
        return (0, false);
    };
    let coverage = CoverageMap::from_dossier(&dossier);
    (coverage.covered_count(), coverage.is_complete())
}

/// `GET /api/me/onboarding-status`
///
/// # Errors
///
/// Returns `AppError` when authentication fails or the provider-connections
/// query errors.
pub async fn handle_self_get(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let has_provider = onboarding_gate::user_has_real_provider(
        &resources.common.repos.provider_connections,
        auth.user_id,
    )
    .await?;

    // Best-effort pillar coverage (D8: onboarding-status reflects coverage-map
    // state). Reads as zero when there is no active tenant or the dossier can't
    // be composed — never fails the routing gate.
    let (pillars_covered, onboarding_complete) =
        pillar_coverage(&resources, auth.active_tenant_id, auth.user_id).await;

    Ok((
        StatusCode::OK,
        Json(OnboardingStatusResponse {
            needs_provider_connection: !has_provider,
            pillars_covered,
            pillars_total: ONBOARDING_TOPIC_TOTAL,
            onboarding_complete,
        }),
    )
        .into_response())
}

/// A PAR-Q+ question for the structured medical-safety screen.
#[derive(Debug, Serialize)]
pub struct ParqQuestionDto {
    /// Stable identifier echoed back in the answers payload.
    pub id: String,
    /// Question text.
    pub text: String,
}

/// Response body for `GET /api/me/parq`.
#[derive(Debug, Serialize)]
pub struct ParqQuestionsResponse {
    /// The seven PAR-Q+ questions.
    pub questions: Vec<ParqQuestionDto>,
}

/// A single Yes/No answer in a PAR-Q submission.
#[derive(Debug, Deserialize)]
pub struct ParqAnswer {
    /// Question id (from `GET /api/me/parq`).
    pub id: String,
    /// Whether the user answered "yes".
    pub yes: bool,
}

/// Request body for `POST /api/me/parq`.
#[derive(Debug, Deserialize)]
pub struct ParqAnswersRequest {
    /// Answers to the PAR-Q questions.
    pub answers: Vec<ParqAnswer>,
}

/// Response body for `POST /api/me/parq`.
#[derive(Debug, Serialize)]
pub struct ParqSubmitResponse {
    /// Number of medical flags raised (one per "yes").
    pub flags_raised: u64,
}

fn active_tenant(auth: &AuthenticatedUser) -> Result<TenantId, AppError> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| {
            AppError::auth_invalid("onboarding endpoints require an active tenant in the JWT")
        })
}

/// `GET /api/me/parq` — the structured PAR-Q+ questions.
///
/// # Errors
///
/// Returns `AppError` when authentication fails.
pub async fn handle_parq_get(_auth: AuthenticatedUser) -> Result<Response, AppError> {
    // The extractor enforces auth; the question set has no per-user variation.
    let questions = parq::PARQ_QUESTIONS
        .iter()
        .map(|q| ParqQuestionDto {
            id: q.id.to_owned(),
            text: q.text.to_owned(),
        })
        .collect();
    Ok((StatusCode::OK, Json(ParqQuestionsResponse { questions })).into_response())
}

/// `POST /api/me/parq` — submit PAR-Q answers; each "yes" raises a coach-visible
/// medical flag. A "yes" never blocks sign-up.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, no active tenant is present, or
/// persisting a flag fails.
pub async fn handle_parq_post(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(req): Json<ParqAnswersRequest>,
) -> Result<Response, AppError> {
    let tenant_id = active_tenant(&auth)?;
    let yes_ids: Vec<String> = req
        .answers
        .into_iter()
        .filter(|a| a.yes)
        .map(|a| a.id)
        .collect();
    let flags_raised = parq::persist_parq_flags(
        resources.common.repos.memory.as_ref(),
        tenant_id,
        &auth.user_id.to_string(),
        &yes_ids,
    )
    .await?;
    Ok((StatusCode::OK, Json(ParqSubmitResponse { flags_raised })).into_response())
}

/// Mount-helper for the onboarding-status endpoint. Same shape as
/// [`pierre_routes_admin::FeatureFlagsRoutes`].
pub struct OnboardingRoutes;

impl OnboardingRoutes {
    /// User-facing route. Requires a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/onboarding-status", get(handle_self_get))
            .route("/api/me/parq", get(handle_parq_get).post(handle_parq_post))
            .with_state(resources)
    }
}
