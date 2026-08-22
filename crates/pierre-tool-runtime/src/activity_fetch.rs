// ABOUTME: Shared helper to fetch a user's recent activities from their connected providers
// ABOUTME: One auth+fetch path reused by group snapshots and coach recommendations — no duplication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider activity fetching shared across route crates.
//!
//! Authenticating a provider (refreshing OAuth tokens, resolving sciotte
//! mirrors) and pulling activities is identical whether the caller is the
//! group analytics snapshot builder or the coach recommender. This module
//! owns that single path so neither re-implements it.

use std::cmp::{Ordering, Reverse};
use std::env;
use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::ActivityCacheRepository;
use pierre_providers::core::ActivityQueryParams;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_dedup::{ActivityDeduplicator, TimeWindowDeduplicator};
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;
use pierre_providers::backend_resolver;
use serde_json::Value;

/// Cache fallback window when the request carries no `after` lower bound.
const STALE_FALLBACK_WINDOW_DAYS: i64 = 90;

/// Cap on stale rows served from cache when a live fetch fails.
const STALE_FALLBACK_LIMIT: i64 = 500;

/// Default retention + read window (days) for the provider-agnostic activity
/// cache when `PIERRE_ACTIVITY_CACHE_RETENTION_DAYS` is unset. Exceeds the
/// training-load lookback so cached reads always cover CTL/ATL/TSB.
const DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS: i64 = 90;

/// Resolve the activity-cache retention window (days) from the environment,
/// falling back to [`DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS`]. This is both the
/// prune cutoff applied after a write-through and the lookback used when
/// reading cached rows, so widening it deepens the cache into a historical
/// store (e.g. to keep a backfilled season) at the cost of more rows retained.
/// Non-positive or unparseable values fall back to the default.
#[must_use]
pub(crate) fn activity_cache_retention_days() -> i64 {
    env::var("PIERRE_ACTIVITY_CACHE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|days| *days > 0)
        .unwrap_or(DEFAULT_ACTIVITY_CACHE_RETENTION_DAYS)
}

/// Fold the athlete's OTHER connected providers into a primary recent-window
/// fetch, then deduplicate. No-op when the caller pinned an explicit
/// `provider` argument or the ask is the coverage-gated historical branch.
///
/// `resolve_provider_for_tool` picks ONE provider (arg, env, or most recently
/// used connection), which shows a multi-provider athlete only a slice of
/// their training: on 2026-08-22 a most-recently-used WHOOP connection hid a
/// 200km Strava ride behind WHOOP's distance-less, misclassified "run" record
/// of the same session. This reuses the peer tool's cache-degrading
/// [`fetch_provider_activities`] per remaining connection (cross-tenant, like
/// the peer path, so a provider connected under the athlete's own tenant
/// resolves from a group conversation) and the snapshot's deduplicator, whose
/// `pick_best` keeps the GPS row and whose cross-sport rule pairs a watch's
/// misclassified sport with the GPS provider's record of the same workout.
///
/// Best-effort by design: a secondary connection that fails to fetch is
/// skipped (the helper already logs it) — the primary path alone decides
/// auth errors and reconnect handoffs.
pub async fn maybe_merge_other_connections(
    context: &ToolExecutionContext,
    args: &Value,
    is_historical: bool,
    primary_backend: &str,
    params: &ActivityQueryParams,
    mut activities: Vec<Activity>,
) -> Vec<Activity> {
    let explicit_provider_arg = args
        .get("provider")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if explicit_provider_arg || is_historical {
        return activities;
    }

    let tenant = context.tenant_id.map(TenantId::from_uuid);
    let Ok(connections) = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, None)
        .await
    else {
        return activities;
    };

    let mut merged_any = false;
    for conn in &connections {
        let canonical = backend_resolver::resolve_backend(
            &context.resources.repos().auth_repos(),
            context.user_id,
            tenant,
            &conn.provider,
        )
        .await;
        if canonical == primary_backend {
            continue;
        }
        if let Some(fetched) = fetch_provider_activities(
            &context.resources,
            &conn.provider,
            context.user_id,
            &conn.tenant_id,
            params,
        )
        .await
        {
            if !fetched.is_empty() {
                merged_any = true;
            }
            activities.extend(fetched);
        }
    }

    if merged_any {
        activities = TimeWindowDeduplicator::from_env().deduplicate(activities);
    }
    activities
}

/// Fetch activities for a single provider connection, authenticating (and
/// refreshing tokens) as needed.
///
/// A successful fetch is written through to the activity cache so a later
/// failure can be served from it. When the live fetch fails (provider needs
/// re-auth, transient scrape error, timeout), the user's cached activities for
/// the same window are served stale instead of an empty result — a provider
/// blip degrades to "slightly old data" rather than "no data". Returns `None`
/// only when the live fetch fails *and* the cache is empty.
pub async fn fetch_provider_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: &str,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let auth_service = AuthService::new(Arc::clone(runtime));
    let tenant = TenantId::parse_str(tenant_id).ok();

    let live = match auth_service
        .create_authenticated_provider(provider_slug, user_id, Some(tenant_id))
        .await
    {
        Ok(provider) => provider
            .get_activities_with_params(params)
            .await
            .map_err(|e| {
                warn!(
                    user_id = %user_id,
                    provider = %provider_slug,
                    error = %e,
                    "fetch_provider_activities: failed to fetch activities"
                );
            })
            .ok(),
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = ?e.error,
                "fetch_provider_activities: failed to create authenticated provider"
            );
            None
        }
    };

    if let Some(activities) = live {
        // Warm the stale-while-revalidate cache so the next outage serves these.
        if let Some(tenant) = tenant {
            write_through_activity_cache(
                &auth_service,
                user_id,
                tenant,
                provider_slug,
                &activities,
                activity_cache_retention_days(),
            )
            .await;
        }
        return Some(activities);
    }

    // Live fetch failed — serve the user's cached activities for this window
    // rather than returning nothing.
    serve_stale_activities(runtime, provider_slug, user_id, tenant?, params).await
}

/// Read a provider's cached activities for the request window from the durable
/// activity cache, newest first.
///
/// Returns `None` when the cache is empty or the read fails — the caller then
/// treats the provider as "no activities". Shared by the stale-fallback path
/// and the historical-backfill gate (which serves a deep window straight from
/// cache once a prior backfill has populated it).
pub(crate) async fn read_cached_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let now = Utc::now();
    // Honor the request's `before` upper bound. A historical query like
    // "2022 races" (after=2022, before=2023) must read the bounded [after,
    // before] window — reading [after, now] would return recent rows that fall
    // inside the open window and mask whether the deep history is actually
    // cached.
    let end = params
        .before
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or(now);
    let start = params
        .after
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(|| now - Duration::days(STALE_FALLBACK_WINDOW_DAYS));
    let limit = params
        .limit
        .and_then(|l| i64::try_from(l).ok())
        .unwrap_or(STALE_FALLBACK_LIMIT);

    match runtime
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant, Some(provider_slug), start, end, limit)
        .await
    {
        Ok(cached) if !cached.is_empty() => Some(cached),
        Ok(_) => None,
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                "activity cache window read failed"
            );
            None
        }
    }
}

/// Read a provider's cached activities after a failed live fetch, newest first.
///
/// Thin wrapper over [`read_cached_window`] that logs the stale-serve. Only
/// invoked after a live fetch failure, so any non-empty result is strictly
/// better than the empty fallback.
async fn serve_stale_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let cached = read_cached_window(runtime, provider_slug, user_id, tenant, params).await;
    if let Some(ref activities) = cached {
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            count = activities.len(),
            "fetch_provider_activities: live fetch failed, serving stale cached activities"
        );
    }
    cached
}

/// Fetch recent activities across all of a user's connected providers and
/// merge them into a single list.
///
/// `after_ts` is a Unix timestamp (seconds) lower bound; `limit_per_provider`
/// caps how many activities are pulled from each provider. Providers that
/// fail to authenticate or fetch are skipped (logged at `warn`) rather than
/// failing the whole call.
pub async fn fetch_recent_activities_all_providers(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &str,
    after_ts: i64,
    limit_per_provider: usize,
) -> Vec<Activity> {
    let connections = runtime
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await
        .unwrap_or_default();

    let params = ActivityQueryParams {
        limit: Some(limit_per_provider),
        offset: None,
        before: None,
        after: Some(after_ts),
    };

    let mut all = Vec::new();
    for connection in &connections {
        if let Some(activities) =
            fetch_provider_activities(runtime, &connection.provider, user_id, tenant_id, &params)
                .await
        {
            all.extend(activities);
        }
    }
    all
}

/// Warm the provider-agnostic activity cache after a successful live fetch so
/// the next outage serves these rows stale-while-revalidate. Shared with the
/// group snapshot builder through the `activity_cache` repo.
///
/// `retention_days` is the per-transaction prune window: after the upsert,
/// rows older than `now - retention_days` are garbage-collected. Recent-fetch
/// callers pass [`activity_cache_retention_days`] (the deployment default);
/// a historical backfill passes a deeper window so the season it just wrote is
/// not immediately pruned. Pruning is keyed by `(user_id, tenant_id)` across
/// all providers, so the retention floor for durable history is whatever the
/// *widest-window* writer uses.
/// Returns the count of net distinct rows persisted (deduped by `activity_id`),
/// or `None` when the upsert itself failed. The historical backfill surfaces
/// this honest figure in its completion notice; recent-fetch callers ignore it.
pub(crate) async fn write_through_activity_cache(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activities: &[Activity],
    retention_days: i64,
) -> Option<u64> {
    let data = auth_service.runtime().data();
    let cache = data.repos().activity_cache.clone();
    let persisted = match cache
        .upsert_activities(user_id, &tenant_id, provider, activities)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: write-through failed");
            return None;
        }
    };
    let cutoff = Utc::now() - Duration::days(retention_days);
    if let Err(e) = cache
        .prune_activities_before(user_id, &tenant_id, cutoff)
        .await
    {
        info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: prune failed");
    }
    stamp_fetch_freshness(cache.as_ref(), user_id, tenant_id, provider).await;
    Some(persisted)
}

/// Record that this fetch happened, independent of how many rows it returned.
///
/// The fetch itself is the freshness signal: a provider that answered with
/// zero activities is exactly as current as one that answered with ten, and
/// the upserted rows cannot say so. Best-effort — a failed mark costs one
/// deferred freshness read, never the fetch.
async fn stamp_fetch_freshness(
    cache: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    if let Err(e) = cache
        .record_activity_fetch(user_id, &tenant_id, provider, Utc::now())
        .await
    {
        info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: fetch freshness mark failed");
    }
}

/// Order a fetched activity list in place by the requested key.
///
/// Applied BEFORE the display limit so a "longest to shortest" ask keeps the
/// longest activities rather than the most recent. Recognized keys:
/// `date_desc` (default, newest first), `date_asc`, `distance_desc`,
/// `distance_asc`, `duration_desc`, `duration_asc`. An unknown value falls back
/// to `date_desc`. A missing distance sorts as 0 m, so an activity with no
/// recorded distance lands last on a `distance_desc` sort. `pub` so the ordering
/// is exercisable by the integration test suite.
pub fn sort_activities(activities: &mut [Activity], sort_by: &str) {
    // f64 distances have no total order (NaN), so the distance arms keep
    // `sort_by` with `partial_cmp`; the Ord-keyed arms (date, duration) use
    // `sort_by_key` to satisfy clippy::unnecessary_sort_by.
    let distance = |a: &Activity| a.distance_meters().unwrap_or(0.0);
    match sort_by {
        "date_asc" => activities.sort_by_key(Activity::start_date),
        "distance_desc" => activities.sort_by(|a, b| {
            distance(b)
                .partial_cmp(&distance(a))
                .unwrap_or(Ordering::Equal)
        }),
        "distance_asc" => activities.sort_by(|a, b| {
            distance(a)
                .partial_cmp(&distance(b))
                .unwrap_or(Ordering::Equal)
        }),
        "duration_desc" => activities.sort_by_key(|a| Reverse(a.duration_seconds())),
        "duration_asc" => activities.sort_by_key(Activity::duration_seconds),
        // "date_desc" and any unrecognized value: newest first (the historical
        // default that cached responses also produce).
        _ => activities.sort_by_key(|a| Reverse(a.start_date())),
    }
}

/// Oldest/newest activity date (`"YYYY-MM-DD"`) across `activities`, or `None`
/// when the slice is empty.
///
/// Captured over the FULL post-filter set BEFORE the display-limit truncation so
/// the response can frame the served window's true span — otherwise the LLM
/// anchors on the oldest activity in the truncated slice (e.g. "depuis le 21
/// août") instead of the window's real start.
#[must_use]
pub fn activity_date_span(activities: &[Activity]) -> Option<(String, String)> {
    let oldest = activities.iter().map(Activity::start_date).min()?;
    let newest = activities.iter().map(Activity::start_date).max()?;
    Some((
        oldest.format("%Y-%m-%d").to_string(),
        newest.format("%Y-%m-%d").to_string(),
    ))
}
