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

use dravr_tronc::dates::Timestamp;
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::ActivityCacheRepository;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::live_activities::{LiveIngest, WriteOrigin};
use tracing::{info, warn};
use uuid::Uuid;

use super::session_landing::announce;
use super::sync_verdict::covers_list_head;
use crate::protocol::auth::AuthService;
use crate::runtime::ToolRuntime;

/// How a write-through lands: the read the rows came from, which decides
/// whether they count as a sync.
#[derive(Debug)]
pub struct WriteThrough<'a> {
    /// The window the provider was read for.
    pub read: &'a ActivityQueryParams,
    /// Whether the read is of the provider's present, so a workout it lands
    /// is announced ([`WriteOrigin::Live`]): the coach's reaction or the
    /// athlete's `per_session` digest (carnet#1486). `false` for a history
    /// walk, which copies the past and announces nothing (carnet#1482).
    pub announces_sessions: bool,
}

/// Warm the provider-agnostic activity cache after a successful live fetch so
/// the next outage serves these rows stale-while-revalidate. Shared with the
/// group snapshot builder through the `activity_cache` repo.
///
/// The rows are kept in the activity store, whose one retention policy
/// decides when they go (carnet#1482); this write deletes nothing.
///
/// Only a read of the provider's list head ([`covers_list_head`]) is a sync:
/// its rows are stamped now and the fetch-freshness mark moves. A read that
/// stopped short of the head — a backfill of a closed season, a later page —
/// lands its rows at the head's own last sync ([`head_sync_stamp`]), so
/// writing 2023 cannot make today's head read as current, supersede a
/// failure Home is reporting, or stand the stale-head refresh down.
///
/// Returns what the store did ([`LiveIngest`]): the net distinct rows held
/// after the write (deduped by `activity_id`) and those it inserted, or
/// `None` when the upsert itself failed. The historical backfill surfaces
/// the held count in its completion notice, and a history walk's step the
/// inserted one before it takes rows back; recent-fetch callers ignore it.
///
/// A write that lands a new training session announces it to the runtime's
/// landing sink (see [`announce`]).
pub(crate) async fn write_through_activity_cache(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activities: &[Activity],
    write: WriteThrough<'_>,
) -> Option<LiveIngest> {
    let stamp = write_through_stamp(
        auth_service,
        user_id,
        tenant_id,
        provider,
        write.read,
        activities,
    )
    .await?;
    write_through_at(
        auth_service,
        user_id,
        tenant_id,
        provider,
        activities,
        write,
        stamp,
    )
    .await
}

/// The stamp a write-through of `activities`, read for `read`, lands its
/// rows at ([`head_sync_stamp`]); `None` when it cannot be decided and
/// nothing may be written. A writer that must record when its rows were
/// written before it writes them decides it first, then writes at it
/// ([`write_through_at`]).
pub(crate) async fn write_through_stamp(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    read: &ActivityQueryParams,
    activities: &[Activity],
) -> Option<HeadSyncStamp> {
    let cache = auth_service.runtime().data().repos().activity_cache.clone();
    head_sync_stamp(
        cache.as_ref(),
        user_id,
        tenant_id,
        provider,
        read,
        activities,
    )
    .await
}

/// [`write_through_activity_cache`] at a `stamp` decided beforehand
/// ([`write_through_stamp`]).
pub(crate) async fn write_through_at(
    auth_service: &AuthService,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    activities: &[Activity],
    write: WriteThrough<'_>,
    stamp: HeadSyncStamp,
) -> Option<LiveIngest> {
    let data = auth_service.runtime().data();
    let cache = data.repos().activity_cache.clone();
    // Kept in the activity store, which projects into the cache; its one
    // retention policy, not this write, decides when a row goes (carnet#1482).
    let persisted = match auth_service
        .runtime()
        .live_activities()
        .ingest(
            user_id,
            tenant_id,
            provider,
            activities,
            stamp.synced_at,
            if write.announces_sessions {
                WriteOrigin::Live
            } else {
                WriteOrigin::Walk
            },
        )
        .await
    {
        Ok(ingest) => ingest,
        Err(e) => {
            info!(user_id = %user_id, provider = %provider, error = %e, "Activity cache: write-through failed");
            return None;
        }
    };
    if stamp.is_sync {
        stamp_fetch_freshness(cache.as_ref(), user_id, tenant_id, provider).await;
    }
    announce(
        auth_service.runtime(),
        user_id,
        tenant_id,
        persisted.landed.clone(),
    );
    Some(persisted)
}

/// The `synced_at` a write-through's rows carry, and whether the write is a
/// sync of the provider's list head.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeadSyncStamp {
    synced_at: Timestamp,
    is_sync: bool,
}

impl HeadSyncStamp {
    /// The `synced_at` the write's rows carry.
    #[cfg(feature = "tools-data")]
    pub(crate) const fn synced_at(&self) -> Timestamp {
        self.synced_at
    }
}

/// Decide what a write of `activities`, read for `read`, says about the
/// provider's freshness; `None` when it cannot be decided and nothing may be
/// written (see [`stamp_after_head_lookup`]).
///
/// A read of the list head is a sync: its rows are stamped now. Any other
/// read writes its rows at the head's own last sync, which leaves
/// [`ActivityCacheRepository::latest_activity_sync`] where it was.
async fn head_sync_stamp(
    cache: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    read: &ActivityQueryParams,
    activities: &[Activity],
) -> Option<HeadSyncStamp> {
    let now = Timestamp::now();
    if covers_list_head(read, now.epoch_seconds()) {
        return Some(HeadSyncStamp {
            synced_at: now,
            is_sync: true,
        });
    }
    let head = cache
        .latest_activity_sync(user_id, &tenant_id, provider)
        .await;
    if let Err(e) = &head {
        warn!(
            user_id = %user_id,
            provider = %provider,
            error = %e,
            "Activity cache: head sync time unreadable, a non-head read writes nothing"
        );
    }
    stamp_after_head_lookup(head, read, activities, now)
}

/// The stamp of a read that stopped short of the list head, given what the
/// lookup of the head's last sync returned.
///
/// A provider never synced before (`Ok(None)`) has no such time; its rows
/// then carry the newest instant the read vouches for — the window's closing
/// `before`, else its newest activity — which is in the past, so the head
/// still reads stale. A lookup that failed (`Err`) gives `None`: the stored
/// rows may carry a later receipt than anything the read vouches for, and
/// the cache refuses a row whose stamp is older than the stored one, so a
/// guessed stamp would drop the read's rows without a word; stamping now
/// would instead make the stale head read as current.
fn stamp_after_head_lookup<E>(
    head: Result<Option<Timestamp>, E>,
    read: &ActivityQueryParams,
    activities: &[Activity],
    now: Timestamp,
) -> Option<HeadSyncStamp> {
    let head = head.ok()?;
    let vouched = read
        .before
        .and_then(|ts| Timestamp::from_epoch_seconds(ts).ok())
        .or_else(|| activities.iter().map(Activity::start_date).max());
    Some(HeadSyncStamp {
        synced_at: head.or(vouched).unwrap_or(now).min(now),
        is_sync: false,
    })
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
        .record_activity_fetch(user_id, &tenant_id, provider, Timestamp::now())
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
    let Some(stamp) = head_sync_stamp(
        cache.as_ref(),
        user_id,
        *tenant_id,
        provider_slug,
        read,
        activities,
    )
    .await
    else {
        return;
    };
    if activities.is_empty() {
        if stamp.is_sync {
            stamp_fetch_freshness(cache.as_ref(), user_id, *tenant_id, provider_slug).await;
        }
        return;
    }
    let upserted = runtime
        .live_activities()
        .ingest(
            user_id,
            *tenant_id,
            provider_slug,
            activities,
            stamp.synced_at,
            WriteOrigin::Live,
        )
        .await;
    match upserted {
        Err(e) => warn!(
            user_id = %user_id,
            provider = %provider_slug,
            error = %e,
            "Activity cache: write-through from get_activities failed"
        ),
        Ok(ingest) => {
            if stamp.is_sync {
                stamp_fetch_freshness(cache.as_ref(), user_id, *tenant_id, provider_slug).await;
            }
            announce(runtime, user_id, *tenant_id, ingest.landed);
        }
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_providers::core::ActivityQueryParams;

    use super::stamp_after_head_lookup;

    /// A backfill page closing on 2026-03-01.
    fn backfill_page() -> ActivityQueryParams {
        ActivityQueryParams {
            limit: Some(50),
            offset: None,
            before: Some(
                Timestamp::parse_from_rfc3339("2026-03-01T00:00:00Z")
                    .unwrap()
                    .epoch_seconds(),
            ),
            after: None,
        }
    }

    #[test]
    fn an_unreadable_head_sync_time_writes_nothing() {
        let now = Timestamp::parse_from_rfc3339("2026-10-31T06:00:00Z").unwrap();
        let stamp = stamp_after_head_lookup(Err("pool timed out"), &backfill_page(), &[], now);
        assert!(
            stamp.is_none(),
            "a guessed stamp older than the stored rows would drop them silently"
        );
    }

    #[test]
    fn a_page_lands_at_the_head_last_sync() {
        let now = Timestamp::parse_from_rfc3339("2026-10-31T06:00:00Z").unwrap();
        let head = Timestamp::parse_from_rfc3339("2026-10-31T05:00:00Z").unwrap();
        let stamp = stamp_after_head_lookup(Ok::<_, &str>(Some(head)), &backfill_page(), &[], now)
            .expect("a readable head gives a stamp");
        assert_eq!(
            stamp.synced_at, head,
            "a page behind the head keeps the head's own sync time"
        );
        assert!(!stamp.is_sync, "a page behind the head is not a sync");
    }

    #[test]
    fn a_never_synced_provider_lands_at_the_window_close() {
        let now = Timestamp::parse_from_rfc3339("2026-10-31T06:00:00Z").unwrap();
        let stamp = stamp_after_head_lookup(Ok::<_, &str>(None), &backfill_page(), &[], now)
            .expect("a provider never synced still gets a stamp");
        assert_eq!(
            stamp.synced_at,
            Timestamp::parse_from_rfc3339("2026-03-01T00:00:00Z").unwrap(),
            "a provider never synced lands at the window's close, still stale"
        );
    }
}
