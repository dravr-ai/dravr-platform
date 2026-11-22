// ABOUTME: Handlers for a coach's "build from family" editor in their thread about one athlete — the families, a session's preview, the draft-day write
// ABOUTME: Each request re-runs the subject-plan gates; the catalogue, the preview and the write are pierre-tool-runtime's coach_session_editor calls

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1515. Each day of the plan card in a coach's thread about one
//! athlete opens an editor that builds the day from a workout family. These
//! routes back it:
//!
//! - `GET …/subject-plan/families` — the families the thread's agent offers,
//!   each preset with its evidence ranges per sport;
//! - `POST …/subject-plan/session-preview` — the session a build makes on
//!   the athlete's stored anchors, measured by the envelope check and worded
//!   as the plan card and the envelope notice word it. Nothing is written;
//! - `PUT …/subject-plan/draft/days/{date}` — the build saved as that day of
//!   the coach's draft, opened from the live plan when the coach holds none,
//!   answered with what `GET …/subject-plan` reads after the write.
//!
//! Every request runs the subject-plan gates again ([`coached_thread`]). The
//! preview and the write read the athlete's anchors, so each goes on the
//! athlete's record as a `coach_thread` read before anything of theirs is
//! read (carnet#1445); the catalogue reads nothing of theirs. The write
//! changes the athlete's plan draft, so an outside client acting for the
//! coach is refused it, as the Publish and Discard buttons refuse one
//! (carnet#1533). The preview writes nothing and reads what `GET
//! …/subject-plan` already serves the same caller under the same gates — a
//! saved family day's steps on the athlete's anchors — so it is not refused
//! as a write.
//!
//! A build the catalogue cannot make — an unknown family or preset, a sport
//! the preset has no form for, a value past a session's bounds — is a 400
//! naming its `details.reason`. A value outside the preset's evidence range
//! is never refused: it is built, saved, and earns a finding (D3).

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::CivilDate;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode, ErrorResponse};
use pierre_memory::training_plans::parse_plan_date;
use pierre_middleware::AuthenticatedUser;
use pierre_services::coach_plan_draft::CoachDraftTarget;
use pierre_tool_runtime::implementations::coach_session_editor::{
    preview_session, save_draft_day, thread_families, DraftDayOutcome, EditorThread, EditorWords,
};
use pierre_tool_runtime::implementations::plan_scope::PlanWriter;

use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::tools::runtime_adapter::into_runtime;

use super::common::{get_tenant_id, refusal, refuse_external_coach_write};
use super::dto::{DraftDayRequest, FamilyCatalogueResponse, SessionPreviewRequest};
use super::subject_plan::{
    coach_locale, coached_thread, record_thread_read, subject_plan_response, CoachedThread,
};

/// The 400 for a date that is not a plan date.
fn invalid_date(raw: &str) -> AppError {
    refusal(
        ErrorCode::InvalidInput,
        "invalid_date",
        &format!("'{raw}' is not a date (YYYY-MM-DD)"),
    )
}

/// `raw` as a plan date, or the 400 naming it.
fn plan_date(raw: &str) -> Result<CivilDate, AppError> {
    parse_plan_date(raw).ok_or_else(|| invalid_date(raw))
}

/// The coach's draft of the thread's athlete's plan, and the coach writing
/// it with the thread's agent: the agent the thread binds, as the coach's
/// save from the thread names it.
async fn editor_thread(
    resources: &Arc<ServerContext>,
    auth: &AuthResult,
    thread: &CoachedThread,
) -> Result<EditorThread, AppError> {
    let access = &thread.access;
    Ok(EditorThread {
        target: CoachDraftTarget::new(access.tenant, access.athlete_user_id, auth.user_id),
        writer: PlanWriter {
            agent_id: thread.conversation.agent_id.clone(),
            tenant: get_tenant_id(auth, resources).await?,
            user_id: auth.user_id,
        },
    })
}

/// `GET /api/chat/conversations/{id}/subject-plan/families`: the workout
/// families the thread's agent offers, as the coach's save resolves them, in
/// the coach's locale (or `?locale=`).
pub(super) async fn get_subject_families(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let thread = coached_thread(&resources, &auth, &conversation_id).await?;
    let locale = coach_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let editor = editor_thread(&resources, &auth, &thread).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let families = thread_families(&into_runtime(&resources), &editor.writer, words).await?;
    Ok((StatusCode::OK, Json(FamilyCatalogueResponse { families })).into_response())
}

/// `POST /api/chat/conversations/{id}/subject-plan/session-preview`: the
/// session a build makes for one of the athlete's days, on their stored
/// anchors, with what the kernel derives and the envelope findings, in the
/// coach's locale (or `?locale=`) and units. Nothing is written.
pub(super) async fn preview_subject_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<SessionPreviewRequest>,
) -> Result<Response, AppError> {
    let thread = coached_thread(&resources, &auth, &conversation_id).await?;
    let date = plan_date(&request.date)?;
    record_thread_read(&resources, &thread.access, auth.user_id).await?;
    let locale = coach_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let editor = editor_thread(&resources, &auth, &thread).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
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

/// `PUT /api/chat/conversations/{id}/subject-plan/draft/days/{date}`: the
/// build saved as that day of the coach's draft of the athlete's plan,
/// answered with the subject plan as `GET …/subject-plan` reads it after the
/// write.
///
/// 404 `no_plan` when the athlete has no live plan and the coach no draft;
/// 422 `no_plan_week` when no week of the plan holds the date; 403
/// `external_transport` for an outside client. Nothing is written on any of
/// them.
pub(super) async fn put_subject_draft_day(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date)): Path<(String, String)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(build): Json<DraftDayRequest>,
) -> Result<Response, AppError> {
    refuse_external_coach_write(&auth, "plan")?;
    let thread = coached_thread(&resources, &auth, &conversation_id).await?;
    let date = plan_date(&raw_date)?;
    record_thread_read(&resources, &thread.access, auth.user_id).await?;
    let editor = editor_thread(&resources, &auth, &thread).await?;
    match save_draft_day(&into_runtime(&resources), &editor, date, &build).await? {
        DraftDayOutcome::Saved => {}
        DraftDayOutcome::NoPlan => {
            return Err(refusal(
                ErrorCode::ResourceNotFound,
                "no_plan",
                &format!(
                    "{} has no plan and you hold no draft of one to add a day to",
                    thread.access.display_name
                ),
            ));
        }
        DraftDayOutcome::NoPlanWeek => {
            let refused = refusal(
                ErrorCode::InvalidInput,
                "no_plan_week",
                &format!(
                    "No week of {}'s plan holds {date}",
                    thread.access.display_name
                ),
            );
            // The request is well formed and names a real day; the plan
            // has no week to put it in.
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorResponse::from(refused)),
            )
                .into_response());
        }
    }
    let response = subject_plan_response(
        &resources,
        thread.access,
        auth.user_id,
        query.locale.as_deref(),
    )
    .await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}
