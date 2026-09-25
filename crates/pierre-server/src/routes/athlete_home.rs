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
//!   across every provider, served from the durable cache in one query. A
//!   login never waits on a provider: when the cache is older than the
//!   freshness bands allow, a refresh is started in the background through
//!   the same stale-head path the chat turn uses, and the answer says so
//!   (`stale`), so the client asks once more a little later.
//! - `GET /api/me/activities/{provider}/{activity_id}/route` — one cached
//!   activity's privacy-trimmed route, in the shape both clients' map already
//!   draws. Read at most once per activity; see [`crate::services::activity_route`].
//! - `GET /api/me/training-plan?locale=xx` — what `/plan` shows, as the
//!   structured plan card: the active plan under the agent `/plan` reads it
//!   under, projected on the athlete's own "today".
//!
//! Every JSON key is always present; an absent value is `null`.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use photograveur::{RouteBounds as ViewBounds, RouteView};
use pierre_core::civil_time::{clock_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, ConnectionStatus, DataFreshness, TenantId};
use pierre_database::repositories::{sport_type_string, CachedActivityRow};
use pierre_fitness_compute::route_track::{trimmed_overview_polyline, RouteTrack};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_providers::backend_resolver::{backend_pair_for, user_facing_name};
use pierre_services::locale::user_locale;
use pierre_services::personas::resolve_persona_locale;
use pierre_services::plan_card::{try_load_plan_card, PlanCard};
use pierre_services::training_plan_render::resolve_plan_agent_slug;
use pierre_tool_runtime::activity_fetch::{activity_cache_retention_days, refresh_stale_head};
use pierre_tool_runtime::revalidation::{RevalidationRegistry, REVALIDATION_TIMEOUT_SECS};
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

/// Query parameters for `GET /api/me/activities/recent`.
#[derive(Debug, Deserialize)]
pub struct RecentActivitiesQuery {
    /// How many activities to return, clamped to
    /// `[MIN_RECENT_LIMIT, MAX_RECENT_LIMIT]`; [`DEFAULT_RECENT_LIMIT`] when
    /// absent.
    #[serde(default)]
    pub limit: Option<i64>,
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
    /// Whether the activity has a route to draw: a start position, or a route
    /// overview.
    pub has_gps: bool,
    /// The provider's route overview with its endpoint neighbourhoods
    /// removed, as a Google encoded polyline at precision 5; `null` when there
    /// is none, it does not decode, or too little of it survives the trim.
    pub summary_polyline: Option<String>,
}

impl HomeActivity {
    /// Project one cached row for the Home list.
    fn from_row(row: &CachedActivityRow) -> Self {
        let activity = &row.activity;
        let overview = raw_overview(activity);
        let has_start = activity.start_latitude().is_some() && activity.start_longitude().is_some();
        Self {
            id: activity.id().to_owned(),
            provider: user_facing_name(&row.provider).to_owned(),
            name: activity.name().to_owned(),
            sport_type: sport_type_string(activity).unwrap_or_default(),
            start_date: activity.start_date(),
            duration_seconds: activity.duration_seconds(),
            distance_meters: activity.distance_meters(),
            elevation_gain_meters: activity.elevation_gain(),
            has_gps: has_start || overview.is_some(),
            summary_polyline: overview.and_then(trimmed_overview_polyline),
        }
    }
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
    /// When a provider fetch last succeeded for the athlete in this tenant;
    /// `null` when none ever has.
    pub as_of: Option<DateTime<Utc>>,
    /// `true` when `as_of` is past the freshness bands (or absent) while a
    /// provider is connected: a background refresh has been started, and the
    /// client may ask once more.
    pub stale: bool,
}

/// Body of `GET /api/me/activities/{provider}/{activity_id}/route`: exactly
/// one of `route` and `reason` is non-null.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityRouteResponse {
    /// The drawable route.
    pub route: Option<RouteView>,
    /// Why there is no route: `no_gps` or `too_short`.
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
    let rows = repos
        .activity_cache
        .get_cached_activity_rows(
            user_id,
            &tenant_id,
            now - Duration::days(activity_cache_retention_days()),
            now,
            limit,
        )
        .await?;
    let as_of = repos
        .activity_cache
        .latest_activity_sync_any(user_id, &tenant_id)
        .await?;
    let connected: Vec<String> = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await?
        .into_iter()
        .filter(|connection| connection.status == ConnectionStatus::Active)
        .map(|connection| connection.provider)
        .collect();
    let stale = !connected.is_empty() && is_stale(as_of);
    if stale {
        spawn_stale_refresh(&resources, user_id, tenant_id, connected);
    }
    Ok(Json(RecentActivitiesResponse {
        activities: home_activities(&rows),
        as_of,
        stale,
    }))
}

/// The Home rows for cached rows, newest first, one per activity.
///
/// A mirror backend and the provider it mirrors can both hold a copy of the
/// same activity; both read as the same user-facing provider and id, so the
/// first — the newest-stored — is kept.
fn home_activities(rows: &[CachedActivityRow]) -> Vec<HomeActivity> {
    let mut activities: Vec<HomeActivity> = Vec::with_capacity(rows.len());
    for row in rows {
        let activity = HomeActivity::from_row(row);
        if !activities
            .iter()
            .any(|seen| seen.provider == activity.provider && seen.id == activity.id)
        {
            activities.push(activity);
        }
    }
    activities
}

/// Whether a last successful fetch is past the bands the chat path refreshes
/// at: the same test `refresh_stale_head` applies per provider.
fn is_stale(as_of: Option<DateTime<Utc>>) -> bool {
    !matches!(
        DataFreshness::from_last_sync(as_of),
        DataFreshness::Fresh | DataFreshness::Recent
    )
}

/// Refresh each connected provider's recent head in the background.
///
/// One refresh per `(user, tenant)` at a time, shared with every other
/// stale-cache revalidation: a Home page reloaded ten times while a two-minute
/// scrape runs starts it once. Each provider re-checks its own freshness
/// inside `refresh_stale_head`, so one that was fetched a moment ago costs
/// nothing. Tracked on the server's drain tracker so shutdown waits for it,
/// and capped at the shared revalidation timeout so a hung scrape frees the
/// slot.
fn spawn_stale_refresh(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    providers: Vec<String>,
) {
    let Some(slot) = RevalidationRegistry::global().try_claim((user_id, tenant_id)) else {
        debug!(%user_id, "home: activity refresh already in flight; not starting another");
        return;
    };
    let runtime = into_runtime(resources);
    resources.common.turns.spawn(async move {
        let refresh = refresh_providers(&runtime, user_id, tenant_id, &providers);
        if timeout(StdDuration::from_secs(REVALIDATION_TIMEOUT_SECS), refresh)
            .await
            .is_err()
        {
            warn!(
                %user_id,
                timeout_secs = REVALIDATION_TIMEOUT_SECS,
                "home: activity refresh timed out; releasing its slot"
            );
        }
        drop(slot);
    });
}

/// Top up each provider's recent head, one after the other.
async fn refresh_providers(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    providers: &[String],
) {
    for provider in providers {
        // The refresh writes through to the cache; the rows it returns are
        // what the next request reads, so nothing is kept here.
        let mut refreshed = Vec::new();
        refresh_stale_head(runtime, provider, user_id, tenant_id, None, &mut refreshed).await;
        debug!(
            %user_id,
            provider = %provider,
            fetched = refreshed.len(),
            "home: provider head checked"
        );
    }
}

async fn get_activity_route(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
) -> AppResult<Json<ActivityRouteResponse>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let (stored_provider, activity) =
        owned_cached_activity(&resources, user_id, tenant_id, &provider, &activity_id).await?;
    let runtime = into_runtime(&resources);
    let outcome = activity_route(
        &runtime,
        tenant_id,
        user_id,
        CachedActivityRef {
            provider: &stored_provider,
            activity: &activity,
        },
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
    // No conversation binds the page, so the plan is the one the athlete's
    // selected agent holds — the rung `/plan` reaches outside a conversation.
    let agent = resolve_plan_agent_slug(repos, None, tenant_id, user_id).await?;
    let plan = try_load_plan_card(
        repos,
        tenant_id,
        user_id,
        agent.as_deref(),
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
