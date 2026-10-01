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
//! because the request carrying them failed — the Garmin API folds `/laps`
//! and `/splits` into the detail read and serves the activity without either
//! when its request fails, since a swim has no laps to serve. It is stored
//! with a recheck instant [`EMPTY_DETAIL_RECHECK_MINUTES`] out: until then
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

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::ActivityDetail;
use pierre_database::RepositoryRegistry;
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

/// Minutes a stored detail read that carried neither splits nor laps answers
/// the activity view before the detail is read again.
///
/// Such a read proves nothing — the request carrying the laps may have
/// failed — so it is not kept for good. Long enough that an activity that
/// truly has none, opened again and again, costs one provider read per half
/// hour rather than one per open; short enough that a failure the provider
/// has since recovered from does not hide the splits for the day.
pub const EMPTY_DETAIL_RECHECK_MINUTES: i64 = 30;

/// When a stored detail is read again: [`EMPTY_DETAIL_RECHECK_MINUTES`] out
/// for one that carried neither splits nor laps, never for one that carried
/// either.
#[must_use]
pub fn detail_recheck_at(detail: &ActivityDetail, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    detail
        .is_empty()
        .then(|| now + Duration::minutes(EMPTY_DETAIL_RECHECK_MINUTES))
}

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
        Ok(detailed) => store_detail(runtime.repos(), tenant_id, user_id, cached, &detailed).await,
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

/// Keep the splits and laps a detail read carried beside the activity's
/// cached copy, which no list read carries and every list sync replaces.
///
/// The activity's view shows them after any later sync. A read that carried
/// neither is stored too, until [`detail_recheck_at`]: it answers the view
/// until then, and the detail is read again past it.
///
/// A detail that cannot be stored is logged, and answers `false`: the caller
/// read it for something else, or answers from the cache as it stands.
pub async fn store_detail(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    cached: CachedActivityRef<'_>,
    detailed: &Activity,
) -> bool {
    let detail = ActivityDetail::from_activity(detailed);
    let recheck_at = detail_recheck_at(&detail, Utc::now());
    let activity_id = cached.activity.id();
    match repos
        .activity_cache
        .store_activity_detail(
            user_id,
            &tenant_id,
            cached.provider,
            activity_id,
            &detail,
            recheck_at,
        )
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            debug!(
                activity_id,
                provider = cached.provider,
                "activity left the cache during its detail read; its splits and laps are not kept"
            );
            false
        }
        Err(e) => {
            warn!(
                activity_id,
                provider = cached.provider,
                error = %e,
                "storing the splits and laps of a detail read failed"
            );
            false
        }
    }
}
