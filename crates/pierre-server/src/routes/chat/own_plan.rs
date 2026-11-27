// ABOUTME: Handlers for /api/me/plan — the athlete's own plan as their Plan tab reads it: live, their agent's proposals, what « Envoyer » sends
// ABOUTME: Scoped by the token alone (tenant from it, user = the caller); the families, preview and history are the coach editor's own calls

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1841. A self-coached athlete's Plan tab is the coach workspace
//! turned toward their own plan. These routes back its reads:
//!
//! - `GET /api/me/plan` — the live plan, the overlay of what their agent
//!   changed unasked (« Proposé »), the dates it proposes grouped as they are
//!   accepted (a moved session is one proposal over both dates), and what
//!   « Envoyer » would send, judged from the push ledger alone;
//! - `GET /api/me/plan/families` — the workout families « Idées » offers,
//!   the catalogue's own;
//! - `POST /api/me/plan/session-preview` — the session a family build makes
//!   on the athlete's own anchors, in their own units, read under their own
//!   AI consents. Nothing is written;
//! - `GET /api/me/plan/history` — the changes made to their plan from it,
//!   newest first;
//! - `POST /api/me/plan/thread` — their own plan thread, the Dravr tab:
//!   found, or opened bound to their Home thread's agent.
//!
//! The plan is the caller's own, under the tenant their token names: there
//! is no coaching gate, no conversation and no coach's read to record. The
//! routes reuse the service calls the coach's subject-plan routes make, with
//! the athlete as the owner of their own overlay
//! ([`CoachDraftTarget::own`]), so a coach's routes are never asked to read
//! a self thread as a coach thread.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::AuthenticatedUser;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{coach_plan_view, CoachDraftTarget};
use pierre_services::locale::user_locale;
use pierre_services::personal_thread::{resolve_plan_thread, PersonalForge};
use pierre_services::personas::resolve_persona_locale;
use pierre_services::plan_card::PlanCard;
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_tool_runtime::context::ToolExecutionContext;
use pierre_tool_runtime::implementations::coach_session_editor::{
    preview_session, thread_families, EditorWords, OwnPlan, PlanEditor,
};
use pierre_tool_runtime::implementations::plan_scope::external_own_write_error;
use pierre_tool_runtime::implementations::training_plan_push::own_pending_send;
use uuid::Uuid;

use crate::mcp::host_seams::tool_auth_method;
use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::services::conversation_title::fresh_title_fallback;
use crate::tools::runtime_adapter::into_runtime;

use super::common::{conversation_response, get_tenant_id};
use super::dto::{FamilyCatalogueResponse, SessionPreviewRequest, SubjectPlanHistoryQuery};
use super::own_plan_dto::{OwnPlanResponse, PendingSendResponse, PlanThreadResponse};
use super::subject_plan::plan_history;
use super::subject_session::plan_date;

/// The caller's own plan: the athlete is the caller, under the tenant their
/// token names.
pub(super) struct OwnCaller {
    /// The athlete's tenant and the athlete.
    pub own: OwnPlan,
    /// The caller's context for the plan tools' calls, built as their own
    /// direct call with no conversation.
    pub context: ToolExecutionContext,
}

/// The caller as the owner of their own plan.
///
/// # Errors
///
/// The tenant resolution's refusal.
pub(super) async fn own_caller(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
) -> Result<OwnCaller, AppError> {
    let tenant = get_tenant_id(auth, resources).await?;
    let context = ToolExecutionContext::new(
        auth.user_id,
        Some(tenant),
        into_runtime(resources),
        tool_auth_method(auth),
    );
    Ok(OwnCaller {
        own: OwnPlan {
            tenant,
            athlete: auth.user_id,
        },
        context,
    })
}

/// The locale the athlete reads their plan in: `requested` (`?locale=`) when
/// supported, else their stored locale, else the default.
///
/// # Errors
///
/// Returns the repository error from the athlete's read.
pub(super) async fn athlete_locale(
    resources: &Arc<ServerContext>,
    athlete: Uuid,
    requested: Option<&str>,
) -> Result<String, AppError> {
    let user = resources.common.repos.users.get_global(athlete).await?;
    Ok(resolve_persona_locale(
        requested,
        Some(&user_locale(user.as_ref())),
    ))
}

/// `GET /api/me/plan`: the athlete's own plan, their agent's proposals over
/// it and what « Envoyer » would send, projected on their today in their
/// locale (or `?locale=`).
pub(super) async fn get_own_plan(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let caller = own_caller(&resources, &auth).await?;
    let response = own_plan_response(&resources, &caller, query.locale.as_deref()).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// What `GET /api/me/plan` answers for `caller`, in their locale (or
/// `requested`): what every write on their plan answers with too.
///
/// # Errors
///
/// The errors of [`coach_plan_view`], of the athlete's and medical-flag
/// reads, and of the pending send's calendar resolution.
pub(super) async fn own_plan_response(
    resources: &Arc<ServerContext>,
    caller: &OwnCaller,
    requested: Option<&str>,
) -> Result<OwnPlanResponse, AppError> {
    let repos = &resources.common.repos;
    let OwnPlan { tenant, athlete } = caller.own;
    let view = coach_plan_view(repos, &CoachDraftTarget::own(tenant, athlete)).await?;
    let today = athlete_today(repos, athlete).await;
    let locale = athlete_locale(resources, athlete, requested).await?;
    let fueling = FuelingDisclosure::for_athlete(repos, tenant, athlete).await?;
    let registry = &resources.mcp.messaging_strings_registry;
    let card = |plan, weeks: &[_]| PlanCard::build(plan, weeks, today, registry, &locale, &fueling);
    let live = view
        .live
        .as_ref()
        .map(|season| card(&season.plan, &season.weeks));
    let draft = view
        .draft
        .as_ref()
        .map(|draft| card(&draft.plan, &draft.weeks));
    let live_weeks = view
        .live
        .as_ref()
        .map_or(&[][..], |season| season.weeks.as_slice());
    let pending = own_pending_send(&caller.context, live_weeks, today).await?;
    Ok(OwnPlanResponse {
        today,
        live,
        draft,
        proposed_dates: view.proposed_dates(),
        proposals: view.proposal_groups(),
        pending_send: PendingSendResponse::of(pending),
    })
}

/// `GET /api/me/plan/families`: the workout families « Idées » offers — the
/// catalogue's own, since no agent's turn names a package — in the athlete's
/// locale (or `?locale=`).
pub(super) async fn get_own_families(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let caller = own_caller(&resources, &auth).await?;
    let locale = athlete_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let writer = caller.own.writer();
    let families = thread_families(&into_runtime(&resources), &writer, words).await?;
    Ok((StatusCode::OK, Json(FamilyCatalogueResponse { families })).into_response())
}

/// `POST /api/me/plan/session-preview`: the session a build makes for one of
/// the athlete's days, on their own stored anchors, with what the kernel
/// derives and the envelope findings, in their locale (or `?locale=`) and
/// units. Nothing is written.
///
/// 400 `invalid_date`, and the build's 400 refusals.
pub(super) async fn preview_own_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<SessionPreviewRequest>,
) -> Result<Response, AppError> {
    let caller = own_caller(&resources, &auth).await?;
    let date = plan_date(&request.date)?;
    let locale = athlete_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let editor = PlanEditor::Own(caller.own);
    let preview = preview_session(
        &into_runtime(&resources),
        &editor,
        date,
        &request.build,
        words,
    )
    .await?;
    Ok((StatusCode::OK, Json(preview)).into_response())
}

/// `GET /api/me/plan/history`: the changes made to the athlete's plan from
/// their Plan tab and by their agent there, newest first: their own day
/// writes, their agent's saves and proposals, what they accepted or
/// refused, and each send.
///
/// `?before=` (RFC 3339) and `?limit=` page it as the coach's history does.
/// 400 `invalid_before` for a `before` that is no RFC 3339 instant.
pub(super) async fn get_own_plan_history(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<SubjectPlanHistoryQuery>,
) -> Result<Response, AppError> {
    let tenant: TenantId = get_tenant_id(&auth, &resources).await?;
    let athlete = auth.user_id;
    let response = plan_history(&resources, tenant, athlete, athlete, &query).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `POST /api/me/plan/thread`: the athlete's own plan thread, their Plan
/// tab's Dravr tab — the one they have, or one opened now bound to the agent
/// of their Home thread. Never their personal thread, and outside the cap of
/// threads they keep. Idempotent: every call answers the same thread.
///
/// 403 `external_transport` for an outside client: opening a thread is a
/// write.
pub(super) async fn open_own_plan_thread(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    if auth.acts_for_outside_client() {
        return Err(external_own_write_error());
    }
    let tenant = get_tenant_id(&auth, &resources).await?;
    let user = auth.user_id.to_string();
    let title_fallback = fresh_title_fallback(&resources, &user).await;
    let thread = resolve_plan_thread(
        &resources.common.repos,
        &user,
        tenant,
        PersonalForge {
            title_fallback: &title_fallback,
            guided_flow: false,
            llm_chain: &resources.common.config.llm_chain,
        },
    )
    .await?;
    let response = PlanThreadResponse {
        conversation: conversation_response(&resources, thread.conversation).await,
        opened: thread.opened,
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}
