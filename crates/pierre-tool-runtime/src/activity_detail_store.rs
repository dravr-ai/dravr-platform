// ABOUTME: The one store of activity detail reads — what a detail read found, kept beside the cached row
// ABOUTME: Stored once and served from then on; a read that found nothing is read again after a recheck

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity detail, stored once.
//!
//! A provider serves an activity's splits, laps, description, device and
//! segment efforts only on a detail read, while every list sync rewrites the
//! cached copy from a list read that carries none of them. What a detail read
//! found is stored beside the cached row
//! (`ActivityCacheRepository::store_activity_detail`) and every read of the
//! row fills the fields its list copy lacks from it. Both detail readers go
//! through this module: the activity view (`pierre-server`'s activity detail
//! service) and `get_activities`, which details the activities of the last
//! fourteen days.

use std::collections::HashMap;
use std::future::Future;
use std::slice;

use chrono::{DateTime, Duration, Utc};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{ActivityDetail, CachedRowsQuery};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::mirror_backend_for;
use pierre_services::live_activities::LiveActivityStore;
use pierre_services::training_history_read::history_store_keys;
use tracing::{debug, warn};
use uuid::Uuid;

/// Minutes a stored detail read that carried no detail-only field answers
/// the row before the detail is read again.
///
/// Such a read proves nothing — the request carrying the laps may have
/// failed — so it is not kept for good. Long enough that an activity that
/// truly has none costs one provider read per half hour rather than one per
/// open; short enough that a failure the provider has since recovered from
/// does not hide the detail for the day.
pub const EMPTY_DETAIL_RECHECK_MINUTES: i64 = 30;

/// Most cached rows one stored-detail lookup reads.
const STORED_DETAIL_LOOKUP_LIMIT: i64 = 500;

/// When a stored detail is read again: [`EMPTY_DETAIL_RECHECK_MINUTES`] out
/// for one that carried no detail-only field, never for one that carried any.
#[must_use]
fn detail_recheck_at(detail: &ActivityDetail, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    detail
        .is_empty()
        .then(|| now + Duration::minutes(EMPTY_DETAIL_RECHECK_MINUTES))
}

/// The cache key what a read of a row listed under `listed_under` brought
/// back belongs under: the backend that answered it.
///
/// A by-id read of the athlete's Strava session that the session cannot
/// answer is made again through their Strava grant
/// (`protocol::auth::grant_by_id`), and what the grant answers is Strava API
/// data, named `strava` by the activity it returns. It is kept under the
/// grant's key, never the session's, so the grant's revocation purges it as
/// Strava's terms require (carnet#1494). Any other answer is the listed
/// backend's own.
#[must_use]
pub fn answered_key<'a>(listed_under: &'a str, detailed: &'a Activity) -> &'a str {
    let answered = detailed.provider();
    if mirror_backend_for(answered) == Some(listed_under) {
        answered
    } else {
        listed_under
    }
}

/// The cache key of the row a summary listed by `listed_by` (its
/// `Activity::provider`) lives under, in a window `serving` read.
///
/// A window a session reads serves its grant's rows too
/// (`training_history_read::history_store_keys`): a ride only the Strava
/// grant listed is a `strava` row in a `sciotte` window, and its detail
/// belongs beside that row, as the activity view keeps it, whichever half of
/// the pair read the detail ([`answered_key`] keeps a grant's answer under the grant). Any
/// other summary is `serving`'s own row: a capture names the scraper
/// (`sciotte`), never the session key it is cached under.
#[must_use]
pub fn listed_key<'a>(serving: &'a str, listed_by: &str) -> &'a str {
    history_store_keys(serving)
        .into_iter()
        .find(|key| *key == listed_by)
        .unwrap_or(serving)
}

/// Keep what a detail read of one cached activity found, beside the row its
/// list read wrote under `provider`, or under the grant that answered it in
/// the session's place ([`answered_key`]).
///
/// Answers `true` when it was stored. A row that left the cache meanwhile
/// stores nothing (`false`). A grant's answer for an activity the grant
/// never listed is first written as the grant's own copy of it
/// ([`copy_under_grant`]), so the view that merges a workout's copies serves
/// it and the grant's revocation purges it. A store that fails is logged and
/// answers `false`: the caller has the detail in hand and answers from it
/// either way.
pub async fn store_detail(
    repos: &RepositoryRegistry,
    store: &dyn LiveActivityStore,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: &str,
    detailed: &Activity,
) -> bool {
    let answered = answered_key(provider, detailed);
    let copy = copy_under_grant(repos, store, tenant_id, user_id, answered, detailed);
    store_detail_from_live_grant(repos, tenant_id, user_id, provider, detailed, copy).await
        == DetailKept::Stored
}

/// What keeping a detail read came to
/// ([`store_detail_from_live_grant`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailKept {
    /// The detail is stored beside its row.
    Stored,
    /// No row stands to store it beside: the row left the cache, or the
    /// grant's copy of it was not written.
    NotCached,
    /// The store failed (logged).
    Failed,
}

/// [`store_detail`] for a caller that has just read `detailed` through a
/// live grant itself.
///
/// The grant's copy of an activity only its session listed is written by
/// `copy`, which answers how many rows it wrote, and not only while the
/// grant shows a last sync, as [`store_detail`]'s is.
///
/// A detail walk reads through the athlete's live grant and writes the copy
/// through its owed-write guard, so a purge of the grant landing between its
/// read and its write takes the copy back (`activity_backfill::detail_walk`).
/// `copy` runs only when the grant answered for a row it holds no copy of;
/// what the grant answered for a row it holds is stored beside that row.
pub async fn store_detail_from_live_grant(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: &str,
    detailed: &Activity,
    copy: impl Future<Output = AppResult<u64>>,
) -> DetailKept {
    let answered = answered_key(provider, detailed);
    match keep_detail(repos, tenant_id, user_id, answered, detailed).await {
        Ok(false) if answered != provider => match copy.await {
            Ok(0) => DetailKept::NotCached,
            Ok(_) => kept(keep_detail(repos, tenant_id, user_id, answered, detailed).await),
            Err(e) => {
                warn!(
                    activity_id = detailed.id(),
                    provider = answered,
                    error = %e,
                    "writing the grant's copy of an activity it answered failed; its detail is not kept"
                );
                DetailKept::Failed
            }
        },
        other => kept(other),
    }
}

/// What one store of a detail beside its row came to.
const fn kept(stored: Result<bool, ()>) -> DetailKept {
    match stored {
        Ok(true) => DetailKept::Stored,
        Ok(false) => DetailKept::NotCached,
        Err(()) => DetailKept::Failed,
    }
}

/// Write `detailed` as the grant's own cached copy, under `grant`, and
/// answer how many rows were written.
///
/// A by-id read is no read of the grant's list head, so the copy carries the
/// head's own last sync and never makes the grant look freshly synced
/// (the activity store's ingest at that stamp). A grant never
/// synced gets no copy (`0`): a row of it would read as a fetch that never
/// happened, so its answer is read again while the session stays down.
async fn copy_under_grant(
    repos: &RepositoryRegistry,
    store: &dyn LiveActivityStore,
    tenant_id: TenantId,
    user_id: Uuid,
    grant: &str,
    detailed: &Activity,
) -> AppResult<u64> {
    let Some(head_synced) = repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant_id, grant)
        .await?
    else {
        return Ok(0);
    };
    Ok(store
        .ingest(
            user_id,
            tenant_id,
            grant,
            slice::from_ref(detailed),
            head_synced,
        )
        .await?
        .stored)
}

/// One store of `detailed`'s detail beside the row under `provider`:
/// `Ok(false)` when no such row is cached, `Err` when the store failed
/// (logged).
async fn keep_detail(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    provider: &str,
    detailed: &Activity,
) -> Result<bool, ()> {
    let detail = ActivityDetail::from_activity(detailed);
    let recheck_at = detail_recheck_at(&detail, Utc::now());
    let activity_id = detailed.id();
    match repos
        .activity_cache
        .store_activity_detail(
            user_id,
            &tenant_id,
            provider,
            activity_id,
            &detail,
            recheck_at,
        )
        .await
    {
        Ok(true) => Ok(true),
        Ok(false) => {
            debug!(
                activity_id,
                provider, "activity left the cache during its detail read; its detail is not kept"
            );
            Ok(false)
        }
        Err(e) => {
            warn!(activity_id, provider, error = %e, "storing an activity detail read failed");
            Err(())
        }
    }
}

/// The cached activities started at or after `since` whose stored detail
/// still answers them, each with its detail filled in.
///
/// Keyed by the activity's provider and id: an id is unique only within its
/// provider.
///
/// A lookup that fails is logged and answers empty: every activity is then
/// read from its provider, as if nothing were stored.
pub async fn stored_details(
    repos: &RepositoryRegistry,
    policies: &dyn ProviderTerms,
    tenant_id: TenantId,
    user_id: Uuid,
    since: DateTime<Utc>,
) -> HashMap<(String, String), Activity> {
    let rows = match repos
        .activity_cache
        .get_cached_activity_rows(
            user_id,
            &tenant_id,
            &CachedRowsQuery::window(since, Utc::now(), STORED_DETAIL_LOOKUP_LIMIT),
        )
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .filter(|row| row.detail_read)
            .map(|row| row.activity)
            .collect::<Vec<_>>(),
        Err(e) => {
            warn!(error = %e, "reading stored activity details failed; reading from the provider");
            return HashMap::new();
        }
    };
    // The cache keeps every row whole; what the provider terms withhold from
    // a model is withheld from a stored detail as from a live read. The list
    // read already tallied those items, so this filter tallies apart.
    let (kept, _) = ai_scope::tallied(async { ai_scope::filter_activities(policies, rows) }).await;
    kept.into_iter()
        .map(|activity| {
            let key = (activity.provider().to_owned(), activity.id().to_owned());
            (key, activity)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::{ActivityBuilder, SportType};

    use super::*;

    fn read_by(backend: &str) -> Activity {
        ActivityBuilder::new(
            "15550001111",
            "Ride",
            SportType::Ride,
            Utc::now(),
            3_600,
            backend,
        )
        .build()
    }

    #[test]
    fn a_grants_answer_for_the_session_is_kept_under_the_grant() {
        assert_eq!(
            answered_key("sciotte", &read_by("strava")),
            "strava",
            "bug: Strava API data the grant answered was kept under the session's key"
        );
    }

    #[test]
    fn every_other_answer_is_kept_under_the_row_it_details() {
        assert_eq!(
            answered_key("sciotte", &read_by("sciotte")),
            "sciotte",
            "bug: the session's own answer left the session's row"
        );
        assert_eq!(
            answered_key("strava", &read_by("strava")),
            "strava",
            "bug: the grant's answer for its own row left that row"
        );
        assert_eq!(
            answered_key("sciotte_garmin", &read_by("sciotte")),
            "sciotte_garmin",
            "a capture names the scraper, never the grant"
        );
        assert_eq!(
            answered_key("intervals_icu", &read_by("strava")),
            "intervals_icu",
            "bug: a provider outside a mirror pair keeps its answer under its own row"
        );
        assert_eq!(
            answered_key("strava", &read_by("sciotte")),
            "strava",
            "bug: the session's detail of a row the grant listed left that row"
        );
    }

    #[test]
    fn a_summary_is_stored_under_the_key_its_row_lives_under() {
        assert_eq!(
            listed_key("sciotte", "strava"),
            "strava",
            "bug: a ride the grant listed is detailed beside a session row that does not exist"
        );
        assert_eq!(
            listed_key("sciotte", "sciotte"),
            "sciotte",
            "bug: the session's own ride left the session's row"
        );
        assert_eq!(
            listed_key("sciotte_garmin", "sciotte"),
            "sciotte_garmin",
            "bug: a Garmin capture names the scraper, and lives under the session's key"
        );
        assert_eq!(
            listed_key("sciotte_garmin", "garmin"),
            "garmin",
            "bug: a ride Garmin's grant pushed is detailed beside a session row that does not exist"
        );
        assert_eq!(
            listed_key("intervals_icu", "strava"),
            "intervals_icu",
            "bug: a provider outside a mirror pair reads only its own key"
        );
    }
}
