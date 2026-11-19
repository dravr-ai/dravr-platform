// ABOUTME: A walk step lands its rows in the store only while its row is owed under its lease, checked before and after
// ABOUTME: A purge of the step's key between the checks takes back what it reached; an unreadable second check leaves it to the sweep

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A walk step's write to the activity store, against a purge of what it
//! reads.
//!
//! Every walk kind writes through here: a history walk's page
//! ([`BackfillJobKind::HistoryWalk`](pierre_database::repositories::BackfillJobKind::HistoryWalk)), and the grant's copy of an activity
//! only its session listed, which a detail walk writes before it stores the
//! detail the grant read
//! ([`BackfillJobKind::DetailWalk`](pierre_database::repositories::BackfillJobKind::DetailWalk)). The step's row is
//! the one of its job's kind (`job.kind`) its lease was taken on.
//!
//! A step reads its page from the provider, then writes it through the store
//! ([`persist_backfill_activities`]) under the key it reads
//! (`job.provider_name`); a purge of that key can land at any point of that.
//! The step checks its row right before the write and lands nothing unless
//! the row is still owed under its own lease, and notes, in one read, which
//! of the page's activities the store already holds under that key. It
//! checks its row again right after the write, and takes rows back, through
//! the store
//! ([`LiveActivityStore::discard`](pierre_services::live_activities::LiveActivityStore::discard)),
//! only on positive evidence that the key it wrote under was purged for the
//! athlete in between ([`KeyPurge`]):
//!
//! - **Every activity of the key went** (a disconnect, a revocation, an
//!   ended link, the operator's termination): every activity of the page
//!   goes back, those the store held before the step included. The purge
//!   deleted them, with their samples and gear; what stands of them now the
//!   step's write put back.
//! - **A retention expiry** deleted some of the key's activities and reset
//!   its sync state: only the activities the step added go back. Those the
//!   store held before the write stay, with their samples and gear. So does
//!   one another writer stored under the key between the step's read of the
//!   store's records and its write: the step takes back what it added only
//!   when the store counts its write as inserting (or refusing, as the
//!   athlete's deleted) every row it held no record of. When it counts
//!   fewer, another writer stored some of them in between, the step cannot
//!   tell which, and it takes none of them back.
//!
//! Either way only the rows still carrying the step's write go back
//! ([`LiveActivityStore::discard_written`](pierre_services::live_activities::LiveActivityStore::discard_written)):
//! a copy another writer stored after the write, at a later sync stamp,
//! stays with its samples and gear. The step decides its write's stamp
//! before it writes
//! ([`write_through_stamp`](crate::activity_fetch::write_through::write_through_stamp)),
//! so it knows the stamp its own rows carry.
//!
//! The evidence:
//!
//! - A row still there: a step writing under the key its row is filed
//!   under (a session's phase, or a grant walked alone) was not purged, since
//!   that key's purge deletes the row. A step of a session's API phase,
//!   writing under its grant's key while the row stays filed under the
//!   session's, was purged when the grant's purge left the row owed a
//!   reopen, or the operator's termination settled it as its session walked
//!   alone.
//! - A row that is gone says nothing by itself: the session's own purge
//!   removes it, and so does the reaper of a row whose lease lapsed. Only the
//!   attestation a purge of the step's key writes, for the athlete or for
//!   the whole provider, committed between the step's first check and its
//!   second, the read of it that succeeds (the attestations read at both,
//!   compared), takes rows back, and
//!   names how much. A purge of wellness alone (a wellness consent
//!   withdrawn, a wellness expiry) deleted no activity and takes nothing
//!   back. With none, the page stays and warms the training-history rollup,
//!   and the step ends: its row is no longer there to record it.
//!
//! A purge that commits after the second check deletes the step's rows
//! itself.
//!
//! **A second check that cannot be read.** When the ledger or the
//! attestations cannot be read after the write, the step reads them again
//! after each of [`AFTER_WRITE_RETRY_WAITS`] (250 ms, 1 s, 2 s): a transient
//! error is then decided as above. Still unreadable after those, it takes
//! nothing back and fails. Taking back what it cannot prove purged would
//! delete rides of a key no purge reached. When its row remains, its retry
//! is due a lease later and checks the row again before writing.
//!
//! The write is not left at that: before it, the step recorded it in the
//! ledger's database ([`intent`]): which activities, under which key, at
//! which stamp, and from when the attestations answer for it. A write the
//! step settled, its check read, clears the record. One it could not settle
//! keeps it, and the resume sweep settles it once the attestations read
//! again ([`settle_write_intents`]): a purge of the key attested since the
//! step's first check, and not before it, takes the page back as the step
//! would have; with none the record goes and the page stays. So a purge
//! that committed just before an outage, whose delete ran before the write
//! landed, still gets the page taken back once the outage ends, though the
//! step's row is gone and nothing retries it. A step that cannot record its
//! write writes nothing.
//!
//! Only the activities are taken back: a step that read the list head also
//! stamped the grant's fetch freshness through the write-through, and that
//! mark stays.
//!
//! A walk the ledger could not record holds no row and no lease: nothing
//! purges it, and its rows land as any read's do.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::AppError;
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::{BackfillJobState, JobLease, LeasedWalkRow, PurgeReason};
use tokio::time;
use tracing::{info, warn};
use uuid::Uuid;

use super::{
    persist_backfill_activities, persist_backfill_activities_at, ActivityBackfillJob, Persisted,
};
use crate::activity_fetch::write_through::{write_through_stamp, WriteThrough};
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;

/// The durable record of a step's write, and the sweep that settles one
/// its step left behind
mod intent;
pub(crate) use intent::settle_write_intents;

/// How far behind the step's clock a purging instance's may run, in
/// seconds: the attestations the step compares around its write are read
/// from this long before its first check, so one committed between its
/// checks is seen though its clock ran behind.
const PURGE_CLOCK_SKEW_SECS: i64 = 60;

/// How long a step waits before each retry of its check after its write,
/// when that check could not be read: four reads in all, the last about
/// 3.25 s after the first.
const AFTER_WRITE_RETRY_WAITS: [StdDuration; 3] = [
    StdDuration::from_millis(250),
    StdDuration::from_secs(1),
    StdDuration::from_secs(2),
];

/// What writing a step's activities came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landed {
    /// The step's rows landed and stay: the net distinct rows persisted.
    Rows(usize),
    /// The step's rows landed and stay, but its row went while it wrote
    /// with no purge of its key attested: there is no row to record them on.
    Kept,
    /// The step's row is no longer its own: it records nothing on it, and
    /// whatever of its page a purge of its key reached is not in the store,
    /// save what it added after a retention expiry when another writer
    /// stored some of that beside it.
    Stopped,
    /// The write, or a check around it, failed: nothing is recorded. A check
    /// after the write that stays unreadable through its retries takes
    /// nothing back there and then: the record of the write the step made
    /// before it stays, for the resume sweep to settle.
    Failed,
}

/// What a purge of the key a step wrote under deleted of the athlete's
/// activities under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyPurge {
    /// All of them: the whole page goes back.
    Whole,
    /// Those a retention expiry reached: only what the step added goes back.
    Expired,
}

/// What became of the step's row, and of the key it wrote under, once its
/// write landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterWrite {
    /// The row is still the step's.
    Owed,
    /// The row went, and no purge of the key the step wrote under is
    /// attested: its session's purge alone, or the reaper, took it.
    Gone,
    /// The key the step wrote under was purged for the athlete.
    Purged(KeyPurge),
}

/// Write `activities` through the store for the step holding `lease`, only
/// while its row is still owed under that lease. `None` for a walk the
/// ledger could not record, which writes as any read does.
pub(super) async fn land_owed(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    activities: &[Activity],
    write: WriteThrough<'_>,
    lease: Option<&JobLease>,
) -> Landed {
    let Some(lease) = lease else {
        return persist_backfill_activities(job, executor, activities, write)
            .await
            .map_or(Landed::Failed, |persisted| Landed::Rows(persisted.rows));
    };
    let since = Utc::now() - Duration::seconds(PURGE_CLOCK_SKEW_SECS);
    let Ok(attested_before) = purges_since(job, since).await else {
        return Landed::Failed;
    };
    let Ok(before) = leased_row(job, lease).await else {
        return Landed::Failed;
    };
    let Some(row) = before.filter(|row| row.leased_here && row.state == BackfillJobState::Owed)
    else {
        info!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, "Walk step: the step's row is no longer its own; nothing it read is written");
        return Landed::Stopped;
    };
    let Ok(held) = held_before(job, activities).await else {
        return Landed::Failed;
    };
    let Some(stamp) = write_through_stamp(
        &executor.auth_service,
        job.user_id,
        job.tenant_id,
        &job.provider_name,
        write.read,
        activities,
    )
    .await
    else {
        warn!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, "Walk step: the stamp of its write could not be decided; nothing it read is written");
        return Landed::Failed;
    };
    let attested = Attested {
        since,
        before: &attested_before,
    };
    let written_at = stamp.synced_at();
    let Ok(intent) = intent::record_intent(job, activities, &held, written_at, attested).await
    else {
        return Landed::Failed;
    };
    let landed = persist_backfill_activities_at(job, executor, activities, write, stamp).await;
    let after = check_after_write(job, lease, &row.provider, attested).await;
    let write = OwnWrite {
        activities,
        held: &held,
        written_at,
    };
    let (outcome, settled) = settle(job, &write, landed, after).await;
    if settled {
        intent::clear_intent(job, &intent).await;
    }
    outcome
}

/// What a step wrote: `activities`, at the sync stamp `written_at`, of which
/// the store `held` some before.
struct OwnWrite<'step> {
    activities: &'step [Activity],
    held: &'step Held,
    written_at: DateTime<Utc>,
}

/// The step's check after its write of what became of its row, filed under
/// `key`, read again after each of [`AFTER_WRITE_RETRY_WAITS`] while the
/// ledger or the attestations cannot be read; `Err` once every read failed.
async fn check_after_write(
    job: &ActivityBackfillJob,
    lease: &JobLease,
    key: &str,
    attested: Attested<'_>,
) -> Result<AfterWrite, ()> {
    let mut waits = AFTER_WRITE_RETRY_WAITS.iter();
    loop {
        let read = match leased_row(job, lease).await {
            Ok(after) => after_write(job, key, after.as_ref(), attested).await,
            Err(()) => Err(()),
        };
        match (read, waits.next()) {
            (Ok(after), _) => return Ok(after),
            (Err(()), Some(wait)) => time::sleep(*wait).await,
            (Err(()), None) => return Err(()),
        }
    }
}

/// What the step's `write`, which persisted `landed`, comes to once its row
/// is known `after` it, and whether the write is settled: `false` while a
/// purge may have reached it and its rows are not all known taken back,
/// which leaves its record to the resume sweep.
async fn settle(
    job: &ActivityBackfillJob,
    write: &OwnWrite<'_>,
    landed: Option<Persisted>,
    after: Result<AfterWrite, ()>,
) -> (Landed, bool) {
    match after {
        Ok(AfterWrite::Owed) => (
            landed.map_or(Landed::Failed, |persisted| Landed::Rows(persisted.rows)),
            true,
        ),
        Ok(AfterWrite::Gone) => (
            landed.map_or(Landed::Failed, |persisted| {
                info!(user_id = %job.user_id, provider = %job.provider_name, rows = persisted.rows, "Walk step: the step's row went with no purge of its key; its page stays");
                Landed::Kept
            }),
            true,
        ),
        Ok(AfterWrite::Purged(purge)) => {
            let reach = Reach::from(purge);
            if reach == Reach::Added
                && !inserted_every_one(write.activities, write.held, landed)
            {
                info!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, "Walk step: the step's write could not be shown to have inserted all it added; nothing it added is taken back");
                return (Landed::Stopped, true);
            }
            let reached = reached_ids(write.activities, write.held, reach);
            let taken = take_back(
                &job.resources,
                job.user_id,
                &job.tenant_id,
                &job.provider_name,
                &reached,
                write.written_at,
            )
            .await;
            (Landed::Stopped, taken)
        }
        Err(()) => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, "Walk step: the step's check after its write stayed unreadable through its retries; nothing is taken back now, the resume sweep settles its write, and the step fails");
            (Landed::Failed, false)
        }
    }
}

/// The row of the job's kind `lease` was taken on, as it stands now; `Err`
/// (logged) when the ledger could not be read.
async fn leased_row(
    job: &ActivityBackfillJob,
    lease: &JobLease,
) -> Result<Option<LeasedWalkRow>, ()> {
    job.resources
        .repos()
        .activity_backfill_jobs
        .leased_walk_row(job.user_id, job.tenant_id, lease, job.kind)
        .await
        .map_err(|e| {
            warn!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, error = %e, "Walk step: the step's row could not be read around its write");
        })
}

/// What the store held of a step's page under its key, before its write.
struct Held {
    /// The ids the store keeps a record of, read first.
    recorded: HashSet<String>,
    /// The ids it serves, read after: a row the cache served before the
    /// store held the provider's reads is among them, though not recorded.
    served: HashSet<String>,
}

/// What the store held of `activities` under the step's key, before its
/// write; `Err` (logged) when the store could not be read. The records are
/// read first: a row another writer stores between the two reads is served
/// but was not recorded, so the step's write does not count it inserted
/// and the step takes nothing back ([`inserted_every_one`]).
async fn held_before(job: &ActivityBackfillJob, activities: &[Activity]) -> Result<Held, ()> {
    let store = job.resources.live_activities();
    let read = async {
        let recorded = store
            .recorded_among(job.user_id, job.tenant_id, &job.provider_name, activities)
            .await?;
        let served = store
            .held_among(job.user_id, job.tenant_id, &job.provider_name, activities)
            .await?;
        Ok::<_, AppError>(Held { recorded, served })
    };
    read.await.map_err(|e| {
        warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "Walk step: the store could not be read before the step's write");
    })
}

/// The purges of the step's key attested from `since` on, as read before
/// its first check.
#[derive(Debug, Clone, Copy)]
struct Attested<'step> {
    since: DateTime<Utc>,
    before: &'step [String],
}

/// The reasons of the purges of the step's key that reached the athlete,
/// attested from `since` on; `Err` (logged) when they could not be read.
async fn purges_since(job: &ActivityBackfillJob, since: DateTime<Utc>) -> Result<Vec<String>, ()> {
    job.resources
        .repos()
        .provider_data
        .purge_reasons_since(job.user_id, &job.tenant_id, &job.provider_name, since)
        .await
        .map_err(|e| {
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "Walk step: the purges of the step's key could not be read");
        })
}

/// What became of the row filed under `key`, which the step writing under
/// `job.provider_name` checked before its write, given the row `after` it;
/// a row that is gone is read against the purges `attested` before the
/// check. `Err` (logged) when the attestations could not be read.
async fn after_write(
    job: &ActivityBackfillJob,
    key: &str,
    after: Option<&LeasedWalkRow>,
    attested: Attested<'_>,
) -> Result<AfterWrite, ()> {
    match after {
        Some(_) if key == job.provider_name => Ok(AfterWrite::Owed),
        Some(row)
            if row.state == BackfillJobState::PhasePurged
                || row.walk_backend.as_deref() != Some(job.provider_name.as_str()) =>
        {
            Ok(AfterWrite::Purged(KeyPurge::Whole))
        }
        Some(_) => Ok(AfterWrite::Owed),
        None => {
            let landed_since = purges_since(job, attested.since).await?;
            Ok(purge_of(&unanswered(landed_since, attested.before))
                .map_or(AfterWrite::Gone, AfterWrite::Purged))
        }
    }
}

/// The purge reasons attested since a step's first check, `landed_since`,
/// less those already attested `before` it: the purges the step's write
/// answers to.
fn unanswered(mut landed_since: Vec<String>, before: &[String]) -> Vec<String> {
    for reason in before {
        if let Some(at) = landed_since.iter().position(|seen| seen == reason) {
            landed_since.swap_remove(at);
        }
    }
    landed_since
}

/// The purges that delete wellness alone, never an activity.
const WELLNESS_ONLY: [PurgeReason; 2] = [
    PurgeReason::WellnessConsentWithdrawn,
    PurgeReason::WellnessExpired,
];

/// What the purges attested by `reasons` deleted of the key's activities;
/// `None` when none of them deleted any.
fn purge_of(reasons: &[String]) -> Option<KeyPurge> {
    let mut deleting = reasons
        .iter()
        .filter(|reason| {
            !WELLNESS_ONLY
                .iter()
                .any(|only| only.as_str() == reason.as_str())
        })
        .peekable();
    deleting.peek()?;
    if deleting.all(|reason| reason.as_str() == PurgeReason::CacheExpired.as_str()) {
        Some(KeyPurge::Expired)
    } else {
        Some(KeyPurge::Whole)
    }
}

/// Which of its page's activities a step takes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// All of them.
    Page,
    /// Those the store did not hold under the step's key before its write.
    Added,
}

impl From<KeyPurge> for Reach {
    fn from(purge: KeyPurge) -> Self {
        match purge {
            KeyPurge::Whole => Self::Page,
            KeyPurge::Expired => Self::Added,
        }
    }
}

/// Whether the step's write, which persisted `landed`, inserted every one of
/// `activities` the store held no record of before it, the athlete's deleted
/// ones aside: the store counts as many inserted and refused as there are
/// such ids, so none of them was stored by another writer between the
/// step's read of the records and its write. `false` when the write's
/// counts are unknown.
fn inserted_every_one(activities: &[Activity], held: &Held, landed: Option<Persisted>) -> bool {
    let unrecorded: HashSet<&str> = activities
        .iter()
        .map(Activity::id)
        .filter(|id| !held.recorded.contains(*id))
        .collect();
    landed.is_some_and(|persisted| {
        u64::try_from(unrecorded.len()).ok() == persisted.inserted.checked_add(persisted.refused)
    })
}

/// The ids of `activities` that `reach` names: all of them, or those the
/// store neither served nor recorded before the write (`held`).
fn reached_ids(activities: &[Activity], held: &Held, reach: Reach) -> Vec<String> {
    let mut seen = HashSet::new();
    activities
        .iter()
        .map(Activity::id)
        .filter(|id| {
            reach == Reach::Page || !(held.served.contains(*id) || held.recorded.contains(*id))
        })
        .filter(|id| seen.insert(*id))
        .map(str::to_owned)
        .collect()
}

/// Take back from the store, under `provider` for the athlete, the rows of
/// `ids` still carrying the write stamped `written_at`: a copy another
/// writer stored since, at a later stamp, stays. Returns whether every one
/// was taken back or found not the write's; `false` when the store failed
/// on one (logged).
async fn take_back(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: &TenantId,
    provider: &str,
    ids: &[String],
    written_at: DateTime<Utc>,
) -> bool {
    let store = resources.live_activities();
    let (mut taken, mut spared, mut failed) = (0_usize, 0_usize, 0_usize);
    for id in ids {
        match store
            .discard_written(user_id, *tenant_id, provider, id, written_at)
            .await
        {
            Ok(true) => taken += 1,
            Ok(false) => spared += 1,
            Err(e) => {
                failed += 1;
                warn!(user_id = %user_id, provider = %provider, activity_id = %id, error = %e, "Walk step: a row the step wrote could not be taken back");
            }
        }
    }
    info!(user_id = %user_id, provider = %provider, taken, spared, failed, "Walk step: rows the step wrote are taken back");
    failed == 0
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{TimeZone, Utc};
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_database::repositories::PurgeReason;

    use super::{purge_of, reached_ids, unanswered, Held, KeyPurge, Reach};

    fn ride(id: &str) -> Activity {
        ActivityBuilder::new(
            id,
            "Ride",
            SportType::Ride,
            Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            3_600,
            "strava",
        )
        .build()
    }

    fn ids(of: &[&str]) -> HashSet<String> {
        of.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_page_reaches_every_id_once_and_an_expiry_only_those_the_store_did_not_hold() {
        let page = [ride("1"), ride("2"), ride("3"), ride("4"), ride("1")];
        let held = Held {
            recorded: ids(&["2"]),
            served: ids(&["3"]),
        };
        assert_eq!(
            reached_ids(&page, &held, Reach::Page),
            ["1", "2", "3", "4"],
            "bug: a whole-page take-back misses an id, or names a repeated one twice"
        );
        assert_eq!(
            reached_ids(&page, &held, Reach::Added),
            ["1", "4"],
            "bug: an expiry's take-back reaches a ride the store recorded or served before the write"
        );
    }

    #[test]
    fn only_purges_beyond_those_attested_before_the_first_check_are_answered() {
        let before = [
            PurgeReason::CacheExpired.as_str().to_owned(),
            PurgeReason::WellnessExpired.as_str().to_owned(),
        ];
        let since = reasons(&[
            PurgeReason::CacheExpired,
            PurgeReason::CacheExpired,
            PurgeReason::ProviderRevoked,
        ]);
        assert_eq!(
            unanswered(since, &before),
            reasons(&[PurgeReason::ProviderRevoked, PurgeReason::CacheExpired]),
            "bug: a purge attested before the step's check counted, or one after it did not"
        );
        assert_eq!(
            unanswered(reasons(&[PurgeReason::Termination]), &[]),
            reasons(&[PurgeReason::Termination]),
            "bug: with nothing attested before, every purge since is answered"
        );
    }

    fn reasons(of: &[PurgeReason]) -> Vec<String> {
        of.iter().map(|reason| reason.as_str().to_owned()).collect()
    }

    #[test]
    fn only_activity_purges_count_and_an_expiry_alone_reaches_what_was_added() {
        assert_eq!(
            purge_of(&[]),
            None,
            "bug: no purge attested takes nothing back"
        );
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::WellnessConsentWithdrawn])),
            None,
            "bug: a wellness consent withdrawn deletes no activity"
        );
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::WellnessExpired])),
            None,
            "bug: a wellness expiry deletes no activity"
        );
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::CacheExpired])),
            Some(KeyPurge::Expired),
            "bug: a retention expiry reaches only what the step added"
        );
        assert_eq!(
            purge_of(&reasons(&[
                PurgeReason::WellnessExpired,
                PurgeReason::CacheExpired
            ])),
            Some(KeyPurge::Expired),
            "bug: a wellness purge beside an expiry leaves it an expiry"
        );
        assert_eq!(
            purge_of(&reasons(&[
                PurgeReason::CacheExpired,
                PurgeReason::ProviderRevoked
            ])),
            Some(KeyPurge::Whole),
            "bug: a revocation beside an expiry takes the whole page back"
        );
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::Termination])),
            Some(KeyPurge::Whole),
            "bug: a termination takes the whole page back"
        );
    }
}
