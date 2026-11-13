// ABOUTME: One cached activity's detail read — the splits and laps a provider serves only on a detail read, stored beside the cached row
// ABOUTME: Read for its view, detached and bounded, once for good or again after a recheck when it found none; a route's streams read stores it too

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity detail reads for the activity view.
//!
//! A provider serves an activity's splits and laps only on a detail read,
//! while every list sync rewrites the cached copy from a list read that
//! carries neither. What a detail read found is therefore stored beside the
//! cached row (`ActivityCacheRepository::store_activity_detail`), where no
//! later list sync reaches it, and every read of the activity fills the
//! fields its list copy lacks from it.
//!
//! Two reads store it. A route drawn from the activity's streams is one
//! (`crate::services::activity_route`). The activity view is the other: a
//! route drawn from the activity's own overview (Strava's
//! `summary_polyline`) spends no provider call at all, so for most outdoor
//! activities the view is the only thing that ever asks for the detail. A
//! stored read that carried splits or laps settles it, and the view never
//! asks again. One that carried neither settles nothing: an activity with no
//! laps reads that way, and so does one whose provider served it without them
//! because the request carrying them failed — a provider that reads its laps
//! on a request of their own serves the activity without them when that
//! request fails, since a swim has no laps to serve. It is stored
//! with a recheck instant
//! ([`pierre_tool_runtime::activity_detail_store::EMPTY_DETAIL_RECHECK_MINUTES`]) out: until then
//! the view answers from it without a provider read, and past it the next
//! open reads the detail again.
//!
//! The read takes the athlete's provider turn, the one the route reads take
//! (`crate::services::activity_route::take_turn`), so it never runs beside
//! another read on the same scraper session, and once the turn is its own it
//! looks at the cache again: a read of the same activity that went first —
//! the view opened twice, or read again when its thread was linked — has
//! stored the splits and laps, and the provider is not asked twice.
//!
//! The read runs detached on the server's drain tracker, bounded by
//! [`DETAIL_PROVIDER_READ_TIMEOUT_SECS`]; the view waits for it only within
//! [`DETAIL_ANSWER_TIMEOUT_SECS`] and past that shows the activity without
//! its splits and laps, while the read goes on and stores them for the next
//! open. A read that fails or times out stores nothing, so the next open asks
//! again.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_tool_runtime::activity_detail_store;
use pierre_tool_runtime::protocol::provider_helpers::configured_provider;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::services::activity_route::{take_turn, CachedActivityRef, TurnEntry};
use crate::services::turn_lifecycle::InFlightTurns;

/// Seconds the activity view waits for a detail read it started.
///
/// An API provider answers a detail read in well under a second; a mirror
/// backend scrapes the page and takes seconds. Past this the view is answered
/// from what the cache holds, and the read stores its detail for the next
/// open.
pub const DETAIL_ANSWER_TIMEOUT_SECS: u64 = 8;

/// Seconds one detail read may run before it is given up, storing nothing.
///
/// Past the scraper's own request deadline (320 s), as the route read's
/// bound is, so a scrape is answered by the scraper and not cut off here.
pub const DETAIL_PROVIDER_READ_TIMEOUT_SECS: u64 = 330;

/// Read the activity's detail from its provider, store it beside the cached
/// row, and answer whether it was stored within [`DETAIL_ANSWER_TIMEOUT_SECS`].
///
/// The read is spawned on `turns`, so a view that stops waiting leaves it
/// running to store its detail. `false` is a read that failed, timed out, has
/// not finished yet, or whose row left the cache meanwhile: the view answers
/// from the cache as it stands.
pub async fn read_activity_detail(
    runtime: &Arc<dyn ToolRuntime>,
    turns: &InFlightTurns,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> bool {
    let (answer, answered) = oneshot::channel();
    let runtime = Arc::clone(runtime);
    let provider = cached.provider.to_owned();
    let activity = cached.activity.clone();
    turns.spawn(async move {
        let cached = CachedActivityRef {
            provider: &provider,
            activity: &activity,
        };
        // Held until the detail is stored; each holder's read is bounded, so
        // the wait for it is too.
        let turn = take_turn(user_id, tenant_id, activity.start_date(), TurnEntry::Alone).await;
        let stored = if detail_held(runtime.repos(), tenant_id, user_id, cached).await {
            true
        } else {
            let bound = StdDuration::from_secs(DETAIL_PROVIDER_READ_TIMEOUT_SECS);
            timeout(bound, read_and_store(&runtime, tenant_id, user_id, cached))
                .await
                .unwrap_or_else(|_| {
                    warn!(
                        activity_id = activity.id(),
                        provider,
                        timeout_secs = bound.as_secs(),
                        "activity detail read did not finish in time; nothing stored"
                    );
                    false
                })
        };
        drop(turn);
        if answer.send(stored).is_err() {
            debug!(
                activity_id = activity.id(),
                "activity detail read finished after its view answered"
            );
        }
    });
    let wait = StdDuration::from_secs(DETAIL_ANSWER_TIMEOUT_SECS);
    // A read task that ended without answering has panicked, and stored
    // nothing.
    matches!(timeout(wait, answered).await, Ok(Ok(true)))
}

/// Whether the cached copy already carries splits or laps: a read of it that
/// held the turn first stored them.
async fn detail_held(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> bool {
    match repos
        .activity_cache
        .get_cached_activity(user_id, &tenant_id, cached.provider, cached.activity.id())
        .await
    {
        Ok(Some(activity)) => activity.splits().is_some() || activity.laps().is_some(),
        Ok(None) => false,
        Err(e) => {
            warn!(
                activity_id = cached.activity.id(),
                provider = cached.provider,
                error = %e,
                "reading the cached copy before its detail read failed; reading the provider"
            );
            false
        }
    }
}

/// One detail read of the activity, stored when it succeeds.
async fn read_and_store(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
) -> bool {
    let tenant = tenant_id.to_string();
    let read =
        match configured_provider(runtime, user_id, cached.provider, Some(tenant.as_str())).await {
            Ok(provider) => provider.get_activity_detailed(cached.activity.id()).await,
            Err(e) => Err(e),
        };
    match read {
        Ok(detailed) => {
            activity_detail_store::store_detail(
                runtime.repos(),
                runtime.live_activities().as_ref(),
                tenant_id,
                user_id,
                cached.provider,
                &detailed,
            )
            .await
        }
        Err(e) => {
            warn!(
                activity_id = cached.activity.id(),
                provider = cached.provider,
                error = %e,
                "activity detail read failed; nothing stored"
            );
            false
        }
    }
}
