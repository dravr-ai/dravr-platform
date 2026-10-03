// ABOUTME: Whether a live read of a provider's activity list counts as a sync, and what a failed one records
// ABOUTME: The head verdict, the failure record with its consecutive count, and the backoff it paces retries by
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The judgement every live list read passes through.
//!
//! A provider's answer counts as a sync only when it vouches for the list
//! head: an incomplete capture, an empty answer over a window the cache holds
//! activities in, and a read that failed are each recorded as a failed sync
//! instead, and a provider that keeps failing is paused before its head is
//! read again. On 2026-09-29 a scrape whose page died answered `count=0` as a
//! success, was stamped fresh, and nothing looked again for four hours. An
//! empty answer the provider repeats across an hour is the one exception: it
//! is believed, and the cached rows it contradicts are evicted
//! ([`believes_repeated_empty`]).

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{ActivityFetchFailure, ActivityFetchFailureRecord};
use pierre_providers::connect_prefetch::{ConnectPrefetches, PrefetchKey};
use pierre_providers::core::{ActivityQueryParams, FitnessProvider};
use tracing::{info, warn};
use uuid::Uuid;

use super::{
    before_bounds_a_closed_window, cached_window_bounds, provider_auth_failure, read_cached_window,
};
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// Whether a list read of `params` reads the provider's list head: its
/// newest activities, the ones a sync exists to bring in.
///
/// A window whose `before` is closed ([`before_bounds_a_closed_window`]) ends
/// before the head, and a later page (`offset` past zero) starts below it, so
/// neither says anything about whether the head is current. Only a read that
/// covers the head may count as a sync — move the provider's freshness — or,
/// failing, as a failed sync.
#[must_use]
pub fn covers_list_head(params: &ActivityQueryParams, now_ts: i64) -> bool {
    params.offset.unwrap_or(0) == 0 && !before_bounds_a_closed_window(params.before, now_ts)
}

/// Minutes a provider whose head refreshes keep failing is left alone before
/// its head is read again, by how many have failed in a row: the second
/// entry for the second failure, the last for every one after it.
///
/// A failed refresh leaves the head stale, so without a pause every Home
/// load, each of its follow-ups and every stale-head turn would start
/// another scrape against a scraper that has just failed — the shared
/// service whose failures the athlete is already being told about.
const SYNC_FAILURE_BACKOFF_MINUTES: [i64; 4] = [2, 5, 15, 60];

/// How many empty answers in a row over cached activities, this one
/// included, make an empty list believed ([`believes_repeated_empty`]).
const EMPTY_ANSWERS_BELIEVED: u32 = 3;

/// How long before the latest of those empty answers the first of them must
/// have come in: a burst of empty answers inside this span is a scraper
/// failing the same way on each retry, not a list the athlete emptied.
const EMPTY_ANSWERS_SPAN_MINUTES: i64 = 60;

/// When a provider whose last head refresh failed may be read again, or
/// `None` when it may be read now: no failure is recorded, a good sync since
/// has superseded it, or its pause is over.
///
/// The pause grows with the failures in a row
/// ([`SYNC_FAILURE_BACKOFF_MINUTES`]). An athlete's explicit retry is not
/// bound by it: the Home route that asks on their behalf reads regardless.
#[must_use]
pub fn sync_backoff_until(
    failure: Option<ActivityFetchFailureRecord>,
    last_sync: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let failure = failure.filter(|f| last_sync.is_none_or(|synced| f.failed_at > synced))?;
    let step = usize::try_from(failure.consecutive.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(SYNC_FAILURE_BACKOFF_MINUTES.len() - 1);
    let until = failure.failed_at + Duration::minutes(SYNC_FAILURE_BACKOFF_MINUTES[step]);
    (until > now).then_some(until)
}

/// What one live read of a provider's list produced, and whether it counts
/// as a sync.
pub struct HeadRead {
    /// The activities the provider answered with.
    pub activities: Vec<Activity>,
    /// The judgement of that answer.
    pub verdict: HeadVerdict,
}

/// Whether a live list read counts as a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadVerdict {
    /// The provider vouched for the list: it is written through and moves
    /// freshness.
    Complete,
    /// The capture never reached the list head
    /// (`FitnessProvider::head_complete` is false): served, not persisted.
    Incomplete,
    /// No activities over a window the cache holds `cached` of this
    /// provider's activities in ([`implausibly_empty`]), and not yet repeated
    /// long enough to be believed ([`believes_repeated_empty`]): a failed
    /// read.
    EmptyOverCached {
        /// How many cached rows of the provider sit inside the window.
        cached: usize,
    },
}

impl HeadVerdict {
    /// The failure a verdict records, or `None` for a sync.
    const fn failure(self) -> Option<ActivityFetchFailure> {
        match self {
            Self::Complete => None,
            Self::Incomplete => Some(ActivityFetchFailure::HeadIncomplete),
            Self::EmptyOverCached { .. } => Some(ActivityFetchFailure::EmptyOverCached),
        }
    }
}

/// Authenticate the provider and read `params` live, judging whether the
/// answer counts as a sync ([`judge_live_read`]). Records nothing and writes
/// nothing.
pub(super) async fn read_provider_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> AppResult<HeadRead> {
    let auth_service = AuthService::new(Arc::clone(runtime));
    // Taken before the credential is read, so a reconnect that lands while
    // the fetch is in flight is newer than any failure it reports.
    let attempt_started_at = Utc::now();
    let provider = auth_service
        .create_authenticated_provider(provider_slug, user_id, Some(tenant_id))
        .await
        .map_err(|response| provider_auth_failure(provider_slug, &response))?;
    judge_live_read(
        runtime,
        LiveRead {
            provider_slug,
            user_id,
            tenant_id: Some(tenant_id),
            params,
            attempt_started_at,
        },
        provider.as_ref(),
    )
    .await
}

/// One live list read, before it is made: whose, of which provider, for
/// which window, and when the attempt began.
pub struct LiveRead<'a> {
    /// The provider key the athlete's connection and cache rows use.
    pub provider_slug: &'a str,
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant the connection lives in; `None` for a caller outside any
    /// tenant, whose read is judged without the cache and re-arms nothing.
    pub tenant_id: Option<&'a str>,
    /// The window asked for.
    pub params: &'a ActivityQueryParams,
    /// When the attempt began, before the credential was read: a reconnect
    /// that lands while the read is in flight is newer than any refusal it
    /// reports.
    pub attempt_started_at: DateTime<Utc>,
}

/// Read `read.params` live through an already authenticated `provider` and
/// judge the answer.
///
/// [`HeadVerdict::Incomplete`] for a capture that never reached its head,
/// [`HeadVerdict::EmptyOverCached`] for an empty answer the cache
/// contradicts ([`implausibly_empty`]), [`HeadVerdict::Complete`] otherwise.
/// Records nothing: [`record_head_outcome`] records the attempt, and the
/// caller decides what to persist. The one write it makes is the eviction
/// that comes with believing a repeated empty answer
/// ([`believes_repeated_empty`]), which is judged
/// [`HeadVerdict::Complete`].
///
/// A read the provider answered re-arms a connection flagged `needs_reauth`
/// ([`rearm_after_live_read`]) — except one judged
/// [`HeadVerdict::EmptyOverCached`], which is a failed read presented as a
/// success (a page that died, a scrape that read nothing) and so shows
/// nothing about whether the session is alive. `pub` for the chat tool's
/// primary read, which authenticates on its own to keep its reconnect
/// handling and lives behind the `tools-data` feature.
///
/// A read that arrives while the connection's connect pre-fetch is in flight
/// waits for it and takes its answer when that covers `params`
/// ([`connect_prefetch_answer`]), so a connect's reads never press one fresh
/// session with two scrapes at once.
///
/// # Errors
///
/// Returns the provider's error when the read fails, after the
/// `TrainingPeaks` refusal handling every read shares.
pub async fn judge_live_read(
    runtime: &Arc<dyn ToolRuntime>,
    read: LiveRead<'_>,
    provider: &dyn FitnessProvider,
) -> AppResult<HeadRead> {
    let LiveRead {
        provider_slug,
        user_id,
        tenant_id,
        params,
        attempt_started_at,
    } = read;
    let tenant = tenant_id.and_then(|tenant_id| TenantId::parse_str(tenant_id).ok());
    let prefetched = match tenant {
        Some(tenant) => connect_prefetch_answer(tenant, user_id, provider_slug, params).await,
        None => None,
    };
    let read = match prefetched {
        Some(answer) => Ok(answer),
        None => provider
            .get_activities_with_params(params)
            .await
            .map(|activities| (activities, provider.head_complete())),
    };
    let (activities, head_complete) = match read {
        Ok(answer) => answer,
        Err(e) => {
            if let Some(tenant_id) = tenant_id {
                AuthService::new(Arc::clone(runtime))
                    .react_to_trainingpeaks_refusal(
                        user_id,
                        tenant_id,
                        provider,
                        &e,
                        attempt_started_at,
                    )
                    .await;
            }
            return Err(e);
        }
    };

    let verdict = if !head_complete {
        HeadVerdict::Incomplete
    } else if activities.is_empty() {
        match tenant {
            Some(tenant) => {
                judge_empty_answer(runtime, provider_slug, user_id, tenant, params).await
            }
            None => HeadVerdict::Complete,
        }
    } else {
        HeadVerdict::Complete
    };

    // The provider answered through the stored credential, which is what a
    // `needs_reauth` flag says it cannot do. A capture that is incomplete is
    // still not unauthenticated: the provider accepted the credential before
    // it read anything at all. An empty answer the cache contradicts is not
    // evidence either way — the page that should have held the list died —
    // so it leaves the flag where it was.
    if let (Some(tenant_id), false) = (
        tenant_id,
        matches!(verdict, HeadVerdict::EmptyOverCached { .. }),
    ) {
        rearm_after_live_read(runtime, user_id, tenant_id, provider.name()).await;
    }
    Ok(HeadRead {
        activities,
        verdict,
    })
}

/// The answer the connect pre-fetch of this connection gives a read of
/// `params`, waiting for it when one is in flight; `None` when none is, it
/// read nothing, or it does not cover the window
/// ([`PrefetchedWindow::serve`](pierre_providers::connect_prefetch::PrefetchedWindow::serve)).
///
/// A connect pre-fetches the fresh session's newest activities, and the
/// screen the athlete lands on asks for them again within a second. Reading
/// live beside the pre-fetch put a second browser on the scraper service for
/// the same session, which shed it (carnet#736); the pre-fetch's own read is
/// the same provider answer, moments old, so it is judged and written through
/// exactly as a live read would be.
async fn connect_prefetch_answer(
    tenant: TenantId,
    user_id: Uuid,
    provider_slug: &str,
    params: &ActivityQueryParams,
) -> Option<(Vec<Activity>, bool)> {
    let key = PrefetchKey::new(tenant, user_id, provider_slug);
    let window = ConnectPrefetches::global()
        .in_flight(&key)?
        .finished()
        .await?;
    let served = window.serve(params)?;
    info!(
        user_id = %user_id,
        provider = %provider_slug,
        count = served.len(),
        "List read answered by the connect pre-fetch in flight"
    );
    Some((served, window.head_complete()))
}

/// Record what a live read of `params` said about the provider's sync.
///
/// A read that failed (other than for authentication, which the reconnect
/// path reports) or that the verdict rejects is recorded as a failed sync
/// ([`record_sync_failure`]) — when it read the list head
/// ([`covers_list_head`]). A closed window or a later page that failed says
/// nothing about whether the head is current, so it records nothing.
pub async fn record_head_outcome(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
    provider_slug: &str,
    params: &ActivityQueryParams,
    read: &AppResult<HeadRead>,
) {
    let failure = match read {
        Ok(head) => head.verdict.failure(),
        Err(e) if e.provider_auth_required_provider().is_some() => None,
        Err(_) => Some(ActivityFetchFailure::FetchError),
    };
    if let Some(failure) = failure {
        if covers_list_head(params, Utc::now().timestamp()) {
            record_sync_failure(runtime, user_id, tenant, provider_slug, failure).await;
        }
    }
}

/// Judge an empty answer for `params`: an honest zero over a window the cache
/// holds nothing in, a failed read over one it holds activities in
/// ([`implausibly_empty`]) — unless the provider has now given that answer
/// long enough to be believed ([`believes_repeated_empty`]), in which case the
/// cached rows it contradicts are evicted and the answer is a sync.
async fn judge_empty_answer(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> HeadVerdict {
    let Some(cached) = implausibly_empty(runtime, provider_slug, user_id, tenant, params).await
    else {
        return HeadVerdict::Complete;
    };
    let now = Utc::now();
    if believes_repeated_empty(runtime, provider_slug, user_id, tenant, now).await
        && evict_window(runtime, provider_slug, user_id, tenant, params, now).await
    {
        return HeadVerdict::Complete;
    }
    warn!(
        user_id = %user_id,
        provider = %provider_slug,
        cached,
        "provider answered no activities over a window the cache holds \
         activities in; not counted as a sync"
    );
    HeadVerdict::EmptyOverCached { cached }
}

/// Evict the provider's cached activities in the window `params` read, which a
/// believed empty answer says the provider no longer holds. `false` when the
/// eviction failed: the answer then stays a failed read, since writing it
/// through would stamp fresh a cache that still serves the rows.
async fn evict_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
    now: DateTime<Utc>,
) -> bool {
    let (start, end) = cached_window_bounds(params, now);
    match runtime
        .repos()
        .activity_cache
        .delete_cached_activities_between(user_id, &tenant, provider_slug, start, end)
        .await
    {
        Ok(evicted) => {
            info!(
                user_id = %user_id,
                provider = %provider_slug,
                evicted,
                "provider kept answering no activities over cached ones for over an hour; \
                 believed, and the cached activities it no longer holds evicted"
            );
            true
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                "evicting activities a repeated empty answer contradicts failed; \
                 the answer stays a failed read"
            );
            false
        }
    }
}

/// Whether an empty answer over cached activities, arriving at `now`, is the
/// [`EMPTY_ANSWERS_BELIEVED`]th in a row with the first of them at least
/// [`EMPTY_ANSWERS_SPAN_MINUTES`] earlier — counted from the provider's
/// current failure record, whose streak holds only failures of that same
/// reason since the last good sync.
///
/// A provider has no delete signal on a scrape mirror (the Strava OAuth
/// webhook is the only one), so an athlete who deletes the only activity of
/// the head window is seen as the provider answering an empty list the cache
/// contradicts. Judged a failed read every time, that answer would never be
/// written through, the row would never be pruned, and Home would say "Sync
/// failed" until the row aged out of the window, for up to thirty days. A
/// scraper that answers a failed list walk with an error rather than an
/// empty success (dravr-sciotte#654) makes an empty answer that holds across
/// an hour of retries a list the athlete emptied, not a page that died; a
/// burst inside the hour is still treated as the failing scrape it most
/// likely is.
async fn believes_repeated_empty(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    now: DateTime<Utc>,
) -> bool {
    let cache = &runtime.repos().activity_cache;
    let Ok(last_sync) = cache
        .latest_activity_sync(user_id, &tenant, provider_slug)
        .await
    else {
        return false;
    };
    let Ok(Some(failure)) = cache
        .latest_activity_fetch_failure(user_id, &tenant, provider_slug)
        .await
    else {
        return false;
    };
    let current = last_sync.is_none_or(|synced| failure.failed_at > synced);
    current
        && failure.failure == ActivityFetchFailure::EmptyOverCached
        && failure.streak.saturating_add(1) >= EMPTY_ANSWERS_BELIEVED
        && now - failure.streak_started_at >= Duration::minutes(EMPTY_ANSWERS_SPAN_MINUTES)
}

/// Whether an empty answer for `params` contradicts the cache, and if so how
/// many cached rows of this provider sit inside the window it asked for.
///
/// The rule: a provider that answers "no activities" for a window the cache
/// holds this provider's activities in is not believed. Activities recorded
/// on a provider do not disappear between two reads, while a scrape that
/// failed half-way — a browser closed under it, a page read before it
/// rendered — has answered exactly that as a success. So the empty answer is
/// a failed read: it is not written through, freshness stays where it was,
/// and the read is tried again on the freshness bands. The one honest empty
/// answer it misjudges is an athlete who deleted every activity in the
/// window on the provider itself; the provider repeating that answer across
/// an hour is what tells the two apart ([`believes_repeated_empty`]).
/// A window the cache holds nothing in keeps the old meaning: an honest zero.
///
/// A later page (`offset` past zero) is never judged: an empty page past the
/// provider's last activity is the honest end of the list, and the cached
/// window it would be compared with starts at the first page.
async fn implausibly_empty(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<usize> {
    if params.offset.unwrap_or(0) > 0 {
        return None;
    }
    read_cached_window(runtime, provider_slug, user_id, tenant, params)
        .await
        .map(|rows| rows.len())
}

/// Record a head fetch that did not count as a sync, counting it into the
/// failures in a row since the provider's last good fetch.
///
/// That count is what [`sync_backoff_until`] paces the next attempt by; the
/// streak of failures for this same reason is what
/// [`believes_repeated_empty`] reads.
///
/// Best-effort: a failed record costs Home one "sync failed" notice and one
/// pause, never the fetch. `pub` so every caller that gives up on a head
/// read — Home's bounded refresh, the capture sweep's — records it the same
/// way.
pub async fn record_sync_failure(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    failure: ActivityFetchFailure,
) {
    let cache = &runtime.repos().activity_cache;
    let last_sync = cache
        .latest_activity_sync(user_id, &tenant_id, provider)
        .await
        .unwrap_or(None);
    let previous = cache
        .latest_activity_fetch_failure(user_id, &tenant_id, provider)
        .await
        .unwrap_or(None)
        .filter(|f| last_sync.is_none_or(|synced| f.failed_at > synced));
    let failed_at = Utc::now();
    let (streak, streak_started_at) = previous
        .filter(|f| f.failure == failure)
        .map_or((1, failed_at), |f| {
            (f.streak.saturating_add(1), f.streak_started_at)
        });
    let record = ActivityFetchFailureRecord {
        failed_at,
        failure,
        consecutive: previous.map_or(1, |f| f.consecutive.saturating_add(1)),
        streak,
        streak_started_at,
    };
    if let Err(e) = cache
        .record_activity_fetch_failure(user_id, &tenant_id, provider, &record)
        .await
    {
        info!(
            user_id = %user_id,
            provider = %provider,
            error = %e,
            "Activity cache: fetch failure record failed"
        );
    }
}

/// Re-arm a connection flagged `needs_reauth` once its stored credential has
/// served a live read.
///
/// A flag is a verdict one failed attempt reached, and nothing else revisits
/// it for a scrape session: those are never refreshed, so the re-arm a
/// successful token refresh performs never runs for them. A connection flagged
/// over an answer that was not the session dying (on 2026-09-25 the capture
/// sweep flagged one over a single `401` from a scraper instance that had not
/// seen the session import) then stays flagged while every later read through
/// the same session succeeds, and the sweep and the athlete's Home page, which
/// act on `active` connections only, stop refreshing it.
///
/// `backend` is the provider that served the read, by the key its connection
/// row is stored under: the provider's own name, never the name the caller
/// asked for, which may be the user-facing half of a mirror pair. Only a
/// `needs_reauth` row flips; a `revoked` or `active` one is left as it is.
///
/// Best-effort: the read has already succeeded, so a failed write is logged
/// and the activities are served regardless.
async fn rearm_after_live_read(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &str,
    backend: &str,
) {
    let Ok(tenant) = TenantId::parse_str(tenant_id) else {
        return;
    };
    match runtime
        .repos()
        .provider_connections
        .mark_active_if_needs_reauth(user_id, tenant, backend)
        .await
    {
        Ok(true) => info!(
            user_id = %user_id,
            provider = %backend,
            "connection re-armed: a live read through its stored credential succeeded"
        ),
        Ok(false) => {}
        Err(e) => warn!(
            user_id = %user_id,
            provider = %backend,
            error = %e,
            "connection re-arm after a live read failed; its status is left as it was"
        ),
    }
}
