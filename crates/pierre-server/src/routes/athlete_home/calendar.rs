// ABOUTME: GET /api/me/calendar — the athlete's days between two civil dates: cached activities on each day, the plan's weeks over them
// ABOUTME: Served from the activity cache and the plan store alone; it never reaches a provider nor starts a backfill

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Home's calendar.
//!
//! The week strip on Home pages to past and next weeks and expands to a
//! month, so it reads any span of up to [`MAX_CALENDAR_DAYS`] days:
//!
//! - **activities** — every workout the durable cache holds whose start falls
//!   on one of the days, on the athlete's own calendar (`local_date`, so a
//!   21:00 Toronto run is the day it was run and a date-only row keeps its
//!   day), merged one row per workout exactly as the recent list merges
//!   them, oldest first;
//! - **plan weeks** — every week of the athlete's active plan that overlaps
//!   the span, projected as the plan card projects a week, or `null` when
//!   there is no active plan, so a client can tell "no plan" from "the plan
//!   says nothing about these days";
//! - **history start** — the first day the cache still holds in full. Rows
//!   older than the retention window are pruned, so a day before it is
//!   unknown, never a day without training; a client stops paging there and
//!   says so rather than drawing an empty grid that reads as zeros.
//!
//! The read is the cache's: a span with nothing cached answers an empty list
//! and starts no refresh and no historical backfill. Freshness is the recent
//! list's job, which every Home load already runs.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Days, Duration, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::plan_card::{try_load_plan_weeks_between, WeekCard};
use pierre_tool_runtime::activity_fetch::activity_cache_retention_days;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::{active_tenant, copies_per_workout, home_activities, served_rows, HomeActivity};
use crate::mcp::resources::ServerContext;

/// Most days one read spans, both ends included: the six weeks a month grid
/// that starts on a Monday can need.
pub const MAX_CALENDAR_DAYS: u64 = 42;

/// Cached rows the read makes room for per day and per connection: far more
/// workouts than anyone records in a day, so the bound is never what decides
/// which of an athlete's days are shown.
const ROWS_PER_DAY_PER_CONNECTION: i64 = 12;

/// Query parameters for `GET /api/me/calendar`.
#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    /// The first day, `YYYY-MM-DD`, on the athlete's calendar.
    pub from: NaiveDate,
    /// The last day, `YYYY-MM-DD`, included; at most
    /// [`MAX_CALENDAR_DAYS`] days after `from`, counting both.
    pub to: NaiveDate,
}

/// One cached workout, on the athlete's day it belongs to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CalendarActivity {
    /// The athlete's civil date of the workout's start, `YYYY-MM-DD`.
    pub date: NaiveDate,
    /// The workout exactly as its Home row projects it.
    pub activity: HomeActivity,
}

/// Body of `GET /api/me/calendar`.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarResponse {
    /// The athlete's civil today.
    pub today: NaiveDate,
    /// The first day read, as asked.
    pub from: NaiveDate,
    /// The last day read, as asked.
    pub to: NaiveDate,
    /// The first day whose activities the cache still holds in full; no
    /// activity is answered before it, and the days before it are unknown.
    pub history_start: NaiveDate,
    /// The workouts on the days read, oldest first.
    pub activities: Vec<CalendarActivity>,
    /// The active plan's weeks overlapping the days read, in calendar order;
    /// `null` when the athlete has no active plan.
    pub plan_weeks: Option<Vec<WeekCard>>,
}

pub(super) async fn get_calendar(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<CalendarQuery>,
) -> AppResult<Json<CalendarResponse>> {
    let CalendarQuery { from, to } = query;
    check_span(from, to)?;
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let repos = resources.repos();
    let user = repos.users.get_global(user_id).await?;
    let zone = resolve_zone(user.as_ref().and_then(|u| u.timezone.as_deref()));
    let now = Utc::now();
    let today = clock_date(now, zone);
    let history_start = history_start(now, zone, activity_cache_retention_days());

    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await?;
    let (read_from, read_to) = read_window(from, to);
    let limit = row_limit(from, to, connections.len());
    let rows = repos
        .activity_cache
        .get_cached_activity_rows(user_id, &tenant_id, read_from, read_to, limit)
        .await?;
    if usize::try_from(limit).is_ok_and(|limit| rows.len() >= limit) {
        warn!(
            %user_id,
            %from,
            %to,
            limit,
            "calendar read filled its row bound; the oldest days of the span may be short"
        );
    }
    let workouts = home_activities(served_rows(&resources, rows), usize::MAX);
    let activities = on_days(workouts, zone, from.max(history_start), to);

    let plan_weeks =
        try_load_plan_weeks_between(repos, tenant_id, user_id, today, from, to).await?;
    Ok(Json(CalendarResponse {
        today,
        from,
        to,
        history_start,
        activities,
        plan_weeks,
    }))
}

/// Refuse a span that ends before it starts or is longer than
/// [`MAX_CALENDAR_DAYS`].
fn check_span(from: NaiveDate, to: NaiveDate) -> AppResult<()> {
    if to < from {
        return Err(AppError::invalid_input("`to` is before `from`"));
    }
    let last_allowed = from
        .checked_add_days(Days::new(MAX_CALENDAR_DAYS - 1))
        .unwrap_or(NaiveDate::MAX);
    if to > last_allowed {
        return Err(AppError::invalid_input(format!(
            "a calendar read spans at most {MAX_CALENDAR_DAYS} days"
        )));
    }
    Ok(())
}

/// The first civil day the cache holds in full: the day after the one the
/// retention cutoff falls in, since that day is pruned up to the cutoff.
fn history_start(now: DateTime<Utc>, zone: Tz, retention_days: i64) -> NaiveDate {
    let cutoff = clock_date(now - Duration::days(retention_days), zone);
    cutoff.succ_opt().unwrap_or(cutoff)
}

/// The instants the cache is read between: a day wider than the span on both
/// sides in UTC, which holds every start that falls on one of its days in any
/// zone, a date-only row at midnight UTC included. [`on_days`] keeps the ones
/// that do.
fn read_window(from: NaiveDate, to: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = from.pred_opt().unwrap_or(from).and_time(NaiveTime::MIN);
    let end = to
        .checked_add_days(Days::new(2))
        .unwrap_or(to)
        .and_time(NaiveTime::MIN);
    (start.and_utc(), end.and_utc())
}

/// How many cached rows the read takes: room for
/// [`ROWS_PER_DAY_PER_CONNECTION`] on each day read (the window's extra day on
/// either side included) for each connection's copy.
fn row_limit(from: NaiveDate, to: NaiveDate, connections: usize) -> i64 {
    let days = (to - from).num_days() + 3;
    days * ROWS_PER_DAY_PER_CONNECTION * copies_per_workout(connections)
}

/// The workouts that started on a day in `from..=to` on the athlete's
/// calendar, each with that day, oldest first.
fn on_days(
    workouts: Vec<HomeActivity>,
    zone: Tz,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<CalendarActivity> {
    let mut days: Vec<CalendarActivity> = workouts
        .into_iter()
        .filter_map(|activity| {
            let date = local_date(activity.start_date, zone);
            (from..=to)
                .contains(&date)
                .then_some(CalendarActivity { date, activity })
        })
        .collect();
    days.sort_by_key(|day| day.activity.start_date);
    days
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap_or(NaiveDate::MIN)
    }

    fn workout(id: &str, start: DateTime<Utc>) -> HomeActivity {
        HomeActivity {
            id: id.to_owned(),
            provider: "strava".to_owned(),
            name: id.to_owned(),
            sport_type: "run".to_owned(),
            start_date: start,
            duration_seconds: 1_800,
            distance_meters: None,
            elevation_gain_meters: None,
            has_gps: true,
            summary_polyline: None,
            attribution: None,
        }
    }

    #[test]
    fn a_span_of_six_weeks_is_read_and_a_longer_or_reversed_one_refused() {
        assert!(check_span(date(2026, 9, 28), date(2026, 11, 8)).is_ok());
        assert!(check_span(date(2026, 9, 28), date(2026, 11, 9)).is_err());
        assert!(check_span(date(2026, 9, 28), date(2026, 9, 27)).is_err());
        assert!(check_span(date(2026, 9, 28), date(2026, 9, 28)).is_ok());
    }

    #[test]
    fn an_evening_run_is_the_day_it_was_run_and_a_date_only_row_keeps_its_day() {
        let toronto: Tz = "America/Toronto".parse().unwrap_or(Tz::UTC);
        let evening = Utc.with_ymd_and_hms(2026, 9, 25, 1, 0, 0).single();
        let date_only = Utc.with_ymd_and_hms(2026, 9, 25, 0, 0, 0).single();
        let (Some(evening), Some(date_only)) = (evening, date_only) else {
            panic!("fixture instants are on the calendar");
        };
        let days = on_days(
            vec![workout("date-only", date_only), workout("evening", evening)],
            toronto,
            date(2026, 9, 24),
            date(2026, 9, 25),
        );
        let placed: Vec<(&str, NaiveDate)> = days
            .iter()
            .map(|day| (day.activity.id.as_str(), day.date))
            .collect();
        assert_eq!(
            placed,
            vec![
                ("date-only", date(2026, 9, 25)),
                ("evening", date(2026, 9, 24))
            ]
        );
    }

    #[test]
    fn a_workout_outside_the_days_is_left_out() {
        let start = Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).single();
        let Some(start) = start else {
            panic!("fixture instant is on the calendar");
        };
        let days = on_days(
            vec![workout("later", start)],
            Tz::UTC,
            date(2026, 9, 21),
            date(2026, 9, 27),
        );
        assert!(days.is_empty());
    }

    #[test]
    fn history_starts_the_day_after_the_retention_cutoff_on_the_athletes_calendar() {
        let now = Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).single();
        let Some(now) = now else {
            panic!("fixture instant is on the calendar");
        };
        // 03:00 UTC is still the 6th in Toronto; 90 days before is 8 July.
        let toronto: Tz = "America/Toronto".parse().unwrap_or(Tz::UTC);
        assert_eq!(history_start(now, toronto, 90), date(2026, 7, 9));
        assert_eq!(history_start(now, Tz::UTC, 90), date(2026, 7, 10));
    }

    #[test]
    fn the_read_window_is_a_day_wider_than_the_span_on_each_side() {
        let (start, end) = read_window(date(2026, 9, 21), date(2026, 9, 27));
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 9, 20, 0, 0, 0).unwrap());
        assert_eq!(end, Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap());
    }
}
