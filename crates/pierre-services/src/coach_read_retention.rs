// ABOUTME: The athlete's record of their human coach's reads: kept 12 months, listed across their tenants, purged hourly
// ABOUTME: One cross-instance DB sweep on the shared spawn_periodic loop and worker ledger; one listing for every surface (carnet#1445)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach read retention
//!
//! An athlete's record of their human coach's reads of their data
//! (`coach_data_reads`) is kept 12 months, rolling: each pass deletes every
//! read recorded more than [`COACH_READ_RETENTION_MONTHS`] months before it.
//! Every surface that lists the record to the athlete lists it through
//! [`own_coach_reads`].

use std::cmp::Reverse;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Months, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{CoachDataRead, CoachDataReadRepository, WorkerRunRepository};
use pierre_database::RepositoryRegistry;
use tokio::task::AbortHandle;
use tracing::info;
use uuid::Uuid;

use crate::periodic::spawn_periodic;

/// How long an athlete's record keeps a coach's read, in months.
pub const COACH_READ_RETENTION_MONTHS: u32 = 12;

/// Ledger name of the worker.
const WORKER_NAME: &str = "coach read retention";

/// One pass an hour: a read is held at most an hour past its retention.
const PERIOD: Duration = Duration::from_hours(1);

/// Start the hourly purge of coach reads past their retention.
///
/// Returns the worker's abort handle; callers discard it, as a restart
/// re-arms the worker from the ledger.
pub fn start_coach_read_retention_task(
    reads: Arc<dyn CoachDataReadRepository>,
    ledger: Arc<dyn WorkerRunRepository>,
) -> AbortHandle {
    spawn_periodic(WORKER_NAME, PERIOD, ledger, move || {
        let reads = Arc::clone(&reads);
        async move {
            purge_expired_coach_reads(reads.as_ref(), Utc::now())
                .await
                .map(|_| ())
        }
    })
}

/// `athlete`'s own record of their human coach's reads in `group`, newest
/// first, at most `limit`.
///
/// A read is recorded under the athlete's one home tenant at the time of the
/// read, and the athlete's session or chat may sit in another tenant they
/// joined since, so each tenant they belong to is read, each filtered on its
/// own tenant id and on the athlete. Group info on web and phone and
/// `/group reads` list through here, so no surface shows less of the record
/// than another.
///
/// # Errors
///
/// Returns the repository error from the tenant or record reads.
pub async fn own_coach_reads(
    repos: &RepositoryRegistry,
    athlete: Uuid,
    group: Uuid,
    limit: u32,
) -> AppResult<Vec<CoachDataRead>> {
    let mut reads = Vec::new();
    for tenant in repos.tenants.list_for_user(athlete).await? {
        reads.extend(
            repos
                .coach_data_reads
                .list_coach_reads(tenant.id, athlete, group, limit)
                .await?,
        );
    }
    reads.sort_by_key(|read| Reverse(read.read_at));
    reads.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(reads)
}

/// The instant before which a read recorded is past its retention at `now`.
///
/// # Errors
///
/// Returns an internal error when `now` is too close to the start of the
/// calendar to step back the retention.
pub fn coach_read_cutoff(now: DateTime<Utc>) -> AppResult<DateTime<Utc>> {
    now.checked_sub_months(Months::new(COACH_READ_RETENTION_MONTHS))
        .ok_or_else(|| AppError::internal("coach read retention cutoff is out of range"))
}

/// Delete every coach read recorded before its retention at `now`, returning
/// how many went.
///
/// # Errors
///
/// Returns the repository error when the delete fails; `spawn_periodic` logs
/// it and retries the pass.
pub async fn purge_expired_coach_reads(
    reads: &dyn CoachDataReadRepository,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    let purged = reads
        .purge_coach_reads_before(coach_read_cutoff(now)?)
        .await?;
    if purged > 0 {
        info!(
            worker = WORKER_NAME,
            purged, "purged coach reads past their retention"
        );
    }
    Ok(purged)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn the_cutoff_is_twelve_calendar_months_back() {
        let now = Utc.with_ymd_and_hms(2026, 11, 12, 16, 0, 0).unwrap();
        assert_eq!(
            coach_read_cutoff(now).unwrap(),
            Utc.with_ymd_and_hms(2025, 11, 12, 16, 0, 0).unwrap(),
            "bug: a read is kept twelve months to the day"
        );
    }
}
