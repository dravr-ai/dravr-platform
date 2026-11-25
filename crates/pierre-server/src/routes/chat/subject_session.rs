// ABOUTME: Handlers for a coach's day editor in their thread about one athlete — the families, a session's preview, each session's writes and call
// ABOUTME: Each request re-runs the subject-plan gates; the catalogue, the preview and the writes are pierre-tool-runtime's coach_session_editor calls

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
//!   as the plan card and the envelope notice word it. Nothing is written.
//!
//! carnet#1714 and carnet#1716. The editor opens on one session of a date,
//! which may hold several. A session is addressed by its 0-based position
//! `{n}` among the date's entries, and every write names it as the client
//! last saw it (`expect`: the card's `sport` and `workout`); a position that
//! no longer holds that session is a 409 `session_moved`, nothing written.
//! Each write lands in the coach's draft, opened from the live plan when the
//! coach holds none, and is answered with what `GET …/subject-plan` reads
//! after it:
//!
//! - `PUT …/subject-plan/draft/days/{date}/{n}` — a family build over
//!   session n;
//! - `PUT …/draft/days/{date}/{n}/session` — session n as the coach wrote it
//!   by hand, or a rest entry. It names no family, so no envelope check
//!   measures it; it passes the agent's own day bounds;
//! - `POST …/draft/days/{date}/{n}/restore` — session n put back as it stood
//!   before the coach's first edit, or removed when the coach added it. It
//!   never opens a draft;
//! - `POST …/draft/days/{date}/{n}/remove` — session n removed; never the
//!   date's last entry (422 `last_session`), which is made a rest day;
//! - `POST …/draft/days/{date}/sessions` — a session added after the date's
//!   entries, from a family build or written by hand; 422 `date_full` at five.
//!
//! carnet#1814. `PUT …/draft/days/{date}/{n}/coach-call` makes the coach's
//! call on session n, « C'est mon choix », or takes it off. It is no edit of
//! the session: the card marks the day `coach_call`, never `edited`, and the
//! call goes live with the draft. A session that already stands as asked is
//! answered as it is, and no draft is opened for it.
//!
//! carnet#1815. `POST …/draft/dates/{date}/revert` puts the live plan's
//! entries back on a date of the draft, whole, in place of every entry the
//! draft holds there: the workspace's « Refuser » on a date the agent
//! proposed. It names no session and carries no body — it writes no session
//! over another, and lands the date on what the athlete already has, so a
//! draft that moved since the client read it comes to the same state. A date
//! with nothing to put back is answered as it stands, and no draft is opened
//! for it.
//!
//! Every write that lands is recorded in the plan's change history as the
//! coach's, naming its date (`GET …/subject-plan/history`).
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
//! naming its `details.reason`, as is a hand-written day past the agent's day
//! bounds (`invalid_session`). A value outside the preset's evidence range
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
use pierre_memory::training_plans::{parse_plan_date, MAX_SESSIONS_PER_DATE};
use pierre_middleware::AuthenticatedUser;
use pierre_services::coach_plan_draft::CoachDraftTarget;
use pierre_tool_runtime::implementations::coach_session_editor::{
    add_draft_session, preview_session, remove_draft_day, restore_draft_day, revert_draft_date,
    save_draft_coach_call, save_draft_day, save_draft_hand_day, thread_families, DraftDayOutcome,
    EditorThread, EditorWords, SessionAt,
};
use pierre_tool_runtime::implementations::plan_scope::PlanWriter;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::tools::runtime_adapter::into_runtime;

use super::common::{get_tenant_id, refusal, refuse_external_coach_write};
use super::dto::{
    AddSessionRequest, CoachCallRequest, DraftSessionRequest, FamilyCatalogueResponse,
    HandSessionRequest, SessionExpectRequest, SessionPreviewRequest,
};
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

/// A write on one day of the coach's draft, past the gates every such write
/// runs.
struct DayWrite {
    /// The coach's thread about the athlete.
    thread: CoachedThread,
    /// The day written.
    date: CivilDate,
    /// The draft and its writer.
    editor: EditorThread,
}

/// The gates a write on one day of the coach's draft runs, in order: an
/// outside client is refused before anything is read; the thread must be
/// the caller's own about an athlete they coach; the day must be a date; and
/// the write goes on the athlete's record as a `coach_thread` read before
/// anything of theirs is read.
async fn day_write(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
    conversation_id: &str,
    raw_date: &str,
) -> Result<DayWrite, AppError> {
    refuse_external_coach_write(auth, "plan")?;
    let thread = coached_thread(resources, auth, conversation_id).await?;
    let date = plan_date(raw_date)?;
    record_thread_read(resources, &thread.access, auth.user_id).await?;
    let editor = editor_thread(resources, auth, &thread).await?;
    Ok(DayWrite {
        thread,
        date,
        editor,
    })
}

/// `refused` answered with `status`: a 422 when the request is well formed
/// and names a real day and the plan has nothing to apply it to, a 409 when
/// the session it names moved.
fn answered(status: StatusCode, refused: AppError) -> Response {
    (status, Json(ErrorResponse::from(refused))).into_response()
}

/// A 422 naming `reason`.
fn unprocessable(reason: &str, message: &str) -> Response {
    answered(
        StatusCode::UNPROCESSABLE_ENTITY,
        refusal(ErrorCode::InvalidInput, reason, message),
    )
}

/// The answer to a write on one day of the coach's draft: the subject plan
/// as `GET …/subject-plan` reads it after the write, in the coach's locale
/// (or `requested`), or the refusal saying why nothing was written.
async fn day_write_answer(
    resources: &Arc<ServerContext>,
    coach: Uuid,
    write: DayWrite,
    outcome: DraftDayOutcome,
    requested: Option<&str>,
) -> Result<Response, AppError> {
    let DayWrite { thread, date, .. } = write;
    let athlete = &thread.access.display_name;
    match outcome {
        DraftDayOutcome::Saved | DraftDayOutcome::Unchanged => {}
        DraftDayOutcome::NoPlan => {
            return Err(refusal(
                ErrorCode::ResourceNotFound,
                "no_plan",
                &format!("{athlete} has no plan and you hold no draft of one to add a day to"),
            ));
        }
        DraftDayOutcome::NoDraft => {
            return Err(refusal(
                ErrorCode::ResourceNotFound,
                "no_plan",
                &format!("You hold no draft of {athlete}'s plan to restore a day of"),
            ));
        }
        DraftDayOutcome::NoPlanWeek => {
            return Ok(unprocessable(
                "no_plan_week",
                &format!("No week of {athlete}'s plan holds {date}"),
            ));
        }
        DraftDayOutcome::NothingToRestore => {
            return Ok(unprocessable(
                "nothing_to_restore",
                &format!("You have not edited or added this session of {date} in your draft"),
            ));
        }
        DraftDayOutcome::SessionMoved => {
            return Ok(answered(
                StatusCode::CONFLICT,
                refusal(
                    ErrorCode::InvalidInput,
                    "session_moved",
                    &format!("The session you opened on {date} has changed; open the day again"),
                ),
            ));
        }
        DraftDayOutcome::LastSession => {
            return Ok(unprocessable(
                "last_session",
                &format!("This is the only session on {date}; make it a rest day instead"),
            ));
        }
        DraftDayOutcome::DateFull => {
            return Ok(unprocessable(
                "date_full",
                &format!("{date} already holds {MAX_SESSIONS_PER_DATE} sessions"),
            ));
        }
    }
    let response = subject_plan_response(resources, thread.access, coach, requested).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `PUT /api/chat/conversations/{id}/subject-plan/draft/days/{date}/{n}`: a
/// family build saved over session n of the date in the coach's draft
/// (carnet#1716), answered with the subject plan as `GET …/subject-plan`
/// reads it after the write.
///
/// 409 `session_moved` when position n no longer holds the session `expect`
/// names; the build's 400 refusals; 404 `no_plan` when the athlete has no
/// live plan and the coach no draft; 422 `no_plan_week` when no week of the
/// plan holds the date; 403 `external_transport` for an outside client.
/// Nothing is written on any of them.
pub(super) async fn put_subject_draft_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date, position)): Path<(String, String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<DraftSessionRequest>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let runtime = into_runtime(&resources);
    let outcome = save_draft_day(&runtime, &write.editor, write.date, &at, &request.build).await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}

/// `PUT /api/chat/conversations/{id}/subject-plan/draft/days/{date}/{n}/session`:
/// session n of the date as the coach wrote it by hand, or a rest entry, saved
/// into the coach's draft (carnet#1714, carnet#1716), a rest entry's session
/// named in the coach's locale (or `?locale=`). Answered with the subject
/// plan as `GET …/subject-plan` reads it after the write.
///
/// 409 `session_moved`; 400 `invalid_session` for a day past the agent's day
/// bounds or a second rest entry on the date; 404 `no_plan`; 422
/// `no_plan_week`; 403 `external_transport`. Nothing is written on any of
/// them.
pub(super) async fn put_subject_draft_hand_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date, position)): Path<(String, String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<HandSessionRequest>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let locale = coach_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let runtime = into_runtime(&resources);
    let outcome = save_draft_hand_day(
        &runtime,
        &write.editor,
        write.date,
        &at,
        &request.day,
        words,
    )
    .await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}

/// `POST /api/chat/conversations/{id}/subject-plan/draft/days/{date}/{n}/restore`:
/// session n put back as it stood before the coach's first edit in their
/// draft (carnet#1714), or removed when the coach added it (carnet#1716),
/// answered with the subject plan as `GET …/subject-plan` reads it after the
/// write.
///
/// 409 `session_moved`; 404 `no_plan` when the coach holds no draft (a
/// restore never opens one); 422 `no_plan_week`; 422 `nothing_to_restore`
/// when the coach neither edited nor added the session; 403
/// `external_transport`. Nothing is written on any of them.
pub(super) async fn restore_subject_draft_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date, position)): Path<(String, String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<SessionExpectRequest>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let outcome =
        restore_draft_day(&into_runtime(&resources), &write.editor, write.date, &at).await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}

/// `POST /api/chat/conversations/{id}/subject-plan/draft/days/{date}/{n}/remove`:
/// session n removed from the coach's draft (carnet#1716), opened from the
/// live plan when the coach holds none, answered with the subject plan as
/// `GET …/subject-plan` reads it after the write.
///
/// 409 `session_moved`; 422 `last_session` when it is the date's only entry,
/// which the coach makes a rest day instead; 404 `no_plan`; 422
/// `no_plan_week`; 403 `external_transport`. Nothing is written on any of
/// them.
pub(super) async fn remove_subject_draft_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date, position)): Path<(String, String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<SessionExpectRequest>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let outcome =
        remove_draft_day(&into_runtime(&resources), &write.editor, write.date, &at).await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}

/// `PUT /api/chat/conversations/{id}/subject-plan/draft/days/{date}/{n}/coach-call`:
/// the coach's call on session n of the date made (`on: true`) or taken off
/// in their draft (carnet#1814), opened from the live plan when the coach
/// holds none. The session is left as it stands and is not marked edited.
/// Answered with the subject plan as `GET …/subject-plan` reads it after the
/// write, or as it stands when the session already stood as asked.
///
/// 409 `session_moved`; 404 `no_plan`; 422 `no_plan_week`; 403
/// `external_transport`. Nothing is written on any of them.
pub(super) async fn put_subject_draft_coach_call(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date, position)): Path<(String, String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<CoachCallRequest>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let runtime = into_runtime(&resources);
    let outcome =
        save_draft_coach_call(&runtime, &write.editor, write.date, &at, request.on).await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}

/// `POST /api/chat/conversations/{id}/subject-plan/draft/dates/{date}/revert`:
/// every entry on the date in the coach's draft replaced by the athlete's
/// live plan's entries on it, whole (carnet#1815); a date the live plan holds
/// nothing on is left empty. No body. Answered with the subject plan as `GET
/// …/subject-plan` reads it after the write, or as it stands when there was
/// nothing to put back: no draft, a week the draft never wrote, or a date
/// already holding the live entries — none of which opens a draft.
///
/// 400 `invalid_date`; 403 `external_transport`. Nothing is written on
/// either.
pub(super) async fn revert_subject_draft_date(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date)): Path<(String, String)>,
    Query(query): Query<TrainingPlanQuery>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let outcome = revert_draft_date(&into_runtime(&resources), &write.editor, write.date).await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}

/// `POST /api/chat/conversations/{id}/subject-plan/draft/days/{date}/sessions`:
/// a session added after the date's entries in the coach's draft, built from
/// a family (`{ "build": … }`) or written by hand (`{ "day": … }`), marked as
/// one the coach added (carnet#1716). The date need hold no entry yet.
/// Answered with the subject plan as `GET …/subject-plan` reads it after the
/// write.
///
/// 422 `date_full` when the date holds five entries; 400 `invalid_session`
/// for a day past the agent's day bounds or a second rest entry, and the
/// build's 400 refusals; 404 `no_plan`; 422 `no_plan_week`; 403
/// `external_transport`. Nothing is written on any of them.
pub(super) async fn add_subject_draft_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((conversation_id, raw_date)): Path<(String, String)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(session): Json<AddSessionRequest>,
) -> Result<Response, AppError> {
    let write = day_write(&resources, &auth, &conversation_id, &raw_date).await?;
    let locale = coach_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let runtime = into_runtime(&resources);
    let outcome = add_draft_session(&runtime, &write.editor, write.date, &session, words).await?;
    day_write_answer(
        &resources,
        auth.user_id,
        write,
        outcome,
        query.locale.as_deref(),
    )
    .await
}
