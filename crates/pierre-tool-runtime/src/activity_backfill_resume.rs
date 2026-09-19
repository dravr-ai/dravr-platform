// ABOUTME: The sweep that re-runs activity backfills an instance died holding — claims lapsed job rows and spawns them here
// ABOUTME: One periodic worker on the shared ledger loop; exhausted rows are reaped so a fresh ask can record again

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Resume sweep for recorded activity backfills.
//!
//! [`crate::activity_backfill::spawn_activity_backfill`] writes a job row
//! before it spawns and leases it to the instance that runs it. When that
//! instance is reclaimed mid-scrape the lease lapses and the row is all that
//! is left of the athlete's ask. [`resume_backfill_jobs`] is the other half:
//! every [`SWEEP_INTERVAL`] it claims the rows whose lease lapsed, rebuilds
//! each job from what the row kept — the window, the provider, the
//! conversation the notice goes back to — and runs it through the same path
//! the spawn used, so the finished notice reaches the athlete from whichever
//! instance is alive. A job that keeps dying is offered at most
//! [`MAX_BACKFILL_ATTEMPTS`] times; a row past that whose final runner died
//! too is reaped, so the pair's next ask records a fresh job instead of being
//! refused as already owed.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_database::repositories::{
    ActivityBackfillJobRepository, ActivityBackfillJobRow, BackfillJobClaim, WorkerRunRepository,
};
use pierre_providers::backend_resolver;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::periodic::spawn_periodic;
use tracing::{info, warn};

use crate::activity_backfill::{
    provider_tenant_id_str, resume_recorded_backfill, ActivityBackfillJob, BACKFILL_JOB_LEASE,
    MAX_BACKFILL_ATTEMPTS,
};
use crate::runtime::ToolRuntime;

/// How often the sweep looks for lapsed rows.
pub const SWEEP_INTERVAL: Duration = Duration::from_mins(1);

/// How old a row must be before the sweep considers it at all.
///
/// A row is written a moment before its own spawn takes it, on the instance
/// that wrote it; the sweep must not race that start.
const QUEUED_GRACE: Duration = Duration::from_mins(2);

/// Upper bound on jobs one pass claims. Each is a provider scrape that
/// takes the provider's global concurrency permit, so a handful per minute
/// is plenty and a burst never stampedes Chrome.
const SWEEP_BATCH: i64 = 5;

/// Claim the backfills whose lease lapsed and run them on this instance.
///
/// Returns how many this pass started. Rows past the attempt cap whose
/// lease has ended are reaped first — nothing will claim them again, and
/// while they stand every new ask for their pair is refused as a duplicate.
///
/// # Errors
///
/// Returns the ledger's error when the claim itself fails; a reap failure is
/// logged and the pass goes on.
pub async fn resume_backfill_jobs(resources: Arc<dyn ToolRuntime>) -> AppResult<usize> {
    let ledger = Arc::clone(&resources.repos().activity_backfill_jobs);
    let now_ms = Utc::now().timestamp_millis();
    reap_exhausted_jobs(ledger.as_ref(), now_ms).await;

    let claim = BackfillJobClaim {
        now_ms,
        queued_older_than_ms: duration_ms(QUEUED_GRACE),
        lease_ms: duration_ms(BACKFILL_JOB_LEASE),
        max_attempts: MAX_BACKFILL_ATTEMPTS,
        limit: SWEEP_BATCH,
    };
    let rows = ledger.claim_stale_backfill_jobs(claim).await?;

    let mut resumed = 0;
    for row in rows {
        let (row_id, attempts) = (row.id.clone(), row.attempts);
        let job = job_from_row(&resources, row).await;
        if resume_recorded_backfill(job, row_id, attempts) {
            resumed += 1;
        }
    }
    Ok(resumed)
}

/// Drop the rows past the attempt cap whose last runner died too.
async fn reap_exhausted_jobs(ledger: &dyn ActivityBackfillJobRepository, now_ms: i64) {
    match ledger
        .reap_exhausted_backfill_jobs(now_ms, MAX_BACKFILL_ATTEMPTS)
        .await
    {
        Ok(0) => {}
        Ok(reaped) => warn!(
            reaped,
            "Activity backfill resume: dropped jobs past the attempt cap whose last runner died; each athlete's next ask starts over"
        ),
        Err(e) => warn!(error = %e, "Activity backfill resume: exhausted jobs could not be reaped"),
    }
}

/// Rebuild the job a claimed row stands for.
///
/// The row kept the backend the ask resolved to; resolving again from the
/// user-facing name is what the ask itself did, so an athlete who
/// reconnected through a different backend since is served by the one they
/// hold now rather than a dead session.
async fn job_from_row(
    resources: &Arc<dyn ToolRuntime>,
    row: ActivityBackfillJobRow,
) -> ActivityBackfillJob {
    let provider_name = backend_resolver::resolve_backend(
        &resources.repos().auth_repos(),
        row.user_id,
        (!row.tenant_id.is_nil()).then_some(row.tenant_id),
        backend_resolver::user_facing_name(&row.provider),
    )
    .await;
    info!(
        user_id = %row.user_id,
        provider = %provider_name,
        row_id = %row.id,
        attempts = row.attempts,
        after_ts = ?row.after_ts,
        "Activity backfill resume: re-running a backfill an instance left behind"
    );
    ActivityBackfillJob {
        resources: Arc::clone(resources),
        user_id: row.user_id,
        tenant_id: row.tenant_id,
        tenant_id_str: provider_tenant_id_str(row.tenant_id),
        provider_name,
        query_params: ActivityQueryParams {
            limit: row
                .fetch_limit
                .and_then(|limit| usize::try_from(limit).ok()),
            offset: None,
            before: row.before_ts,
            after: row.after_ts,
        },
        pierre_conversation_id: row.conversation_id,
    }
}

/// Start the resume sweep: one pass every [`SWEEP_INTERVAL`] for the life of
/// the process.
///
/// It runs on the shared worker ledger, so a due pass runs once across
/// instances and a fresh instance picks the schedule up where the last left
/// it.
pub fn start_activity_backfill_resume(
    resources: Arc<dyn ToolRuntime>,
    ledger: Arc<dyn WorkerRunRepository>,
) {
    spawn_periodic(
        "activity backfill resume",
        SWEEP_INTERVAL,
        ledger,
        move || {
            let resources = Arc::clone(&resources);
            async move {
                let resumed = resume_backfill_jobs(resources).await?;
                if resumed > 0 {
                    info!(
                        resumed,
                        "Activity backfill resume: sweep took over backfills"
                    );
                }
                Ok(())
            }
        },
    );
}

fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
