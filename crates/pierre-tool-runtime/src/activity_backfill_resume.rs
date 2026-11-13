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
    ActivityBackfillJobRepository, ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind,
    JobLease, WorkerRunRepository,
};
use pierre_providers::backend_resolver;
use pierre_providers::core::ActivityQueryParams;
use pierre_services::periodic::spawn_periodic;
use tracing::{info, warn};

use crate::activity_backfill::history_walk::resolve_purged_walk;
use crate::activity_backfill::{
    answer_reaped_live_read, provider_tenant_id_str, resume_recorded_backfill, ActivityBackfillJob,
    BACKFILL_JOB_LEASE, MAX_BACKFILL_ATTEMPTS,
};
use crate::runtime::{ToolRuntime, WalkTrigger};

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
/// History walks a purge left owed a reopen are resolved next, so a walk
/// reopened here is claimed in the same pass.
///
/// # Errors
///
/// Returns the ledger's error when the claim itself fails; a reap failure is
/// logged and the pass goes on.
pub async fn resume_backfill_jobs(resources: Arc<dyn ToolRuntime>) -> AppResult<usize> {
    let ledger = Arc::clone(&resources.repos().activity_backfill_jobs);
    let now_ms = Utc::now().timestamp_millis();
    reap_exhausted_jobs(&resources, now_ms).await;
    resolve_purged_walks(&resources, now_ms).await;

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
        // The claim wrote a token on every row it returned; a row without one
        // was not leased to this sweep and is not this sweep's to run.
        let Some(token) = row.lease_token.clone() else {
            continue;
        };
        let lease = JobLease {
            id: row.id.clone(),
            token,
        };
        let attempts = row.attempts;
        let job = job_from_row(&resources, row).await;
        if resume_recorded_backfill(job, lease.clone(), attempts) {
            resumed += 1;
        } else {
            // A run of the job's kind for the pair is in flight here, the
            // ask's own spawn still running, say. Waiting on it must not spend
            // the job's attempts, or the sweep reaps it before that run ends.
            release_claim(ledger.as_ref(), &lease).await;
        }
    }
    Ok(resumed)
}

/// Hand back a claim this sweep could not run, with its attempt. A release
/// that fails is logged: the claim's lease lapses as it would have.
async fn release_claim(ledger: &dyn ActivityBackfillJobRepository, lease: &JobLease) {
    if let Err(e) = ledger.release_backfill_job_claim(lease).await {
        warn!(row_id = %lease, error = %e, "Activity backfill resume: a claim could not be handed back");
    }
}

/// Resolve the history walks a purge left owed the reopen of their API
/// phase whose step stopped without resolving them: it died, or its run
/// ended on a write that failed. Each is reopened below its session's own
/// rows when a grant is live — this pass's claim then takes it — or kept as
/// walked when none is ([`resolve_purged_walk`]). A row a step still holds
/// waits for its lease to lapse.
async fn resolve_purged_walks(resources: &Arc<dyn ToolRuntime>, now_ms: i64) {
    let purged = match resources
        .repos()
        .activity_backfill_jobs
        .purged_history_walks(now_ms, SWEEP_BATCH)
        .await
    {
        Ok(purged) => purged,
        Err(e) => {
            warn!(error = %e, "Activity backfill resume: purged history walks could not be read");
            return;
        }
    };
    for walk in purged {
        resolve_purged_walk(
            resources,
            walk.user_id,
            walk.tenant_id,
            &walk.provider,
            WalkTrigger::PhaseReset,
        )
        .await;
    }
}

/// Drop the rows past the attempt cap whose last runner died too. A live
/// read among them still owes the ask it was bound to its follow-up, as a
/// read that gave up: no run of it will ever answer it now.
async fn reap_exhausted_jobs(resources: &Arc<dyn ToolRuntime>, now_ms: i64) {
    let reaped = match resources
        .repos()
        .activity_backfill_jobs
        .reap_exhausted_backfill_jobs(now_ms, MAX_BACKFILL_ATTEMPTS)
        .await
    {
        Ok(reaped) => reaped,
        Err(e) => {
            warn!(error = %e, "Activity backfill resume: exhausted jobs could not be reaped");
            return;
        }
    };
    if reaped.is_empty() {
        return;
    }
    warn!(
        reaped = reaped.len(),
        "Activity backfill resume: dropped jobs past the attempt cap whose last runner died; each athlete's next ask starts over"
    );
    for row in reaped {
        if row.kind == BackfillJobKind::LiveRead && row.conversation_id.is_some() {
            let job = job_from_row(resources, row).await;
            answer_reaped_live_read(job);
        }
    }
}

/// Rebuild the job a claimed row stands for.
///
/// The row kept the backend the ask resolved to; resolving again from the
/// user-facing name is what the ask itself did, so an athlete who
/// reconnected through a different backend since is served by the one they
/// hold now rather than a dead session. A history walk's row keeps the
/// source it names: the walk reads the source its own order chose
/// (`history_source_order`, carnet#1482), which the live-read plan a
/// resolution reads does not follow — its session, or the API grant its
/// phase handed off to (`walk_backend`).
async fn job_from_row(
    resources: &Arc<dyn ToolRuntime>,
    row: ActivityBackfillJobRow,
) -> ActivityBackfillJob {
    let provider_name = if row.kind == BackfillJobKind::HistoryWalk {
        row.walk_backend
            .clone()
            .unwrap_or_else(|| row.provider.clone())
    } else {
        backend_resolver::resolve_backend(
            resources.repos(),
            row.user_id,
            (!row.tenant_id.is_nil()).then_some(row.tenant_id),
            backend_resolver::user_facing_name(&row.provider),
        )
        .await
    };
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
        kind: row.kind,
        turn_id: row.turn_id,
        asked_message_id: row.asked_message_id,
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
