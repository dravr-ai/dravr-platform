// ABOUTME: A covered historical window as get_activities serves it: covered under any history key, its head slice, a stale head topped up
// ABOUTME: The top-up is a budgeted live read, so a slow scrape answers from the cache and lands for the next turn
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::models::refresh::DataFreshness;
use pierre_core::models::{Activity, RefreshConfig, TenantId};
use pierre_database::repositories::{historical_depth_covered, BackfillJobKind, BackfillJobState};
use pierre_providers::core::ActivityQueryParams;
use pierre_services::training_history_read::history_store_keys;
use tracing::{info, warn};
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
        walk_running: false,
    }
}

/// Serve the historical `window` of `provider_slug` from the store when the
/// store holds it to its depth, or `None` when a read must fetch it.
///
/// `window` is the coverage read: the asked window, its open `before`
/// bounded a year above `after` so recent rows cannot mask missing depth.
/// It is covered when a walk or a backfill under ANY of the provider's
/// history keys reached `after` at its key's current capture
/// ([`history_depth_covered`]), and only a covered window is read, once,
/// every history key's rows merged ([`read_cached_window`]): a Strava athlete walked through the session
/// holds the depth under `sciotte`, one walked through the API under
/// `strava`, and both are read whichever backend the ask resolved to
/// (carnet#1482). A covered window that holds no rows is not served, so an
/// empty answer is never taken from the store alone. The served list is
/// [`serve_historical_window`]'s, `before` the athlete's own bound.
///
/// A window the athlete's history walk has not reached yet, while that walk
/// runs under one of those keys, is served from what the store already holds
/// and says so ([`HistoricalServe::walk_running`]): the walk is filling it,
/// so a read of its own would page the same months twice.
pub async fn serve_covered_history(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    window: &ActivityQueryParams,
    before: Option<i64>,
) -> Option<HistoricalServe> {
    let after_ts = window.after.unwrap_or(0);
    let depth_covered =
        history_depth_covered(runtime, provider_slug, user_id, tenant_id, after_ts).await;
    let walk_running =
        !depth_covered && history_walk_owed(runtime, provider_slug, user_id, tenant_id).await;
    // A window no record vouches for, and no walk is filling, is fetched
    // whatever the store holds, so it is not read.
    let cached = if depth_covered || walk_running {
        read_cached_window(runtime, provider_slug, user_id, tenant_id, window).await
    } else {
        None
    };
    info!(
        user_id = %user_id,
        provider = %provider_slug,
        after = ?window.after,
        ?before,
        covered = cached.is_some(),
        depth_covered,
        walk_running,
        window_len = cached.as_ref().map_or(0, Vec::len),
        "get_activities historical gate decision"
    );
    let served = cached?;
    let serve = serve_historical_window(
        runtime,
        provider_slug,
        user_id,
        tenant_id,
        before,
        window.before,
        served,
    )
    .await;
    Some(HistoricalServe {
        walk_running,
        ..serve
    })
}

/// Whether a history walk under any of `provider_slug`'s history keys is
/// owed: recorded, running or waiting for the resume sweep. A ledger that
/// cannot be read counts as no walk, so the window is read as before.
async fn history_walk_owed(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> bool {
    let jobs = &runtime.repos().activity_backfill_jobs;
    for key in history_store_keys(provider_slug) {
        match jobs
            .backfill_job_state(user_id, tenant_id, key, BackfillJobKind::HistoryWalk)
            .await
        {
            Ok(Some(BackfillJobState::Owed)) => return true,
            Ok(_) => {}
            Err(e) => warn!(
                user_id = %user_id,
                provider = %key,
                error = %e,
                "history walk state read failed; the key counts as not walking"
            ),
        }
    }
    false
}

/// Whether a walk or a backfill under any of `provider_slug`'s history keys
/// ([`history_store_keys`]) reached `after_ts`, each record
/// judged at its own key's current capture ([`historical_depth_covered`]).
/// A record that cannot be read counts as absent.
async fn history_depth_covered(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    after_ts: i64,
) -> bool {
    let cache = &runtime.repos().activity_cache;
    for key in history_store_keys(provider_slug) {
        let coverage = match cache.get_backfill_coverage(user_id, &tenant_id, key).await {
            Ok(coverage) => coverage,
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    provider = %key,
                    error = %e,
                    "backfill coverage read failed; the key counts as not covered"
                );
                None
            }
        };
        if historical_depth_covered(coverage, after_ts, current_capture_version(key)) {
            return true;
        }
    }
    false
}

/// A covered historical window as [`serve_historical_window`] serves it.
#[derive(Debug)]
pub struct HistoricalServe {
    /// The window, head slice and top-up included.
    pub activities: Vec<Activity>,
    /// Why the head is stale and was not refreshed this turn, so the newest
    /// sessions may be missing; `None` when it is current.
    pub behind: Option<StaleReason>,
    /// Whether the window was served before the athlete's history walk
    /// reached it, so older activities of it are still to land.
    pub walk_running: bool,
}
