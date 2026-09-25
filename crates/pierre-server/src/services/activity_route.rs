// ABOUTME: One cached activity's drawable route: the stored read, else the provider's route overview, else its streams
// ABOUTME: Each activity costs at most one provider read — the outcome, drawn or not, is persisted tenant-scoped

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity route reads for the Home page.
//!
//! A completed activity's route does not change, so its geometry is read once
//! and kept in `activity_route_tracks`. The read takes the cheapest source
//! that yields a drawable line:
//!
//! 1. the route overview the cached activity already carries (Strava's
//!    `summary_polyline`), which costs no provider call at all;
//! 2. the activity's recorded streams, one detail read against the athlete's
//!    rate-limited provider account.
//!
//! Both go through [`RouteTrack`], the derivation the chat map uses, so the
//! endpoints are trimmed before anything is stored. An overview that trims to
//! nothing drawable does not settle the question — its handful of points can
//! all sit inside the privacy radius of a short ride whose full track still
//! has a middle to draw — so the streams decide. What the streams say is
//! stored either way: a track, or the reason there is none, so an indoor ride
//! costs one read too, not one per page load.

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::StoredRouteTrack;
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::polyline::decode_polyline;
use pierre_fitness_compute::route_track::{RouteTrack, RouteTrackError};
use pierre_tool_runtime::protocol::provider_helpers::fetch_activity_from_provider;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::warn;
use uuid::Uuid;

/// Most coordinates a Home map carries.
///
/// A recorded ride runs to thousands of
/// points; a card-sized map cannot show more than a few hundred, and every
/// point past that is payload the phone downloads for nothing.
pub const HOME_ROUTE_MAX_POINTS: usize = 200;

/// What reading one activity's route produced: the track, or why there is
/// none.
pub type ActivityRouteOutcome = Result<RouteTrack, RouteTrackError>;

/// Where a stored route's geometry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteGeometrySource {
    /// The route overview on the cached activity (Strava's `summary_polyline`).
    SummaryPolyline,
    /// The activity's recorded streams, read from the provider.
    Streams,
}

impl RouteGeometrySource {
    /// The slug the stored row carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SummaryPolyline => "summary_polyline",
            Self::Streams => "streams",
        }
    }
}

/// The cached activity a route is read for, with the provider key its cached
/// row is stored under — the key a stored route is filed under too, so the
/// provider-disconnect purge deletes both together.
#[derive(Debug, Clone, Copy)]
pub struct CachedActivityRef<'a> {
    /// The provider key of the cached row.
    pub provider: &'a str,
    /// The cached activity.
    pub activity: &'a Activity,
}

/// The drawable route of one cached activity, read at most once from its
/// provider.
///
/// # Errors
///
/// Returns the repository error when the stored read cannot be loaded or the
/// outcome cannot be stored, and the provider's error when the streams read
/// fails — including [`AppError::provider_auth_required`] when the connection
/// needs reconnecting. A failed read stores nothing, so the next request
/// tries again.
pub async fn activity_route(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> AppResult<ActivityRouteOutcome> {
    let repos = runtime.repos();
    let activity_id = cached.activity.id();
    if let Some(stored) = repos
        .activity_route_tracks
        .get_route_track(&tenant_id, user_id, cached.provider, activity_id)
        .await?
    {
        if let Some(outcome) = stored_outcome(&stored) {
            return Ok(outcome);
        }
        // A row whose track no longer decodes (the track shape has evolved
        // since it was written) is a miss: read again and the upsert
        // overwrites it.
        warn!(
            activity_id,
            "stored route track no longer decodes; reading it again"
        );
    }
    let (source, outcome) = read_route(runtime, tenant_id, user_id, cached).await?;
    let outcome = outcome.map(|track| track.simplified(HOME_ROUTE_MAX_POINTS));
    store_outcome(repos, tenant_id, user_id, cached, source, &outcome).await?;
    Ok(outcome)
}

/// Read the route from the cheapest source that settles it.
async fn read_route(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> AppResult<(RouteGeometrySource, ActivityRouteOutcome)> {
    if let Some(track) = overview_track(cached.activity) {
        return Ok((RouteGeometrySource::SummaryPolyline, Ok(track)));
    }
    let tenant = tenant_id.to_string();
    let detailed = fetch_activity_from_provider(
        runtime,
        user_id,
        cached.provider,
        Some(tenant.as_str()),
        cached.activity.id(),
    )
    .await?;
    let outcome = detailed
        .time_series_data()
        .map_or(Err(RouteTrackError::NoGps), RouteTrack::from_streams);
    Ok((RouteGeometrySource::Streams, outcome))
}

/// The drawable track the activity's own route overview yields, or `None`
/// when it carries none, it does not decode, or too little of it survives the
/// trim to settle the question.
fn overview_track(activity: &Activity) -> Option<RouteTrack> {
    let encoded = activity
        .summary_polyline()
        .map(str::trim)
        .filter(|encoded| !encoded.is_empty())?;
    let points = decode_polyline(encoded)?;
    RouteTrack::from_overview(&points).ok()
}

/// The outcome a stored row records, or `None` when the row no longer decodes.
fn stored_outcome(stored: &StoredRouteTrack) -> Option<ActivityRouteOutcome> {
    match stored {
        StoredRouteTrack::Drawn { track_json, .. } => {
            serde_json::from_str::<RouteTrack>(track_json).ok().map(Ok)
        }
        StoredRouteTrack::Unavailable { reason, .. } => RouteTrackError::from_slug(reason).map(Err),
    }
}

/// Persist a read's outcome under the cached row's provider key.
async fn store_outcome(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    source: RouteGeometrySource,
    outcome: &ActivityRouteOutcome,
) -> AppResult<()> {
    let source = source.as_str().to_owned();
    let stored = match outcome {
        Ok(track) => StoredRouteTrack::Drawn {
            source,
            track_json: serde_json::to_string(track)
                .map_err(|e| AppError::internal(format!("serialize route track: {e}")))?,
        },
        Err(reason) => StoredRouteTrack::Unavailable {
            source,
            reason: reason.as_str().to_owned(),
        },
    };
    repos
        .activity_route_tracks
        .upsert_route_track(
            &tenant_id,
            user_id,
            cached.provider,
            cached.activity.id(),
            &stored,
        )
        .await
}
