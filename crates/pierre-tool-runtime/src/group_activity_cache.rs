// ABOUTME: Stale-while-revalidate activity cache for group member snapshots
// ABOUTME: Bounded-blocking refresh, single-flight revalidation registry, and the served_stale signal

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cached activity fetching for group member snapshots.
//!
//! [`fetch_member_activities`] is the single read path
//! [`crate::group_fitness`] builds snapshots from: cold caches fetch live,
//! warm-and-fresh caches serve immediately, and warm-but-stale caches refresh
//! within the same bounded budget the self path spends
//! ([`RefreshConfig::wait_for_refresh_timeout`]) before serving — with rows
//! that are *still* stale after the attempt flagged so the context renderer
//! can direct the model to fetch fresh data instead of narrating the
//! staleness as a provider problem (the 2026-08-13 inverted-recovery-verdict
//! incident).

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, PoisonError};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, RefreshConfig, TenantId};
use pierre_runtime_context::DataContext;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::activity_fetch::activity_cache_retention_days;
use crate::group_fitness::{
    ActivityDeduplicator, ActivityMergeStrategy, AllProvidersMerge, TimeWindowDeduplicator,
};
use crate::protocol::AuthService;
use crate::runtime::ToolRuntime;

/// Age beyond which cached activities trigger a revalidation
/// (stale-while-revalidate). Cached data is still what gets served; this only
/// decides whether a refresh runs first.
const ACTIVITY_CACHE_FRESH_SECS: i64 = 4 * 3600;

/// Upper bound on a single revalidation. A sciotte/Garmin scrape that hangs on
/// a provider-side throttle must not hold the single-flight slot (nor the
/// per-profile Chrome `SingletonLock`) open indefinitely, blocking every later
/// revalidation for the same user. Generous relative to a healthy ~2-minute
/// scrape so it only fires on a genuine stall.
const REVALIDATION_TIMEOUT_SECS: u64 = 240;

/// Get all connected provider names for a user.
///
/// Returns provider names in connection order, or empty vec if none connected.
async fn get_connected_providers(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<String> {
    data.repos()
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.provider)
        .collect()
}

/// Resolve a member's connected providers and fetch activities using the merge strategy.
///
/// Fetches from ALL providers, merges, and deduplicates to produce a complete
/// activity list for training load computation.
///
/// The second element is true when the rows come from a cache that was stale
/// and could not be freshened within the turn's refresh budget — the caller
/// marks the snapshot
/// [`served_stale`](pierre_core::models::groups::MemberFitnessSnapshot::served_stale)
/// so the context renderer can direct the model to fetch fresh data instead of
/// narrating the staleness as a provider problem.
pub(crate) async fn fetch_member_activities(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> (Vec<Activity>, bool) {
    let data = runtime.data();
    let providers = get_connected_providers(&data, user_id, tenant_id).await;
    if providers.is_empty() {
        info!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            "Snapshot: member has no connected providers — emitting empty snapshot (weekly counts and CTL/ATL/TSB will all be zero/None)"
        );
        return (Vec::new(), false);
    }

    let now = Utc::now();
    let window_start = now - Duration::days(activity_cache_retention_days());
    let read_limit = i64::try_from(
        runtime
            .config()
            .activity_fetch_limit
            .saturating_mul(providers.len()),
    )
    .unwrap_or(i64::MAX);

    // Stale-while-revalidate: serve cached activities immediately when present.
    let cached = data
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, read_limit)
        .await
        .unwrap_or_else(|e| {
            info!(user_id = %user_id, error = %e, "Activity cache: read failed; falling back to live fetch");
            Vec::new()
        });

    if cached.is_empty() {
        // Cold cache: fetch live now; write-through persists for next time.
        return (
            fetch_and_persist_live(runtime, &providers, user_id, tenant_id).await,
            false,
        );
    }

    // Warm-but-stale cache: refresh within the same bounded budget the self
    // path spends (`RefreshConfig::wait_for_refresh_timeout`) instead of
    // detaching the refresh and serving rows the very turn that kicked it —
    // the 2026-08-13 incident where a coach read a stale TSB of +43 ("très
    // frais") while the refresh that would have shown −66 (overreaching)
    // landed 3 seconds after the answer.
    let (cached, served_stale) =
        if activity_cache_is_stale(&data, user_id, tenant_id, &providers, now).await {
            refresh_stale_cache(runtime, &providers, user_id, tenant_id, now, cached).await
        } else {
            (cached, false)
        };

    // Dedup on read. Write-through persists each provider's activities under its
    // own cache key, so a workout synced from two providers (e.g. Strava +
    // sciotte/Garmin) is cached twice. The live-fetch path dedups before
    // returning (see `AllProvidersMerge::fetch_and_merge`); the warm-cache path
    // must do the same or cross-provider duplicates double-count in the snapshot.
    let before_dedup = cached.len();
    let cached = TimeWindowDeduplicator::from_env().deduplicate(cached);
    info!(
        user_id = %user_id,
        before_dedup,
        after_dedup = cached.len(),
        served_stale,
        "Snapshot: served activities from cache (stale-while-revalidate, deduplicated)"
    );
    (cached, served_stale)
}

/// Refresh a warm-but-stale cache within the bounded budget, re-read the rows
/// write-through produced, and report whether what is served is still stale.
///
/// The re-read is the point: fresh rows land only via write-through, so the
/// refresh task's own return value is never served — a refresh whose providers
/// all failed must not replace real cached rows with nothing. `cached` is the
/// pre-refresh row set, served as the fallback when the re-read itself errors.
async fn refresh_stale_cache(
    runtime: &Arc<dyn ToolRuntime>,
    providers: &[String],
    user_id: Uuid,
    tenant_id: TenantId,
    now: DateTime<Utc>,
    cached: Vec<Activity>,
) -> (Vec<Activity>, bool) {
    let completed = revalidate_within_budget(runtime, providers.to_vec(), user_id, tenant_id).await;

    let data = runtime.data();
    let window_start = now - Duration::days(activity_cache_retention_days());
    let read_limit = i64::try_from(
        runtime
            .config()
            .activity_fetch_limit
            .saturating_mul(providers.len()),
    )
    .unwrap_or(i64::MAX);
    let rows = match data
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, read_limit)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            info!(
                user_id = %user_id,
                error = %e,
                "Activity cache: re-read after revalidation failed; serving the pre-refresh rows"
            );
            cached
        }
    };

    // Still-stale after the attempt (budget elapsed, another turn's refresh in
    // flight, or dead providers): the rows are served, but marked so the
    // renderer directs the model to fetch fresh data.
    let served_stale =
        !completed || activity_cache_is_stale(&data, user_id, tenant_id, providers, now).await;
    (rows, served_stale)
}

/// Fetch activities live from all connected providers and persist them
/// (write-through happens inside [`AllProvidersMerge::fetch_and_merge`]).
///
/// Used on a cold cache and by the revalidation task.
async fn fetch_and_persist_live(
    runtime: &Arc<dyn ToolRuntime>,
    providers: &[String],
    user_id: Uuid,
    tenant_id: TenantId,
) -> Vec<Activity> {
    let auth_service = AuthService::new(Arc::clone(runtime));
    let strategy = AllProvidersMerge::new(runtime.config().activity_fetch_limit);
    strategy
        .fetch_and_merge(&auth_service, providers, user_id, tenant_id)
        .await
}

/// True when any connected provider's cached activities are missing or older
/// than [`ACTIVITY_CACHE_FRESH_SECS`], signalling a revalidation.
async fn activity_cache_is_stale(
    data: &DataContext,
    user_id: Uuid,
    tenant_id: TenantId,
    providers: &[String],
    now: DateTime<Utc>,
) -> bool {
    let fresh_cutoff = now - Duration::seconds(ACTIVITY_CACHE_FRESH_SECS);
    for provider in providers {
        let last_sync = data
            .repos()
            .activity_cache
            .latest_activity_sync(user_id, &tenant_id, provider)
            .await
            .unwrap_or(None);
        // Stale when never cached or the newest cached activity predates the
        // freshness window.
        if last_sync.is_none_or(|ts| ts < fresh_cutoff) {
            return true;
        }
    }
    false
}

/// Tracks which `(user, tenant)` background revalidations are in flight so
/// concurrent stale-cache chat turns collapse onto a single refresh.
///
/// Without this, every stale-cache turn spawned its own revalidation. The
/// per-profile Chrome `SingletonLock` (see [`pierre_providers`]) serializes
/// those scrapes rather than crashing, so N rapid turns queue N ~2-minute
/// scrapes behind one lock — the later ones redundant by the time they run.
/// Stale-while-revalidate only needs one in-flight refresh per user; the rest
/// are dropped. Entries are removed when the revalidation task finishes (or
/// times out), so the slot frees for the next genuinely-stale turn.
///
/// Production code shares one registry via [`Self::global`]; tests construct
/// isolated instances with [`Self::new`].
pub struct RevalidationRegistry {
    in_flight: Arc<StdMutex<HashSet<(Uuid, TenantId)>>>,
}

impl RevalidationRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    /// The process-global registry shared across every chat turn.
    #[must_use]
    pub fn global() -> &'static Self {
        static GLOBAL: LazyLock<RevalidationRegistry> = LazyLock::new(RevalidationRegistry::new);
        &GLOBAL
    }

    /// Claim the revalidation slot for a user. Returns a guard that frees the
    /// slot on drop, or `None` if a revalidation is already in flight (the
    /// caller then skips spawning a duplicate).
    pub fn try_claim(&self, key: (Uuid, TenantId)) -> Option<RevalidationGuard> {
        // Recover from poisoning: the set stays consistent even if a holder
        // panicked, and refusing all future revalidations would be worse.
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if set.insert(key) {
            Some(RevalidationGuard {
                in_flight: Arc::clone(&self.in_flight),
                key,
            })
        } else {
            None
        }
    }
}

impl Default for RevalidationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Frees a claimed revalidation slot when dropped, including on panic or
/// timeout, so a single failed refresh never wedges a user's slot shut.
pub struct RevalidationGuard {
    in_flight: Arc<StdMutex<HashSet<(Uuid, TenantId)>>>,
    key: (Uuid, TenantId),
}

impl Drop for RevalidationGuard {
    fn drop(&mut self) {
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        set.remove(&self.key);
    }
}

/// Re-fetch and persist a member's activities, waiting up to the same bounded
/// budget the self path spends on a blocking refresh
/// ([`RefreshConfig::wait_for_refresh_timeout`]) so the very turn that found
/// the cache stale can serve the refreshed rows. Returns true when the refresh
/// finished inside that budget.
///
/// Past the budget the task keeps running detached — capped at
/// [`REVALIDATION_TIMEOUT_SECS`] so a hung scrape releases its slot — and its
/// write-through still lands for the next turn. Deduplicated: only one
/// revalidation runs per `(user, tenant)` at a time; when one is already in
/// flight this reports "not fresh" rather than waiting on a task it has no
/// handle to.
async fn revalidate_within_budget(
    runtime: &Arc<dyn ToolRuntime>,
    providers: Vec<String>,
    user_id: Uuid,
    tenant_id: TenantId,
) -> bool {
    let Some(guard) = RevalidationRegistry::global().try_claim((user_id, tenant_id)) else {
        debug!(
            user_id = %user_id,
            "Activity cache: revalidation already in flight; serving cached rows"
        );
        return false;
    };

    let runtime = Arc::clone(runtime);
    let task = tokio::spawn(async move {
        // Hold the slot for the whole refresh; dropping `guard` frees it.
        let _guard = guard;
        info!(user_id = %user_id, "Activity cache: revalidation started");
        match timeout(
            StdDuration::from_secs(REVALIDATION_TIMEOUT_SECS),
            fetch_and_persist_live(&runtime, &providers, user_id, tenant_id),
        )
        .await
        {
            Ok(activities) => info!(
                user_id = %user_id,
                refreshed = activities.len(),
                "Activity cache: revalidation complete"
            ),
            Err(_elapsed) => warn!(
                user_id = %user_id,
                timeout_secs = REVALIDATION_TIMEOUT_SECS,
                "Activity cache: revalidation timed out; releasing slot"
            ),
        }
    });

    let budget = RefreshConfig::default().wait_for_refresh_timeout();
    match timeout(budget, task).await {
        Ok(Ok(())) => true,
        Ok(Err(join_error)) => {
            // The refresh task panicked; its guard already freed the slot.
            warn!(
                user_id = %user_id,
                error = %join_error,
                "Activity cache: revalidation task failed"
            );
            false
        }
        // Budget elapsed: the task keeps running detached so the refresh
        // still lands for the next turn.
        Err(_elapsed) => false,
    }
}
