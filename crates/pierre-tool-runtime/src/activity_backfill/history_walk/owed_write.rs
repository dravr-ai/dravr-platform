// ABOUTME: A walk step lands its rows in the store only while its row is owed under its lease, checked before and after
// ABOUTME: A purge that lands between the two checks gets exactly the step's rows taken back, so none of the purged grant stays

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A step's write to the activity store, against a purge of what it reads.
//!
//! A step reads its page from the provider, then writes it through the store
//! ([`persist_backfill_activities`]); the purge of the grant it reads can
//! land at any point of that. The step checks its row right before the
//! write and lands nothing unless the row is still owed under its own lease.
//! It checks again right after: when a purge took the row in between, the
//! step's write may have landed after the purge deleted the grant's rows, so
//! the step takes back exactly the activities it wrote, through the store
//! ([`LiveActivityStore::discard`](pierre_services::live_activities::LiveActivityStore::discard)),
//! as the purge itself deletes them. A purge that commits after the second
//! check deletes the step's rows itself.
//!
//! Only the activities are taken back: a step that read the list head also
//! stamped the grant's fetch freshness through the write-through, and that
//! mark stays.
//!
//! A walk the ledger could not record holds no row and no lease: nothing
//! purges it, and its rows land as any read's do.

use pierre_core::models::Activity;
use pierre_database::repositories::{BackfillJobState, JobLease, LeasedWalkRow};
use tracing::{info, warn};

use super::super::{persist_backfill_activities, ActivityBackfillJob};
use crate::activity_fetch::write_through::WriteThrough;
use crate::protocol::UniversalExecutor;

/// What writing a step's activities came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landed {
    /// The step's rows landed and stay: the net distinct rows persisted.
    Rows(usize),
    /// The step's row is no longer its own: nothing it read is in the store.
    Stopped,
    /// The write, or a check around it, failed: nothing is recorded, and
    /// nothing the step read stays when the row could not be read after it.
    Failed,
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
    let Ok(before) = leased_row(job, lease).await else {
        return Landed::Failed;
    };
    let Some(row) = before.filter(|row| row.leased_here && row.state == BackfillJobState::Owed)
    else {
        info!(user_id = %job.user_id, provider = %job.provider_name, "History walk: the step's row is no longer its own; nothing it read is written");
        return Landed::Stopped;
    };
    let landed = persist_backfill_activities(job, executor, activities, write).await;
    match leased_row(job, lease).await {
        Ok(after) if !purged(job, &row.provider, after.as_ref()) => {
            landed.map_or(Landed::Failed, Landed::Rows)
        }
        Ok(_) => {
            take_back(job, activities).await;
            Landed::Stopped
        }
        Err(()) => {
            take_back(job, activities).await;
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

/// Whether a purge took the row filed under `key` that the step reading
/// `job.provider_name` checked before its write, given the row `after` it.
///
/// - A step reading the key the row is filed under (a session's phase, or a
///   grant walked alone): its own purge removed the row.
/// - A step of a session's API phase, reading its grant: the grant's purge
///   left the row owed a reopen, or a whole-provider purge settled it as its
///   session walked alone, naming no grant. A row the session's own purge
///   removed says nothing of the grant, whose rows stay.
fn purged(job: &ActivityBackfillJob, key: &str, after: Option<&LeasedWalkRow>) -> bool {
    let reads_own_key = key == job.provider_name;
    match after {
        None => reads_own_key,
        Some(_) if reads_own_key => false,
        Some(row) => {
            row.state == BackfillJobState::PhasePurged
                || row.walk_backend.as_deref() != Some(job.provider_name.as_str())
        }
    }
}

/// Take back from the store exactly the `activities` the step wrote under
/// its source's key, a purge having landed while it wrote.
async fn take_back(job: &ActivityBackfillJob, activities: &[Activity]) {
    let store = job.resources.live_activities();
    let mut taken = 0_usize;
    for activity in activities {
        let id = activity.id();
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
    info!(user_id = %job.user_id, provider = %job.provider_name, taken, "History walk: a purge landed while the step wrote; its rows are taken back");
}
