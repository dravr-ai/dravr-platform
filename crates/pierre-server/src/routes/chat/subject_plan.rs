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
//! - `DELETE …/subject-plan/draft` — the draft thrown away.
//!
//! The conversation must be the caller's own thread about an athlete, and the
//! coaching gates ([`coached_athlete_access`]) run again on every request, so
//! a consent revoked since the thread opened refuses the next one. Publish
//! goes through [`publish_draft_live`], the entry point `publish_training_plan`
//! calls, so the button and the tool publish the same way.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_middleware::AuthenticatedUser;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{
    coach_plan_view, discard_coach_draft, CoachDraftTarget, DraftView, PlanWithWeeks,
};
use pierre_services::coached_athlete::{
    coached_athlete_access, CoachedAthleteAccess, CoachedAthleteRefusal,
};
use pierre_services::locale::user_locale;
use pierre_services::personas::resolve_persona_locale;
use pierre_services::plan_card::{PlanCard, PlanSubject};
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_tool_runtime::implementations::training_plan_drafts::{
    publish_draft_live, DraftPublishOutcome,
};

use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::tools::runtime_adapter::into_runtime;

use super::common::{get_tenant_id, refusal};
use super::dto::{
    PlanPublishConflictResponse, PublishConflictDetails, PublishSubjectPlanRequest,
    PublishedSubjectPlanResponse, SubjectPlanResponse,
};

/// The code of a publish refused because the athlete's plan moved under the
/// draft, in the 409 body's `error_code` and `details.reason`.
const PLAN_PUBLISH_CONFLICT: &str = "plan_publish_conflict";

/// The 404 for a coach who holds no draft of the athlete's plan.
fn no_draft() -> AppError {
    refusal(
        ErrorCode::ResourceNotFound,
        "no_draft",
        "You hold no draft of this athlete's plan",
    )
}

/// The caller's thread about an athlete, and the gates that let them act for
/// that athlete, checked again now.
///
/// 404 when the conversation is not the caller's own (a participant in it is
/// not its owner) or is bound to no athlete (`details.reason` `no_subject`);
/// the gate refusals otherwise, with their own status and reason.
async fn coached_subject(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
    conversation_id: &str,
) -> Result<CoachedAthleteAccess, AppError> {
    let auth: &AuthResult = auth;
    let tenant_id = get_tenant_id(auth, resources).await?;
    let repos = &resources.common.repos;
    let caller = auth.user_id.to_string();
    let conversation = repos
        .chat
        .get_conversation(conversation_id, &caller, tenant_id)
        .await?
        .filter(|conv| conv.user_id == caller)
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    let Some(subject) = conversation.subject() else {
        return Err(refusal(
            ErrorCode::ResourceNotFound,
            "no_subject",
            "This conversation is not about an athlete",
        ));
    };
    coached_athlete_access(
        repos,
        auth.user_id,
        subject.athlete_user_id,
        subject.group_id,
    )
    .await?
    .map_err(CoachedAthleteRefusal::into_error)
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
    let coach = auth.user_id;
    let repos = &resources.common.repos;
    let athlete = access.athlete_user_id;
    let target = CoachDraftTarget::new(access.tenant, athlete, coach);
    let view = coach_plan_view(repos, &target).await?;

    let today = athlete_today(repos, athlete).await;
    let coach_user = repos.users.get_global(coach).await?;
    let locale = resolve_persona_locale(
        query.locale.as_deref(),
        Some(&user_locale(coach_user.as_ref())),
    );
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
    let response = SubjectPlanResponse {
        athlete: PlanSubject {
            user_id: athlete,
            display_name: access.display_name,
        },
        today,
        live,
        draft,
        draft_state: view.draft.map(|draft| draft.state),
    };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `POST /api/chat/conversations/{id}/subject-plan/publish`: the coach's
/// draft made the athlete's live plan.
///
/// 409 `plan_publish_conflict` — with the weeks the athlete changed since the
/// coach drafted them and whether their season was re-laid — when the
/// athlete's plan moved under the draft and the body did not ask to
/// overwrite it; nothing is written. 404 `no_draft` when the coach holds no
/// draft.
pub(super) async fn publish_subject_plan(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Json(request): Json<PublishSubjectPlanRequest>,
) -> Result<Response, AppError> {
    let access = coached_subject(&resources, &auth, &conversation_id).await?;
    let target = CoachDraftTarget::new(access.tenant, access.athlete_user_id, auth.user_id);
    // The room the publish is said in is the group the thread acts under now:
    // reopening the thread from another group rebinds it there.
    let outcome = publish_draft_live(
        &into_runtime(&resources),
        &target,
        access.group.id,
        request.overwrite_athlete_changes,
    )
    .await?;
    match outcome {
        DraftPublishOutcome::Published(report) => {
            let posted_in_room = report.room_post.in_room;
            let published = report.published;
            let response = PublishedSubjectPlanResponse {
                plan_id: published.plan.id,
                superseded_plan_id: published.superseded_plan_id,
                athlete_notified: published.athlete_notified,
                posted_in_room,
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
    let access = coached_subject(&resources, &auth, &conversation_id).await?;
    let target = CoachDraftTarget::new(access.tenant, access.athlete_user_id, auth.user_id);
    match discard_coach_draft(&resources.common.repos, &target).await? {
        Some(_) => Ok(StatusCode::NO_CONTENT.into_response()),
        None => Err(no_draft()),
    }
}
