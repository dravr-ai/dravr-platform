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
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_core::models::{CoverageMap, TenantId};
use pierre_middleware::extract_auth_from_headers;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::{about_you, onboarding_gate, parq};

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
    /// Durable per-step onboarding progress. Only steps the user has reached
    /// appear; a step absent here is still pending. Lets the flow be
    /// server-driven and survive device changes rather than `localStorage`-only.
    pub steps: Vec<OnboardingStepState>,
    /// The messaging channel the user chose during onboarding, if any.
    pub chosen_channel: Option<String>,
}

/// A single onboarding step's persisted status, for the client progress model.
#[derive(Debug, Serialize)]
pub struct OnboardingStepState {
    /// Step id (`profile_type`, `connect_provider`, `coach_proposal`,
    /// `messaging_channel`, `messaging_configure`).
    pub step_id: String,
    /// `complete` or `skipped`.
    pub status: String,
}

/// Total onboarding topics: the North Star plus the six pillars.
const ONBOARDING_TOPIC_TOTAL: usize = 7;

/// The onboarding step ids the `PUT` endpoint accepts, kept in sync with the
/// client step registry (`frontend/src/onboarding/steps.ts`).
const ONBOARDING_STEP_IDS: [&str; 7] = [
    "profile_type",
    "about_you",
    "parq",
    "connect_provider",
    "coach_proposal",
    "messaging_channel",
    "messaging_configure",
];

/// The statuses a step may be set to. A missing row means "pending".
const ONBOARDING_STATUSES: [&str; 2] = ["complete", "skipped"];

/// Max length for a `chosen_channel` value (channel names are short slugs).
const MAX_CHOSEN_CHANNEL_LEN: usize = 32;

/// A `chosen_channel` value must look like a channel slug: non-empty, lowercase
/// ASCII alphanumerics and underscores only. Anything else (markup, whitespace,
/// mixed case) is rejected so the stored value stays inert — defense in depth,
/// since it is only ever echoed back to its own authenticated owner.
fn is_channel_slug(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

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

    // Durable per-step onboarding state. Best-effort like pillar coverage: a read
    // error degrades to "no persisted steps" rather than failing the routing gate.
    let step_records = resources
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(&auth.user_id.to_string())
        .await
        .unwrap_or_default();
    let chosen_channel = step_records.iter().find_map(|r| r.chosen_channel.clone());
    let steps = step_records
        .into_iter()
        .map(|r| OnboardingStepState {
            step_id: r.step_id,
            status: r.status,
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(OnboardingStatusResponse {
            needs_provider_connection: !has_provider,
            pillars_covered,
            pillars_total: ONBOARDING_TOPIC_TOTAL,
            onboarding_complete,
            steps,
            chosen_channel,
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

/// Request body for `POST /api/me/about-you`.
///
/// Every field is optional: the step is skippable, and a partial answer is worth
/// more than none.
#[derive(Debug, Deserialize)]
pub struct AboutYouRequest {
    /// Why they train — the life motivation the pillars orient around.
    #[serde(default)]
    pub north_star: Option<String>,
    /// Primary sport.
    #[serde(default)]
    pub primary_sport: Option<String>,
    /// What they are working toward.
    #[serde(default)]
    pub goal: Option<String>,
}

/// Response body for `POST /api/me/about-you`.
#[derive(Debug, Serialize)]
pub struct AboutYouResponse {
    /// How many facts were actually written (empty answers are dropped).
    pub facts_written: u64,
}

/// `POST /api/me/about-you` — persist the about-you answers as onboarding facts.
///
/// These are the inputs `build_coach_proposal` already reads and currently never
/// finds: without them the coach proposal falls back to sport-mix, and on a
/// first-run connection there is no sport-mix either.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, no active tenant is present, or
/// persisting a fact fails.
pub async fn handle_about_you_post(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(req): Json<AboutYouRequest>,
) -> Result<Response, AppError> {
    let tenant_id = active_tenant(&auth)?;
    let answers = about_you::AboutYouAnswers {
        north_star: req.north_star,
        primary_sport: req.primary_sport,
        goal: req.goal,
    };
    let facts_written = about_you::persist_about_you(
        resources.common.repos.memory.as_ref(),
        tenant_id,
        &auth.user_id.to_string(),
        &answers,
    )
    .await?;

    Ok((StatusCode::OK, Json(AboutYouResponse { facts_written })).into_response())
}

/// Request body for `PUT /api/me/onboarding/steps/{step_id}`.
#[derive(Debug, Deserialize)]
pub struct SetOnboardingStepRequest {
    /// `complete` or `skipped`.
    pub status: String,
    /// For the `messaging_channel` step: the messaging app the user chose.
    #[serde(default)]
    pub chosen_channel: Option<String>,
}

/// `PUT /api/me/onboarding/steps/{step_id}` — persist a step's completion status.
///
/// Durable, server-side onboarding progress: the web/mobile flows call this as
/// each step completes so onboarding follows the user across devices. Scoped by
/// the authenticated `user_id`.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, the step id or status is not
/// recognised, `chosen_channel` is too long, or persistence fails.
pub async fn handle_step_put(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(step_id): Path<String>,
    Json(req): Json<SetOnboardingStepRequest>,
) -> Result<Response, AppError> {
    if !ONBOARDING_STEP_IDS.contains(&step_id.as_str()) {
        return Err(AppError::invalid_input(format!(
            "unknown onboarding step id: {step_id}"
        )));
    }
    if !ONBOARDING_STATUSES.contains(&req.status.as_str()) {
        return Err(AppError::invalid_input(format!(
            "invalid onboarding status: {}",
            req.status
        )));
    }
    let chosen_channel = match req.chosen_channel.as_deref() {
        None | Some("") => None,
        Some(c) if c.len() > MAX_CHOSEN_CHANNEL_LEN => {
            return Err(AppError::invalid_input(
                "chosen_channel exceeds the maximum length",
            ));
        }
        Some(c) if !is_channel_slug(c) => {
            return Err(AppError::invalid_input(
                "chosen_channel must be a lowercase channel slug (a-z, 0-9, underscore)",
            ));
        }
        Some(c) => Some(c),
    };
    let tenant_id = auth.active_tenant_id.map(|t| t.to_string());
    resources
        .common
        .repos
        .user_onboarding
        .set_onboarding_step(
            &auth.user_id.to_string(),
            &step_id,
            &req.status,
            chosen_channel,
            tenant_id.as_deref(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Mount-helper for the onboarding-status endpoint. Same shape as
/// [`pierre_routes_admin::FeatureFlagsRoutes`].
pub struct OnboardingRoutes;

impl OnboardingRoutes {
    /// User-facing route. Requires a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/onboarding-status", get(handle_self_get))
            .route("/api/me/onboarding/steps/{step_id}", put(handle_step_put))
            .route("/api/me/parq", get(handle_parq_get).post(handle_parq_post))
            .route("/api/me/about-you", post(handle_about_you_post))
            .with_state(resources)
    }
}
