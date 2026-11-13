// ABOUTME: A walk step lands its rows in the store only while its row is owed under its lease, checked before and after
// ABOUTME: Only a purge of the step's own key landing between the two checks gets rows taken back, and only the ones it reached

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A step's write to the activity store, against a purge of what it reads.
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
//!   store held before the write stay, with their samples and gear.
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
//!   second (the attestations read at both, compared), takes rows back, and
//!   names how much. A purge of wellness alone (a wellness consent
//!   withdrawn, a wellness expiry) deleted no activity and takes nothing
//!   back. With none, the page stays and warms the training-history rollup,
//!   and the step ends: its row is no longer there to record it.
//!
//! A purge that commits after the second check deletes the step's rows
//! itself. When the ledger or the attestations cannot be read after the
//! write, nothing is taken back: the step fails, its retry due a lease
//! later, and that attempt's own check before its write decides.
//!
//! Only the activities are taken back: a step that read the list head also
//! stamped the grant's fetch freshness through the write-through, and that
//! mark stays.
//!
//! A walk the ledger could not record holds no row and no lease: nothing
//! purges it, and its rows land as any read's do.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::Activity;
use pierre_database::repositories::{BackfillJobState, JobLease, LeasedWalkRow, PurgeReason};
use tracing::{info, warn};

use super::super::{persist_backfill_activities, ActivityBackfillJob};
use crate::activity_fetch::write_through::WriteThrough;
use crate::protocol::UniversalExecutor;

/// How far behind the step's clock a purging instance's may run, in
/// seconds: the attestations the step compares around its write are read
/// from this long before its first check, so one committed between its
/// checks is seen though its clock ran behind.
const PURGE_CLOCK_SKEW_SECS: i64 = 60;

/// What writing a step's activities came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landed {
    /// The step's rows landed and stay: the net distinct rows persisted.
    Rows(usize),
    /// The step's rows landed and stay, but its row went while it wrote
    /// with no purge of its key attested: there is no row to record them on.
    Kept,
    /// The step's row is no longer its own: it records nothing on it, and
    /// whatever of its page a purge of its key reached is not in the store.
    Stopped,
    /// The write, or a check around it, failed: nothing is recorded, and
    /// nothing is taken back.
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
            .map_or(Landed::Failed, Landed::Rows);
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
        info!(user_id = %job.user_id, provider = %job.provider_name, "History walk: the step's row is no longer its own; nothing it read is written");
        return Landed::Stopped;
    };
    let Ok(held) = held_before(job, activities).await else {
        return Landed::Failed;
    };
    let landed = persist_backfill_activities(job, executor, activities, write).await;
    let after = match leased_row(job, lease).await {
        Ok(after) => {
            let attested = Attested {
                since,
                before: &attested_before,
            };
            after_write(job, &row.provider, after.as_ref(), attested).await
        }
        Err(()) => Err(()),
    };
    settle(job, activities, &held, landed, after).await
}

/// What the step's write of `activities`, which persisted `landed`, comes
/// to once its row is known `after` it; `held` are the ids the store held
/// before the write.
async fn settle(
    job: &ActivityBackfillJob,
    activities: &[Activity],
    held: &HashSet<String>,
    landed: Option<usize>,
    after: Result<AfterWrite, ()>,
) -> Landed {
    match after {
        Ok(AfterWrite::Owed) => landed.map_or(Landed::Failed, Landed::Rows),
        Ok(AfterWrite::Gone) => landed.map_or(Landed::Failed, |rows| {
            info!(user_id = %job.user_id, provider = %job.provider_name, rows, "History walk: the step's row went with no purge of its key; its page stays");
            Landed::Kept
        }),
        Ok(AfterWrite::Purged(purge)) => {
            take_back(job, activities, held, purge).await;
            Landed::Stopped
        }
        Err(()) => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, "History walk: no purge could be ruled out after the step's write; nothing is taken back, and its retry checks again");
            Landed::Failed
        }
    }
}

/// The row `lease` was taken on, as it stands now; `Err` (logged) when the
/// ledger could not be read.
async fn leased_row(
    job: &ActivityBackfillJob,
    lease: &JobLease,
) -> Result<Option<LeasedWalkRow>, ()> {
    job.resources
        .repos()
        .activity_backfill_jobs
        .leased_walk_row(job.user_id, job.tenant_id, lease)
        .await
        .map_err(|e| {
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "History walk: the step's row could not be read around its write");
        })
}

/// The ids among `activities` the store already holds under the step's key,
/// before its write; `Err` (logged) when the store could not be read.
async fn held_before(
    job: &ActivityBackfillJob,
    activities: &[Activity],
) -> Result<HashSet<String>, ()> {
    job.resources
        .live_activities()
        .held_among(job.user_id, job.tenant_id, &job.provider_name, activities)
        .await
        .map_err(|e| {
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "History walk: the store could not be read before the step's write");
        })
}

/// The purges of the step's key attested from `since` on, as read before
/// its first check.
#[derive(Debug, Clone, Copy)]
struct Attested<'a> {
    since: DateTime<Utc>,
    before: &'a [String],
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
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "History walk: the purges of the step's key could not be read");
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
            let mut landed_since = purges_since(job, attested.since).await?;
            for reason in attested.before {
                if let Some(at) = landed_since.iter().position(|seen| seen == reason) {
                    landed_since.swap_remove(at);
                }
            }
            Ok(purge_of(&landed_since).map_or(AfterWrite::Gone, AfterWrite::Purged))
        }
    }
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

/// Take back from the store, under the step's key, what `purge` reached of
/// `activities`: all of them, or those not `held` before the write.
async fn take_back(
    job: &ActivityBackfillJob,
    activities: &[Activity],
    held: &HashSet<String>,
    purge: KeyPurge,
) {
    let store = job.resources.live_activities();
    let mut taken = 0_usize;
    let reached = activities
        .iter()
        .map(Activity::id)
        .filter(|id| purge == KeyPurge::Whole || !held.contains(*id));
    for id in reached {
        match store
            .discard(job.user_id, job.tenant_id, &job.provider_name, id)
            .await
        {
            Ok(_) => taken += 1,
            Err(e) => {
                warn!(user_id = %job.user_id, provider = %job.provider_name, activity_id = id, error = %e, "History walk: a row the step wrote through a purge could not be taken back");
            }
        }
    }
    info!(user_id = %job.user_id, provider = %job.provider_name, taken, ?purge, "History walk: a purge of the step's key landed while it wrote; what it reached is taken back");
}

#[cfg(test)]
mod tests {
    use super::{purge_of, KeyPurge};
    use pierre_database::repositories::PurgeReason;

    fn reasons(of: &[PurgeReason]) -> Vec<String> {
        of.iter().map(|reason| reason.as_str().to_owned()).collect()
    }

    #[test]
    fn only_activity_purges_count_and_an_expiry_alone_reaches_what_was_added() {
        assert_eq!(purge_of(&[]), None);
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::WellnessConsentWithdrawn])),
            None
        );
        assert_eq!(purge_of(&reasons(&[PurgeReason::WellnessExpired])), None);
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::CacheExpired])),
            Some(KeyPurge::Expired)
        );
        assert_eq!(
            purge_of(&reasons(&[
                PurgeReason::WellnessExpired,
                PurgeReason::CacheExpired
            ])),
            Some(KeyPurge::Expired)
        );
        assert_eq!(
            purge_of(&reasons(&[
                PurgeReason::CacheExpired,
                PurgeReason::ProviderRevoked
            ])),
            Some(KeyPurge::Whole)
        );
        assert_eq!(
            purge_of(&reasons(&[PurgeReason::Termination])),
            Some(KeyPurge::Whole)
        );
    }
}
