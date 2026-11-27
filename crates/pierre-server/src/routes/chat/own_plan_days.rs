// ABOUTME: Handlers for the athlete's own day writes under /api/me/plan/days — add, family build, hand edit, restore, remove, move
// ABOUTME: Each lands live through write_live_weeks, refuses an outside client, and answers with the plan as GET /api/me/plan reads it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1841. The athlete edits their own plan from their Plan tab, and
//! what they ask for applies at once, live. A session is addressed by its
//! date and 0-based position `{n}` among the date's entries, and every write
//! names it as the client last saw it (`expect`); a position that no longer
//! holds that session is a 409 `session_moved`, nothing written.
//!
//! - `POST /api/me/plan/days/{date}/sessions` — a session added after the
//!   date's entries, built from a family or written by hand; with no plan
//!   yet it starts the season (no race) and the week. 422 `date_full` at
//!   five, `past_date` before the athlete's today;
//! - `PUT /api/me/plan/days/{date}/{n}` — a family build over session n;
//! - `PUT /api/me/plan/days/{date}/{n}/session` — session n written by hand,
//!   or a rest entry;
//! - `POST /api/me/plan/days/{date}/{n}/restore` — session n put back, or
//!   removed when the athlete added it; 422 `nothing_to_restore`;
//! - `POST /api/me/plan/days/{date}/{n}/remove` — session n removed; never
//!   the date's last entry (422 `last_session`);
//! - `POST /api/me/plan/days/{date}/{n}/move` — session n moved to
//!   `to_date`, in one write over the one or two weeks it touches; 422
//!   `same_date`, `past_date`, `date_full`.
//!
//! Every write is refused to an outside client (403 `external_transport`)
//! before anything is read, and answers with the plan as `GET /api/me/plan`
//! reads it after the write.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::CivilDate;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_memory::training_plans::MAX_SESSIONS_PER_DATE;
use pierre_middleware::AuthenticatedUser;
use pierre_services::athlete_clock::athlete_today;
use pierre_tool_runtime::implementations::coach_session_editor::{
    add_own_session, move_own_session, remove_own_day, restore_own_day, save_own_day,
    save_own_hand_day, DraftDayOutcome, EditorWords, SessionAt,
};
use pierre_tool_runtime::implementations::plan_scope::external_own_write_error;

use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::TrainingPlanQuery;
use crate::tools::runtime_adapter::into_runtime;

use super::common::refusal;
use super::dto::{
    AddSessionRequest, DraftSessionRequest, HandSessionRequest, MoveSessionRequest,
    SessionExpectRequest,
};
use super::own_plan::{athlete_locale, own_caller, own_plan_response, OwnCaller};
use super::subject_session::{answered, plan_date, unprocessable};

/// A write on one day of the athlete's own plan, past the gates every such
/// write runs.
pub(super) struct OwnDayWrite {
    /// The athlete.
    pub caller: OwnCaller,
    /// The day written.
    pub date: CivilDate,
    /// The athlete's civil date: nothing is added or moved before it.
    pub today: CivilDate,
}

/// The gates a write on the athlete's own plan runs, in order: an outside
/// client is refused before anything is read, then the day must be a date.
pub(super) async fn own_day_write(
    resources: &Arc<ServerContext>,
    auth: &AuthenticatedUser,
    raw_date: &str,
) -> Result<OwnDayWrite, AppError> {
    if auth.acts_for_outside_client() {
        return Err(external_own_write_error());
    }
    let caller = own_caller(resources, auth).await?;
    let date = plan_date(raw_date)?;
    let today = athlete_today(&resources.common.repos, auth.user_id).await;
    Ok(OwnDayWrite {
        caller,
        date,
        today,
    })
}

/// The answer to a write on the athlete's own plan: the plan as `GET
/// /api/me/plan` reads it after the write, in their locale (or
/// `requested`), or the refusal saying why nothing was written.
pub(super) async fn own_day_answer(
    resources: &Arc<ServerContext>,
    write: &OwnDayWrite,
    outcome: DraftDayOutcome,
    requested: Option<&str>,
) -> Result<Response, AppError> {
    let date = write.date;
    match outcome {
        DraftDayOutcome::Saved | DraftDayOutcome::Unchanged => {}
        DraftDayOutcome::NoPlan | DraftDayOutcome::NoDraft => {
            return Err(refusal(
                ErrorCode::ResourceNotFound,
                "no_plan",
                "You have no plan yet: add a session to start one",
            ));
        }
        DraftDayOutcome::NoPlanWeek => {
            return Ok(unprocessable(
                "no_plan_week",
                &format!("No week of your plan holds {date}"),
            ));
        }
        DraftDayOutcome::NothingToRestore => {
            return Ok(unprocessable(
                "nothing_to_restore",
                &format!("You have not edited or added this session of {date}"),
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
                &format!("That date already holds {MAX_SESSIONS_PER_DATE} sessions"),
            ));
        }
        DraftDayOutcome::PastDate => {
            return Ok(unprocessable(
                "past_date",
                "Sessions are added and moved only from today on",
            ));
        }
        DraftDayOutcome::SameDate => {
            return Ok(unprocessable(
                "same_date",
                &format!("The session is already on {date}"),
            ));
        }
    }
    let response = own_plan_response(resources, &write.caller, requested).await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// `POST /api/me/plan/days/{date}/sessions`: a session added after the
/// date's entries of the athlete's live plan, built from a family (`{
/// "build": … }`) or written by hand (`{ "day": … }`); with no plan yet it
/// starts the season, with no race, and the week.
///
/// 422 `date_full`, `past_date`; 400 `invalid_session` and the build's 400
/// refusals; 403 `external_transport`. Nothing is written on any of them.
pub(super) async fn add_own_day_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(raw_date): Path<String>,
    Query(query): Query<TrainingPlanQuery>,
    Json(session): Json<AddSessionRequest>,
) -> Result<Response, AppError> {
    let write = own_day_write(&resources, &auth, &raw_date).await?;
    let locale = athlete_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let runtime = into_runtime(&resources);
    let own = &write.caller.own;
    let outcome = add_own_session(&runtime, own, write.date, &session, words, write.today).await?;
    own_day_answer(&resources, &write, outcome, query.locale.as_deref()).await
}

/// `PUT /api/me/plan/days/{date}/{n}`: a family build saved over session n
/// of the date on the athlete's live plan.
///
/// 409 `session_moved`; the build's 400 refusals; 404 `no_plan`; 422
/// `no_plan_week`; 403 `external_transport`. Nothing is written on any of
/// them.
pub(super) async fn put_own_day_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((raw_date, position)): Path<(String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<DraftSessionRequest>,
) -> Result<Response, AppError> {
    let write = own_day_write(&resources, &auth, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let runtime = into_runtime(&resources);
    let outcome =
        save_own_day(&runtime, &write.caller.own, write.date, &at, &request.build).await?;
    own_day_answer(&resources, &write, outcome, query.locale.as_deref()).await
}

/// `PUT /api/me/plan/days/{date}/{n}/session`: session n of the date on the
/// athlete's live plan as they wrote it by hand, or a rest entry named in
/// their locale (or `?locale=`).
///
/// 409 `session_moved`; 400 `invalid_session`; 404 `no_plan`; 422
/// `no_plan_week`; 403 `external_transport`. Nothing is written on any of
/// them.
pub(super) async fn put_own_day_hand_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((raw_date, position)): Path<(String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<HandSessionRequest>,
) -> Result<Response, AppError> {
    let write = own_day_write(&resources, &auth, &raw_date).await?;
    let locale = athlete_locale(&resources, auth.user_id, query.locale.as_deref()).await?;
    let words = EditorWords {
        strings: &resources.mcp.messaging_strings_registry,
        locale: &locale,
    };
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let runtime = into_runtime(&resources);
    let own = &write.caller.own;
    let outcome = save_own_hand_day(&runtime, own, write.date, &at, &request.day, words).await?;
    own_day_answer(&resources, &write, outcome, query.locale.as_deref()).await
}

/// `POST /api/me/plan/days/{date}/{n}/restore`: session n put back as it
/// stood before the athlete's first edit, or removed when they added it.
///
/// 409 `session_moved`; 422 `nothing_to_restore`; 404 `no_plan`; 422
/// `no_plan_week`; 403 `external_transport`. Nothing is written on any of
/// them.
pub(super) async fn restore_own_day_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((raw_date, position)): Path<(String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<SessionExpectRequest>,
) -> Result<Response, AppError> {
    let write = own_day_write(&resources, &auth, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let outcome = restore_own_day(
        &into_runtime(&resources),
        &write.caller.own,
        write.date,
        &at,
    )
    .await?;
    own_day_answer(&resources, &write, outcome, query.locale.as_deref()).await
}

/// `POST /api/me/plan/days/{date}/{n}/remove`: session n removed from the
/// athlete's live plan.
///
/// 409 `session_moved`; 422 `last_session`; 404 `no_plan`; 422
/// `no_plan_week`; 403 `external_transport`. Nothing is written on any of
/// them.
pub(super) async fn remove_own_day_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((raw_date, position)): Path<(String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<SessionExpectRequest>,
) -> Result<Response, AppError> {
    let write = own_day_write(&resources, &auth, &raw_date).await?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let outcome = remove_own_day(
        &into_runtime(&resources),
        &write.caller.own,
        write.date,
        &at,
    )
    .await?;
    own_day_answer(&resources, &write, outcome, query.locale.as_deref()).await
}

/// `POST /api/me/plan/days/{date}/{n}/move`: session n of the date moved to
/// `to_date` on the athlete's live plan, in one write over the one or two
/// weeks it touches.
///
/// 409 `session_moved`; 422 `same_date`, `past_date`, `date_full`; 400
/// `invalid_date` for a `to_date` that is no date; 404 `no_plan`; 403
/// `external_transport`. Nothing is written on any of them.
pub(super) async fn move_own_day_session(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((raw_date, position)): Path<(String, u32)>,
    Query(query): Query<TrainingPlanQuery>,
    Json(request): Json<MoveSessionRequest>,
) -> Result<Response, AppError> {
    let write = own_day_write(&resources, &auth, &raw_date).await?;
    let to = plan_date(&request.to_date)?;
    let at = SessionAt {
        position,
        expect: request.expect,
    };
    let runtime = into_runtime(&resources);
    let own = &write.caller.own;
    let outcome = move_own_session(&runtime, own, write.date, &at, to, write.today).await?;
    own_day_answer(&resources, &write, outcome, query.locale.as_deref()).await
}
