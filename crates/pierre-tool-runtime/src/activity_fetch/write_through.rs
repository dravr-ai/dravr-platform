// ABOUTME: Writes a provider's live activity read through to the durable activity cache
// ABOUTME: Only a read of the list head is a sync: it stamps now and moves freshness; any other read keeps the head's age
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity cache's write-through.
//!
//! Every writer lands its rows through here so upsert, prune, coverage and
//! the fetch-freshness mark cannot disagree between a chat turn, the capture
//! sweep, a backfill and a group snapshot. What a write says about the
//! provider's freshness is decided by the read it came from
//! ([`covers_list_head`](crate::activity_fetch::sync_verdict::covers_list_head)).

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::ActivityCacheRepository;
use pierre_providers::core::ActivityQueryParams;
use tracing::{info, warn};
use uuid::Uuid;

#[cfg(feature = "client-notifications")]
use super::session_landing::{deliver_session_digest, lands_a_session};
use super::sync_verdict::covers_list_head;
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// How a write-through lands: the retention its prune keeps, and the read
/// the rows came from, which decides whether they count as a sync.
pub struct WriteThrough<'a> {
    /// The per-transaction prune window: after the upsert, rows older than
    /// `now - retention_days` are garbage-collected.
    pub retention_days: i64,
    /// The window the provider was read for.
    pub read: &'a ActivityQueryParams,
}

/// Warm the provider-agnostic activity cache after a successful live fetch so
/// the next outage serves these rows stale-while-revalidate. Shared with the
/// group snapshot builder through the `activity_cache` repo.
///
/// `retention_days` is the per-transaction prune window: after the upsert,
/// rows older than `now - retention_days` are garbage-collected. Recent-fetch
/// callers pass [`activity_cache_retention_days`](crate::activity_fetch::activity_cache_retention_days) (the deployment default);
/// a historical backfill passes a deeper window so the season it just wrote is
/// not immediately pruned. Pruning is keyed by `(user_id, tenant_id)` across
/// all providers, so the retention floor for durable history is whatever the
/// *widest-window* writer uses.
///
/// Only a read of the provider's list head ([`covers_list_head`]) is a sync:
/// its rows are stamped now and the fetch-freshness mark moves. A read that
/// stopped short of the head — a backfill of a closed season, a later page —
/// lands its rows at the head's own last sync ([`head_sync_stamp`]), so
/// writing 2023 cannot make today's head read as current, supersede a
/// failure Home is reporting, or stand the stale-head refresh down.
///
/// Returns the count of net distinct rows persisted (deduped by `activity_id`),
/// or `None` when the upsert itself failed. The historical backfill surfaces
/// this honest figure in its completion notice; recent-fetch callers ignore it.
///
/// A write that lands a new training session delivers the athlete's
/// `per_session` persona digest (see [`lands_a_session`]).
pub(crate) async fn write_through_activity_cache(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activities: &[Activity],
    write: WriteThrough<'_>,
) -> Option<u64> {
    let data = auth_service.runtime().data();
    let cache = data.repos().activity_cache.clone();
    #[cfg(feature = "client-notifications")]
    let landed = lands_a_session(
        auth_service.runtime(),
        user_id,
        &tenant_id,
        provider,
        activities,
    )
    .await;
    let stamp = head_sync_stamp(
        cache.as_ref(),
        user_id,
        tenant_id,
        provider,
        write.read,
        activities,
    )
    .await;
    let persisted = match cache
        .upsert_activities_synced_at(user_id, &tenant_id, provider, activities, stamp.synced_at)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: write-through failed");
            return None;
        }
    };
    let cutoff = Utc::now() - Duration::days(write.retention_days);
    prune_and_realign_coverage(cache.as_ref(), user_id, tenant_id, provider, cutoff).await;
    if stamp.is_sync {
        stamp_fetch_freshness(cache.as_ref(), user_id, tenant_id, provider).await;
    }
    #[cfg(feature = "client-notifications")]
    if landed {
        deliver_session_digest(auth_service.runtime(), user_id, tenant_id);
    }
    Some(persisted)
}

/// The `synced_at` a write-through's rows carry, and whether the write is a
/// sync of the provider's list head.
struct HeadSyncStamp {
    synced_at: DateTime<Utc>,
    is_sync: bool,
}

/// Decide what a write of `activities`, read for `read`, says about the
/// provider's freshness.
///
/// A read of the list head is a sync: its rows are stamped now. Any other
/// read writes its rows at the head's own last sync, which leaves
/// [`ActivityCacheRepository::latest_activity_sync`] where it was. A
/// provider never synced before has no such time; its rows then carry the
/// newest instant the read vouches for — the window's closing `before`, else
/// its newest activity — which is in the past, so the head still reads stale.
async fn head_sync_stamp(
    cache: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    read: &ActivityQueryParams,
    activities: &[Activity],
) -> HeadSyncStamp {
    let now = Utc::now();
    if covers_list_head(read, now.timestamp()) {
        return HeadSyncStamp {
            synced_at: now,
            is_sync: true,
        };
    }
    let head = cache
        .latest_activity_sync(user_id, &tenant_id, provider)
        .await
        .unwrap_or(None);
    let vouched = read
        .before
        .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
        .or_else(|| activities.iter().map(Activity::start_date).max());
    HeadSyncStamp {
        synced_at: head.or(vouched).unwrap_or(now).min(now),
        is_sync: false,
    }
}

/// Prune below `cutoff`, then raise any coverage floor the prune just falsified.
///
/// The two writes belong together: a prune that removed rows has deleted
/// exactly what a deeper coverage record vouches for, and a claim left standing
/// makes the historical gate serve the now-shallow cache as a complete window
/// without calling a provider. Best-effort throughout — retention is not worth
/// failing a fetch over, and a clamp that fails costs one stale claim, which
/// the next prune retries.
async fn prune_and_realign_coverage(
    cache: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    cutoff: DateTime<Utc>,
) {
    match cache
        .prune_activities_before(user_id, &tenant_id, cutoff)
        .await
    {
        Ok(0) => {}
        Ok(pruned) => {
            realign_coverage_floor(cache, user_id, tenant_id, provider, cutoff, pruned).await;
        }
        Err(e) => {
            info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: prune failed");
        }
    }
}

/// Raise coverage floors the prune just falsified. Best-effort: a failed clamp
/// costs one stale claim, which the next prune retries.
async fn realign_coverage_floor(
    cache: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    cutoff: DateTime<Utc>,
    pruned: u64,
) {
    match cache
        .clamp_backfill_coverage(user_id, &tenant_id, cutoff)
        .await
    {
        Ok(0) => {}
        Ok(clamped) => info!(
            user_id = %user_id,
            provider = %provider,
            pruned,
            clamped,
            "Activity cache: prune raised backfill coverage floors"
        ),
        Err(e) => {
            info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: coverage clamp failed");
        }
    }
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

/// Persist a window `get_activities` just served.
///
/// So the stale-while-revalidate path and later turns can answer it without
/// re-fetching. Best-effort: a cache failure never blocks the response. An empty
/// window writes nothing but the freshness mark of a head read.
///
/// The caller decides provenance, and only rows a PROVIDER produced, in an
/// answer the provider vouched for ([`HeadVerdict::Complete`](crate::activity_fetch::sync_verdict::HeadVerdict::Complete)), belong here.
/// Two kinds never do:
///
/// * Rows a sibling connection produced while the elected provider was auth-dead.
///   They belong to the providers that produced them — each already wrote its own
///   through — and filing them under the dead provider's key would have a
///   reconnect restore history it never recorded.
/// * Rows the historical branch read out of this very table. Writing them back
///   changes no data; it only moves their `synced_at` to now.
///   [`ActivityCacheRepository::latest_activity_sync`] takes the max of that
///   column, [`DataFreshness`](pierre_core::models::refresh::DataFreshness) reads the result as `Fresh`, and
///   [`refresh_stale_head`](crate::activity_fetch::refresh_stale_head) returns early on `Fresh` — so the one path that would
///   top the head up is disarmed by the act of serving the stale window, and every
///   later ask re-arms the disarming. A capture that stops then reports itself
///   current forever: jf@dravr.ai's sciotte capture froze at 2026-08-28 02:59Z and
///   two days later all 109 cached rows carried one identical `synced_at`, while
///   `activity_fetch_freshness` still held the last real fetch, five days older.
///   A stale-head top-up that does reach the provider is written through by
///   [`fetch_provider_activities`](crate::activity_fetch::fetch_provider_activities), which is what should move freshness.
///
/// Like every write-through, only a read of the list head (`read`,
/// [`covers_list_head`]) is a sync: its rows are stamped now and the
/// fetch-freshness mark moves, so an honest empty week counts too. A closed
/// window ("last week") or a later page lands its rows at the head's own
/// last sync.
///
/// `pub` like its siblings here: the only caller is `implementations::data`, which is
/// behind the `tools-data` feature, so a narrower visibility reads as dead code in a
/// `--no-default-features` build.
pub async fn write_through_served_window(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider_slug: &str,
    read: &ActivityQueryParams,
    activities: &[Activity],
) {
    let cache = &runtime.repos().activity_cache;
    let stamp = head_sync_stamp(
        cache.as_ref(),
        user_id,
        *tenant_id,
        provider_slug,
        read,
        activities,
    )
    .await;
    if activities.is_empty() {
        if stamp.is_sync {
            stamp_fetch_freshness(cache.as_ref(), user_id, *tenant_id, provider_slug).await;
        }
        return;
    }
    #[cfg(feature = "client-notifications")]
    let landed = lands_a_session(runtime, user_id, tenant_id, provider_slug, activities).await;
    let upserted = cache
        .upsert_activities_synced_at(
            user_id,
            tenant_id,
            provider_slug,
            activities,
            stamp.synced_at,
        )
        .await;
    if let Err(e) = &upserted {
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            error = %e,
            "Activity cache: write-through from get_activities failed"
        );
    } else if stamp.is_sync {
        stamp_fetch_freshness(cache.as_ref(), user_id, *tenant_id, provider_slug).await;
    }
    #[cfg(feature = "client-notifications")]
    if landed && upserted.is_ok() {
        deliver_session_digest(runtime, user_id, *tenant_id);
    }
}
