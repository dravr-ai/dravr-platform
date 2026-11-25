// ABOUTME: Handlers for /api/chat/conversations/{id}/subject-plan — a coach's draft of their thread's athlete's plan: read, publish, discard
// ABOUTME: Each request re-runs the by-id coach gates; the read, publish and discard are the plan tools' own service calls

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1069. In a coach's thread bound to one athlete (carnet#1070), the
//! plan card shows the coach's draft of the athlete's plan with Publish and
//! Discard. These routes back those buttons and the card's read:
//!
//! - `GET …/subject-plan` — the athlete's live plan and the coach's draft over
//!   it, each in the shape `GET /api/me/training-plan` returns, and where the
//!   draft stands;
//! - `POST …/subject-plan/publish` — the draft made live, or a 409
//!   `plan_publish_conflict` naming what it would overwrite;
//! - `DELETE …/subject-plan/draft` — the draft thrown away;
//! - `GET …/subject-plan/history` — the changes the coach and the thread's
//!   agent made to the athlete's plan in the coach's workspace, newest first
//!   (carnet#1815).
//!
//! The conversation must be the caller's own thread about an athlete, and the
//! coaching gates ([`coached_athlete_access`]) run again on every request, so
//! a consent revoked since the thread opened refuses the next one. Publish
//! goes through [`publish_draft_live`], the entry point `publish_training_plan`
//! calls, so the button and the tool publish the same way.
//!
//! The read and the publish read the athlete's plan (the publish also their
//! load, for its rails), so each goes on the athlete's own record as a
//! `coach_thread` read before anything of theirs is read (carnet#1445). The
//! discard reads nothing of theirs: the draft is the coach's. Neither does
//! the history: it lists the changes the coach and the thread's agent made,
//! the caller's own, and only theirs.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::Timestamp;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::ConversationRecord;
use pierre_database::repositories::{
    CoachReadClass, CoachReadSurface, PlanChangeQuery, PlanChangeSource,
};
use pierre_middleware::AuthenticatedUser;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{
    coach_plan_view, discard_coach_draft, CoachDraftTarget, DraftView, PlanWithWeeks,
};
use pierre_services::coached_athlete::{coached_athlete_access, CoachedAthleteAccess};
use pierre_services::locale::user_locale;
use pierre_services::personas::resolve_persona_locale;
use pierre_services::plan_card::{PlanCard, PlanSubject};
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_tool_runtime::context::ToolExecutionContext;
use pierre_tool_runtime::implementations::training_plan_drafts::{
    publish_draft_live, DraftPublish, DraftPublishOutcome, PublishedDraftReport,
};
use pierre_tool_runtime::read_subject::record_coach_read;

use uuid::Uuid;

use crate::mcp::host_seams::tool_auth_method;
use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::tools::runtime_adapter::into_runtime;

use super::common::{
    coaching_refusal, conversation_not_found, get_tenant_id, refusal, refuse_external_coach_write,
    refuse_unshared_subject,
};
use super::dto::{
    PlanChangeEventResponse, PlanPublishConflictResponse, PublishConflictDetails,
    PublishSubjectPlanRequest, PublishedSubjectPlanResponse, SubjectPlanHistoryQuery,
    SubjectPlanHistoryResponse, SubjectPlanResponse,
};

/// The code of a publish refused because the athlete's plan moved under the
/// draft, in the 409 body's `error_code` and `details.reason`.
const PLAN_PUBLISH_CONFLICT: &str = "plan_publish_conflict";

/// What the calendar push's notify events name as its trigger when the
/// coach's Publish button published.
const PUBLISH_TRIGGER: &str = "subject_plan_publish";

/// Changes one history read returns when the client names no limit.
const DEFAULT_HISTORY_LIMIT: u32 = 50;

/// Most changes one history read returns.
const MAX_HISTORY_LIMIT: u32 = 100;

/// The 404 for a coach who holds no draft of the athlete's plan.
fn no_draft() -> AppError {
    refusal(
        ErrorCode::ResourceNotFound,
        "no_draft",
        "You hold no draft of this athlete's plan",
    )
}

/// The caller's thread about an athlete, and what the gates answered for it.
pub(super) struct CoachedThread {
    /// The thread: its agent is the one the coach's writes in it answer as.
    pub conversation: ConversationRecord,
    /// What the coaching gates let the caller do for the thread's athlete.
    pub access: CoachedAthleteAccess,
}

/// The caller's thread about an athlete, and the gates that let them act for
/// that athlete, checked again now.
///
/// 404 when the conversation is not the caller's own (a participant in it is
/// not its owner) or is bound to no athlete (`details.reason` `no_subject`);
/// the gate refusals otherwise, with their own status and reason.
pub(super) async fn coached_thread(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
    conversation_id: &str,
) -> Result<CoachedThread, AppError> {
    let auth: &AuthResult = auth;
    let tenant_id = get_tenant_id(auth, resources).await?;
    let repos = &resources.common.repos;
    let caller = auth.user_id.to_string();
    let conversation = repos
        .chat
        .get_conversation(conversation_id, &caller, tenant_id)
        .await?
        .filter(|conv| conv.user_id == caller)
        .ok_or_else(conversation_not_found)?;
    refuse_unshared_subject(resources, auth, &conversation).await?;
    let Some(subject) = conversation.subject() else {
        return Err(refusal(
            ErrorCode::ResourceNotFound,
            "no_subject",
            "This conversation is not about an athlete",
        ));
    };
    let access = coached_athlete_access(
        repos,
        auth.user_id,
        subject.athlete_user_id,
        subject.group_id,
    )
    .await?
    .map_err(|refused| coaching_refusal(auth, refused))?;
    Ok(CoachedThread {
        conversation,
        access,
    })
}

/// [`coached_thread`]'s gates, for a handler that needs only what they
/// answered.
async fn coached_subject(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
    conversation_id: &str,
) -> Result<CoachedAthleteAccess, AppError> {
    Ok(coached_thread(resources, auth, conversation_id)
        .await?
        .access)
}

/// Record on the athlete's own record that the coach's thread read their plan
/// (carnet#1445). A row that cannot be written fails the request: no read
/// goes unrecorded.
pub(super) async fn record_thread_read(
    resources: &Arc<ServerContext>,
    access: &CoachedAthleteAccess,
    coach: Uuid,
) -> Result<(), AppError> {
    record_coach_read(
        &resources.common.repos,
        access,
        coach,
        CoachReadSurface::CoachThread.as_str(),
        CoachReadClass::Training,
    )
    .await
}

/// `GET /api/chat/conversations/{id}/subject-plan`: the thread's athlete's
/// live plan and the coach's draft over it, each projected on the athlete's
/// today in the shape `GET /api/me/training-plan` returns, in the coach's
/// locale (or `?locale=`), and where the draft stands.
pub(super) async fn get_subject_plan(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let access = coached_subject(&resources, &auth, &conversation_id).await?;
    record_thread_read(&resources, &access, auth.user_id).await?;
    let response =
        subject_plan_response(&resources, access, auth.user_id, query.locale.as_deref()).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// The locale a coach reads their thread in: `requested` (`?locale=`) when
/// supported, else the coach's stored locale, else the default.
///
/// # Errors
///
/// Returns the repository error from the coach's read.
pub(super) async fn coach_locale(
    resources: &Arc<ServerContext>,
    coach: Uuid,
    requested: Option<&str>,
) -> Result<String, AppError> {
    let coach_user = resources.common.repos.users.get_global(coach).await?;
    Ok(resolve_persona_locale(
        requested,
        Some(&user_locale(coach_user.as_ref())),
    ))
}

/// What `GET …/subject-plan` answers: the thread's athlete's live plan and
/// `coach`'s draft over it, each projected on the athlete's today in the
/// shape `GET /api/me/training-plan` returns, in the coach's locale (or
/// `requested`), and where the draft stands. The caller has run the gates
/// for `access` and recorded the read.
///
/// # Errors
///
/// The errors of [`coach_plan_view`], and of the coach, medical-flag reads.
pub(super) async fn subject_plan_response(
    resources: &Arc<ServerContext>,
    access: CoachedAthleteAccess,
    coach: Uuid,
    requested: Option<&str>,
) -> Result<SubjectPlanResponse, AppError> {
    let repos = &resources.common.repos;
    let athlete = access.athlete_user_id;
    let target = CoachDraftTarget::new(access.tenant, athlete, coach);
    let view = coach_plan_view(repos, &target).await?;

    let today = athlete_today(repos, athlete).await;
    let locale = coach_locale(resources, coach, requested).await?;
    let fueling = FuelingDisclosure::for_athlete(repos, access.tenant, athlete).await?;
    let registry = &resources.mcp.messaging_strings_registry;
    let card = |plan, weeks: &[_]| PlanCard::build(plan, weeks, today, registry, &locale, &fueling);
    let live = view
        .live
        .as_ref()
        .map(|PlanWithWeeks { plan, weeks }| card(plan, weeks));
    let draft = view
        .draft
        .as_ref()
        .map(|DraftView { plan, weeks, .. }| card(plan, weeks));
    Ok(SubjectPlanResponse {
        athlete: PlanSubject {
            user_id: athlete,
            display_name: access.display_name,
        },
        today,
        live,
        draft,
        draft_state: view.draft.map(|draft| draft.state),
    })
}

/// `POST /api/chat/conversations/{id}/subject-plan/publish`: the coach's
/// draft made the athlete's live plan, and put on the athlete's chosen
/// calendar (carnet#1492).
///
/// 409 `plan_publish_conflict` — with the weeks the athlete changed since the
/// coach drafted them and whether their season was re-laid — when the
/// athlete's plan moved under the draft and the body did not ask to
/// overwrite it; nothing is written. 404 `no_draft` when the coach holds no
/// draft. A push a gate refuses or that fails leaves the publish standing:
/// the 200 says why in `calendar_push`.
pub(super) async fn publish_subject_plan(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Json(request): Json<PublishSubjectPlanRequest>,
) -> Result<Response, AppError> {
    refuse_external_coach_write(&auth, "plan")?;
    let access = coached_subject(&resources, &auth, &conversation_id).await?;
    record_thread_read(&resources, &access, auth.user_id).await?;
    let target = CoachDraftTarget::new(access.tenant, access.athlete_user_id, auth.user_id);
    // The push runs as the coach's own push_training_plan from this thread
    // would: the thread names the athlete, the coach is the requester, and an
    // API-key caller is refused it as an external transport. Only the coach
    // reads this route's answer, and the thread is their own (checked by
    // `coached_subject`), so the call is said to be a direct message: the
    // room classifier takes a conversation with no group for private only
    // then (carnet#1422).
    let coach_tenant = get_tenant_id(&auth, &resources).await?;
    let context = ToolExecutionContext::new(
        auth.user_id,
        Some(coach_tenant),
        into_runtime(&resources),
        tool_auth_method(&auth),
    )
    .with_conversation_id(conversation_id.clone())
    .with_turn_direct_message(true);
    // The room the publish is said in is the group the thread acts under now:
    // reopening the thread from another group rebinds it there.
    let outcome = publish_draft_live(
        &context,
        &DraftPublish {
            target: &target,
            group_id: access.group.id,
            overwrite: request.overwrite_athlete_changes,
            athlete: None,
            trigger: PUBLISH_TRIGGER,
            source: PlanChangeSource::Coach,
        },
    )
    .await?;
    match outcome {
        DraftPublishOutcome::Published(report) => {
            let PublishedDraftReport {
                published,
                room_post,
                calendar_push,
                ..
            } = *report;
            let response = PublishedSubjectPlanResponse {
                plan_id: published.plan.id,
                superseded_plan_id: published.superseded_plan_id,
                athlete_notified: published.athlete_notified,
                posted_in_room: room_post.in_room,
                calendar_push,
            };
            Ok((StatusCode::OK, Json(response)).into_response())
        }
        DraftPublishOutcome::Conflict(state) => {
            let response = PlanPublishConflictResponse {
                error_code: PLAN_PUBLISH_CONFLICT,
                message: format!(
                    "{}'s plan changed under this draft; publish with \
                     overwrite_athlete_changes to replace their changes",
                    access.display_name
                ),
                details: PublishConflictDetails {
                    reason: PLAN_PUBLISH_CONFLICT,
                },
                state,
            };
            Ok((StatusCode::CONFLICT, Json(response)).into_response())
        }
        DraftPublishOutcome::NoDraft => Err(no_draft()),
    }
}

/// `DELETE /api/chat/conversations/{id}/subject-plan/draft`: the coach's
/// draft thrown away. The athlete's plan is untouched and nothing reaches
/// them. 204, or 404 `no_draft` when the coach holds none.
pub(super) async fn discard_subject_plan_draft(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    refuse_external_coach_write(&auth, "plan")?;
    let access = coached_subject(&resources, &auth, &conversation_id).await?;
    let target = CoachDraftTarget::new(access.tenant, access.athlete_user_id, auth.user_id);
    match discard_coach_draft(&resources.common.repos, &target, PlanChangeSource::Coach).await? {
        Some(_) => Ok(StatusCode::NO_CONTENT.into_response()),
        None => Err(no_draft()),
    }
}

/// `GET /api/chat/conversations/{id}/subject-plan/history`: the changes the
/// coach and the thread's agent made to the thread's athlete's plan in the
/// coach's workspace, newest first (carnet#1815): each day write, each save
/// the agent made into the draft, each publish and discard, with the dates
/// it touched.
///
/// `?before=` (RFC 3339) reads the changes that landed strictly before it —
/// the `created_at` of the oldest change the client holds — and `?limit=`
/// how many, clamped to `1..=100` (50 when absent). Only the caller's own
/// changes on the athlete's plan, under the athlete's tenant: never another
/// coach's. The gates are the subject-plan read's ([`coached_thread`]).
///
/// 400 `invalid_before` for a `before` that is no RFC 3339 instant.
pub(super) async fn get_subject_plan_history(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Query(query): Query<SubjectPlanHistoryQuery>,
) -> Result<Response, AppError> {
    let access = coached_subject(&resources, &auth, &conversation_id).await?;
    let before = match query.before.as_deref().filter(|raw| !raw.is_empty()) {
        Some(raw) => Some(raw.parse::<Timestamp>().map_err(|_| {
            refusal(
                ErrorCode::InvalidInput,
                "invalid_before",
                &format!("'{raw}' is not an RFC 3339 instant"),
            )
        })?),
        None => None,
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    let events = resources
        .common
        .repos
        .plan_change_events
        .list_plan_changes(&PlanChangeQuery {
            tenant: access.tenant,
            athlete: access.athlete_user_id,
            coach: auth.user_id,
            before,
            limit,
        })
        .await?;
    let response = SubjectPlanHistoryResponse {
        events: events
            .into_iter()
            .map(PlanChangeEventResponse::of)
            .collect(),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}
