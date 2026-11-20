// ABOUTME: The one write path of every provider's activities: each read, push or upload goes into the activity store
// ABOUTME: The server implements it over barrage, which keeps the record and projects it into the cache readers read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live activity writes.
//!
//! Every provider read the platform makes — the chat turn's live read, a
//! backfill, a push, a webhook, an upload — lands its activities through
//! [`LiveActivityStore`], and every deletion of one goes through it too. The
//! server's implementation keeps each one in dravr-barrage, the athlete's
//! activity store, under one retention policy, and projects it into
//! `cached_activities`, which every reader reads (carnet#1482).

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, TenantId};
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use uuid::Uuid;
/// Which side of the live/walk line a write is on (carnet#1486).
///
/// Only a live write — one that reads the provider's present — can land a
/// session; every direct [`LiveActivityStore::ingest`] caller names its side,
/// so a new writer has to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// A read of the provider's present: a chat turn's read, the capture
    /// sweep, a webhook-driven read, a backfill, a group snapshot, a push.
    Live,
    /// A copy of the past: the history walk, the detail walk, a rewrite of a
    /// held row.
    Walk,
    /// Data that did not come from reading the provider: an adoption of the
    /// cache, an upload, a seed.
    Import,
}

/// A workout a live write landed: newer than every workout the athlete held
/// from any source, and no copy of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandedSession {
    /// The live source key the write was made under.
    pub provider: String,
    /// The landed workout's id under that source.
    pub activity_id: String,
    /// When the workout started.
    pub start: DateTime<Utc>,
    /// The store held nothing before this write: the athlete's first data.
    pub first_in_store: bool,
}

/// What one live ingest did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveIngest {
    /// Rows held after the write: inserted, updated or unchanged. A row the
    /// athlete deleted is not one of them.
    pub stored: u64,
    /// Rows the write inserted: the store held no record of them before it.
    pub inserted: u64,
    /// Rows the write refused: the athlete deleted them
    /// ([`LiveActivityStore::erase`]), or they start in the future, past a
    /// clock's skew of the read.
    pub refused: u64,
    /// Whether a session newer than every activity held from the write's own
    /// source landed.
    pub newer_session: bool,
    /// What a [`WriteOrigin::Live`] write landed ([`landed_session`]); always
    /// `None` for a walk or an import.
    pub landed: Option<LandedSession>,
}

/// The write path of every provider's activities.
#[async_trait]
pub trait LiveActivityStore: Send + Sync {
    /// Keep `activities`, read from `provider` for the athlete, at the sync
    /// stamp `synced_at`, and serve them to the cache readers. An activity
    /// the athlete deleted ([`Self::erase`]) is neither kept nor served.
    ///
    /// `origin` says which side of the live/walk line the write is on: only a
    /// [`WriteOrigin::Live`] write reports what it landed
    /// ([`LiveIngest::landed`]).
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source the store knows,
    /// or the store or its projection fails.
    async fn ingest(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
        synced_at: DateTime<Utc>,
        origin: WriteOrigin,
    ) -> AppResult<LiveIngest>;

    /// Keep one pushed `activity`, received at `received_at`, unless a copy
    /// of it received later is already held: a push retried after a newer
    /// one leaves the newer copy standing. A push reads the provider's
    /// present, so the write is [`WriteOrigin::Live`] and reports what it
    /// landed; [`LiveIngest::stored`] says whether it was kept.
    ///
    /// # Errors
    ///
    /// As [`Self::ingest`].
    async fn ingest_received(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
        received_at: DateTime<Utc>,
    ) -> AppResult<LiveIngest>;

    /// Rewrite the content of the athlete's standing `activity` of
    /// `provider` in place, keeping its receipt: a figure derived from data
    /// stored after the activity's own write (a pushed ride's power, from
    /// samples that landed later) belongs on whichever copy stands, received
    /// before or after those samples. The store's record and the cached row
    /// change together. One the cache served before the store held the
    /// provider's reads is amended in the cache alone. Returns whether a copy
    /// stood.
    ///
    /// # Errors
    ///
    /// As [`Self::ingest`].
    async fn amend(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
    ) -> AppResult<bool>;

    /// The athlete or the provider deleted the workout `activity_id`: it
    /// stops being served (its cached row, samples and gear attribution
    /// go), the store forgets it, and no later read of `provider` brings it
    /// back — one never read yet included, as when a delete webhook outruns
    /// the sync that would have stored it. Returns whether the store or the
    /// cache held it.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, the id is not
    /// one the store can hold, or the store or its projection fails.
    async fn erase(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool>;

    /// Drop the store's copy of `activity_id` without refusing it later:
    /// the athlete deleted a file they uploaded and may upload it again.
    /// It stops being served first. Returns whether the store held it.
    ///
    /// # Errors
    ///
    /// As [`Self::erase`].
    async fn discard(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool>;

    /// Drop every copy of `provider`'s activities for the athlete that
    /// started within `[start, end]`, both ends included, as [`Self::discard`]
    /// drops one: a list read of that window vouched the provider no longer
    /// holds them, and a later read that shows one again keeps it again. The
    /// cache's rows in the window and the store's records there both go, so
    /// the two never disagree. Returns how many activities were dropped.
    ///
    /// Each activity goes on its own: a failure part-way leaves the ones
    /// before it dropped and returns the error, and a retry drops the rest.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the cache,
    /// the store or its projection fails.
    async fn discard_between(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<u64>;

    /// How many records the store holds of `provider` for the athlete: what
    /// an adoption of the cache checks the cache's count against.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the store
    /// fails.
    async fn held(&self, user_id: Uuid, tenant_id: TenantId, provider: &str) -> AppResult<u64>;

    /// The ids among `activities` the athlete's `provider` rows already
    /// hold, read in one pass over the window their start dates span: a
    /// write of them adds the rest.
    ///
    /// # Errors
    ///
    /// Returns an error when the rows cannot be read.
    async fn held_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>>;

    /// The ids among `activities` the store keeps a record of under the
    /// athlete's `provider`, read in one pass over the window their start
    /// dates span. Unlike [`Self::held_among`], a row the cache served
    /// before the store held the provider's reads is not one of them: a
    /// write of `activities` inserts exactly those it does not name, the
    /// athlete's deleted ones aside.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the store
    /// fails.
    async fn recorded_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>>;

    /// Delete everything the store holds of `provider` for the athlete — a
    /// disconnect or a purge — after its cached rows stop being served, and
    /// forget what the athlete had deleted of it, so a reconnection starts
    /// over. What the platform kept beside those rows (routes, pushed
    /// samples, best efforts, gear, sync state) and the purge's attestation
    /// are the provider purge's (`pierre_services::provider_purge`), which
    /// calls this after it: on its own this is not a whole disconnect.
    ///
    /// # Errors
    ///
    /// Returns an error when `provider` is not a live source, or the store or
    /// its projection fails.
    async fn delete_source(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()>;

    /// Whether `provider` is a live source the store keeps records of. An
    /// imported archive is not one (its records arrive through an import and
    /// go only with it), nor is a key the store does not know.
    fn keeps(&self, provider: &str) -> bool;

    /// Delete everything the store holds of `provider`, for every athlete in
    /// every tenant, as [`Self::delete_source`] does for one: the admin's
    /// termination purge. Returns how many athletes' records were deleted.
    ///
    /// # Errors
    ///
    /// As [`Self::delete_source`].
    async fn delete_source_everywhere(&self, provider: &str) -> AppResult<u64>;

    /// Delete every record the store holds of the athlete, of every live
    /// source in every tenant, and what they had deleted: the account's
    /// delete or reset. Returns how many records were deleted.
    ///
    /// The cached rows are not retracted here: the account delete and the
    /// reset wipe the athlete's cache themselves, and this runs beside that
    /// wipe so the store holds nothing a later sync would serve again.
    ///
    /// # Errors
    ///
    /// Returns an error when the store fails.
    async fn delete_user(&self, user_id: Uuid) -> AppResult<u64>;
}

/// Fraction digits the store keeps of a start: PostgreSQL's `TIMESTAMPTZ`
/// holds microseconds.
const STORED_START_DIGITS: u16 = 6;

/// The one landing rule (carnet#1486): which workout, if any, a live write
/// of `written` under `provider` landed. Pure.
///
/// A write lands a session when barrage saw a newer session of the source
/// (`newer_session`, computed over the rows it kept) and one of the rows it
/// kept is new across every source and no copy of a held workout:
///
/// - `held_before` is the newest start the athlete held from any source,
///   read before the write; `None` when the store held nothing, which lands
///   the newest kept row as the athlete's first data;
/// - `kept_around` is every source's cache rows around the write, read after
///   it, so a row barrage refused (future-dated, or one the athlete erased)
///   is never in it;
/// - the candidates are the written rows found in `kept_around` that start
///   after `held_before`; `kept_around` is grouped as every reader's dedup
///   groups it ([`merge_duplicates`]), and the newest candidate whose group
///   holds no workout started at or before `held_before` lands. An id that
///   two providers share in the window counts as held — a scrape keeps the
///   provider's own id — so that case fails closed.
#[must_use]
pub fn landed_session(
    provider: &str,
    written: &[Activity],
    newer_session: bool,
    held_before: Option<DateTime<Utc>>,
    kept_around: &[Activity],
    dedup: &DedupConfig,
) -> Option<LandedSession> {
    use chrono::SubsecRound;

    if !newer_session {
        return None;
    }
    let written_keys: HashSet<(&str, &str)> = written
        .iter()
        .map(|activity| (activity.provider(), activity.id()))
        .collect();
    // The newest start held is read from the store's start column, which
    // PostgreSQL keeps to the microsecond; a row's start comes from its
    // record, which keeps the nanosecond. Compared at the store's precision,
    // a held ride is held on every backend.
    let held = |start: DateTime<Utc>| {
        held_before.is_some_and(|held| start.trunc_subsecs(STORED_START_DIGITS) <= held)
    };
    let mut candidates: Vec<&Activity> = kept_around
        .iter()
        .filter(|row| written_keys.contains(&(row.provider(), row.id())) && !held(row.start_date()))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| {
        b.start_date()
            .cmp(&a.start_date())
            .then_with(|| a.id().cmp(b.id()))
    });
    let mut by_id: HashMap<&str, Vec<&Activity>> = HashMap::new();
    for row in kept_around {
        by_id.entry(row.id()).or_default().push(row);
    }
    let member_held = |id: &str| {
        by_id.get(id).is_none_or(|rows| {
            let providers: HashSet<&str> = rows.iter().map(|row| row.provider()).collect();
            providers.len() > 1 || rows.iter().any(|row| held(row.start_date()))
        })
    };
    let (_, report) = merge_duplicates(kept_around.to_vec(), dedup);
    let landed = candidates.into_iter().find(|candidate| {
        report
            .groups
            .iter()
            .find(|group| group.fragment_ids.iter().any(|id| id == candidate.id()))
            .is_none_or(|group| !group.fragment_ids.iter().any(|id| member_held(id)))
    })?;
    Some(LandedSession {
        provider: provider.to_owned(),
        activity_id: landed.id().to_owned(),
        start: landed.start_date(),
        first_in_store: held_before.is_none(),
    })
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::*;
    use chrono::{Duration, TimeZone};
    use pierre_core::models::{ActivityBuilder, SportType};

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap()
    }

    fn ride(id: &str, provider: &str, start: DateTime<Utc>) -> Activity {
        ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, provider)
            .distance_meters(30_000.0)
            .build()
    }

    fn dedup() -> DedupConfig {
        DedupConfig {
            overlap_tolerance_secs: 300,
            time_window_minutes: 15,
            distance_tolerance_pct: 10.0,
        }
    }

    /// What a write of `written` under `provider` lands, the store having
    /// held up to `held_before` and kept `kept` around the write.
    fn land(
        provider: &str,
        written: &[&Activity],
        newer_session: bool,
        held_before: Option<DateTime<Utc>>,
        kept: &[&Activity],
    ) -> Option<LandedSession> {
        let written: Vec<Activity> = written.iter().map(|&a| a.clone()).collect();
        let kept: Vec<Activity> = kept.iter().map(|&a| a.clone()).collect();
        landed_session(
            provider,
            &written,
            newer_session,
            held_before,
            &kept,
            &dedup(),
        )
    }

    #[test]
    fn an_empty_store_lands_the_first_data() {
        let first = ride("s1", "strava", noon());
        let landed = landed_session(
            "strava",
            slice::from_ref(&first),
            true,
            None,
            slice::from_ref(&first),
            &dedup(),
        )
        .expect("bug: the first workout in an empty store must land");
        assert!(
            landed.first_in_store,
            "bug: an empty store's landing is the first data"
        );
        assert_eq!(
            landed.activity_id, "s1",
            "bug: the written ride must be the one landed"
        );
    }

    #[test]
    fn a_held_ride_read_back_at_the_stores_precision_does_not_land_again() {
        // PostgreSQL keeps a start to the microsecond while the row's own
        // record keeps the nanosecond: the newest start held reads back
        // short of the same ride's start.
        let start = noon() + Duration::nanoseconds(123_456_789);
        let served = ride("s1", "strava", start);
        let held_column = noon() + Duration::microseconds(123_456);
        assert_eq!(
            land("strava", &[&served], true, Some(held_column), &[&served]),
            None,
            "bug: a ride already held lands again when its start is read at the store's precision"
        );
    }

    #[test]
    fn a_second_sources_copy_of_a_held_ride_does_not_land() {
        let held = ride("g1", "garmin", noon());
        let copy = ride("s1", "strava", noon() + Duration::minutes(4));
        assert_eq!(
            land(
                "strava",
                &[&copy],
                true,
                Some(held.start_date()),
                &[&held, &copy]
            ),
            None,
            "bug: Strava's copy of a held Garmin ride must not land"
        );
    }

    #[test]
    fn the_timed_copy_of_a_held_date_only_scrape_does_not_land() {
        let midnight = Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap();
        let scrape = ride("t1", "sciotte_strava", midnight);
        let api = ride("s1", "strava", midnight + Duration::hours(10));
        assert_eq!(
            land("strava", &[&api], true, Some(midnight), &[&scrape, &api]),
            None,
            "bug: the API's timed copy of a held date-only scrape of the same day must not land"
        );
    }

    #[test]
    fn a_refused_future_row_beside_a_copy_lands_nothing() {
        let held = ride("g1", "garmin", noon());
        let copy = ride("s1", "strava", noon() + Duration::minutes(3));
        let future = ride("s2", "strava", noon() + Duration::days(3));
        // Barrage refused the future row, so the read after the write does
        // not hold it.
        assert_eq!(
            land(
                "strava",
                &[&copy, &future],
                true,
                Some(held.start_date()),
                &[&held, &copy]
            ),
            None,
            "bug: a future-dated row the store refused must not make a copy land"
        );
    }

    #[test]
    fn an_erased_row_not_kept_does_not_land() {
        let held = ride("g1", "garmin", noon());
        let erased = ride("s9", "strava", noon() + Duration::hours(5));
        assert_eq!(
            land(
                "strava",
                &[&erased],
                true,
                Some(held.start_date()),
                &[&held]
            ),
            None,
            "bug: a row the store did not keep must not land"
        );
    }

    #[test]
    fn a_new_ride_after_the_newest_held_lands_under_its_source() {
        let held = ride("g1", "garmin", noon());
        let next = ride("s2", "strava", noon() + Duration::minutes(80));
        assert_eq!(
            land(
                "strava",
                &[&next],
                true,
                Some(held.start_date()),
                &[&held, &next]
            ),
            Some(LandedSession {
                provider: "strava".to_owned(),
                activity_id: "s2".to_owned(),
                start: next.start_date(),
                first_in_store: false,
            }),
            "bug: a distinct ride after the newest held must land, named with its source"
        );
    }

    #[test]
    fn the_newest_new_ride_of_a_write_lands() {
        let held = ride("g1", "garmin", noon());
        let first = ride("s2", "strava", noon() + Duration::hours(3));
        let second = ride("s3", "strava", noon() + Duration::hours(6));
        let landed = land(
            "strava",
            &[&first, &second],
            true,
            Some(held.start_date()),
            &[&held, &first, &second],
        )
        .expect("bug: a write of two new rides must land one");
        assert_eq!(
            landed.activity_id, "s3",
            "bug: the newest new ride must be the one landed"
        );
    }

    #[test]
    fn no_newer_session_never_lands() {
        let held = ride("g1", "garmin", noon());
        let next = ride("s2", "strava", noon() + Duration::hours(2));
        assert_eq!(
            land(
                "strava",
                &[&next],
                false,
                Some(held.start_date()),
                &[&held, &next]
            ),
            None,
            "bug: no newer session must never land"
        );
    }

    #[test]
    fn an_api_copy_sharing_a_held_scrapes_id_does_not_land() {
        let midnight = Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap();
        // A scrape of Strava keeps Strava's own id, under the sciotte key.
        let scrape = ride("1234567", "sciotte", midnight);
        let api = ride("1234567", "strava", midnight + Duration::hours(10));
        assert_eq!(
            land("strava", &[&api], true, Some(midnight), &[&scrape, &api]),
            None,
            "bug: the API copy of a held scrape with the same id must not land"
        );
        let run = ActivityBuilder::new(
            "1234567",
            "Run",
            SportType::Run,
            midnight + Duration::hours(20),
            1_800,
            "strava",
        )
        .build();
        assert_eq!(
            land("strava", &[&run], true, Some(midnight), &[&scrape, &run]),
            None,
            "bug: an id another source's held row carries must fail closed"
        );
    }
}
