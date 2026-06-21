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
/// slice of a deep window" (re-backfill) from "cached and the provider feed ends
/// here" (covered, never re-scrape).
#[derive(Debug, Clone, Copy)]
pub struct BackfillCoverage {
    /// Oldest activity start (unix seconds) the deepest backfill fetched.
    pub oldest_reached_ts: i64,
    /// `true` when that scrape exhausted the provider feed (next-page disabled
    /// before reaching the requested `after`), so no older data exists.
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

    /// Most recent `synced_at` for a user+provider — the freshness signal that
    /// drives background revalidation. `None` when never cached.
    async fn latest_activity_sync(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Option<DateTime<Utc>>>;

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
