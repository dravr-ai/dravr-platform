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
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;
#[cfg(feature = "client-chat")]
use crate::routes::chat::welcome_onboarding_lead;
#[cfg(feature = "client-chat")]
use crate::routes::getting_started::get_getting_started;
use crate::services::archive_import::ops as archive_ops;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::extract_auth_from_headers;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::intake::{self, INTAKE_TOPICS};
use pierre_services::locale::resolve_user_locale;
use pierre_services::{about_you, athlete_state, onboarding_gate, parq};

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
    /// The user answered "I coach others" (`users.coaches_others`, which both
    /// coach choices record) — the group step is part of their journey.
    pub coaches_others: bool,
}

/// A single onboarding step's persisted status, for the client progress model.
#[derive(Debug, Serialize)]
pub struct OnboardingStepState {
    /// Step id (`profile_type`, `connect_provider`, `agent_proposal`,
    /// `messaging_channel`, `messaging_configure`).
    pub step_id: String,
    /// `complete`, `skipped` or `not_applicable`.
    pub status: String,
}

/// Total onboarding topics: the North Star plus the six pillars.
const ONBOARDING_TOPIC_TOTAL: usize = 7;

/// The onboarding step ids the `PUT` endpoint accepts, kept in sync with the
/// client step registry (`frontend/src/onboarding/steps.ts`).
const ONBOARDING_STEP_IDS: [&str; 9] = [
    "profile_type",
    "about_you",
    "parq",
    "connect_provider",
    "coach_proposal",
    "coach_group",
    intake::STEP_CHECK_IN,
    "messaging_channel",
    "messaging_configure",
];

/// The onboarding step where a coach creates their group.
const COACH_GROUP_STEP_ID: &str = "coach_group";

/// The statuses a step may be set to. A missing row means "pending".
///
/// `not_applicable` marks a step outside this person's journey — the athlete
/// steps for a coach who does not train (see
/// [`pierre_services::intake::STATUS_NOT_APPLICABLE`]).
const ONBOARDING_STATUSES: [&str; 3] = [
    intake::STATUS_COMPLETE,
    intake::STATUS_SKIPPED,
    intake::STATUS_NOT_APPLICABLE,
];

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

/// Best-effort (covered, complete) pillar coverage from the user's dossier,
/// through the athlete-state reader the use-case starters share (carnet#828).
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
    athlete_state::dossier_coverage(
        &resources.common.repos,
        TenantId::from_uuid(tenant_uuid),
        user_id,
    )
    .await
    .map_or((0, false), |coverage| (coverage.covered, coverage.complete))
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
    .await?
        || imports_own_history(&resources, auth.active_tenant_id, auth.user_id).await;

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

    // Best-effort like the rest: an unreadable user reads as an athlete,
    // which only leaves the group step out.
    let coaches_others = resources
        .common
        .repos
        .users
        .coaches_others(auth.user_id)
        .await
        .unwrap_or(false);

    Ok((
        StatusCode::OK,
        Json(OnboardingStatusResponse {
            needs_provider_connection: !has_provider,
            pillars_covered,
            pillars_total: ONBOARDING_TOPIC_TOTAL,
            onboarding_complete,
            steps,
            chosen_channel,
            coaches_others,
        }),
    )
        .into_response())
}

/// A PAR-Q+ question for the structured medical-safety screen.
#[derive(Debug, Serialize)]
pub struct ParqQuestionDto {
    /// Stable identifier echoed back in the answers payload.
    pub id: String,
    /// Question text, in the caller's locale.
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

/// The caller's active tenant, which every onboarding answer — the check-in
/// cadence among them — is stored under.
pub(super) fn active_tenant(auth: &AuthenticatedUser) -> Result<TenantId, AppError> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| {
            AppError::auth_invalid("onboarding endpoints require an active tenant in the JWT")
        })
}

/// `GET /api/me/parq` — the structured PAR-Q+ questions, in the caller's
/// language.
///
/// The question set never varies per athlete; its language does. The text is
/// the same five-locale registry string the messaging intake asks with, so an
/// athlete screened on the web and one screened over Telegram read the same
/// instrument — and the ids the client echoes back are locale-independent.
///
/// # Errors
///
/// Returns `AppError` when authentication fails.
pub async fn handle_parq_get(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    // The stored preference, resolved the way every REST read resolves it.
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), auth.user_id).await;
    let registry = &resources.mcp.messaging_strings_registry;
    let questions = INTAKE_TOPICS
        .iter()
        .filter_map(|topic| {
            let id = topic.parq_id()?;
            Some(ParqQuestionDto {
                id: id.to_owned(),
                text: registry.get(topic.string_key(), &locale),
            })
        })
        .collect();
    Ok((StatusCode::OK, Json(ParqQuestionsResponse { questions })).into_response())
}

/// `POST /api/me/parq` — submit PAR-Q answers.
///
/// Each "yes" raises an agent-visible medical flag, and each "no" retires the
/// flag an earlier "yes" to that same question raised. A "yes" never blocks
/// sign-up.
///
/// A re-screen may answer only some questions: a question the submission does
/// not mention keeps whatever is on file, and a medical flag a coach tool
/// raised is never touched here.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, no active tenant is present, or
/// persisting or retiring a flag fails.
pub async fn handle_parq_post(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(req): Json<ParqAnswersRequest>,
) -> Result<Response, AppError> {
    let tenant_id = active_tenant(&auth)?;
    let (yes, no): (Vec<ParqAnswer>, Vec<ParqAnswer>) =
        req.answers.into_iter().partition(|a| a.yes);
    let yes_ids: Vec<String> = yes.into_iter().map(|a| a.id).collect();
    let cleared_ids: Vec<String> = no.into_iter().map(|a| a.id).collect();
    let memory = resources.common.repos.memory.as_ref();
    let user_id = auth.user_id.to_string();
    let flags_raised = parq::persist_parq_flags(memory, tenant_id, &user_id, &yes_ids).await?;
    let retired = parq::retire_parq_flags(memory, tenant_id, &user_id, &cleared_ids).await?;
    if retired > 0 {
        info!(
            retired,
            "PAR-Q re-screen retired flags its clean answers cleared"
        );
    }
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
/// These are the inputs `build_agent_proposal` already reads and currently never
/// finds: without them the agent proposal falls back to sport-mix, and on a
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
    /// `complete`, `skipped` or `not_applicable`.
    pub status: String,
    /// For the `messaging_channel` step: the messaging app the user chose.
    #[serde(default)]
    pub chosen_channel: Option<String>,
    /// For the `coach_proposal` step: the lead agent the proposal offered,
    /// sent on Start and on Skip; it joins the team and becomes the athlete's
    /// selected agent.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// For the `coach_proposal` step: every agent the athlete ticked (and
    /// added from Discover). Each joins the team with the lead; Skip sends
    /// the lead alone. Absent reads as none.
    #[serde(default)]
    pub team_agent_ids: Vec<String>,
}

/// Post the onboarding lead's welcome into the athlete's personal thread.
#[cfg(feature = "client-chat")]
async fn welcome_lead(
    resources: &ServerContext,
    headers: &HeaderMap,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
    lead: &str,
) {
    welcome_onboarding_lead(resources, headers, user_id, tenant_id, lead).await;
}

/// A build without the in-app chat has no personal thread to welcome the
/// lead into; the team is still joined and the lead still selected.
#[cfg(not(feature = "client-chat"))]
async fn welcome_lead(
    _resources: &ServerContext,
    _headers: &HeaderMap,
    _user_id: uuid::Uuid,
    _tenant_id: TenantId,
    _lead: &str,
) {
}

/// `PUT /api/me/onboarding/steps/{step_id}` — persist a step's completion status.
///
/// Durable, server-side onboarding progress: the web/mobile flows call this as
/// each step completes so onboarding follows the user across devices. Scoped by
/// the authenticated `user_id`. A write that changes the stored status
/// reports `onboarding.step_completed` (carnet#1100); the team step also
/// posts the lead's welcome into the personal thread (carnet#1099).
///
/// # Errors
///
/// Returns `AppError` when authentication fails, the step id or status is not
/// recognised, `chosen_channel` is too long, the `coach_proposal` lead or a
/// team member is an agent the athlete cannot be given or join, or
/// persistence fails.
pub async fn handle_step_put(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
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
    // The team step, in one order: every agent it names is checked, then
    // each joins, then the lead is selected, then the step is written — so a
    // refused agent leaves nothing behind, and a retry converges.
    if let Some(active_tenant) = auth.active_tenant_id {
        let active_tenant = TenantId::from_uuid(active_tenant);
        let repos = &resources.common.repos;
        let team = intake::validate_onboarding_team(
            repos,
            auth.user_id,
            active_tenant,
            &step_id,
            &req.status,
            req.agent_id.as_deref(),
            &req.team_agent_ids,
        )
        .await?;
        intake::join_onboarding_team(repos, auth.user_id, active_tenant, &team).await?;
        intake::select_onboarding_lead(
            repos,
            auth.user_id,
            active_tenant,
            &step_id,
            &req.status,
            req.agent_id.as_deref(),
        )
        .await?;
        // The team step's lead opens the personal thread, on Skip as on Start.
        let lead = req.agent_id.as_deref().filter(|_| !team.is_empty());
        if let Some(lead) = lead {
            welcome_lead(&resources, &headers, auth.user_id, active_tenant, lead).await;
        }
    }
    let tenant_id = auth.active_tenant_id.map(|t| t.to_string());
    let previous = stored_step_status(&resources, auth.user_id, &step_id).await;
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
    intake::release_coach_only_agent(
        resources.common.repos.tenants.as_ref(),
        auth.user_id,
        &step_id,
        &req.status,
    )
    .await?;
    let written = WrittenStep {
        step_id: &step_id,
        status: &req.status,
        changed: previous.as_deref() != Some(req.status.as_str()),
    };
    report_step_written(&resources, auth.user_id, tenant_id.as_deref(), &written).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The status `step_id` holds for `user_id` before this write, read so the
/// write can say whether it changed anything. An unreadable list reads as no
/// status (logged): the step then counts as reached once more, never as not
/// reached at all.
async fn stored_step_status(
    resources: &ServerContext,
    user_id: uuid::Uuid,
    step_id: &str,
) -> Option<String> {
    match resources
        .common
        .repos
        .user_onboarding
        .get_onboarding_steps(&user_id.to_string())
        .await
    {
        Ok(steps) => steps
            .into_iter()
            .find(|step| step.step_id == step_id)
            .map(|step| step.status),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "onboarding step: stored status unreadable");
            None
        }
    }
}

/// A step write as the funnel events report it.
struct WrittenStep<'a> {
    step_id: &'a str,
    status: &'a str,
    /// The stored status differed, or there was none.
    changed: bool,
}

/// Report a stored step write, timed from the athlete's signup: the funnel's
/// `onboarding.step_completed` when the status changed (carnet#1100), and
/// `onboarding.coach_group_skipped` when a coach tapped **Later** on the
/// group step (carnet#739). The step is already stored, so a failed user
/// lookup is logged rather than failing the request.
async fn report_step_written(
    resources: &ServerContext,
    user_id: uuid::Uuid,
    tenant_id: Option<&str>,
    step: &WrittenStep<'_>,
) {
    let group_skipped =
        step.step_id == COACH_GROUP_STEP_ID && step.status == intake::STATUS_SKIPPED;
    if !step.changed && !group_skipped {
        return;
    }
    let user = match resources.common.repos.users.get_global(user_id).await {
        Ok(Some(user)) => user,
        Ok(None) => return,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "onboarding step: user lookup failed");
            return;
        }
    };
    let seconds_since_signup = Utc::now()
        .signed_duration_since(user.created_at)
        .num_seconds()
        .max(0);
    if step.changed {
        emit_step_completed(user_id, tenant_id, step, seconds_since_signup);
    }
    if group_skipped {
        emit_coach_group_skipped(user_id, tenant_id, seconds_since_signup);
    }
}

/// `onboarding.step_completed`: the funnel's step, reported once per change.
fn emit_step_completed(
    user_id: uuid::Uuid,
    tenant_id: Option<&str>,
    step: &WrittenStep<'_>,
    seconds_since_signup: i64,
) {
    info!(
        target: "notify",
        event = "onboarding.step_completed",
        user_id = %user_id,
        tenant_id = tenant_id.unwrap_or_default(),
        step_id = step.step_id,
        status = step.status,
        seconds_since_signup,
        "onboarding step recorded"
    );
}

/// `onboarding.coach_group_skipped`: the coach tapped **Later** on the group
/// step (carnet#739).
fn emit_coach_group_skipped(
    user_id: uuid::Uuid,
    tenant_id: Option<&str>,
    seconds_since_signup: i64,
) {
    info!(
        target: "notify",
        event = "onboarding.coach_group_skipped",
        user_id = %user_id,
        tenant_id = tenant_id.unwrap_or_default(),
        seconds_since_signup,
        "coach skipped the onboarding group step"
    );
}

/// Mount-helper for the onboarding-status endpoint. Same shape as
/// [`pierre_routes_admin::FeatureFlagsRoutes`].
#[derive(Debug)]
pub struct OnboardingRoutes;

impl OnboardingRoutes {
    /// User-facing route. Requires a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        let router = Router::new()
            .route("/api/me/onboarding-status", get(handle_self_get))
            .route("/api/me/onboarding/steps/{step_id}", put(handle_step_put))
            .route("/api/me/parq", get(handle_parq_get).post(handle_parq_post))
            .route("/api/me/about-you", post(handle_about_you_post));
        // Getting started reads the first-run flows the chat pipeline records.
        #[cfg(feature = "client-chat")]
        let router = router.route("/api/me/getting-started", get(get_getting_started));
        router.with_state(resources)
    }
}

/// Whether the athlete is bringing their own history in, in their active
/// tenant: an import that started (queued, downloading, importing) or
/// completed answers the connect step as a connection does (carnet#1218). One
/// still waiting for its link does not, until the product owner settles whether it
/// should (design Q6). Best effort: imports not running here, or a read that
/// fails, count as none.
async fn imports_own_history(
    resources: &ServerContext,
    tenant: Option<uuid::Uuid>,
    user_id: uuid::Uuid,
) -> bool {
    let Some(tenant) = tenant else {
        return false;
    };
    if !resources.common.archive_imports.is_enabled() {
        return false;
    }
    let scope = archive_ops::scope_of(TenantId::from_uuid(tenant), user_id);
    archive_ops::latest_per_source(resources, &scope)
        .await
        .is_ok_and(|imports| {
            imports.iter().any(|import| {
                matches!(
                    import.status,
                    "queued" | "downloading" | "importing" | "completed"
                )
            })
        })
}
