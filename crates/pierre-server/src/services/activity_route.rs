// ABOUTME: One cached activity's drawable route: the stored read, else the provider's route overview, else its streams
// ABOUTME: The outcome is persisted tenant-scoped; only a no-GPS answer the read could not prove is read again, a day later

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
//!
//! A `no_gps` answer is only as good as the read behind it. A detail read that
//! carries a stream set without coordinates is the provider saying the
//! activity recorded none (Strava's and intervals.icu's streams of a trainer
//! ride), and it stands. A detail read that carries no stream set at all
//! proves nothing: a mirror backend's scrape answers that way whenever the
//! page it read held no map — a trainer ride, but also a page read before it
//! finished rendering, or a detail read past the scraper's navigation budget —
//! and an API provider serves its activity without streams when the streams
//! request fails. That answer is stored with an expiry,
//! [`UNPROVEN_NO_GPS_RECHECK_HOURS`] out, after which the Home list treats the
//! route as not read and the next request reads it again.
//!
//! A Home page asks for several routes at once, and a streams read is the
//! expensive step: through a mirror backend it is a headless scrape of a few
//! seconds, on a service that sheds the requests it cannot queue. So one
//! athlete's streams reads take turns — a `(user, tenant)` reads its provider
//! for one activity at a time — and a request that waited for its turn looks
//! at the store again before it reads, so any number of requests for the same
//! activity cost one read between them. The turns are held in this process:
//! each server instance keeps its own, and two instances can read for the
//! same athlete at the same time.

use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::StoredRouteTrack;
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::polyline::decode_polyline;
use pierre_fitness_compute::route_track::{RouteTrack, RouteTrackError};
use pierre_tool_runtime::protocol::provider_helpers::fetch_activity_from_provider;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::sync::Mutex as TokioMutex;
use tracing::warn;
use uuid::Uuid;

/// Most coordinates a Home map carries.
///
/// A recorded ride runs to thousands of
/// points; a card-sized map cannot show more than a few hundred, and every
/// point past that is payload the phone downloads for nothing.
pub const HOME_ROUTE_MAX_POINTS: usize = 200;

/// Hours a `no_gps` answer from a detail read that carried no stream set
/// stands before the route is read again.
///
/// A day. Long enough that the read-again reaches the provider, not the
/// scraper's own detail cache (fifteen minutes by default) or a navigation
/// budget that has not had time to recover, and that an activity which truly
/// recorded no GPS on a provider whose detail read never carries streams costs
/// one read a day while it sits on the Home page rather than one per visit.
/// Short enough that a GPS ride a failed read labelled "no GPS" is drawn the
/// next day.
pub const UNPROVEN_NO_GPS_RECHECK_HOURS: i64 = 24;

/// What reading one activity's route produced: the track, or why there is
/// none.
pub type ActivityRouteOutcome = Result<RouteTrack, RouteTrackError>;

/// What a streams read found, and whether its answer can be kept for good.
struct StreamsRead {
    outcome: ActivityRouteOutcome,
    /// `false` when the detail read carried no stream set at all, so its
    /// `no_gps` says the read found no route, not that the activity recorded
    /// none.
    proven: bool,
}

impl StreamsRead {
    /// When the stored answer is read again: a day out for an unproven
    /// `no_gps`, never for anything else.
    fn expires_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        (!self.proven && matches!(self.outcome, Err(RouteTrackError::NoGps)))
            .then(|| now + Duration::hours(UNPROVEN_NO_GPS_RECHECK_HOURS))
    }
}

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

/// Whose turn at the provider a lock is: an athlete, in the tenant they act
/// in.
type TurnKey = (Uuid, TenantId);

/// One athlete's lock. An `Arc` because the map and every request queued for
/// that athlete share it: they have to contend for one lock, and a request
/// keeps it across awaits the map's own borrow cannot span.
type TurnLock = Arc<TokioMutex<()>>;

/// The provider-read turns in use: one lock per `(user, tenant)` with a
/// streams read in flight or waiting.
///
/// An entry lives while a request holds or waits for it and is removed by the
/// last one to leave, so the map holds the athletes being read for at this
/// moment, not every athlete ever read for.
static PROVIDER_READ_TURNS: LazyLock<DashMap<TurnKey, TurnLock>> = LazyLock::new(DashMap::new);

/// One request's place in an athlete's queue for their provider.
///
/// Claimed before the wait and given up on drop, so a request that fails, or
/// is dropped while it waits or reads because its client went away, leaves
/// the map as it found it.
struct ProviderReadTurn {
    key: TurnKey,
    /// The athlete's lock, shared with the map and with every other request
    /// queued for the same athlete.
    lock: TurnLock,
}

impl ProviderReadTurn {
    /// Join the queue for one athlete's provider.
    fn claim(user_id: Uuid, tenant_id: TenantId) -> Self {
        let key = (user_id, tenant_id);
        let lock = Arc::clone(
            PROVIDER_READ_TURNS
                .entry(key)
                .or_insert_with(|| Arc::new(TokioMutex::new(())))
                .value(),
        );
        Self { key, lock }
    }
}

impl Drop for ProviderReadTurn {
    fn drop(&mut self) {
        // Two strong references are the map's and this one: nobody else is
        // queued, so the entry goes. A request that claims after that finds
        // no entry and inserts a lock of its own, which nobody holds; one
        // that claimed before it holds a third reference, so the entry stays
        // for that request to remove. Two requests leaving at the same
        // moment can each count the other's reference and both leave the
        // entry behind: it is unlocked and nobody waits on it, and the next
        // request for the athlete claims it and removes it when it leaves.
        PROVIDER_READ_TURNS.remove_if(&self.key, |_, stored| {
            Arc::ptr_eq(stored, &self.lock) && Arc::strong_count(stored) <= 2
        });
    }
}

/// The drawable route of one cached activity, read at most once from its
/// provider.
///
/// A route overview settles it without a provider call. Otherwise the request
/// takes the athlete's turn at their provider, so one `(user, tenant)` has
/// one streams read in flight, and looks at the store again once the turn is
/// its own: a request for the same activity that went first has stored the
/// answer. The turns are this server instance's own. A `no_gps` answer from a
/// detail read without a stream set is stored with an expiry, and once it has
/// passed the stored answer is no answer: the activity is read again.
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
    if let Some(outcome) = stored_route(repos, tenant_id, user_id, cached).await? {
        return Ok(outcome);
    }
    if let Some(track) = overview_track(cached.activity) {
        let source = RouteGeometrySource::SummaryPolyline;
        return settle(repos, tenant_id, user_id, cached, source, Ok(track), None).await;
    }
    let turn = ProviderReadTurn::claim(user_id, tenant_id);
    let reading = turn.lock.lock().await;
    if let Some(outcome) = stored_route(repos, tenant_id, user_id, cached).await? {
        return Ok(outcome);
    }
    let read = read_streams(runtime, tenant_id, user_id, cached).await?;
    let expires_at = read.expires_at(Utc::now());
    let source = RouteGeometrySource::Streams;
    let settled = settle(
        repos,
        tenant_id,
        user_id,
        cached,
        source,
        read.outcome,
        expires_at,
    )
    .await;
    // Held until the outcome is stored, so the next request in the queue
    // finds it.
    drop(reading);
    settled
}

/// The outcome stored for the activity, or `None` when none is stored, the
/// stored one has expired or it no longer decodes.
async fn stored_route(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> AppResult<Option<ActivityRouteOutcome>> {
    let activity_id = cached.activity.id();
    let Some(stored) = repos
        .activity_route_tracks
        .get_route_track(&tenant_id, user_id, cached.provider, activity_id)
        .await?
    else {
        return Ok(None);
    };
    let outcome = stored_outcome(&stored);
    if outcome.is_none() {
        // A row whose track no longer decodes (the track shape has evolved
        // since it was written) is a miss: read again and the upsert
        // overwrites it.
        warn!(
            activity_id,
            "stored route track no longer decodes; reading it again"
        );
    }
    Ok(outcome)
}

/// Simplify a read's track to the points a Home map carries, store the
/// outcome — until `expires_at` when it has one — and answer it.
async fn settle(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    source: RouteGeometrySource,
    outcome: ActivityRouteOutcome,
    expires_at: Option<DateTime<Utc>>,
) -> AppResult<ActivityRouteOutcome> {
    let outcome = outcome.map(|track| track.simplified(HOME_ROUTE_MAX_POINTS));
    let read = StoredRead {
        source,
        outcome: &outcome,
        expires_at,
    };
    store_outcome(repos, tenant_id, user_id, cached, read).await?;
    Ok(outcome)
}

/// Read the route from the activity's recorded streams: one detail read
/// against the athlete's provider.
///
/// A detail answer with no stream set is `no_gps` unproven: the mirror
/// backends fold a scraped route into the streams and leave them out when the
/// page held no route, and the API providers serve an activity without them
/// when the streams request fails. A stream set without coordinates is the
/// provider's own word that none were recorded.
async fn read_streams(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> AppResult<StreamsRead> {
    let tenant = tenant_id.to_string();
    let detailed = fetch_activity_from_provider(
        runtime,
        user_id,
        cached.provider,
        Some(tenant.as_str()),
        cached.activity.id(),
    )
    .await?;
    let unproven = StreamsRead {
        outcome: Err(RouteTrackError::NoGps),
        proven: false,
    };
    Ok(detailed
        .time_series_data()
        .map_or(unproven, |streams| StreamsRead {
            outcome: RouteTrack::from_streams(streams),
            proven: true,
        }))
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

/// One read's outcome as it is stored.
struct StoredRead<'a> {
    source: RouteGeometrySource,
    outcome: &'a ActivityRouteOutcome,
    /// When the stored answer is read again; `None` when it stands.
    expires_at: Option<DateTime<Utc>>,
}

/// Persist a read's outcome under the cached row's provider key.
async fn store_outcome(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    read: StoredRead<'_>,
) -> AppResult<()> {
    let source = read.source.as_str().to_owned();
    let stored = match read.outcome {
        Ok(track) => StoredRouteTrack::Drawn {
            source,
            track_json: serde_json::to_string(track)
                .map_err(|e| AppError::internal(format!("serialize route track: {e}")))?,
        },
        Err(reason) => StoredRouteTrack::Unavailable {
            source,
            reason: reason.as_str().to_owned(),
            expires_at: read.expires_at,
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
