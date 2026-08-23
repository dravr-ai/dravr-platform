// ABOUTME: Repository trait for the provider-agnostic activity cache (stale-while-revalidate)
// ABOUTME: Persists fetched Activity records so chat reads serve cached data instead of re-fetching
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use uuid::Uuid;

/// How deep a historical backfill has reached for a `(tenant, user, provider)`.
///
/// Lets the historical-activity gate distinguish "cached but only the recent
/// slice of a deep window" (re-backfill) from "cached down to the requested
/// floor" (covered, serve inline).
#[derive(Debug, Clone, Copy)]
pub struct BackfillCoverage {
    /// Deepest floor (unix seconds) a backfill has confirmed covered. When the
    /// scrape returned the whole requested window (not count-capped) this is the
    /// requested `after`, not the oldest activity fetched — so a sparse year
    /// whose oldest activity sits just after Jan 1 00:00 still reads as covered.
    pub oldest_reached_ts: i64,
    /// `true` only when the provider EXPLICITLY reports its feed exhausted
    /// (next-page disabled), so no older data exists and a deeper ask is covered
    /// without re-scraping. The date-floored scrape path cannot prove this (it
    /// stops at `after`, never below it), so it leaves this `false`; the gate
    /// still honors a `true` set by a provider that does report feed-end.
    pub hit_feed_end: bool,
}

/// Persistence for activities fetched from any provider, enabling
/// stale-while-revalidate reads on the chat path.
///
/// Every provider fetch writes through here; chat reads serve the cached rows
/// immediately and trigger a background revalidation when the data is stale.
/// This removes the slow per-request scrape (notably Garmin/sciotte) from the
/// chat critical path and avoids redundant API calls for token providers.
#[async_trait]
pub trait ActivityCacheRepository: Send + Sync {
    /// Insert or update a batch of activities for a user+provider.
    ///
    /// Keyed on `(user_id, tenant_id, provider, activity_id)`; a later fetch of
    /// the same activity overwrites the stored copy. Returns the count of NET
    /// DISTINCT rows persisted (deduped by `activity_id`), not the raw input
    /// length — a provider feed that repeats an `activity_id` within one batch
    /// upserts the same row, so the input length would overstate the rows
    /// actually stored. This honest count feeds the backfill completion notice.
    async fn upsert_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<u64>;

    /// Fetch cached activities for a user within `[start, end]`, newest first.
    ///
    /// `provider = None` returns activities across all providers.
    async fn get_cached_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<Activity>>;

    /// Delete every cached activity a provider contributed for a user.
    ///
    /// The provider-disconnect path calls this so revoking consent also
    /// removes the provider-derived rows we hold (Strava API Policy §7.4
    /// treats deletion on deauthorization as an obligation, not hygiene).
    /// Returns the number of rows removed.
    async fn delete_provider_activities(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;

    /// Most recent successful activity fetch for a user+provider — the
    /// freshness signal that drives background revalidation. The later of the
    /// cached rows' `synced_at` and the fetch mark recorded by
    /// [`Self::record_activity_fetch`], so a fetch that truthfully returned
    /// nothing still reads as fresh. `None` when the provider has never been
    /// fetched.
    async fn latest_activity_sync(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;

    /// Most recent successful activity fetch for a user across **every**
    /// provider — the later of the cached rows' `synced_at` and the fetch
    /// marks recorded by [`Self::record_activity_fetch`]. `None` when nothing
    /// has ever been fetched for them.
    ///
    /// The provider-scoped sibling above answers "should I revalidate this
    /// provider"; this one answers "is an empty window real, or has the cache
    /// simply not caught up". A caller that has to distinguish *the athlete did
    /// not do it* from *we do not know yet* cannot enumerate providers to find
    /// out — an athlete who connected a second device mid-window would look
    /// stale on the first one forever.
    async fn latest_activity_sync_any(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<DateTime<Utc>>>;

    /// Record that a provider activity fetch completed successfully at
    /// `fetched_at`, whether or not it returned any activities.
    ///
    /// The cached rows' `synced_at` only advances when a fetch returns rows,
    /// so without this mark an athlete whose provider truthfully reports no
    /// activities looks forever stale — and a freshness-guarded reader (the
    /// commitment sweep) could never believe an honest zero. Both
    /// `latest_activity_sync*` reads take the later of the row signal and
    /// this mark.
    async fn record_activity_fetch(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        fetched_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Delete a user's cached activities whose `start_date` is older than
    /// `cutoff` (retention pruning). Returns the number of rows removed.
    async fn prune_activities_before(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        cutoff: DateTime<Utc>,
    ) -> AppResult<u64>;

    /// Record how deep a completed backfill reached for `(tenant, user,
    /// provider)`. Overwrites any prior row — coverage only deepens, because a
    /// query shallower than the recorded floor is already covered and never
    /// reaches this write.
    async fn upsert_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        coverage: BackfillCoverage,
    ) -> AppResult<()>;

    /// Read the recorded backfill coverage for `(tenant, user, provider)`, or
    /// `None` when no deep backfill has run for this athlete+provider yet.
    async fn get_backfill_coverage(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<BackfillCoverage>>;
}
