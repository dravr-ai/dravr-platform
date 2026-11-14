// ABOUTME: One step of a detail walk: read its activities through the backend, store each beside its row, settle the row
// ABOUTME: A spent budget pauses it, a dead credential ends it incomplete, its end is told once in the personal thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::slice;
use std::time::Instant;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::Activity;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::repositories::{DetailWalkEnd, JobLease, JobProgressStep};
use pierre_providers::ai_scope;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::native_activity_key;
use pierre_providers::request_budget::in_detail_walk;
use pierre_providers::CoreFitnessProvider;
use tracing::{info, instrument, warn};

use super::candidates::{take_seconds, Athlete, Unread};
use super::{walk_daily_calls, walk_keys, walk_row};
use crate::activity_backfill::history_walk::retry_at_ms;
use crate::activity_backfill::owed_write::{land_owed, Landed};
use crate::activity_backfill::{
    keep_backfill_lease, lease_ms, notify_backfill_reauth, now_ms, ActivityBackfillJob,
    MAX_BACKFILL_ATTEMPTS,
};
use crate::activity_detail_store::{store_detail_from_live_grant, DetailKept};
use crate::activity_fetch::write_through::WriteThrough;
use crate::protocol::types::auth_required_provider;
use crate::protocol::UniversalExecutor;
use crate::runtime::DetailWalkCompletion;

/// What one step of a detail walk came to.
#[derive(Debug)]
enum Step {
    /// The step read what it took; `ended` once its cursor passed the floor.
    Walked { step: JobProgressStep, ended: bool },
    /// A spent budget stopped the step: until the refusing window ends
    /// (unix ms) when the refusal named its end.
    Paused {
        step: JobProgressStep,
        until_ms: Option<i64>,
    },
    /// The backend's credential is dead.
    AuthRequired,
    /// The backend's budget leaves the walk no call a day.
    NoBudget,
    /// The store, or the backend before any read, failed.
    Failed { step: JobProgressStep },
    /// The step's row is no longer its own: it records nothing.
    Stopped,
}

/// Run one step of the detail walk `job` is, and settle its row. A walk the
/// ledger could not record runs nothing: no step writes without a row to
/// guard its writes by, and the next ask records it again.
#[instrument(
    skip_all,
    name = "detail_walk",
    fields(user_id = %job.user_id, tenant_id = %job.tenant_id, provider = %job.provider_name)
)]
pub(in crate::activity_backfill) async fn run_detail_walk(
    job: &ActivityBackfillJob,
    recorded: Option<(JobLease, i64)>,
) {
    let Some((lease, attempts)) = recorded else {
        warn!(user_id = %job.user_id, provider = %job.provider_name, "Detail walk: the walk could not be recorded; nothing is read until it is asked again");
        return;
    };
    let renewal = keep_backfill_lease(job, &lease);
    let attempt_started_at = Utc::now();
    let step = walk_step(job, &lease).await;
    settle(job, &lease, attempts, step, attempt_started_at).await;
    drop(renewal);
}

/// Where a step stands while it reads: the cursor it started from, the
/// oldest second it finished, and what it counted.
///
/// A ride walked past (the backend could not read it) counts as done only
/// once its whole second is finished: a step stopped part-way through a
/// second leaves the cursor above it, so the next step reads that second
/// again, and with it every ride walked past there. A ride stored there
/// counts at once: it holds its efforts now, and is never read again.
#[derive(Debug, Clone, Copy)]
struct Progress {
    walked_to: i64,
    units_done: i64,
    rows_landed: i64,
    /// Rides walked past in the second being read, not yet finished.
    walked_past: i64,
    started: Instant,
}

impl Progress {
    /// The step as the row records it.
    fn step(self) -> JobProgressStep {
        let secs_per_unit = i32::try_from(self.units_done)
            .ok()
            .filter(|done| *done > 0)
            .map(|done| self.started.elapsed().as_secs_f64() / f64::from(done));
        JobProgressStep {
            before_ts: self.walked_to,
            units_done: self.units_done,
            rows_landed: self.rows_landed,
            secs_per_unit,
            floor: None,
        }
    }
}

/// Read one step of the walk below its cursor, for the row `lease` holds.
async fn walk_step(job: &ActivityBackfillJob, lease: &JobLease) -> Step {
    let floor = job.query_params.after.unwrap_or(0);
    let cursor = job
        .query_params
        .before
        .unwrap_or_else(|| Utc::now().timestamp());
    let mut progress = Progress {
        walked_to: cursor,
        units_done: 0,
        rows_landed: 0,
        walked_past: 0,
        started: Instant::now(),
    };
    if cursor <= floor {
        progress.walked_to = floor;
        return Step::Walked {
            step: progress.step(),
            ended: true,
        };
    }
    let athlete = Athlete {
        runtime: &job.resources,
        user_id: job.user_id,
        tenant_id: job.tenant_id,
    };
    let keys = walk_keys(&job.provider_name);
    // The range an earlier walk tried, which this one reads nothing of
    // again, is on the walk's own row.
    let Ok(row) = walk_row(&athlete, &job.provider_name).await else {
        return Step::Failed {
            step: progress.step(),
        };
    };
    let tried = row.and_then(|row| row.tried);
    let Ok(taken) = take_seconds(
        &athlete,
        &keys,
        (floor, cursor),
        job.walk_sport.as_deref(),
        tried.as_ref(),
    )
    .await
    else {
        return Step::Failed {
            step: progress.step(),
        };
    };
    if taken.seconds.iter().all(|second| second.unread.is_empty()) {
        progress.walked_to = taken.walked_to(floor, cursor);
        return Step::Walked {
            step: progress.step(),
            ended: taken.reached_floor,
        };
    }
    if walk_daily_calls(&athlete, &job.provider_name).await == Some(0) {
        return Step::NoBudget;
    }
    let executor =
        UniversalExecutor::new(job.resources.clone()).with_scopes(OAuthScope::self_grant());
    let tenant = job.tenant_id_str.as_deref();
    let provider = match executor
        .auth_service
        .create_exact_provider(&job.provider_name, job.user_id, tenant)
        .await
    {
        Ok(provider) => provider,
        Err(response) => {
            let auth_required = auth_required_provider(&response).is_some();
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = ?response.error, auth_required, "Detail walk: the backend could not be authenticated");
            return if auth_required {
                Step::AuthRequired
            } else {
                Step::Failed {
                    step: progress.step(),
                }
            };
        }
    };
    let reader = Reader {
        job,
        executor: &executor,
        provider: provider.as_ref(),
        lease,
    };
    for second in &taken.seconds {
        for unread in &second.unread {
            match reader.read(unread).await {
                Read::Stored => {
                    progress.units_done += 1;
                    progress.rows_landed += 1;
                }
                Read::Skipped => progress.walked_past += 1,
                Read::Paused(until_ms) => {
                    return Step::Paused {
                        step: progress.step(),
                        until_ms,
                    };
                }
                Read::AuthRequired => return Step::AuthRequired,
                Read::Failed => {
                    return Step::Failed {
                        step: progress.step(),
                    }
                }
                Read::Stopped => return Step::Stopped,
            }
        }
        progress.walked_to = second.at;
        progress.units_done += progress.walked_past;
        progress.walked_past = 0;
    }
    if taken.reached_floor {
        progress.walked_to = floor;
    }
    Step::Walked {
        step: progress.step(),
        ended: taken.reached_floor,
    }
}

/// What reading one activity's detail came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    /// Its detail is stored beside its row.
    Stored,
    /// The backend answered that it cannot be read ([`answered_unreadable`]),
    /// or no row stands to store it beside: it is walked past.
    Skipped,
    /// A spent budget refused the read.
    Paused(Option<i64>),
    /// The backend's credential is dead.
    AuthRequired,
    /// The store failed, or the backend failed to answer for it: the step
    /// stops, and the activity is read again.
    Failed,
    /// The step's row is no longer its own.
    Stopped,
}

/// What the backend failing to read the activity `id` comes to
/// ([`failure_read`]), logged.
fn read_failed(job: &ActivityBackfillJob, id: &str, e: &AppError) -> Read {
    let read = failure_read(e);
    // Walked past or paused is the walk working as it should; the rest
    // leaves it short.
    if matches!(read, Read::Skipped | Read::Paused(_)) {
        info!(user_id = %job.user_id, provider = %job.provider_name, activity_id = %id, error = %e, ?read, "Detail walk: the backend did not read an activity");
    } else {
        warn!(user_id = %job.user_id, provider = %job.provider_name, activity_id = %id, error = %e, ?read, "Detail walk: the backend failed to read an activity");
    }
    read
}

/// What a failed detail read comes to: a spent budget pauses the walk, a
/// dead credential ends it, an answer that the activity cannot be read
/// walks past it ([`answered_unreadable`]), and any other failure — the
/// backend down, unreachable, its breaker open or its answer undecodable —
/// says nothing about the activity, so the step stops at it and the walk
/// reads it again.
fn failure_read(e: &AppError) -> Read {
    if e.code == ErrorCode::ExternalRateLimited {
        Read::Paused(retry_at_ms(e))
    } else if e.provider_auth_required_provider().is_some() {
        Read::AuthRequired
    } else if answered_unreadable(e) {
        Read::Skipped
    } else {
        Read::Failed
    }
}

/// Whether `error` is the backend's answer that this one activity cannot
/// be read: it is missing (deleted, made private), or the backend refuses
/// its id as malformed. A walk walks past such an activity and never asks
/// for it again. Any other failure says nothing about the activity — the
/// backend down, a request that never reached it, an open circuit breaker
/// — so the step stops at it ([`Read::Failed`]) and the walk reads it again.
const fn answered_unreadable(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::ResourceNotFound | ErrorCode::InvalidInput | ErrorCode::InvalidFormat
    )
}

/// What one step reads through.
struct Reader<'step> {
    job: &'step ActivityBackfillJob,
    executor: &'step UniversalExecutor,
    provider: &'step dyn CoreFitnessProvider,
    lease: &'step JobLease,
}

impl Reader<'_> {
    /// Read one activity's detail through the backend and store it beside
    /// its row.
    async fn read(&self, unread: &Unread) -> Read {
        let job = self.job;
        let Some(first) = unread.copies.first() else {
            return Read::Skipped;
        };
        // The backend's own copy when it holds one, the session's otherwise:
        // the grant's answer for it is kept under the grant either way.
        let listed = unread
            .copies
            .iter()
            .find(|copy| copy.provider == job.provider_name)
            .unwrap_or(first);
        let id = native_activity_key(&listed.activity)
            .map_or_else(|| listed.activity.id().to_owned(), |(_, id)| id.to_owned());
        let fetched = in_detail_walk(
            job.user_id,
            ai_scope::unfiltered(self.provider.get_activity(&id)),
        )
        .await;
        match fetched {
            Ok(detailed) => self.store(&listed.provider, &detailed).await,
            Err(e) => read_failed(job, &id, &e),
        }
    }

    /// Store `detailed` beside the row it details, listed under `listed`;
    /// the grant's copy, when it must be written first, through the walk's
    /// owed-write guard.
    async fn store(&self, listed: &str, detailed: &Activity) -> Read {
        let job = self.job;
        let start = detailed.start_date().timestamp();
        // A read of the one activity, never of the list head: the copy moves
        // no freshness, and carries the head's last sync, or this activity's
        // start when the grant has none.
        let read = ActivityQueryParams {
            after: Some(start.saturating_sub(1)),
            before: Some(start.saturating_add(1)),
            limit: Some(1),
            offset: None,
        };
        let mut guarded: Option<Landed> = None;
        let copy = async {
            let write = WriteThrough {
                read: &read,
                announces_sessions: false,
            };
            let landed = land_owed(
                job,
                self.executor,
                slice::from_ref(detailed),
                write,
                Some(self.lease),
            )
            .await;
            guarded = Some(landed);
            match landed {
                Landed::Rows(rows) => Ok(u64::try_from(rows).unwrap_or(0)),
                Landed::Kept | Landed::Stopped | Landed::Failed => Ok(0),
            }
        };
        let kept = store_detail_from_live_grant(
            job.resources.repos(),
            job.tenant_id,
            job.user_id,
            listed,
            detailed,
            copy,
        )
        .await;
        match (kept, guarded) {
            (_, Some(Landed::Kept | Landed::Stopped)) => Read::Stopped,
            (_, Some(Landed::Failed)) | (DetailKept::Failed, _) => Read::Failed,
            (DetailKept::Stored, _) => Read::Stored,
            (DetailKept::NotCached, _) => Read::Skipped,
        }
    }
}

/// What `step` does to the walk's row.
async fn settle(
    job: &ActivityBackfillJob,
    lease: &JobLease,
    attempts: i64,
    step: Step,
    attempt_started_at: DateTime<Utc>,
) {
    let ledger = &job.resources.repos().activity_backfill_jobs;
    match step {
        Step::Walked { step, ended: true } => end_walk(job, lease, step).await,
        Step::Walked { step, ended: false } => log_write(
            "a step landed; the next reads below it",
            ledger.record_job_progress(lease, step).await,
            job,
        ),
        // The activities read before the refusal are recorded, which
        // releases the row: the next claim meets the refusal and pauses.
        Step::Paused { step, .. } if moved(&step, job) => log_write(
            "a budget refused the step part-way; what it read is recorded",
            ledger.record_job_progress(lease, step).await,
            job,
        ),
        Step::Paused { until_ms, .. } => log_write(
            "a budget refused the step; it resumes once the window ends",
            ledger.release_backfill_job_claim(lease, until_ms).await,
            job,
        ),
        Step::AuthRequired => {
            notify_backfill_reauth(job, attempt_started_at).await;
            log_write(
                "the backend's credential is dead; the walk ends incomplete",
                ledger.end_backfill_job_incomplete(lease).await,
                job,
            );
        }
        // Its app's day shrank to no call for a walk: a pause would last
        // forever.
        Step::NoBudget => log_write(
            "the backend's budget leaves the walk no call a day; the walk ends incomplete",
            ledger.end_backfill_job_incomplete(lease).await,
            job,
        ),
        Step::Failed { step } if moved(&step, job) => log_write(
            "a step failed part-way; what it read is recorded",
            ledger.record_job_progress(lease, step).await,
            job,
        ),
        Step::Failed { .. } if attempts >= MAX_BACKFILL_ATTEMPTS => log_write(
            "failed on its last attempt; the walk ends incomplete",
            ledger.end_backfill_job_incomplete(lease).await,
            job,
        ),
        Step::Failed { .. } => log_write(
            "a step failed; the sweep retries it a lease from now",
            ledger
                .release_backfill_job_lease(lease, now_ms().saturating_add(lease_ms()))
                .await,
            job,
        ),
        Step::Stopped => log_write(
            "the step's row is no longer its own; giving its lease up",
            ledger.renew_backfill_job_lease(lease, 0).await,
            job,
        ),
    }
}

/// Whether `step` moved the walk: read an activity, or moved its cursor.
fn moved(step: &JobProgressStep, job: &ActivityBackfillJob) -> bool {
    step.units_done > 0 || Some(step.before_ts) != job.query_params.before
}

/// End the walk on its last `step`, which passed the floor it claimed, and
/// tell the athlete once. An ask that widened the floor meanwhile keeps the
/// walk owed: the step is recorded and the next one walks on down.
async fn end_walk(job: &ActivityBackfillJob, lease: &JobLease, step: JobProgressStep) {
    let ledger = &job.resources.repos().activity_backfill_jobs;
    let end = DetailWalkEnd {
        before_ts: step.before_ts,
        units_done: step.units_done,
        rows_landed: step.rows_landed,
        secs_per_unit: step.secs_per_unit,
        walked_floor: job.query_params.after,
        ended_at_ms: now_ms(),
    };
    match ledger.end_detail_walk(lease, &end).await {
        Ok(Some(ended)) => {
            info!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                read = ended.progress.units_done,
                stored = ended.progress.rows_landed,
                "Detail walk: reached its floor"
            );
            announce_walked(job, ended.created_at_ms, ended.progress.rows_landed).await;
        }
        Ok(None) => log_write(
            "the floor moved under the last step; the walk goes on below it",
            ledger.record_job_progress(lease, step).await,
            job,
        ),
        Err(e) => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "Detail walk: the walk's end could not be written; the sweep retries the step");
        }
    }
}

/// Tell the athlete the walk asked at `asked_at_ms` ended having stored the
/// efforts of `stored` activities. A walk that stored none says nothing.
async fn announce_walked(job: &ActivityBackfillJob, asked_at_ms: i64, stored: i64) {
    let Some(notifier) = job.resources.backfill_notifier() else {
        return;
    };
    let activity_count = usize::try_from(stored).unwrap_or(0);
    if activity_count == 0 {
        info!(user_id = %job.user_id, provider = %job.provider_name, "Detail walk: ended having stored nothing; nothing to announce");
        return;
    }
    notifier
        .push_detail_walk_complete(DetailWalkCompletion {
            user_id: job.user_id,
            tenant_id: job.tenant_id,
            provider: &job.provider_name,
            asked_at_ms,
            activity_count,
        })
        .await;
}

/// Log how writing a step's settlement to the row went.
fn log_write(what: &str, written: AppResult<bool>, job: &ActivityBackfillJob) {
    let (user_id, provider) = (job.user_id, job.provider_name.as_str());
    match written {
        Ok(true) => info!(user_id = %user_id, provider, what, "Detail walk: step settled"),
        Ok(false) => {
            warn!(user_id = %user_id, provider, what, "Detail walk: the row is no longer this run's");
        }
        Err(e) => {
            warn!(user_id = %user_id, provider, what, error = %e, "Detail walk: the row could not be written");
        }
    }
}
