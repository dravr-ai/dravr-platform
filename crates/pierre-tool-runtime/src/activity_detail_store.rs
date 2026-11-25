// ABOUTME: The one store of activity detail reads — what a detail read found, kept beside the cached row, its route as the stored track
// ABOUTME: Also the one read of a served list's cached rows, which yields their details and places

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity detail, stored once.
//!
//! A provider serves an activity's splits, laps, description, device and
//! segment efforts only on a detail read, while every list sync rewrites the
//! cached copy from a list read that carries none of them. What a detail read
//! found is stored beside the cached row
//! (`ActivityCacheRepository::store_activity_detail`) and every read of the
//! row fills the fields its list copy lacks from it. Every detail reader goes
//! through this module: the activity view and the Home map's route read
//! (`pierre-server`'s activity detail and route services), `get_activities`,
//! which details the activities of the last fourteen days, and the detail
//! walk.
//!
//! A kept detail read's route is kept too, as the activity's stored track
//! (`keep_route`): a sciotte list row carries no route overview, so the
//! session-read detail's `route` is the only line the places path can form a
//! place from (carnet#1679).
//!
//! `get_activities` reads every served list's cached rows once
//! ([`stored_served_rows`]): the same read yields the stored details and each
//! row's place, rendered by [`served_place`] (carnet#1484, slice 6).

use std::collections::HashMap;
use std::future::Future;
use std::slice;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, PlaceRef, RouteGeometrySource, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_core::untrusted::display_line;
use pierre_database::repositories::{
    ActivityDetail, CachedActivityRow, CachedRowCursor, CachedRowsQuery,
};
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::route_track::RouteTrack;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::mirror_backend_for;
use pierre_services::activity_track::{self, RouteRead};
use pierre_services::athlete_places::MAX_PLACE_NAME_CHARS;
use pierre_services::live_activities::{LiveActivityStore, WriteOrigin};
use pierre_services::training_history_read::history_store_keys;
use serde_json::{Map, Value};
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

/// Cached rows one read of a served list takes per served activity before it
/// pages: a session's, a grant's, an export's and a mirror's copy.
const COPIES_PER_SERVED_ACTIVITY: usize = 4;

/// When a stored detail is read again: [`EMPTY_DETAIL_RECHECK_MINUTES`] out
/// for one that carried no detail-only field, never for one that carried any.
#[must_use]
fn detail_recheck_at(detail: &ActivityDetail, now: Timestamp) -> Option<Timestamp> {
    detail
        .is_empty()
        .then(|| now + Elapsed::minutes(EMPTY_DETAIL_RECHECK_MINUTES))
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
/// other summary is `serving`'s own row: a capture names the service
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
/// it and the grant's revocation purges it. A session's answer kept under a
/// grant's key keeps everything but its segment efforts ([`Efforts`]). A
/// store that fails is logged and answers `false`: the caller has the detail
/// in hand and answers from it either way.
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
    // A session's answer (named `sciotte` by the activity it returns, never
    // the grant's `strava`) kept under the grant's key must not stamp that
    // row's segments as read: session-read efforts never hide the row from the
    // session-first detail walk, which files them under the session
    // (carnet#1603). The rest of the detail is kept.
    let session_answer_on_grant_row =
        mirror_backend_for(answered).is_some() && detailed.provider() != answered;
    let efforts = if session_answer_on_grant_row {
        Efforts::Withheld
    } else {
        Efforts::Kept
    };
    let keep = Keep {
        listed: provider,
        answered,
        efforts,
    };
    store_detail_answered_by(repos, tenant_id, user_id, keep, detailed, copy).await
        == DetailKept::Stored
}

/// Where and how a detail read is kept.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keep<'a> {
    /// The key of the row the activity was listed under.
    pub listed: &'a str,
    /// The key of the backend that answered, under which it is kept.
    pub answered: &'a str,
    /// Whether the read's segment efforts are kept.
    pub efforts: Efforts,
}

/// Whether a stored detail carries the segment efforts of the read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Efforts {
    /// The efforts are stored, stamping the row's segments as read.
    Kept,
    /// The efforts are left out, so the row stays a candidate of the detail
    /// walk.
    Withheld,
}

/// What keeping a detail read came to ([`store_detail_answered_by`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailKept {
    /// The detail is stored beside its row.
    Stored,
    /// No row stands to store it beside: the row left the cache, or the
    /// grant's copy of it was not written.
    NotCached,
    /// The store failed (logged).
    Failed,
}

/// Keep a detail beside its row under `answered`, the backend that answered:
/// a detail walk reads through one backend, so what it read is kept under that
/// backend's key and never another's. `copy` writes `answered`'s own copy of
/// an activity only the other half of the pair listed, and answers how many
/// rows it wrote; it runs only when `answered` holds no row of the activity.
///
/// The session's answer for a ride only the grant listed is the session's
/// own copy (`copy` writes it), never a change to the grant's row, so
/// disconnecting the session takes the session-read data and leaves the grant's
/// rows as the API wrote them (carnet#1603).
pub(crate) async fn store_detail_answered_by(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    keep: Keep<'_>,
    detailed: &Activity,
    copy: impl Future<Output = AppResult<u64>>,
) -> DetailKept {
    let Keep {
        listed: provider,
        answered,
        efforts,
    } = keep;
    let outcome = match keep_detail(repos, tenant_id, user_id, answered, detailed, efforts).await {
        Ok(false) if answered != provider => match copy.await {
            Ok(0) => DetailKept::NotCached,
            Ok(_) => {
                kept(keep_detail(repos, tenant_id, user_id, answered, detailed, efforts).await)
            }
            Err(e) => {
                warn!(
                    activity_id = detailed.id(),
                    provider = answered,
                    error = %e,
                    "writing the copy of an activity it answered failed; its detail is not kept"
                );
                DetailKept::Failed
            }
        },
        other => kept(other),
    };
    if outcome == DetailKept::Stored {
        keep_route(repos, tenant_id, user_id, answered, detailed).await;
    }
    outcome
}

/// Keep the route a kept detail read carried as the activity's stored track,
/// under `answered`, the key its detail is kept under (carnet#1679).
///
/// The read's time series is its route: an API provider's recorded streams,
/// or a session-read detail's `route` folded into them. It is turned into a track
/// exactly as the Home map's route read turns one (trimmed past the privacy
/// radius, simplified to a map's points) and stored through the one
/// route-track write, which never replaces a drawn track. A stream
/// set without coordinates — a session-read route whose coordinate list is empty,
/// Strava's streams of a trainer ride — settles `no_gps`. A read with no time
/// series at all proves nothing (a page read before its map rendered) and
/// stores nothing. Only a provider's own answer reaches here: a read that
/// failed or was refused stores no detail, and so no route. A store that
/// fails is logged: the detail is kept either way.
async fn keep_route(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    answered: &str,
    detailed: &Activity,
) {
    let Some(streams) = detailed.time_series_data() else {
        return;
    };
    let track = RouteTrack::from_streams(streams).map(activity_track::map_track);
    let read = RouteRead {
        source: RouteGeometrySource::Streams,
        outcome: track.as_ref().map_err(|miss| miss.as_str()),
        expires_at: None,
    };
    let activity_id = detailed.id();
    if let Err(e) =
        activity_track::store_route_read(repos, &tenant_id, user_id, answered, activity_id, read)
            .await
    {
        warn!(
            activity_id,
            provider = answered,
            error = %e,
            "storing the route a detail read carried failed; its detail is kept"
        );
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
            WriteOrigin::Walk,
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
    efforts: Efforts,
) -> Result<bool, ()> {
    let mut detail = ActivityDetail::from_activity(detailed);
    if efforts == Efforts::Withheld {
        detail.segment_efforts = None;
    }
    let recheck_at = detail_recheck_at(&detail, Timestamp::now());
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

/// What the cached rows of a served list hold beside the activities served:
/// the stored detail of each one a detail read stored, and each one's place.
///
/// Both are keyed by the activity's provider and id, as the activity a served
/// row decodes to names them: an id is unique only within its provider.
#[derive(Debug, Default)]
pub struct ServedRows {
    /// Each served activity whose stored detail still answers it, with its
    /// detail filled in, withheld as a live read withholds it.
    pub details: HashMap<(String, String), Activity>,
    /// Each served activity's place, from the row it was served from.
    pub places: HashMap<(String, String), PlaceRef>,
}

impl ServedRows {
    /// Keep each place on the activity a detail read put in its summary's
    /// stead, position for position.
    ///
    /// A read of a session's row that its grant answered returns the grant's
    /// activity, named for the grant's provider ([`answered_key`]); the row
    /// it details, and so its place, is the summary's.
    pub fn follow_details(&mut self, summaries: &[Activity], detailed: &[Activity]) {
        for (summary, detail) in summaries.iter().zip(detailed) {
            if summary.provider() == detail.provider() || summary.id() != detail.id() {
                continue;
            }
            let listed = (summary.provider().to_owned(), summary.id().to_owned());
            if let Some(place) = self.places.get(&listed).cloned() {
                self.places
                    .entry((detail.provider().to_owned(), detail.id().to_owned()))
                    .or_insert(place);
            }
        }
    }
}

/// Read the cached rows of `served`, a list `serving` served, in one read:
/// every copy of each served activity and no other row.
///
/// The read is narrowed to the served ids and to their starts a day either
/// side, the slack covering copies whose recorded starts differ. It takes
/// four rows per served activity and reads one more page only while a page
/// comes back full, so a long span costs its served rows, not the span.
///
/// When two rows decode to the same activity under different keys, the row
/// under [`listed_key`] gives the place, else the first read. A read that
/// fails is logged and answers nothing stored: every activity is then read
/// from its provider and served with no place, as if nothing were stored.
pub async fn stored_served_rows(
    repos: &RepositoryRegistry,
    policies: &dyn ProviderTerms,
    tenant_id: TenantId,
    user_id: Uuid,
    serving: &str,
    served: &[Activity],
) -> ServedRows {
    let rows = match served_rows(repos, tenant_id, user_id, served).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(error = %e, "reading the served rows' stored details failed; reading from the provider");
            return ServedRows::default();
        }
    };
    // Per activity, the row whose place it is served with: the one under the
    // key the list read it from, else the one stored first.
    let mut ranked: HashMap<(String, String), (PlaceRank, PlaceRef)> = HashMap::new();
    let mut detailed = Vec::new();
    for row in rows {
        if let Some(place) = row.place.clone() {
            let key = (
                row.activity.provider().to_owned(),
                row.activity.id().to_owned(),
            );
            let rank = PlaceRank {
                listed: row.provider == listed_key(serving, row.activity.provider()),
                stored_at: row.stored_at,
            };
            match ranked.get(&key) {
                Some((held, _)) if !rank.outranks(held) => {}
                _ => {
                    ranked.insert(key, (rank, place));
                }
            }
        }
        if row.detail_read {
            detailed.push(row.activity);
        }
    }
    let places = ranked
        .into_iter()
        .map(|(key, (_, place))| (key, place))
        .collect();
    // The cache keeps every row whole; what the provider terms withhold from
    // a model is withheld from a stored detail as from a live read. The list
    // read already tallied those items, so this filter tallies apart.
    let (kept, _) =
        ai_scope::tallied(async { ai_scope::filter_activities(policies, detailed) }).await;
    let details = kept
        .into_iter()
        .map(|activity| {
            let key = (activity.provider().to_owned(), activity.id().to_owned());
            (key, activity)
        })
        .collect();
    ServedRows { details, places }
}

/// Which of two rows of one activity gives its place.
struct PlaceRank {
    /// The row is under the key the served list read it from.
    listed: bool,
    /// When the row was first stored.
    stored_at: Option<Timestamp>,
}

impl PlaceRank {
    /// Whether this row gives the place over `held`: the listed row first,
    /// then the one stored first; a row with no store stamp comes last.
    fn outranks(&self, held: &Self) -> bool {
        match (self.listed, held.listed) {
            (true, false) => true,
            (false, true) => false,
            _ => match (self.stored_at, held.stored_at) {
                (Some(mine), Some(theirs)) => mine < theirs,
                (Some(_), None) => true,
                _ => false,
            },
        }
    }
}

/// Every cached copy of the served activities, newest first.
async fn served_rows(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    served: &[Activity],
) -> AppResult<Vec<CachedActivityRow>> {
    let (Some(oldest), Some(newest)) = (
        served.iter().map(Activity::start_date).min(),
        served.iter().map(Activity::start_date).max(),
    ) else {
        return Ok(Vec::new());
    };
    let mut ids: Vec<&str> = served.iter().map(Activity::id).collect();
    ids.sort_unstable();
    ids.dedup();
    let page_rows =
        i64::try_from(ids.len().saturating_mul(COPIES_PER_SERVED_ACTIVITY)).unwrap_or(i64::MAX);
    let query = CachedRowsQuery::window(
        oldest - Elapsed::days(1),
        newest + Elapsed::days(1),
        page_rows,
    )
    .with_ids(&ids);
    let mut rows = repos
        .activity_cache
        .get_cached_activity_rows(user_id, &tenant_id, &query)
        .await?;
    let mut page_len = rows.len();
    // Only an activity held in more than four copies fills a page; the next
    // page reads on below the last row, and the copies bound how many follow.
    while i64::try_from(page_len).unwrap_or(i64::MAX) >= page_rows {
        let Some(last) = rows.last() else { break };
        let page = repos
            .activity_cache
            .get_cached_activity_rows(
                user_id,
                &tenant_id,
                &query.clone().below(CachedRowCursor::after(last)),
            )
            .await?;
        page_len = page.len();
        rows.extend(page);
    }
    Ok(rows)
}

/// A served row's place as a client reads it: `{id, name}`, the name only
/// where it may be shown.
///
/// A name the athlete gave is shown on every transport; an automatic one (a
/// city, a segment) only on Dravr's own surfaces, through the transport check
/// stored derived content takes. A place without a name is `{id}`. Never a
/// centroid, radius or point.
#[must_use]
pub fn served_place(place: &PlaceRef) -> Value {
    let mut served = Map::new();
    served.insert("id".to_owned(), Value::String(place.id.clone()));
    if let Some(name) = served_place_name(place) {
        served.insert("name".to_owned(), Value::String(name));
    }
    Value::Object(served)
}

/// The name [`served_place`] shows, through [`display_line`]; `None` when
/// the place has none, or has only an automatic one and the reader is not
/// first-party.
#[must_use]
pub fn served_place_name(place: &PlaceRef) -> Option<String> {
    let name = place.name.as_deref()?;
    // Asked only of a name there is: an admitted first-party-only name marks
    // the reply as built on first-party-only content.
    ai_scope::admit_derived(TransportPolicy::from_first_party_only(!place.athlete_named))
        .then(|| display_line(name, MAX_PLACE_NAME_CHARS))
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_core::models::{ActivityBuilder, SportType};

    use super::*;

    fn read_by(backend: &str) -> Activity {
        ActivityBuilder::new(
            "15550001111",
            "Ride",
            SportType::Ride,
            Timestamp::now(),
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
            "a capture names the service, never the grant"
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
    fn a_detail_the_grant_answered_keeps_the_session_rows_place() {
        let place = PlaceRef {
            id: "place-1".to_owned(),
            name: None,
            athlete_named: false,
        };
        let mut served = ServedRows::default();
        served.places.insert(
            ("sciotte".to_owned(), "15550001111".to_owned()),
            place.clone(),
        );
        served.follow_details(&[read_by("sciotte")], &[read_by("strava")]);
        assert_eq!(
            served
                .places
                .get(&("strava".to_owned(), "15550001111".to_owned())),
            Some(&place),
            "bug: the row lost its place when the grant answered its detail"
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
            "bug: a Garmin capture names the service, and lives under the session's key"
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
