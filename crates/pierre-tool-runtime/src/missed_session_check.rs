// ABOUTME: The missed-session check of one coached athlete: their calendars, their Dravr plan days and activities, judged
// ABOUTME: Read live for To handle and the morning brief (carnet#1792); a calendar or activity read that fails is never a miss
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The missed-session check
//!
//! Gathers what [`pierre_services::missed_sessions`] judges, for one athlete
//! a coach reads on their roster:
//!
//! 1. **The calendars the coach reaches** on a coaching platform
//!    (intervals.icu, TrainingPeaks), read live as the coach's own
//!    `get_planned_workouts` reads them
//!    (`read_coached_calendars`): each session with the provider's own
//!    pairing, and the pairing of the events Dravr pushed there.
//! 2. **The athlete's Dravr plan days**, when the coaching gate let the coach
//!    read the plan: a day pushed to a calendar that pairs it is judged by
//!    that pairing; any other by an activity of its sport within a day.
//! 3. **Their recorded activities**, only when a plan day needs them.
//!
//! What the check cannot read, it does not judge. A calendar the coach
//! reaches but cannot read, or activities the reader's gates hold back,
//! make the check [`PlannedVsDone::Unreadable`]: an absent completion is
//! never the answer to a read that did not happen. An athlete who switched
//! the check off for this coach is [`PlannedVsDone::Off`] and nothing of
//! theirs is read for it.
//!
//! With a run of missed planned days, and only under the athlete's wellness
//! grant to the coach, their last wellness note in the window is read and
//! recorded on their who-read record. It is evidence for the coach and is
//! never put in front of a model: an outside client's read, and a read a
//! model receives (the morning brief's), leave it out.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Timestamp, Zone};
use pierre_config::coach_home::CoachHomeConfig;
use pierre_core::civil_time;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CalendarKey, TenantId};
use pierre_database::repositories::CoachReadClass;
use pierre_memory::training_plans::parse_plan_date;
use pierre_providers::ai_scope;
use pierre_services::coached_athlete::CoachedAthleteAccess;
use pierre_services::missed_sessions::{
    self, DoneActivity, MissedRun, Pairing, PlannedSession, PlannedVsDone, WellnessNote,
};
use pierre_services::plan_calendar_push::plan_sport;
use pierre_services::stored_health::{self, SourceFilter};
use uuid::Uuid;

use crate::implementations::planned_workouts::{read_coached_calendars, CoachedCalendars};
use crate::read_subject::record_coach_read;
use crate::runtime::ToolRuntime;

/// The most activities one check reads: two weeks of several a day.
const ACTIVITY_LIMIT: i64 = 200;

/// One athlete's missed-session check, as the roster asks for it.
#[derive(Debug, Clone, Copy)]
pub struct MissedCheckRequest<'req> {
    /// The coach reading the roster.
    pub coach: Uuid,
    /// What the coach-read gate answered for the athlete: their home tenant,
    /// the group and their consents to the coach there.
    pub access: &'req CoachedAthleteAccess,
    /// The athlete's zone, which their calendar days and activities are read
    /// in.
    pub zone: &'req Zone,
    /// The athlete's own civil today.
    pub today: CivilDate,
    /// The run length and the window read.
    pub thresholds: &'req CoachHomeConfig,
    /// The coaching gate let the coach read the athlete's plan.
    pub plan_read: bool,
    /// The surface a wellness read goes on the athlete's record under.
    pub surface: &'req str,
    /// The roster is served to an outside client acting for the coach.
    pub outside: bool,
    /// A model reads the result (a morning brief's findings).
    pub for_model: bool,
}

/// The missed-session check of the athlete `request` names.
///
/// # Errors
///
/// Returns a repository error from the plan, activity, wellness or record
/// reads, or the error of building a calendar reader.
pub async fn check_missed_sessions(
    resources: &Arc<dyn ToolRuntime>,
    request: MissedCheckRequest<'_>,
) -> AppResult<PlannedVsDone> {
    let access = request.access;
    if !access.consents.missed_session_check {
        return Ok(PlannedVsDone::Off);
    }
    let lookback = i64::from(request.thresholds.missed_lookback_days);
    let today = request.today;
    let first = today.checked_sub_days(lookback).unwrap_or(today);
    let athlete = access.athlete_user_id;
    let calendars = read_coached_calendars(
        resources,
        request.coach,
        athlete,
        access.tenant,
        (first, today),
    )
    .await?;
    if calendars.failed {
        return Ok(PlannedVsDone::Unreadable);
    }
    let mut sessions = calendar_sessions(&calendars);
    if request.plan_read {
        sessions.extend(
            plan_sessions(
                resources,
                athlete,
                access.tenant,
                &calendars,
                (first, today),
            )
            .await?,
        );
    }
    let activities = if sessions.iter().any(|s| s.pairing == Pairing::Activity) {
        match recorded_activities(
            resources,
            athlete,
            access.tenant,
            request.zone,
            (first, today),
        )
        .await?
        {
            Some(activities) => activities,
            None => return Ok(PlannedVsDone::Unreadable),
        }
    } else {
        Vec::new()
    };
    let judged = missed_sessions::judge(&sessions, &activities, today);
    let run = missed_sessions::missed_run(&judged, request.thresholds.missed_planned_days);
    // The note is evidence for the coach in Dravr's own app: a read an outside
    // client or a model receives never fetches it, nor records a wellness read
    // nobody is shown.
    let quotes_wellness = access.consents.wellness_read && !request.outside && !request.for_model;
    let wellness = match &run {
        Some(run) if quotes_wellness => last_wellness_note(resources, &request, run).await?,
        _ => None,
    };
    Ok(PlannedVsDone::Read {
        read_at: Timestamp::now(),
        run,
        wellness,
    })
}

/// The sessions others planned on the calendars read, each with its
/// provider's pairing.
fn calendar_sessions(calendars: &CoachedCalendars) -> Vec<PlannedSession> {
    calendars
        .read
        .iter()
        .flat_map(|(provider, calendar)| {
            calendar.planned.iter().map(move |workout| PlannedSession {
                date: workout.date(),
                sport: workout.sport_type().clone(),
                pairing: Pairing::Provider {
                    provider: provider.clone(),
                    completed: workout.completed_activity_id().is_some(),
                },
            })
        })
        .collect()
}

/// The non-rest days of the athlete's live Dravr plan inside `[first, last]`.
///
/// A day pushed to a calendar read that pairs Dravr's own events is judged by
/// that pairing, found by the key the push gave it: its ordinal among the
/// date's sessions in the week's order, as the push numbers them. Any other
/// day is judged by an activity of its sport.
async fn plan_sessions(
    resources: &Arc<dyn ToolRuntime>,
    athlete: Uuid,
    tenant: TenantId,
    calendars: &CoachedCalendars,
    (first, last): (CivilDate, CivilDate),
) -> AppResult<Vec<PlannedSession>> {
    let repos = resources.repos();
    let tenant_key = tenant.to_string();
    let user_key = athlete.to_string();
    let Some(plan) = repos
        .training_plans
        .get_active_plan(&tenant_key, &user_key)
        .await?
    else {
        return Ok(Vec::new());
    };
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant_key, &user_key, &plan.id, false)
        .await?;
    let pushed: HashMap<&str, (&str, bool)> = calendars
        .read
        .iter()
        .flat_map(|(provider, calendar)| {
            calendar.pushed.iter().flatten().map(move |event| {
                (
                    event.external_id.as_str(),
                    (provider.as_str(), event.completed_activity_id.is_some()),
                )
            })
        })
        .collect();
    let mut sessions = Vec::new();
    for week in &weeks {
        let mut ordinals: BTreeMap<CivilDate, usize> = BTreeMap::new();
        for day in week.days.iter().filter(|day| !day.is_rest()) {
            let Some(date) = parse_plan_date(&day.date) else {
                continue;
            };
            let ordinal = ordinals.entry(date).or_insert(0);
            let key = CalendarKey::plan_day(athlete, date, *ordinal);
            *ordinal += 1;
            if date < first || date > last {
                continue;
            }
            let pairing =
                pushed
                    .get(key.as_str())
                    .map_or(Pairing::Activity, |(provider, completed)| {
                        Pairing::Provider {
                            provider: (*provider).to_owned(),
                            completed: *completed,
                        }
                    });
            sessions.push(PlannedSession {
                date,
                sport: plan_sport(&day.sport),
                pairing,
            });
        }
    }
    Ok(sessions)
}

/// The athlete's recorded activities over `[first, last]`, a day either side.
///
/// On their own calendar days; `None` when the gates in force hold any of
/// them back, or the read stopped at [`ACTIVITY_LIMIT`] (newest first, so the
/// window's oldest days may be cut), since a session left out and read as
/// absent would be a miss that did not happen.
async fn recorded_activities(
    resources: &Arc<dyn ToolRuntime>,
    athlete: Uuid,
    tenant: TenantId,
    zone: &Zone,
    (first, last): (CivilDate, CivilDate),
) -> AppResult<Option<Vec<DoneActivity>>> {
    // A day either side of the window, in UTC: the zone moves the edges by
    // less than a day, and the days are read back in the athlete's zone.
    let start = midnight_utc(first.checked_sub_days(1).unwrap_or(first))?;
    let end = midnight_utc(last.checked_add_days(2).unwrap_or(last))?;
    let rows = resources
        .repos()
        .activity_cache
        .get_cached_activities(athlete, &tenant, None, start, end, ACTIVITY_LIMIT)
        .await?;
    let read = rows.len();
    if i64::try_from(read).unwrap_or(i64::MAX) >= ACTIVITY_LIMIT {
        return Ok(None);
    }
    let terms = resources.provider_registry();
    let kept = ai_scope::filter_activities(terms.as_ref(), rows);
    if kept.len() < read {
        return Ok(None);
    }
    Ok(Some(
        kept.iter()
            .map(|activity| DoneActivity {
                date: civil_time::local_date(activity.start_date(), zone),
                sport: activity.sport_type().clone(),
            })
            .collect(),
    ))
}

/// The athlete's last wellness note from `run`'s first day through today.
///
/// Recorded on their who-read record as their coach's wellness read before
/// it is read.
async fn last_wellness_note(
    resources: &Arc<dyn ToolRuntime>,
    request: &MissedCheckRequest<'_>,
    run: &MissedRun,
) -> AppResult<Option<WellnessNote>> {
    let repos = resources.repos();
    let access = request.access;
    record_coach_read(
        repos,
        access,
        request.coach,
        request.surface,
        CoachReadClass::Wellness,
    )
    .await?;
    let start = midnight_utc(run.since)?;
    let end = midnight_utc(request.today.checked_add_days(1).unwrap_or(request.today))?;
    let terms = resources.provider_registry();
    let recovery = stored_health::merged_recovery(
        repos,
        terms.as_ref(),
        access.athlete_user_id,
        &access.tenant,
        (start, end),
        SourceFilter::All,
        stored_health::as_stored,
    )
    .await?;
    Ok(recovery.records.into_iter().rev().find_map(|merged| {
        let text = merged.record.athlete_note?.trim().to_owned();
        (!text.is_empty()).then_some(WellnessNote {
            date: merged.record.date,
            text,
        })
    }))
}

/// The instant `date` begins in UTC.
fn midnight_utc(date: CivilDate) -> AppResult<Timestamp> {
    date.at_midnight()
        .and_utc()
        .map_err(|e| AppError::internal(format!("midnight of {date} in UTC: {e}")))
}
