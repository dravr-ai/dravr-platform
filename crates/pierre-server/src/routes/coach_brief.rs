// ABOUTME: The coach's morning brief as To handle controls it: read and change it, and say an athlete was opened
// ABOUTME: A change records routine.paused, .modified or .dismissed; an open of a listed athlete records routine.acted

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach morning brief's routes (Routines P2, carnet#1791).
//!
//! - `GET /api/coach/brief` — the brief: whether it fires, its hour, what it
//!   leaves out, its last run and its next.
//! - `PUT /api/coach/brief` — pause or resume it, move its hour, switch
//!   reasons off, leave athletes out.
//! - `POST /api/coach/brief/opened` — the coach opened an athlete: counted as
//!   acting on that athlete's item when a brief in the last 24 hours listed
//!   them. Marking an item Handled counts the same way, from
//!   `POST /api/coach/attention/dismiss`.
//!
//! Only a coach who may read a roster reaches any of them. The brief is the
//! built-in routine the engine gives every coach; a coach the engine has not
//! reconciled yet gets theirs on first read.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    BriefSettings, Routine, RoutineKind, RoutineRun, RoutineScope, RoutineState, RoutineTrigger,
    RunOutcome, Schedule, TenantId, Weekdays, MORNING_BRIEF_HOUR, NO_WAKE_BEFORE_HOUR,
};
use pierre_database::repositories::RoutineChange;
use pierre_middleware::AuthenticatedUser;
use pierre_services::coach_attention::AttentionKind;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::coach_roster::roster_groups;
use crate::services::morning_brief::{record_acted, ActedOn};

/// The brief as To handle shows it.
#[derive(Debug, Serialize)]
pub struct BriefResponse {
    /// `active` or `paused`.
    pub state: RoutineState,
    /// The local hour it fires at (5–23).
    pub hour: u32,
    /// Attention reasons it does not check.
    pub reasons_off: Vec<String>,
    /// Athletes it does not list.
    pub muted: Vec<Uuid>,
    /// Its latest run; `None` before it first ran.
    pub last_run: Option<BriefRun>,
    /// When it fires next; `None` while paused or not yet scheduled.
    pub next_run: Option<Timestamp>,
}

/// One run of the brief.
#[derive(Debug, Serialize)]
pub struct BriefRun {
    /// When it ran.
    pub ran_at: Timestamp,
    /// What it did: `posted`, `withheld`, `empty`, `held` or `skipped`.
    pub outcome: RunOutcome,
    /// How many athletes it listed.
    pub items: u32,
    /// How many it left out because they don't share with the coach.
    pub not_sharing: u32,
    /// How many it left out because their data could not be read.
    pub unreadable: u32,
    /// How many it did not check for missed sessions because a calendar
    /// could not be read (carnet#1792).
    pub missed_unreadable: u32,
    /// How many it did not check for missed sessions because they switched
    /// the check off.
    pub missed_off: u32,
}

impl From<RoutineRun> for BriefRun {
    fn from(run: RoutineRun) -> Self {
        Self {
            ran_at: run.ran_at,
            outcome: run.outcome,
            items: run.items,
            not_sharing: run.left_out.not_sharing,
            unreadable: run.left_out.unreadable,
            missed_unreadable: run.left_out.missed_unreadable,
            missed_off: run.left_out.missed_off,
        }
    }
}

/// `PUT /api/coach/brief`: each field changes that setting; an absent one
/// keeps it.
#[derive(Debug, Deserialize)]
pub struct BriefChange {
    /// Pause (`true`) or resume (`false`) it.
    pub paused: Option<bool>,
    /// The local hour it fires at, 5–23.
    pub hour: Option<u32>,
    /// The attention reasons it does not check, in full.
    pub reasons_off: Option<Vec<AttentionKind>>,
    /// The athletes it does not list, in full.
    pub muted: Option<Vec<Uuid>>,
}

/// `POST /api/coach/brief/opened`.
#[derive(Debug, Deserialize)]
pub struct OpenedRequest {
    /// The athlete the coach opened.
    pub athlete_user_id: Uuid,
}

/// Mount-helper for the brief's routes.
#[derive(Debug)]
pub struct CoachBriefRoutes;

impl CoachBriefRoutes {
    /// The brief's routes. Each requires a session; only a coach who may read
    /// a roster reaches them.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/coach/brief", get(handle_get).put(handle_put))
            .route("/api/coach/brief/opened", post(handle_opened))
            .with_state(resources)
    }
}

/// `GET /api/coach/brief`.
///
/// # Errors
///
/// 403 when the caller may not read a roster; the repository error otherwise.
pub async fn handle_get(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let coach = auth.into_inner().user_id;
    roster_groups(&resources.common.repos, coach).await?;
    let routine = coach_brief(&resources, coach).await?;
    let body = respond(&resources, &routine).await?;
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `PUT /api/coach/brief`.
///
/// # Errors
///
/// 403 when the caller may not read a roster, invalid input for an hour
/// before 05:00 or past 23; the repository error otherwise.
pub async fn handle_put(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(change): Json<BriefChange>,
) -> Result<Response, AppError> {
    let coach = auth.into_inner().user_id;
    let repos = &resources.common.repos;
    roster_groups(repos, coach).await?;
    if let Some(hour) = change.hour {
        if !(NO_WAKE_BEFORE_HOUR..24).contains(&hour) {
            return Err(AppError::invalid_input(format!(
                "hour must be {NO_WAKE_BEFORE_HOUR} to 23: nothing fires earlier"
            )));
        }
    }
    let routine = coach_brief(&resources, coach).await?;
    let before = Current::of(&routine);
    let after = before.changed_by(&change);
    let now = Timestamp::now();
    let applied = RoutineChange {
        state: after.state,
        hour: after.hour,
        brief: after.brief.clone(),
    };
    repos
        .routines
        .update_routine(routine.tenant_id, routine.id, &applied, now)
        .await?;
    report_changes(&routine, &before, &after);
    if (after.state, after.hour) != (before.state, before.hour) {
        if let Some(engine) = resources.mcp.routines.get() {
            engine
                .reschedule(routine.tenant_id, coach, RoutineKind::MorningBrief, now)
                .await;
        }
    }
    let updated = repos
        .routines
        .routine(routine.tenant_id, routine.id)
        .await?
        .ok_or_else(|| AppError::not_found("Morning brief"))?;
    let body = respond(&resources, &updated).await?;
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `POST /api/coach/brief/opened` — 204 whether or not a brief listed them.
///
/// # Errors
///
/// 403 when the caller may not read a roster.
pub async fn handle_opened(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(body): Json<OpenedRequest>,
) -> Result<Response, AppError> {
    let coach = auth.into_inner().user_id;
    roster_groups(&resources.common.repos, coach).await?;
    record_acted(&resources, coach, body.athlete_user_id, ActedOn::Opened).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// What a brief's settings are, before or after a change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Current {
    state: RoutineState,
    hour: u32,
    brief: BriefSettings,
}

impl Current {
    /// What `routine` holds now. A brief always has a schedule; a stored row
    /// with none reads as the default hour.
    fn of(routine: &Routine) -> Self {
        let hour = routine
            .trigger
            .map_or(MORNING_BRIEF_HOUR, |trigger| match trigger {
                RoutineTrigger::Schedule(schedule) => schedule.hour(),
            });
        Self {
            state: routine.state,
            hour,
            brief: routine.brief.clone(),
        }
    }

    /// The settings once `change` is applied.
    fn changed_by(&self, change: &BriefChange) -> Self {
        let mut next = self.clone();
        if let Some(paused) = change.paused {
            next.state = if paused {
                RoutineState::Paused
            } else {
                RoutineState::Active
            };
        }
        if let Some(hour) = change.hour {
            next.hour = hour;
        }
        if let Some(reasons) = &change.reasons_off {
            // The brief never lists a stop (it only counts it), so switching
            // the stop off would be a setting that does nothing: it is dropped.
            let unique: BTreeSet<&str> = reasons
                .iter()
                .filter(|kind| **kind != AttentionKind::ConsentRevoked)
                .map(|kind| kind.as_str())
                .collect();
            next.brief.reasons_off = unique.into_iter().map(str::to_owned).collect();
        }
        if let Some(muted) = &change.muted {
            let unique: BTreeSet<Uuid> = muted.iter().copied().collect();
            next.brief.muted = unique.into_iter().collect();
        }
        next
    }
}

/// The coach's brief in their home tenant, created now when the engine has
/// not given them one yet.
async fn coach_brief(resources: &ServerContext, coach: Uuid) -> AppResult<Routine> {
    let repos = &resources.common.repos;
    let home: TenantId = repos
        .tenants
        .first_for_user(coach)
        .await?
        .ok_or_else(|| AppError::not_found("Your tenant"))?;
    if let Some(routine) = repos
        .routines
        .builtin_routine(home, coach, RoutineKind::MorningBrief)
        .await?
    {
        return Ok(routine);
    }
    let schedule = Schedule::try_new(Weekdays::EVERY_DAY, MORNING_BRIEF_HOUR)
        .ok_or_else(|| AppError::internal("the morning brief's default schedule is invalid"))?;
    let now = Timestamp::now();
    repos
        .routines
        .ensure_builtin_routine(
            home,
            coach,
            RoutineKind::MorningBrief,
            RoutineScope::Roster,
            schedule,
            now,
        )
        .await?;
    if let Some(engine) = resources.mcp.routines.get() {
        engine
            .reschedule(home, coach, RoutineKind::MorningBrief, now)
            .await;
    }
    repos
        .routines
        .builtin_routine(home, coach, RoutineKind::MorningBrief)
        .await?
        .ok_or_else(|| AppError::not_found("Morning brief"))
}

/// The response for `routine`, with its last run.
async fn respond(resources: &ServerContext, routine: &Routine) -> AppResult<BriefResponse> {
    let current = Current::of(routine);
    let last_run = resources
        .common
        .repos
        .routines
        .last_run(routine.tenant_id, routine.id)
        .await?;
    Ok(BriefResponse {
        state: current.state,
        hour: current.hour,
        reasons_off: current.brief.reasons_off,
        muted: current.brief.muted,
        last_run: last_run.map(BriefRun::from),
        next_run: (current.state == RoutineState::Active)
            .then_some(routine.next_fire)
            .flatten(),
    })
}

/// Report what the coach changed: paused or resumed, a new hour, and each
/// athlete newly left out or reason newly switched off.
fn report_changes(routine: &Routine, before: &Current, after: &Current) {
    if after.state != before.state {
        info!(
            target: "notify",
            event = "routine.paused",
            user_id = %routine.owner_user_id,
            tenant_id = %routine.tenant_id,
            kind = routine.kind.as_str(),
            paused = after.state == RoutineState::Paused,
            "routine paused or resumed"
        );
    }
    if after.hour != before.hour {
        info!(
            target: "notify",
            event = "routine.modified",
            user_id = %routine.owner_user_id,
            tenant_id = %routine.tenant_id,
            kind = routine.kind.as_str(),
            field = "hour",
            "routine modified"
        );
    }
    let newly_muted = after
        .brief
        .muted
        .iter()
        .filter(|athlete| !before.brief.muted.contains(athlete))
        .count();
    let newly_off = after
        .brief
        .reasons_off
        .iter()
        .filter(|reason| !before.brief.reasons_off.contains(reason))
        .count();
    report_dismissed(routine, "athlete_muted", newly_muted);
    report_dismissed(routine, "reason_off", newly_off);
}

/// Report `count` parts of the brief's content the coach turned away, `what`.
fn report_dismissed(routine: &Routine, what: &str, count: usize) {
    for _ in 0..count {
        info!(
            target: "notify",
            event = "routine.dismissed",
            user_id = %routine.owner_user_id,
            tenant_id = %routine.tenant_id,
            kind = routine.kind.as_str(),
            what,
            "routine content turned away"
        );
    }
}
