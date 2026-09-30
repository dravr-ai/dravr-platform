// ABOUTME: One cached activity's drawable route: the stored read, else the provider's route overview, else its streams
// ABOUTME: The outcome is persisted tenant-scoped; a failed read is stored as unavailable for minutes, never as no GPS, and a retry reads past it

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
//! ride or a manual entry, a scraped detail whose route holds no
//! coordinates), and it stands. So does the answer for a provider
//! integration with no stream source at all (WHOOP, COROS, Terra, the direct
//! Garmin API — `FitnessProvider::serves_activity_streams`): no read of it
//! can ever carry a track, so none is spent, and the activity has no track
//! the platform can draw. Every other way a detail read can come back proves
//! nothing about the activity, and is answered `unavailable` instead: a read
//! that carries no stream set at all (a mirror backend's scrape answers that
//! way whenever no read settled the page's route — a page read before it
//! finished rendering, a detail read past the scraper's navigation budget —
//! and an API provider serves its activity without streams when the streams
//! request fails), a read that failed, and one that did not finish within
//! [`ROUTE_PROVIDER_READ_TIMEOUT_SECS`]. `unavailable` is stored with an
//! expiry [`UNREAD_ROUTE_RECHECK_MINUTES`] out, so a page that asks again at
//! once is answered without reaching a provider that has just failed, and the
//! Home list keeps the activity's `has_gps` true. The athlete's own retry
//! reads past a stored `unavailable`: it is the one request that asks for the
//! provider again, and it still takes the athlete's turn. On 2026-09-29 every
//! detail read came back without streams while the scraper was failing, each
//! was stored as `no_gps` for a day, and Home drew no map and said nothing
//! about why. A read refused for authentication is the one failure answered
//! as an error: the athlete has to reconnect, and the client says so.
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
//!
//! The read itself is never cut short by the request that started it. It
//! runs on the server's drain tracker, holding the athlete's turn until the
//! provider answers and its outcome is stored; the request waits for it only
//! within [`ROUTE_READ_TIMEOUT_SECS`], counted from when it arrived, and past
//! that answers `unavailable` without storing anything. A read queued behind
//! slow ones is then answered by the store on the next request, never stored
//! as a failure the provider did not have, and a request that gives up never
//! leaves a scrape running with the turn released — the next read on the
//! same scraper session waits for it.

use std::env;
use std::sync::{Arc, LazyLock};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::StoredRouteTrack;
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::polyline::decode_polyline;
use pierre_fitness_compute::route_track::{RouteTrack, RouteTrackError};
use pierre_tool_runtime::protocol::provider_helpers::configured_provider;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::sync::{oneshot, Mutex as TokioMutex};
use tokio::time::{timeout, timeout_at, Instant};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::services::turn_lifecycle::InFlightTurns;

/// Most coordinates a Home map carries.
///
/// A recorded ride runs to thousands of
/// points; a card-sized map cannot show more than a few hundred, and every
/// point past that is payload the phone downloads for nothing.
pub const HOME_ROUTE_MAX_POINTS: usize = 200;

/// Minutes an `unavailable` answer stands before the route is read again.
///
/// Long enough that a Home page reloaded, or several of its rows asking at
/// once, does not send a scraper that has just failed a burst of detail
/// reads, and that the read-again reaches past the scraper's own detail cache
/// (fifteen minutes by default) often enough to matter. Short enough that a
/// page opened a few minutes later reaches the provider again; the athlete's
/// own retry reads past it at once.
pub const UNREAD_ROUTE_RECHECK_MINUTES: i64 = 10;

/// Seconds a route request waits for its provider turn and its read.
///
/// Counted from when it arrived: past it the request answers `unavailable`
/// without storing anything. `PIERRE_HOME_ROUTE_ANSWER_SECS` overrides it
/// ([`route_answer_budget`]).
///
/// Under the clients' thirty-second route request timeout
/// (`HOME_ROUTE_REQUEST_TIMEOUT_MS` in `@pierre/shared-constants`), so the
/// map always resolves to a route or a sentence instead of a spinner the
/// client gives up on: a scrape queued behind four others on a slow scraper
/// can take minutes. The read it started keeps running past it and stores
/// what the provider says.
pub const ROUTE_READ_TIMEOUT_SECS: u64 = 25;

/// Seconds one provider read may hold the athlete's turn.
///
/// Past it the read is given up and stored `unavailable`.
/// `PIERRE_HOME_ROUTE_PROVIDER_READ_SECS` overrides it
/// ([`route_provider_read_bound`]).
///
/// Past the scraper's own request deadline (320 s), so a scrape is answered
/// by the scraper and not cut off here; a provider that has not answered by
/// then has failed, and the athlete's next route read must not wait behind it
/// forever.
pub const ROUTE_PROVIDER_READ_TIMEOUT_SECS: u64 = 330;

/// [`ROUTE_READ_TIMEOUT_SECS`], or `PIERRE_HOME_ROUTE_ANSWER_SECS` when it
/// holds a positive number of seconds.
fn route_answer_budget() -> StdDuration {
    seconds_from_env("PIERRE_HOME_ROUTE_ANSWER_SECS", ROUTE_READ_TIMEOUT_SECS)
}

/// [`ROUTE_PROVIDER_READ_TIMEOUT_SECS`], or
/// `PIERRE_HOME_ROUTE_PROVIDER_READ_SECS` when it holds a positive number of
/// seconds.
fn route_provider_read_bound() -> StdDuration {
    seconds_from_env(
        "PIERRE_HOME_ROUTE_PROVIDER_READ_SECS",
        ROUTE_PROVIDER_READ_TIMEOUT_SECS,
    )
}

/// A bound in seconds from `key`, or `default` when it is unset, not a
/// number or not positive.
fn seconds_from_env(key: &str, default: u64) -> StdDuration {
    StdDuration::from_secs(
        env::var(key)
            .ok()
            .and_then(|secs| secs.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(default),
    )
}

/// Why an activity has no route to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMiss {
    /// A read settled that there is no drawable track: the recording held no
    /// GPS, or too little of it survives the privacy trim.
    Settled(RouteTrackError),
    /// No read settled it: the detail read failed, timed out, or carried no
    /// stream set. Read again after [`UNREAD_ROUTE_RECHECK_MINUTES`], or at
    /// the athlete's retry.
    Unavailable,
}

impl RouteMiss {
    /// The slug the wire and the stored row carry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Settled(reason) => reason.as_str(),
            Self::Unavailable => UNAVAILABLE,
        }
    }

    /// The miss a stored reason slug names, or `None` for an unknown slug.
    fn from_slug(slug: &str) -> Option<Self> {
        if slug == UNAVAILABLE {
            return Some(Self::Unavailable);
        }
        RouteTrackError::from_slug(slug).map(Self::Settled)
    }
}

/// The slug of [`RouteMiss::Unavailable`].
const UNAVAILABLE: &str = "unavailable";

/// What reading one activity's route produced: the track, or why there is
/// none.
pub type ActivityRouteOutcome = Result<RouteTrack, RouteMiss>;

/// When a stored answer is read again: [`UNREAD_ROUTE_RECHECK_MINUTES`] out
/// for `unavailable`, never for an answer a read settled.
fn expires_at(outcome: &ActivityRouteOutcome, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    matches!(outcome, Err(RouteMiss::Unavailable))
        .then(|| now + Duration::minutes(UNREAD_ROUTE_RECHECK_MINUTES))
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
/// answer. The turns are this server instance's own.
///
/// The read runs detached on `turns`, holding the turn until the provider
/// answers and the outcome is stored ([`ROUTE_PROVIDER_READ_TIMEOUT_SECS`] at
/// most). This request waits for the turn and the read within
/// [`ROUTE_READ_TIMEOUT_SECS`] of arriving; past it, it answers
/// [`RouteMiss::Unavailable`] and stores nothing, and the read still stores
/// its outcome for the next request. A read that fails or carries no stream
/// set is stored as [`RouteMiss::Unavailable`], which expires.
///
/// `retry` is the athlete asking again after an `unavailable` answer: a
/// stored `unavailable` read before this request arrived does not answer it,
/// and the provider is read again, through the same turn. One stored while it
/// waited — another request's read of the same activity — does.
///
/// # Errors
///
/// Returns the repository error when the stored read cannot be loaded or the
/// outcome cannot be stored, and [`AppError::provider_auth_required`] when the
/// connection needs reconnecting, which stores nothing.
pub async fn activity_route(
    runtime: &Arc<dyn ToolRuntime>,
    turns: &InFlightTurns,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    retry: bool,
) -> AppResult<ActivityRouteOutcome> {
    let repos = runtime.repos();
    let asked = StoredAnswer {
        retry_asked_at: retry.then(Utc::now),
    };
    if let Some(outcome) = stored_route(repos, tenant_id, user_id, cached, asked).await? {
        return Ok(outcome);
    }
    if let Some(track) = overview_track(cached.activity) {
        let source = RouteGeometrySource::SummaryPolyline;
        return settle(repos, tenant_id, user_id, cached, source, Ok(track)).await;
    }
    let deadline = Instant::now() + route_answer_budget();
    let turn = ProviderReadTurn::claim(user_id, tenant_id);
    let Ok(holding) = timeout_at(deadline, Arc::clone(&turn.lock).lock_owned()).await else {
        // Another read of the athlete's still holds the provider: nothing
        // was asked of it here, so nothing is stored either.
        warn!(
            activity_id = cached.activity.id(),
            "route read waited past its bound for the athlete's provider turn"
        );
        return Ok(Err(RouteMiss::Unavailable));
    };
    if let Some(outcome) = stored_route(repos, tenant_id, user_id, cached, asked).await? {
        return Ok(outcome);
    }
    let (answer, answered) = oneshot::channel();
    let read = DetachedRead {
        runtime: Arc::clone(runtime),
        tenant_id,
        user_id,
        provider: cached.provider.to_owned(),
        activity: cached.activity.clone(),
    };
    turns.spawn(async move {
        let outcome = read.read_and_settle().await;
        // The turn is held until the outcome is stored, so the next request
        // in the queue finds it; released lock first, then the queue place,
        // so the queue entry goes with the last one out.
        drop(holding);
        drop(turn);
        if answer.send(outcome).is_err() {
            debug!(
                activity_id = read.activity.id(),
                "route read finished after its request answered; its outcome is stored"
            );
        }
    });
    if let Ok(answer) = timeout_at(deadline, answered).await {
        // A read task that ended without answering has panicked, and stored
        // nothing: nothing settled the route.
        return answer.unwrap_or(Ok(Err(RouteMiss::Unavailable)));
    }
    warn!(
        activity_id = cached.activity.id(),
        provider = cached.provider,
        "route read still running past the request's bound; answered unavailable, \
         the read stores what the provider says"
    );
    Ok(Err(RouteMiss::Unavailable))
}

/// Which stored answer a request takes: every unexpired one, except that an
/// explicit retry (`retry_asked_at`) reads past an `unavailable` stored
/// before it arrived.
#[derive(Debug, Clone, Copy)]
struct StoredAnswer {
    retry_asked_at: Option<DateTime<Utc>>,
}

impl StoredAnswer {
    /// Whether `stored` answers this request.
    fn takes(self, stored: &StoredRouteTrack) -> bool {
        let Some(asked_at) = self.retry_asked_at else {
            return true;
        };
        match stored {
            StoredRouteTrack::Unavailable {
                reason,
                expires_at: Some(expires_at),
                ..
            } if reason == UNAVAILABLE => {
                // An `unavailable` expires a fixed time after its read, so
                // its expiry names when it was read.
                *expires_at - Duration::minutes(UNREAD_ROUTE_RECHECK_MINUTES) >= asked_at
            }
            _ => true,
        }
    }
}

/// One provider read for a route, owned so it can outlive the request that
/// started it.
struct DetachedRead {
    runtime: Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: String,
    activity: Activity,
}

impl DetachedRead {
    /// Read the activity's streams within [`route_provider_read_bound`] and
    /// store what the read settled.
    async fn read_and_settle(&self) -> AppResult<ActivityRouteOutcome> {
        let cached = CachedActivityRef {
            provider: &self.provider,
            activity: &self.activity,
        };
        let outcome = read_streams_by(&self.runtime, self.tenant_id, self.user_id, cached).await?;
        settle(
            self.runtime.repos(),
            self.tenant_id,
            self.user_id,
            cached,
            RouteGeometrySource::Streams,
            outcome,
        )
        .await
    }
}

/// [`read_streams`] bounded by [`route_provider_read_bound`]: a read that
/// fails for any reason but authentication, or does not finish in time, is
/// [`RouteMiss::Unavailable`].
///
/// # Errors
///
/// Returns [`AppError::provider_auth_required`] when the connection needs
/// reconnecting.
async fn read_streams_by(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> AppResult<ActivityRouteOutcome> {
    let bound = route_provider_read_bound();
    match timeout(bound, read_streams(runtime, tenant_id, user_id, cached)).await {
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(e)) if e.provider_auth_required_provider().is_some() => Err(e),
        Ok(Err(e)) => {
            warn!(
                activity_id = cached.activity.id(),
                provider = cached.provider,
                error = %e,
                "route detail read failed; answered unavailable"
            );
            Ok(Err(RouteMiss::Unavailable))
        }
        Err(_) => {
            warn!(
                activity_id = cached.activity.id(),
                provider = cached.provider,
                timeout_secs = bound.as_secs(),
                "route detail read did not finish in time; answered unavailable"
            );
            Ok(Err(RouteMiss::Unavailable))
        }
    }
}

/// The outcome stored for the activity, or `None` when none is stored, the
/// stored one has expired, it does not answer this request (`asked`) or it no
/// longer decodes.
async fn stored_route(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    asked: StoredAnswer,
) -> AppResult<Option<ActivityRouteOutcome>> {
    let activity_id = cached.activity.id();
    let Some(stored) = repos
        .activity_route_tracks
        .get_route_track(&tenant_id, user_id, cached.provider, activity_id)
        .await?
        .filter(|stored| asked.takes(stored))
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
/// outcome — until it expires, for `unavailable` — and answer it.
async fn settle(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    source: RouteGeometrySource,
    outcome: ActivityRouteOutcome,
) -> AppResult<ActivityRouteOutcome> {
    let outcome = outcome.map(|track| track.simplified(HOME_ROUTE_MAX_POINTS));
    let read = StoredRead {
        source,
        outcome: &outcome,
        expires_at: expires_at(&outcome, Utc::now()),
    };
    store_outcome(repos, tenant_id, user_id, cached, read).await?;
    Ok(outcome)
}

/// Read the route from the activity's recorded streams: one detail read
/// against the athlete's provider.
///
/// A provider integration with no stream source
/// (`FitnessProvider::serves_activity_streams`) is asked nothing: no read of
/// it can carry a track, so it settles `no_gps` — the platform has no track
/// to draw for the activity, and asking again would only spend a detail read
/// every [`UNREAD_ROUTE_RECHECK_MINUTES`]. Otherwise a stream set without
/// coordinates is the provider's own word that none were recorded, and
/// settles `no_gps`. A detail answer with no stream set settles nothing — the
/// mirror backends fold a scraped route into the streams and leave them out
/// when no read settled the page's route, and the API providers serve an
/// activity without them when the streams request fails — so it is
/// [`RouteMiss::Unavailable`].
///
/// # Errors
///
/// Returns the provider's error when the provider cannot be authenticated or
/// the detail read fails.
async fn read_streams(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> AppResult<ActivityRouteOutcome> {
    let tenant = tenant_id.to_string();
    let provider =
        configured_provider(runtime, user_id, cached.provider, Some(tenant.as_str())).await?;
    if !provider.serves_activity_streams() {
        return Ok(Err(RouteMiss::Settled(RouteTrackError::NoGps)));
    }
    let detailed = provider
        .get_activity_with_streams(cached.activity.id())
        .await?;
    Ok(detailed
        .time_series_data()
        .map_or(Err(RouteMiss::Unavailable), |streams| {
            RouteTrack::from_streams(streams).map_err(RouteMiss::Settled)
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
        StoredRouteTrack::Unavailable { reason, .. } => RouteMiss::from_slug(reason).map(Err),
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
        Err(miss) => StoredRouteTrack::Unavailable {
            source,
            reason: miss.as_str().to_owned(),
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
