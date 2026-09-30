// ABOUTME: Athlete Home read-side endpoints — recent activities, one activity's route, and the training plan for today
// ABOUTME: Auth: JWT-bearer, scoped by tenant_id from the active session; every read is tenant- and user-filtered

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home.
//!
//! Three reads back the page an athlete lands on after login, on web and
//! mobile alike:
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
//!   minutes later, or at once for `?retry=true`. See
//!   [`crate::services::activity_route`].
//! - `GET /api/me/training-plan?locale=xx` — what `/plan` shows, as the
//!   structured plan card: the athlete's one active season, whichever agent
//!   laid it, projected on the athlete's own "today".
//!
//! Every JSON key is always present; an absent value is `null`.

use std::cmp::Reverse;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::get;
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
use crate::services::activity_route::{activity_route, CachedActivityRef};
use crate::tools::runtime_adapter::into_runtime;

/// Activities the recent list answers with when the client names no limit.
const DEFAULT_RECENT_LIMIT: i64 = 5;

/// Fewest activities the recent list answers with.
const MIN_RECENT_LIMIT: i64 = 1;

/// Most activities the recent list answers with: a landing page, not a log.
const MAX_RECENT_LIMIT: i64 = 20;

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
    /// session, `row` the cached row of the copy that represents it.
    ///
    /// The id, the provider and `has_gps` are the representative row's own,
    /// so the route endpoint serves exactly that row; the numbers and the
    /// overview are the merged session's, which carry what the other copies
    /// filled in.
    fn from_session(row: &CachedActivityRow, activity: &Activity) -> Self {
        let overview = raw_overview(activity);
        Self {
            id: activity.id().to_owned(),
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
    /// Why there is no route: `no_gps` or `too_short`, which a read settled,
    /// or `unavailable`, which no read has yet: ask again later, or retry.
    pub reason: Option<&'static str>,
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
            "/api/me/activities/{provider}/{activity_id}/route",
            get(get_activity_route),
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
    let plan = stale_refresh_plan(&resources, user_id, tenant_id, connections, query.retry).await?;
    let sync_failure = plan.failure.clone();
    let stale = start_stale_refresh(&resources, user_id, tenant_id, plan).await;
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
/// The copy that represents a merged workout is the merger's canonical row,
/// the one chat lists: the copy carrying a distance, then the longest, then
/// the farthest, then the lowest id. It is always one of the athlete's own
/// cached rows, so the route endpoint serves its provider and id; its fields
/// the canonical copy lacks are filled from the other full recordings.
fn home_activities(rows: Vec<CachedActivityRow>, limit: usize) -> Vec<HomeActivity> {
    let mut distinct: Vec<CachedActivityRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let facing = user_facing_name(&row.provider);
        if !distinct.iter().any(|seen| {
            user_facing_name(&seen.provider) == facing && seen.activity.id() == row.activity.id()
        }) {
            distinct.push(row);
        }
    }
    // The merger consumes its input and rewrites the canonical copy, while
    // each row's stored key and route read are still needed to project it.
    let recordings: Vec<Activity> = distinct.iter().map(|row| row.activity.clone()).collect();
    let (mut sessions, _) = merge_duplicates(recordings, &DedupConfig::from_env());
    // A merged workout sits where its canonical copy was read, which need not
    // be where its newest copy was; the list is newest first by the session.
    sessions.sort_by_key(|session| Reverse(session.start_date()));
    sessions
        .iter()
        .filter_map(|session| {
            distinct
                .iter()
                .find(|row| is_copy_of(&row.activity, session))
                .map(|row| HomeActivity::from_session(row, session))
        })
        .take(limit)
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

/// Whether a last successful fetch is past the bands the chat path refreshes
/// at: the same test `refresh_stale_head` applies per provider.
fn is_stale(as_of: Option<DateTime<Utc>>) -> bool {
    !matches!(
        DataFreshness::from_last_sync(as_of),
        DataFreshness::Fresh | DataFreshness::Recent
    )
}

/// The providers a Home load would refresh, each judged by its own last
/// successful fetch.
#[derive(Debug, Default)]
struct StaleRefreshPlan {
    /// Active connections whose own head needs a refresh.
    active: Vec<String>,
    /// Scrape sessions flagged `needs_reauth` whose own head needs a refresh:
    /// each is refreshed only when its throttled retry can be claimed.
    flagged: Vec<String>,
    /// The newest failed refresh among the judged providers that no good
    /// sync of the same provider has superseded.
    failure: Option<SyncFailure>,
}

/// Judge each of the athlete's connections by its own last successful fetch,
/// and find the newest refresh of theirs that failed since.
///
/// Per provider, not across them: the newest fetch of any provider says
/// nothing about another's, and judging by it let a fresh connection hide a
/// stale one for as long as the fresh one kept syncing. An active connection
/// is refreshed when its head is stale, or when its last refresh failed —
/// whoever made that refresh, a Home load, a chat turn or the webhook, the
/// head it did not bring in is missing — unless that failure's pause is not
/// over ([`sync_backoff_until`]) and this is not the athlete's own `retry`.
/// A connection flagged `needs_reauth` is refreshed on the same terms only
/// when it is a scrape session ([`retries_flagged_session`]), since one
/// failed read can be the scraper's and not the session's; a flagged OAuth
/// grant is dead until the athlete reconnects, and a revoked connection is
/// theirs to restore.
async fn stale_refresh_plan(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    connections: Vec<ProviderConnection>,
    retry: bool,
) -> AppResult<StaleRefreshPlan> {
    let cache = &resources.repos().activity_cache;
    let now = Utc::now();
    let mut plan = StaleRefreshPlan::default();
    for connection in connections {
        let bucket = match connection.status {
            ConnectionStatus::Active => &mut plan.active,
            ConnectionStatus::NeedsReauth if retries_flagged_session(&connection.provider) => {
                &mut plan.flagged
            }
            ConnectionStatus::NeedsReauth | ConnectionStatus::Revoked => continue,
        };
        let last_sync = cache
            .latest_activity_sync(user_id, &tenant_id, &connection.provider)
            .await?;
        let recorded = cache
            .latest_activity_fetch_failure(user_id, &tenant_id, &connection.provider)
            .await?;
        let failed = recorded
            .filter(|failure| last_sync.is_none_or(|synced| failure.failed_at > synced))
            .map(|failure| failure.failed_at);
        if let Some(failed_at) = failed {
            if plan
                .failure
                .as_ref()
                .is_none_or(|newest| failed_at > newest.failed_at)
            {
                let provider = user_facing_name(&connection.provider);
                plan.failure = Some(SyncFailure {
                    provider: provider.to_owned(),
                    provider_name: resources
                        .fitness
                        .provider_registry
                        .get_descriptor(provider)
                        .map_or_else(|| provider.to_owned(), |d| d.display_name().to_owned()),
                    failed_at,
                    last_synced_at: last_sync,
                });
            }
        }
        let due = is_stale(last_sync) || failed.is_some();
        let paused = !retry && sync_backoff_until(recorded, last_sync, now).is_some();
        if due && !paused {
            bucket.push(connection.provider);
        }
    }
    Ok(plan)
}

/// Start the background refresh `plan` calls for, and report whether the page
/// is stale.
///
/// Stale means a provider this load refreshes was past the freshness bands:
/// an active connection with a stale head, or a flagged scrape session whose
/// retry this load claimed. The client then asks once more a little later.
///
/// One refresh per `(user, tenant)` at a time, shared with every other
/// stale-cache revalidation: a Home page reloaded ten times while a two-minute
/// scrape runs starts it once, and every one of those loads still says stale.
/// A flagged session's retry is claimed only by the load that starts the
/// refresh, so the claim is never spent on a refresh that does not run; it is
/// throttled across every caller and replica
/// ([`claim_scrape_session_retry`]). Tracked on the server's drain tracker so
/// shutdown waits for it, and capped at the shared revalidation timeout so a
/// hung scrape frees the slot.
async fn start_stale_refresh(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    plan: StaleRefreshPlan,
) -> bool {
    let StaleRefreshPlan {
        active: mut providers,
        flagged,
        ..
    } = plan;
    if providers.is_empty() && flagged.is_empty() {
        return false;
    }
    let Some(slot) = RevalidationRegistry::global().try_claim((user_id, tenant_id)) else {
        debug!(%user_id, "home: activity refresh already in flight; not starting another");
        return !providers.is_empty();
    };
    let runtime = into_runtime(resources);
    for provider in flagged {
        if claim_scrape_session_retry(&runtime, user_id, tenant_id, &provider).await {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        return false;
    }
    resources.common.turns.spawn(async move {
        let started_at = Utc::now();
        let bound = revalidation_timeout();
        let mut checked = Vec::with_capacity(providers.len());
        let refresh = refresh_providers(&runtime, user_id, tenant_id, &providers, &mut checked);
        if timeout(bound, refresh).await.is_err() {
            warn!(
                %user_id,
                timeout_secs = bound.as_secs(),
                "home: activity refresh timed out; releasing its slot"
            );
            let unanswered = TimedOut {
                providers: &providers,
                checked: &checked,
                started_at,
            };
            record_timed_out(&runtime, user_id, tenant_id, unanswered).await;
        }
        drop(slot);
    });
    true
}

/// Re-read each provider's recent head, one after the other, naming each in
/// `checked` once its read has finished — succeeded or failed, it has
/// recorded its own outcome. The plan has already judged each one due, so
/// the head is read whatever its freshness ([`refresh_head`]).
async fn refresh_providers(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    providers: &[String],
    checked: &mut Vec<String>,
) {
    for provider in providers {
        // The refresh writes through to the cache; the rows it returns are
        // what the next request reads, so nothing is kept here.
        let refreshed = refresh_head(runtime, provider, user_id, tenant_id).await;
        debug!(
            %user_id,
            provider = %provider,
            fetched = refreshed.map_or(0, |rows| rows.len()),
            "home: provider head checked"
        );
        checked.push(provider.clone());
    }
}

/// The providers a bounded refresh was asked to read, those it finished, and
/// when it began.
struct TimedOut<'a> {
    providers: &'a [String],
    checked: &'a [String],
    started_at: DateTime<Utc>,
}

/// Record a failed sync for every provider the refresh did not finish before
/// its timeout: the read in flight was dropped, so nothing else records that
/// it never answered, and the ones after it were never asked — the sync this
/// load started did not happen for them either. A provider whose sync has
/// landed since the refresh began is left alone: the read that was cut off
/// had already written its rows and its mark, and a failure recorded after
/// them would report a sync that worked as one that did not. Best-effort,
/// like every failure record.
async fn record_timed_out(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    unanswered: TimedOut<'_>,
) {
    let cache = &runtime.repos().activity_cache;
    for provider in unanswered
        .providers
        .iter()
        .filter(|p| !unanswered.checked.contains(p))
    {
        let synced = cache
            .latest_activity_sync(user_id, &tenant_id, provider)
            .await
            .unwrap_or(None);
        if synced.is_some_and(|synced| synced >= unanswered.started_at) {
            continue;
        }
        record_sync_failure(
            runtime,
            user_id,
            tenant_id,
            provider,
            ActivityFetchFailure::FetchError,
        )
        .await;
    }
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
        query.retry,
    )
    .await?;
    Ok(Json(match outcome {
        Ok(track) => ActivityRouteResponse {
            route: Some(route_view(
                track,
                activity.name(),
                user_facing_name(&provider),
            )),
            reason: None,
        },
        Err(reason) => ActivityRouteResponse {
            route: None,
            reason: Some(reason.as_str()),
        },
    }))
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

/// The track in the shape both clients' map draws. Home marks no climbs.
fn route_view(track: RouteTrack, name: &str, provider: &str) -> RouteView {
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
        title: (!name.trim().is_empty()).then(|| name.to_owned()),
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
