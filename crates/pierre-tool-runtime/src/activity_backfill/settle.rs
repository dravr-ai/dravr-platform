// ABOUTME: What a recorded backfill's run does to its job row: finished, dropped on its last attempt, or left to the sweep
// ABOUTME: A run a request budget refused is released with its attempt returned, due once the refusing window ends

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_database::repositories::JobLease;
use tracing::{info, warn};

use super::{ActivityBackfillJob, BackfillRunOutcome, MAX_BACKFILL_ATTEMPTS};

/// What the run's outcome does to its row: finished on an outcome the
/// athlete is told about, left to the sweep on a transient failure — unless
/// this was the job's last attempt, in which case the row is dropped so the
/// athlete's next ask can record a fresh one instead of being told forever
/// that this one is owed. A run a request budget refused (a session-read cap
/// included, carnet#1661) is released with its attempt returned, due once
/// the refusing window ends.
pub(super) async fn settle_backfill_job(
    job: &ActivityBackfillJob,
    row_id: &JobLease,
    attempts: i64,
    outcome: &BackfillRunOutcome,
) {
    match outcome {
        BackfillRunOutcome::Completed | BackfillRunOutcome::AuthRequired => {
            remove_backfill_job(job, row_id, "finished").await;
        }
        BackfillRunOutcome::Failed if attempts >= MAX_BACKFILL_ATTEMPTS => {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                row_id = %row_id,
                attempts,
                "Activity backfill: failed on its last attempt; dropping the job — the athlete's next ask starts a fresh one"
            );
            remove_backfill_job(job, row_id, "exhausted").await;
        }
        BackfillRunOutcome::Failed => info!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            row_id = %row_id,
            attempts,
            "Activity backfill: failed; leaving the job for the resume sweep once its lease lapses"
        ),
        // Waiting out a spent budget never spends one of the job's attempts.
        BackfillRunOutcome::Paused(until_ms) => wait_out(job, row_id, *until_ms).await,
    }
}

/// Delete the job's row. A delete the ledger refuses is logged and left: the
/// row's lease lapses, and the sweep either re-runs it or reaps it.
pub(super) async fn remove_backfill_job(job: &ActivityBackfillJob, row_id: &JobLease, why: &str) {
    if let Err(e) = job
        .resources
        .repos()
        .activity_backfill_jobs
        .finish_backfill_job(row_id)
        .await
    {
        warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            row_id = %row_id,
            why,
            error = %e,
            "Activity backfill: job row could not be removed from the ledger; the sweep sees it again"
        );
    }
}

/// Release a run a request budget refused, its attempt returned, due again
/// at `until_ms` (unix ms), so waiting out a spent budget never spends one of
/// the job's attempts. A release the ledger refuses is logged: the row's
/// lease lapses, and the sweep re-runs it then.
pub(super) async fn wait_out(job: &ActivityBackfillJob, row_id: &JobLease, until_ms: i64) {
    let ledger = &job.resources.repos().activity_backfill_jobs;
    match ledger
        .release_backfill_job_claim(row_id, Some(until_ms))
        .await
    {
        Ok(released) => info!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            row_id = %row_id,
            kind = ?job.kind,
            released,
            "Activity backfill: a request budget refused the run; its row waits for the window to end"
        ),
        Err(e) => warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            row_id = %row_id,
            error = %e,
            "Activity backfill: a paused job could not be released; its lease lapses"
        ),
    }
}
