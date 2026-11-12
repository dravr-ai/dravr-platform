// ABOUTME: A covered historical window as get_activities serves it: the cached window, its head slice, a stale head topped up
// ABOUTME: The top-up is a budgeted live read, so a slow scrape answers from the cache and lands for the next turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use pierre_core::models::refresh::DataFreshness;
use pierre_core::models::{Activity, RefreshConfig, TenantId};
use pierre_providers::core::ActivityQueryParams;
use tracing::info;
use uuid::Uuid;

use super::budgeted_read::refresh_head_within_budget;
use super::pushed::reads_pushed_store;
use super::sync_verdict::sync_backoff_until;
use super::{
    before_bounds_a_closed_window, historical_head_slice, merge_live_head, read_cached_window,
    StaleReason, HISTORICAL_WINDOW_READ_LIMIT,
};
use crate::runtime::ToolRuntime;

/// Top up a cache-served window with a live read when the head has gone stale.
///
/// The covered-historical path serves the durable cache with no freshness test:
/// `read_cached_window` consults neither `synced_at` nor `activity_fetch_freshness`,
/// and `historical_depth_covered` asks only whether the window is DEEP enough,
/// never whether it is CURRENT. An agent whose window declares sixteen weeks takes
/// that path on every single turn, so its grounding block was as old as whatever
/// last happened to write through — while the block itself instructs the model to
/// "base your analysis on these specific activities" and not to answer from memory.
///
/// The freshness mark was already written and already read, but only by the
/// `get_data_freshness` REPORTING tool. Nothing acted on it. Now the same
/// [`DataFreshness`] bands the agent is TOLD about also decide whether to look
/// again, so the report and the behaviour cannot disagree.
///
/// Bounded windows are exempt: a closed `before` names a period that is over, so
/// there is no live head it could be missing, and refreshing one would re-scrape
/// history that cannot have changed.
///
/// A head whose last refreshes failed is read again only once
/// [`sync_backoff_until`](super::sync_verdict::sync_backoff_until) allows it: a provider that keeps failing is not
/// scraped on every turn and every Home load.
///
/// Best-effort. A failed refresh leaves the cached window exactly as it was —
/// slightly old data beats no data, which is the same posture
/// [`super::fetch_provider_activities`] already takes on a provider blip. The read
/// gets the refresh budget ([`refresh_head_within_budget`]). Returns why the
/// head served is behind — stale and not refreshed this turn — so the caller
/// can say the newest sessions may be missing, and why, instead of serving the
/// window as current; `None` when it is current.
pub async fn refresh_stale_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    before: Option<i64>,
    served: &mut Vec<Activity>,
) -> Option<StaleReason> {
    if before_bounds_a_closed_window(before, Utc::now().timestamp()) {
        return None;
    }
    // A push-only provider has no live head to top up from: the window
    // served is everything it pushed, and its freshness is the last push.
    if reads_pushed_store(runtime, provider_slug, user_id, Some(tenant_id)).await {
        return None;
    }

    let last_sync = runtime
        .repos()
        .activity_cache
        .latest_activity_sync(user_id, &tenant_id, provider_slug)
        .await
        .unwrap_or(None);

    let freshness = DataFreshness::from_last_sync(last_sync);
    if matches!(freshness, DataFreshness::Fresh | DataFreshness::Recent) {
        return None;
    }
    let failure = runtime
        .repos()
        .activity_cache
        .latest_activity_fetch_failure(user_id, &tenant_id, provider_slug)
        .await
        .unwrap_or(None);
    if let Some(retry_at) = sync_backoff_until(failure, last_sync, Utc::now()) {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            %retry_at,
            "activity head is stale, but its last refreshes failed; backing off before reading it again"
        );
        return Some(StaleReason::Backoff);
    }

    info!(
        user_id = %user_id,
        provider = %provider_slug,
        ?last_sync,
        freshness = freshness.label(),
        "activity head is stale; re-reading the recent window before grounding"
    );
    let budget = RefreshConfig::default().wait_for_refresh_timeout();
    let Some(live) =
        refresh_head_within_budget(runtime, provider_slug, user_id, tenant_id, budget).await
    else {
        return Some(StaleReason::Timeout);
    };

    merge_live_head(served, live);
    None
}

/// Assemble the served list for a covered historical window.
///
/// Appends the disjoint open-`before` head slice above the coverage clip (see
/// [`historical_head_slice`] — disjoint windows can't reintroduce the
/// probe/serve divergence the single bounded read prevents; boundary rows can
/// land in both inclusive reads, so the id filter keeps the union exact), then
/// tops up a stale head from the provider via [`refresh_stale_head`]. Shared
/// by the coverage-gated serve and the inline-backfill (task handle) serve so
/// the two cannot drift.
pub async fn serve_historical_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    before: Option<i64>,
    coverage_before: Option<i64>,
    mut served: Vec<Activity>,
) -> HistoricalServe {
    if let Some(head_after) = historical_head_slice(before, coverage_before, Utc::now().timestamp())
    {
        let head_params = ActivityQueryParams {
            after: Some(head_after),
            before: None,
            limit: Some(HISTORICAL_WINDOW_READ_LIMIT),
            offset: None,
        };
        if let Some(head) =
            read_cached_window(runtime, provider_slug, user_id, tenant_id, &head_params).await
        {
            let seen: HashSet<String> = served.iter().map(|a| a.id().to_owned()).collect();
            served.extend(head.into_iter().filter(|a| !seen.contains(a.id())));
        }
    }
    // The covered gate proves the window is deep enough; it says nothing about
    // whether it is current. Top up the head when the provider may be holding
    // something we have never seen.
    let behind = refresh_stale_head(
        runtime,
        provider_slug,
        user_id,
        tenant_id,
        before,
        &mut served,
    )
    .await;
    HistoricalServe {
        activities: served,
        behind,
    }
}

/// A covered historical window as [`serve_historical_window`] serves it.
#[derive(Debug)]
pub struct HistoricalServe {
    /// The window, head slice and top-up included.
    pub activities: Vec<Activity>,
    /// Why the head is stale and was not refreshed this turn, so the newest
    /// sessions may be missing; `None` when it is current.
    pub behind: Option<StaleReason>,
}
