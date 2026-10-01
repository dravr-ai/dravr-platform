// ABOUTME: Athlete Home read-side endpoints — recent activities, one activity's figures and route, and today's plan
// ABOUTME: Auth: JWT-bearer, scoped by tenant_id from the active session; every read is tenant- and user-filtered

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home.
//!
//! Four reads back the page an athlete lands on after login, and the view of
//! one activity a tap on it opens, on web and mobile alike:
//!
//! - `GET /api/me/activities/recent?limit=N` — the newest cached activities
//!   across every provider, served from the durable cache in one query, one
//!   row per workout: a ride a watch synced to Strava, Garmin and COROS is
//!   one row, merged by the same session merger the chat turn lists
//!   activities through. A login never waits on a provider: when a provider's
//!   own cache is older than the freshness bands allow, a refresh is started
//!   in the background through the same stale-head path the chat turn uses,
//!   and the answer says so (`stale`), so the client asks once more a little
//!   later. A scrape session flagged `needs_reauth` is refreshed too, on the
//!   throttle `pierre_tool_runtime::reauth_retry` holds: a read it serves
//!   re-arms it. A refresh that failed — the provider errored or timed out,
//!   or answered with a list it did not vouch for — moves no freshness, and
//!   the answer names the provider, when it failed and when that provider
//!   last synced well (`sync_failure`). A provider whose last refresh failed
//!   is refreshed again whatever its freshness, once the failure's pause is
//!   over (`pierre_tool_runtime::activity_fetch::sync_verdict::sync_backoff_until`), so a
//!   failing scraper is not scraped on every load; `?retry=true` — the
//!   athlete's own retry — refreshes it at once.
//!   Each row's `has_gps` is `false` only when the activity's stored route
//!   read found no GPS; a row whose route has not been read says `true`,
//!   because a list row cannot settle it: only some providers' list
//!   payloads carry a start position or a route overview, and a GPS-recorded
//!   ride cached without them is drawn from its streams by the route read
//!   below.
//! - `GET /api/me/activities/{provider}/{activity_id}/route` — one cached
//!   activity's privacy-trimmed route, in the shape both clients' map already
//!   draws. Read once per activity, and its outcome — drawn or not — is what
//!   the list's `has_gps` reads. A read that failed, timed out or carried no
//!   stream set answers `unavailable`, never `no_gps`, and is read again
//!   minutes later, or at once for `?retry=true`. A read still queued behind
//!   the athlete's other reads, or still running, when the request's bound
//!   runs out answers `pending`, and the client asks again; the newest
//!   activity is read first. See [`crate::services::activity_route`].
//! - `GET /api/me/activities/{provider}/{activity_id}` — one workout as its
//!   own view shows it: the Home row's projection, merged exactly as the list
//!   merges it, with the figures the cache holds for it (heart rate, speed,
//!   power, energy) and its splits and laps when a detailed read
//!   stored them. Served from the cache, and a figure the cache does not
//!   hold is `null`, never estimated. The one provider call it makes is the
//!   workout's detail read, when no copy of it holds one that still answers
//!   it (see [`crate::services::activity_detail`]): a route drawn from its
//!   overview never reads the detail its splits and laps come from. It
//!   names the conversation the view opened about the workout, if any
//!   (`conversation_id`), so the view resumes that thread on any device.
//! - `PUT /api/me/activities/{provider}/{activity_id}/conversation` — link
//!   the conversation the view opened to the workout (`{"conversation_id":
//!   "…"}`), or forget the link (`null`). Only the caller's own conversation,
//!   in the caller's tenant, can be linked; any other answers 404.
//! - `GET /api/me/training-plan?locale=xx` — what `/plan` shows, as the
//!   structured plan card: the athlete's one active season, whichever agent
//!   laid it, projected on the athlete's own "today".
//!
//! Every JSON key is always present; an absent value is `null`.

pub mod activity_view;
/// The background refresh of stale provider heads Home starts.
mod stale_refresh;

use std::cmp::Reverse;
use std::iter;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use photograveur::{RouteBounds as ViewBounds, RouteView};
use pierre_core::civil_time::{clock_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ConnectionStatus, DataFreshness, ProviderConnection, TenantId,
};
use pierre_database::repositories::{
    sport_type_string, ActivityFetchFailure, CachedActivityRow, StoredRouteOutcome,
};
use pierre_fitness_compute::route_track::{trimmed_overview_polyline, RouteTrack, RouteTrackError};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_providers::backend_resolver::{backend_pair_for, user_facing_name};
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_services::locale::user_locale;
use pierre_services::personas::resolve_persona_locale;
use pierre_services::plan_card::{try_load_plan_card, PlanCard};
use pierre_tool_runtime::activity_fetch::sync_verdict::{record_sync_failure, sync_backoff_until};
use pierre_tool_runtime::activity_fetch::{activity_cache_retention_days, refresh_head};
use pierre_tool_runtime::reauth_retry::{claim_scrape_session_retry, retries_flagged_session};
use pierre_tool_runtime::revalidation::{revalidation_timeout, RevalidationRegistry};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::activity_detail::read_activity_detail;
use crate::services::activity_route::{activity_route, CachedActivityRef, RouteAsk, RouteMiss};
use crate::tools::runtime_adapter::into_runtime;
use activity_view::{ActivityLap, ActivitySplit};

/// Activities the recent list answers with when the client names no limit.
const DEFAULT_RECENT_LIMIT: i64 = 5;

/// Fewest activities the recent list answers with.
const MIN_RECENT_LIMIT: i64 = 1;

/// Most activities the recent list answers with: a landing page, not a log.
const MAX_RECENT_LIMIT: i64 = 20;

/// How far either side of an activity's start the detail read looks for the
/// other copies of its workout. The merger chains copies whose starts lie
/// within an hour or so of each other; a day either side holds every copy of
/// any workout a person records.
const DETAIL_MERGE_WINDOW_HOURS: i64 = 24;

/// Most cached rows the detail read takes from that window: far more than
/// the copies of every workout one athlete records in two days.
const DETAIL_MERGE_ROW_LIMIT: i64 = 200;

/// Most cached copies of one workout the recent read makes room for.
///
/// Each of the athlete's connections can hold its own copy of a workout a
/// watch synced everywhere, and copies merge after the read, so the read
/// takes `limit` rows per connection to still fill `limit` workouts. Capped
/// here so an athlete with many connections costs at most
/// `MAX_RECENT_LIMIT * MAX_COPIES_PER_WORKOUT` rows.
const MAX_COPIES_PER_WORKOUT: i64 = 5;

/// Query parameters for `GET /api/me/activities/recent`.
#[derive(Debug, Deserialize)]
pub struct RecentActivitiesQuery {
    /// How many activities to return, clamped to
    /// `[MIN_RECENT_LIMIT, MAX_RECENT_LIMIT]`; [`DEFAULT_RECENT_LIMIT`] when
    /// absent.
    #[serde(default)]
    pub limit: Option<i64>,
    /// The athlete's own retry after a failed sync: a provider whose last
    /// refresh failed is refreshed now, even inside the pause a failure
    /// otherwise earns it.
    #[serde(default)]
    pub retry: bool,
}

/// Query parameters for `GET /api/me/activities/{provider}/{activity_id}/route`.
#[derive(Debug, Deserialize)]
pub struct ActivityRouteQuery {
    /// The athlete's own retry after an `unavailable` answer: the provider is
    /// read again even when that answer is still stored.
    #[serde(default)]
    pub retry: bool,
    /// The request is one of a Home list's burst of route reads: finding the
    /// athlete's provider turn free, it waits for the rest of the burst so
    /// the newest activity is read first. Any other read — an activity
    /// view's map, a retry — takes a free turn at once.
    #[serde(default)]
    pub burst: bool,
}

/// One activity on the Home list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HomeActivity {
    /// The provider's id for the activity; unique only together with
    /// `provider`.
    pub id: String,
    /// The provider the athlete knows the activity from, by its user-facing
    /// slug — a mirror backend reads as the provider it mirrors.
    pub provider: String,
    /// The activity's name, as the athlete or their device wrote it.
    pub name: String,
    /// The sport, as the activity cache's own `sport_type` column spells it.
    pub sport_type: String,
    /// When the activity started.
    pub start_date: DateTime<Utc>,
    /// Elapsed time in seconds.
    pub duration_seconds: u64,
    /// Distance in metres, when recorded.
    pub distance_meters: Option<f64>,
    /// Elevation gained in metres, when recorded.
    pub elevation_gain_meters: Option<f64>,
    /// `false` when, and only when, the activity's stored route read found
    /// no GPS (`no_gps`): the one case a client has nothing to ask the route
    /// endpoint for. `true` in every other case — a drawn route is stored,
    /// the stored read found the track `too_short` (the route endpoint says
    /// so), or no read is stored, where the route endpoint reads the route
    /// once and stores the answer. A row that carries a start position or a
    /// route overview is one of those cases like any other; a row that
    /// carries neither is too, because only some providers' list payloads
    /// carry them. Neither the sport nor the distance decides it.
    pub has_gps: bool,
    /// The provider's route overview with its endpoint neighbourhoods
    /// removed, as a Google encoded polyline at precision 5; `null` when there
    /// is none, it does not decode, or too little of it survives the trim.
    pub summary_polyline: Option<String>,
}

impl HomeActivity {
    /// Project one workout for the Home list: `activity` is the merged
    /// session, `row` the cached row of the copy whose route the row draws
    /// ([`route_copy`]).
    ///
    /// The id, the provider and `has_gps` are that row's own, so the route
    /// endpoint serves exactly that copy; the name, the numbers and the
    /// overview are the merged session's, which carry what the other copies
    /// filled in.
    fn from_session(row: &CachedActivityRow, activity: &Activity) -> Self {
        let overview = raw_overview(activity);
        Self {
            id: row.activity.id().to_owned(),
            provider: user_facing_name(&row.provider).to_owned(),
            name: activity.name().to_owned(),
            sport_type: sport_type_string(activity).unwrap_or_default(),
            start_date: activity.start_date(),
            duration_seconds: activity.duration_seconds(),
            distance_meters: activity.distance_meters(),
            elevation_gain_meters: activity.elevation_gain(),
            has_gps: !read_found_no_gps(row.route.as_ref()),
            summary_polyline: overview.and_then(trimmed_overview_polyline),
        }
    }
}

/// What a cached copy's own data says about its route, least to most: the
/// order [`route_copy`] ranks the copies of one workout by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RouteEvidence {
    /// Its stored read found no GPS: a manual entry, a trainer ride.
    NoGps,
    /// Its stored read found a track too short to draw.
    TooShort,
    /// Nothing settled yet: never read, or a read that settled nothing.
    Unknown,
    /// Nothing settled yet, but a device recorded it: it carries a heart
    /// rate or a cadence, which a manual entry never does.
    Sensed,
    /// A track is stored, or the copy carries its own start position or
    /// route overview.
    Recorded,
}

/// What `row`'s own stored read and payload say about its route.
fn route_evidence(row: &CachedActivityRow) -> RouteEvidence {
    let settled = match row.route.as_ref() {
        Some(StoredRouteOutcome::Drawn) => return RouteEvidence::Recorded,
        Some(StoredRouteOutcome::Unavailable { reason }) => RouteTrackError::from_slug(reason),
        None => None,
    };
    let activity = &row.activity;
    match settled {
        Some(RouteTrackError::NoGps) => RouteEvidence::NoGps,
        Some(RouteTrackError::TooShort) => RouteEvidence::TooShort,
        None if raw_overview(activity).is_some()
            || (activity.start_latitude().is_some() && activity.start_longitude().is_some()) =>
        {
            RouteEvidence::Recorded
        }
        None if device_recorded(activity) => RouteEvidence::Sensed,
        None => RouteEvidence::Unknown,
    }
}

/// Whether a device recorded the activity: it carries a heart rate or a
/// cadence.
const fn device_recorded(activity: &Activity) -> bool {
    activity.average_heart_rate().is_some() || activity.average_cadence().is_some()
}

/// The copy of a merged workout whose route the Home row draws: the copy
/// with the strongest [`RouteEvidence`], the canonical copy among equals.
///
/// The merger picks its canonical copy for the numbers — a copy carrying a
/// distance, then the longest — and a manual entry typed in with the
/// workout's distance and a generous duration wins that over the watch's
/// recording of the same run. On 2026-09-28 that hid a GPS track behind a
/// manual copy whose read had found no GPS, so the row drew nothing.
fn route_copy<'a>(
    canonical: &'a CachedActivityRow,
    copies: impl Iterator<Item = &'a CachedActivityRow>,
) -> &'a CachedActivityRow {
    copies.fold(canonical, |best, copy| {
        if route_evidence(copy) > route_evidence(best) {
            copy
        } else {
            best
        }
    })
}

/// Whether a stored route read settled that the activity recorded no GPS.
///
/// No stored read (the list's join leaves an expired one out), a drawn track
/// and any other reason all answer `false`: the route endpoint still has
/// something to say about the activity.
fn read_found_no_gps(route: Option<&StoredRouteOutcome>) -> bool {
    matches!(
        route,
        Some(StoredRouteOutcome::Unavailable { reason })
            if RouteTrackError::from_slug(reason) == Some(RouteTrackError::NoGps)
    )
}

/// The route overview a cached activity carries, when it is not blank.
fn raw_overview(activity: &Activity) -> Option<&str> {
    activity
        .summary_polyline()
        .filter(|encoded| !encoded.trim().is_empty())
}

/// Body of `GET /api/me/activities/recent`.
#[derive(Debug, Clone, Serialize)]
pub struct RecentActivitiesResponse {
    /// Newest first, at most the requested limit.
    pub activities: Vec<HomeActivity>,
    /// When a provider fetch last succeeded for the athlete in this tenant,
    /// across every provider; `null` when none ever has. For display: it is
    /// not what staleness is judged by.
    pub as_of: Option<DateTime<Utc>>,
    /// `true` when this load started, or found running, a refresh of a
    /// provider that needs one: past the freshness bands (or never fetched),
    /// judged by that provider's own last fetch, or whose last refresh failed
    /// — an active connection, or a flagged scrape session whose throttled
    /// retry this load claimed. The client may ask once more. So `stale` can
    /// be `true` while `as_of` is recent: a fresh provider does not make a
    /// stale one current. `false` while a failing provider is paused between
    /// attempts: nothing is running, and `sync_failure` says why.
    pub stale: bool,
    /// The newest refresh of one of the providers this load judges that
    /// failed while no good sync of that provider has come since; `null` when
    /// none has. It names that provider and its own last good sync, which is
    /// how old its rows on the page are — `as_of` spans every provider.
    pub sync_failure: Option<SyncFailure>,
}

/// A provider's latest refresh that failed, with that provider's own last
/// good sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncFailure {
    /// The provider, by its user-facing slug.
    pub provider: String,
    /// The provider's name as the athlete reads it (`Strava`).
    pub provider_name: String,
    /// When the refresh failed.
    pub failed_at: DateTime<Utc>,
    /// When that provider last synced well; `null` when it never has.
    pub last_synced_at: Option<DateTime<Utc>>,
}

/// Body of `GET /api/me/activities/{provider}/{activity_id}/route`: exactly
/// one of `route` and `reason` is non-null.
///
/// Every answer is stored, and the recent list reads the stored one: after a
/// `no_gps` answer the activity's row says `has_gps: false`. An `unavailable`
/// answer expires within minutes and leaves `has_gps` true.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRouteResponse {
    /// The drawable route.
    pub route: Option<RouteView>,
    /// Why there is no route: `no_gps` or `too_short`, which a read settled;
    /// `unavailable`, a read that finished without settling it: ask again
    /// later, or retry; or `pending`, no read has finished yet: ask again.
    pub reason: Option<&'static str>,
    /// With `pending` only: seconds the read can still take by the server's
    /// own bounds — every read ahead of it in the athlete's provider turn and
    /// its own, each bounded. The client keeps asking within it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settles_within_secs: Option<u64>,
}

/// Body of `GET /api/me/activities/{provider}/{activity_id}`.
///
/// One workout as its own view shows it. Every key is present; what the
/// cache does not hold is `null`, and a workout without splits or laps has
/// an empty list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActivityDetailResponse {
    /// The workout exactly as its Home row projects it — merged the same way,
    /// its `provider` and `id` those of the copy whose route it draws, which
    /// is what the route endpoint serves.
    pub activity: HomeActivity,
    /// Average heart rate in beats per minute.
    pub average_heart_rate: Option<u32>,
    /// Highest heart rate in beats per minute.
    pub max_heart_rate: Option<u32>,
    /// Average speed in metres per second, as the provider computed it.
    pub average_speed_mps: Option<f64>,
    /// Highest speed in metres per second.
    pub max_speed_mps: Option<f64>,
    /// Average power in watts.
    pub average_power: Option<u32>,
    /// Energy in kilocalories.
    pub calories: Option<u32>,
    /// The provider's uniform distance buckets, in order.
    pub splits: Vec<ActivitySplit>,
    /// The laps the athlete or the workout marked, in order.
    pub laps: Vec<ActivityLap>,
    /// The conversation the view opened about this workout, while it is
    /// still the caller's own; `null` before the first question.
    pub conversation_id: Option<String>,
}

/// Body of `PUT /api/me/activities/{provider}/{activity_id}/conversation`,
/// and its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityConversationLink {
    /// The conversation to link, or `null` to forget the link.
    pub conversation_id: Option<String>,
}

/// Query parameters for `GET /api/me/training-plan`.
#[derive(Debug, Deserialize)]
pub struct TrainingPlanQuery {
    /// The locale the plan's labels render in; an unsupported or absent value
    /// falls back to the athlete's stored locale, then to the default.
    #[serde(default)]
    pub locale: Option<String>,
}

/// Body of `GET /api/me/training-plan`.
#[derive(Debug, Clone, Serialize)]
pub struct TrainingPlanResponse {
    /// The active plan as `/plan` reads it, projected on `today`; `null`
    /// when the athlete has no active plan.
    pub plan: Option<PlanCard>,
    /// The athlete's civil date the plan is projected on, `YYYY-MM-DD`.
    pub today: NaiveDate,
}

/// Build the Athlete Home router.
pub fn athlete_home_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/me/activities/recent", get(get_recent_activities))
        .route(
            "/api/me/activities/{provider}/{activity_id}",
            get(get_activity_detail),
        )
        .route(
            "/api/me/activities/{provider}/{activity_id}/route",
            get(get_activity_route),
        )
        .route(
            "/api/me/activities/{provider}/{activity_id}/conversation",
            put(put_activity_conversation),
        )
        .route("/api/me/training-plan", get(get_training_plan))
}

async fn get_recent_activities(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<RecentActivitiesQuery>,
) -> AppResult<Json<RecentActivitiesResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let limit = query.limit.map_or(DEFAULT_RECENT_LIMIT, |limit| {
        limit.clamp(MIN_RECENT_LIMIT, MAX_RECENT_LIMIT)
    });
    let repos = resources.repos();
    let now = Utc::now();
    let connections = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await?;
    // The limit is clamped to at least one, so it always converts.
    let shown = usize::try_from(limit).unwrap_or_default();
    let activities = recent_workouts(
        &resources,
        user_id,
        &tenant_id,
        now,
        shown,
        limit * copies_per_workout(connections.len()),
    )
    .await?;
    let as_of = repos
        .activity_cache
        .latest_activity_sync_any(user_id, &tenant_id)
        .await?;
    let plan =
        stale_refresh::stale_refresh_plan(&resources, user_id, tenant_id, connections, query.retry)
            .await?;
    let sync_failure = plan.failure.clone();
    let stale = stale_refresh::start_stale_refresh(&resources, user_id, tenant_id, plan).await;
    Ok(Json(RecentActivitiesResponse {
        activities,
        as_of,
        stale,
        sync_failure,
    }))
}

/// The newest `limit` workouts in the athlete's cache, one Home row each.
///
/// The first read takes `first_read` rows — `limit` per connection, room for
/// each connection's copy of every workout. A connection can also hold
/// several rows of one workout (a ride auto-split into two uploads), so when
/// the rows read merge into fewer than `limit` workouts and the read came
/// back full, the cache holds more: the read is repeated at twice the size
/// until `limit` workouts are found or the retention window is exhausted.
async fn recent_workouts(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
    now: DateTime<Utc>,
    limit: usize,
    first_read: i64,
) -> AppResult<Vec<HomeActivity>> {
    let since = now - Duration::days(activity_cache_retention_days());
    let mut read = first_read.max(1);
    loop {
        let rows = resources
            .repos()
            .activity_cache
            .get_cached_activity_rows(user_id, tenant_id, since, now, read)
            .await?;
        let exhausted = usize::try_from(read).is_ok_and(|asked| rows.len() < asked);
        let workouts = home_activities(rows, limit);
        if workouts.len() >= limit || exhausted {
            return Ok(workouts);
        }
        read = read.saturating_mul(2);
    }
}

/// How many cached copies of each workout the recent read makes room for:
/// one per connection the athlete holds in this tenant, whatever its status —
/// a connection that needs re-authorising still has its copies cached — and
/// at least one, capped at [`MAX_COPIES_PER_WORKOUT`].
fn copies_per_workout(connections: usize) -> i64 {
    i64::try_from(connections)
        .unwrap_or(MAX_COPIES_PER_WORKOUT)
        .clamp(1, MAX_COPIES_PER_WORKOUT)
}

/// The Home rows for cached rows, newest first, one per workout, at most
/// `limit` of them.
///
/// Two passes, both over the rows already read:
///
/// 1. Identity: a mirror backend and the provider it mirrors can both hold a
///    copy of the same activity; both read as the same user-facing provider
///    and id, so the first, the newest-stored, is kept.
/// 2. Workout: every recording of one workout across providers (a ride a
///    watch synced to Strava, Garmin and COROS) merges into one session
///    through [`merge_duplicates`] under [`DedupConfig::from_env`], exactly
///    as the chat turn's activity list merges them.
///
/// A merged workout's name and numbers are the merger's canonical row, the
/// one chat lists — the copy carrying a distance, then the longest, then the
/// farthest, then the lowest id — with the fields it lacks filled from the
/// other full recordings. Its id, provider and `has_gps` are those of the
/// copy whose route it draws ([`route_copy`]): a GPS recording over a manual
/// entry of the same workout. Either is one of the athlete's own cached
/// rows, so the route endpoint serves its provider and id.
fn home_activities(rows: Vec<CachedActivityRow>, limit: usize) -> Vec<HomeActivity> {
    let distinct = distinct_rows(rows);
    workouts(&distinct)
        .iter()
        .take(limit)
        .map(|workout| HomeActivity::from_session(workout.route_row(), &workout.session))
        .collect()
}

/// One workout the cache holds: the merged session, and the cached rows of
/// the copies it was merged from.
struct Workout<'a> {
    /// The merged session — the merger's canonical copy, with the fields it
    /// lacked filled from the other recordings.
    session: Activity,
    /// The cached row of the canonical copy.
    canonical: &'a CachedActivityRow,
    /// The cached rows of the other copies merged into it.
    copies: Vec<&'a CachedActivityRow>,
}

impl<'a> Workout<'a> {
    /// The copy whose route the workout draws ([`route_copy`]).
    fn route_row(&self) -> &'a CachedActivityRow {
        route_copy(self.canonical, self.copies.iter().copied())
    }

    /// Whether the workout's splits and laps are still to be read: the
    /// merged session carries neither, and no copy of it holds a stored
    /// detail read that still answers it — one that found neither answers
    /// only until its recheck instant
    /// ([`crate::services::activity_detail::EMPTY_DETAIL_RECHECK_MINUTES`]).
    fn detail_unread(&self) -> bool {
        self.session.splits().is_none()
            && self.session.laps().is_none()
            && !iter::once(self.canonical)
                .chain(self.copies.iter().copied())
                .any(|row| row.detail_read)
    }

    /// Whether one of the workout's copies is the activity `id` of the
    /// provider the athlete knows as `facing`.
    fn holds(&self, facing: &str, id: &str) -> bool {
        iter::once(self.canonical)
            .chain(self.copies.iter().copied())
            .any(|row| user_facing_name(&row.provider) == facing && row.activity.id() == id)
    }
}

/// The rows with one copy per activity: a mirror backend and the provider it
/// mirrors can both hold a copy of the same activity; both read as the same
/// user-facing provider and id, so the first, the newest-stored, is kept.
fn distinct_rows(rows: Vec<CachedActivityRow>) -> Vec<CachedActivityRow> {
    let mut distinct: Vec<CachedActivityRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let facing = user_facing_name(&row.provider);
        if !distinct.iter().any(|seen| {
            user_facing_name(&seen.provider) == facing && seen.activity.id() == row.activity.id()
        }) {
            distinct.push(row);
        }
    }
    distinct
}

/// The workouts `distinct` holds, newest first: every recording of one
/// workout across providers merged into one session through
/// [`merge_duplicates`] under [`DedupConfig::from_env`], exactly as the chat
/// turn's activity list merges them.
fn workouts(distinct: &[CachedActivityRow]) -> Vec<Workout<'_>> {
    // The merger consumes its input and rewrites the canonical copy, while
    // each row's stored key and route read are still needed to project it.
    let recordings: Vec<Activity> = distinct.iter().map(|row| row.activity.clone()).collect();
    let (mut sessions, report) = merge_duplicates(recordings, &DedupConfig::from_env());
    // A merged workout sits where its canonical copy was read, which need not
    // be where its newest copy was; the list is newest first by the session.
    sessions.sort_by_key(|session| Reverse(session.start_date()));
    sessions
        .into_iter()
        .filter_map(|session| {
            let canonical = distinct
                .iter()
                .find(|row| is_copy_of(&row.activity, &session))?;
            let copies = report
                .groups
                .iter()
                .find(|group| group.canonical_id == session.id())
                .into_iter()
                .flat_map(|group| {
                    distinct.iter().filter(move |row| {
                        group.fragment_ids.iter().any(|id| id == row.activity.id())
                            && group.window_start <= row.activity.start_date()
                            && row.activity.start_date() <= group.window_end
                    })
                })
                .collect();
            Some(Workout {
                session,
                canonical,
                copies,
            })
        })
        .collect()
}

/// Whether `recording` is the cached copy a merged `session` was built on:
/// the merger never changes a canonical copy's provider, id, start or
/// duration, only the fields it lacked.
fn is_copy_of(recording: &Activity, session: &Activity) -> bool {
    recording.provider() == session.provider()
        && recording.id() == session.id()
        && recording.start_date() == session.start_date()
        && recording.duration_seconds() == session.duration_seconds()
}

async fn get_activity_route(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
    Query(query): Query<ActivityRouteQuery>,
) -> AppResult<Json<ActivityRouteResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let (stored_provider, activity) =
        owned_cached_activity(&resources, user_id, tenant_id, &provider, &activity_id).await?;
    let runtime = into_runtime(&resources);
    let outcome = activity_route(
        &runtime,
        &resources.common.turns,
        tenant_id,
        user_id,
        CachedActivityRef {
            provider: &stored_provider,
            activity: &activity,
        },
        RouteAsk {
            retry: query.retry,
            burst: query.burst,
        },
    )
    .await?;
    Ok(Json(match outcome {
        Ok(track) => ActivityRouteResponse {
            route: Some(route_view(track, user_facing_name(&provider))),
            reason: None,
            settles_within_secs: None,
        },
        Err(reason) => ActivityRouteResponse {
            route: None,
            reason: Some(reason.as_str()),
            settles_within_secs: match reason {
                RouteMiss::Pending {
                    settles_within_secs,
                } => Some(settles_within_secs),
                RouteMiss::Settled(_) | RouteMiss::Unavailable => None,
            },
        },
    }))
}

/// One workout as its view shows it, reading its splits and laps from the
/// provider when no copy of it holds a detail read that still answers it
/// ([`read_activity_detail`]).
async fn get_activity_detail(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
) -> AppResult<Json<ActivityDetailResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    // The ownership check: the caller's own row, in this tenant, or 404.
    let (stored_provider, activity) =
        owned_cached_activity(&resources, user_id, tenant_id, &provider, &activity_id).await?;
    let asked = ActivityAsked {
        user_id,
        tenant_id,
        provider: &provider,
        activity_id: &activity_id,
        start: activity.start_date(),
    };
    let WorkoutView {
        mut view,
        unread,
        copies,
    } = activity_view(&resources, asked).await?;
    let conversation_id =
        workout_conversation(&resources, asked, &stored_provider, &copies).await?;
    view.conversation_id.clone_from(&conversation_id);
    let Some((unread_provider, unread_activity)) = unread else {
        return Ok(Json(view));
    };
    let runtime = into_runtime(&resources);
    let cached = CachedActivityRef {
        provider: &unread_provider,
        activity: &unread_activity,
    };
    if !read_activity_detail(
        &runtime,
        &resources.common.turns,
        tenant_id,
        user_id,
        cached,
    )
    .await
    {
        return Ok(Json(view));
    }
    let mut view = activity_view(&resources, asked).await?.view;
    view.conversation_id = conversation_id;
    Ok(Json(view))
}

/// The thread a view opened about the workout: the one linked to the asked
/// activity, else the one linked to any other copy of the same workout.
///
/// The Home row, and so the view a tap opens, is addressed by the copy whose
/// route it draws ([`route_copy`]), and that copy changes when a copy's route
/// read settles: a watch recording found to hold no GPS falls behind a manual
/// entry of the same run. A thread linked while the row named one copy is
/// found again while it names another.
///
/// # Errors
///
/// Returns the repository error when a link cannot be read.
async fn workout_conversation(
    resources: &Arc<ServerContext>,
    asked: ActivityAsked<'_>,
    stored_provider: &str,
    copies: &[(String, String)],
) -> AppResult<Option<String>> {
    let links = &resources.repos().activity_conversations;
    let asked_key = (stored_provider, asked.activity_id);
    let keys = iter::once(asked_key).chain(
        copies
            .iter()
            .map(|(provider, id)| (provider.as_str(), id.as_str()))
            .filter(|key| *key != asked_key),
    );
    for (provider, activity_id) in keys {
        if let Some(conversation_id) = links
            .get_activity_conversation(&asked.tenant_id, asked.user_id, provider, activity_id)
            .await?
        {
            return Ok(Some(conversation_id));
        }
    }
    Ok(None)
}

/// The activity a view asked for, and whose.
#[derive(Debug, Clone, Copy)]
struct ActivityAsked<'a> {
    user_id: Uuid,
    tenant_id: TenantId,
    /// The provider as the path names it.
    provider: &'a str,
    activity_id: &'a str,
    /// When the caller's cached copy of it started.
    start: DateTime<Utc>,
}

/// The view of the workout holding the asked activity, read from the cache.
struct WorkoutView {
    /// The view, its `conversation_id` not yet looked up.
    view: ActivityDetailResponse,
    /// The copy to read the workout's detail from, when no copy of it holds a
    /// detail read that still answers it and the merged session carries
    /// neither splits nor laps.
    unread: Option<(String, Activity)>,
    /// Every copy of the workout, as `(stored provider, activity id)`.
    copies: Vec<(String, String)>,
}

/// The view of the workout holding the asked activity, from the cache alone.
///
/// # Errors
///
/// Returns the repository error when the rows cannot be read, and
/// [`AppError::not_found`] when no merged workout holds the activity.
async fn activity_view(
    resources: &Arc<ServerContext>,
    asked: ActivityAsked<'_>,
) -> AppResult<WorkoutView> {
    // The workout's other copies start within the merger's window of this
    // one, so the rows around it merge into the same session the Home list
    // shows for it.
    let window = Duration::hours(DETAIL_MERGE_WINDOW_HOURS);
    let rows = resources
        .repos()
        .activity_cache
        .get_cached_activity_rows(
            asked.user_id,
            &asked.tenant_id,
            asked.start - window,
            asked.start + window,
            DETAIL_MERGE_ROW_LIMIT,
        )
        .await?;
    let distinct = distinct_rows(rows);
    let facing = user_facing_name(asked.provider);
    let all = workouts(&distinct);
    let workout = all
        .iter()
        .find(|workout| workout.holds(facing, asked.activity_id))
        .ok_or_else(|| {
            AppError::not_found(format!(
                "activity {} from {}",
                asked.activity_id, asked.provider
            ))
        })?;
    let route_row = workout.route_row();
    let unread = workout
        .detail_unread()
        .then(|| (route_row.provider.clone(), route_row.activity.clone()));
    let copies = iter::once(workout.canonical)
        .chain(workout.copies.iter().copied())
        .map(|row| (row.provider.clone(), row.activity.id().to_owned()))
        .collect();
    Ok(WorkoutView {
        view: ActivityDetailResponse::from_session(route_row, &workout.session),
        unread,
        copies,
    })
}

/// Link the conversation an activity's view opened to the activity, or
/// forget the link.
///
/// The link is filed under the provider key the caller's cached row is stored
/// under — the key the detail read looks it up by, and the one the
/// provider-disconnect purge deletes it with.
///
/// # Errors
///
/// Returns [`AppError::not_found`] when the caller holds no such activity, or
/// when the conversation is not the caller's own in this tenant.
async fn put_activity_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
    Json(link): Json<ActivityConversationLink>,
) -> AppResult<Json<ActivityConversationLink>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let (stored_provider, _) =
        owned_cached_activity(&resources, user_id, tenant_id, &provider, &activity_id).await?;
    let links = &resources.repos().activity_conversations;
    match link.conversation_id.as_deref() {
        Some(conversation_id) => {
            let linked = links
                .link_activity_conversation(
                    &tenant_id,
                    user_id,
                    &stored_provider,
                    &activity_id,
                    conversation_id,
                )
                .await?;
            if !linked {
                return Err(AppError::not_found(format!(
                    "conversation {conversation_id}"
                )));
            }
        }
        None => {
            links
                .unlink_activity_conversation(&tenant_id, user_id, &stored_provider, &activity_id)
                .await?;
        }
    }
    Ok(Json(link))
}

/// The caller's own cached activity, with the provider key its row is stored
/// under — the ownership check. A provider and its mirror backend are one
/// provider to the athlete, so a row under either answers for it.
///
/// # Errors
///
/// Returns [`AppError::not_found`] when no row of the caller's, in this
/// tenant, holds the activity.
async fn owned_cached_activity(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activity_id: &str,
) -> AppResult<(String, Activity)> {
    let cache = &resources.repos().activity_cache;
    for backend in backend_pair_for(provider) {
        if let Some(activity) = cache
            .get_cached_activity(user_id, &tenant_id, &backend, activity_id)
            .await?
        {
            return Ok((backend, activity));
        }
    }
    Err(AppError::not_found(format!(
        "activity {activity_id} from {provider}"
    )))
}

/// The track in the shape both clients' map draws. Home marks no climbs, and
/// the map carries no title: the Home row and the activity view both name the
/// activity above it already.
fn route_view(track: RouteTrack, provider: &str) -> RouteView {
    RouteView {
        coordinates: track.coordinates,
        bounds: ViewBounds {
            min_latitude: track.bounds.min_latitude,
            max_latitude: track.bounds.max_latitude,
            min_longitude: track.bounds.min_longitude,
            max_longitude: track.bounds.max_longitude,
        },
        elevation_meters: track.elevation_meters,
        distances_meters: track.distances_meters,
        climbs: Vec::new(),
        title: None,
        source_tool: provider.to_owned(),
    }
}

async fn get_training_plan(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<TrainingPlanQuery>,
) -> AppResult<Json<TrainingPlanResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let repos = resources.repos();
    let user = repos.users.get_global(user_id).await?;
    // The athlete's civil day, the one `/plan` projects the plan on: a 23:30
    // EDT visit shows today's session, not tomorrow's.
    let today = clock_date(
        Utc::now(),
        resolve_zone(user.as_ref().and_then(|u| u.timezone.as_deref())),
    );
    let locale = resolve_persona_locale(query.locale.as_deref(), Some(&user_locale(user.as_ref())));
    // The athlete's one season, whichever agent laid it — the plan `/plan`
    // and every conversation read.
    let plan = try_load_plan_card(
        repos,
        tenant_id,
        user_id,
        today,
        &resources.mcp.messaging_strings_registry,
        &locale,
    )
    .await?;
    Ok(Json(TrainingPlanResponse { plan, today }))
}

fn active_tenant(auth: &AuthenticatedUser) -> AppResult<TenantId> {
    auth.active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| {
            AppError::auth_invalid(
                "Home endpoints require an active tenant in the JWT — switch tenant first",
            )
        })
}
