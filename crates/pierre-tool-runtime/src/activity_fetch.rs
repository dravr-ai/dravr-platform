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

use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::ActivityQueryParams;
use tracing::warn;
use uuid::Uuid;

use crate::group_fitness::write_through_activity_cache;
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// Cache fallback window when the request carries no `after` lower bound.
const STALE_FALLBACK_WINDOW_DAYS: i64 = 90;

/// Cap on stale rows served from cache when a live fetch fails.
const STALE_FALLBACK_LIMIT: i64 = 500;

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
    let tenant = tenant_id.parse::<TenantId>().ok();

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
            )
            .await;
        }
        return Some(activities);
    }

    // Live fetch failed — serve the user's cached activities for this window
    // rather than returning nothing.
    serve_stale_activities(runtime, provider_slug, user_id, tenant?, params).await
}

/// Read a provider's cached activities for the request window, newest first.
///
/// Returns `None` when the cache is empty or the read fails — the caller then
/// treats the provider as "no activities". Only invoked after a live fetch
/// failure, so any non-empty result is strictly better than the empty fallback.
async fn serve_stale_activities(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    let end = Utc::now();
    let start = params
        .after
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .unwrap_or_else(|| end - Duration::days(STALE_FALLBACK_WINDOW_DAYS));
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
        Ok(cached) if !cached.is_empty() => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                count = cached.len(),
                "fetch_provider_activities: live fetch failed, serving stale cached activities"
            );
            Some(cached)
        }
        Ok(_) => None,
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                "fetch_provider_activities: stale cache read failed"
            );
            None
        }
    }
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
