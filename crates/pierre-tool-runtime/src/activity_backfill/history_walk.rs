// ABOUTME: The history walk: one durable job per athlete and source that pages their whole history into the store
// ABOUTME: Session first, 36 months a step, then its API grant below what it reached; ends at the floor, then says so once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History walks (carnet#1482).
//!
//! An athlete who connects a provider has years behind them that no chat
//! turn should wait on. A walk copies them once, newest first, into the
//! activity store: a [`BackfillJobKind::HistoryWalk`] job, recorded before it
//! runs, leased while it runs, resumed by the sweep if its instance dies.
//!
//! **Which source.** The first live, pageable source of
//! [`walk_source`]'s one order: the usr/pwd session before the grant (D2),
//! since one scrape reads three years where an API pages two hundred
//! activities, even while a grant is alive (carnet#1482). The source is read
//! exactly as named ([`crate::protocol::AuthService::create_exact_provider`]),
//! never resolved to the backend the live-read plan serves, and a session
//! stays after the walk (B1): the next walk reads it again. A push is never
//! walked.
//!
//! **Two phases, one row.** Once the session finds nothing more, a live API
//! grant beside it ([`walk_hand_off`]) pages strictly below the oldest row
//! the session stored, under any history key: what the scraper never listed
//! is still the API's. The hand-off is written on the row before the API is
//! asked anything ([`WalkHandOff`]), so a restart or a budget pause resumes
//! the API phase. The row keeps the session's key and stays owed through
//! both phases, so every reader of the walk sees one walk until its very
//! end, its progress covering both. A session walked alone while no grant
//! stood beside it is reopened for the API phase when one connects
//! ([`start_history_walk`]); its end was told, so the API phase tells only
//! what it brings in, and nothing when that is nothing.
//!
//! **One step per run.** A run reads one step below the row's cursor
//! (`before_ts`), writes it through the activity store, and records the step
//! on the row ([`JobProgress`]): the months it covered, the rows it landed,
//! the seconds a month took. The row is then released and the resume sweep
//! runs the next step, so no run outlives its lease and a dead instance
//! costs one step.
//!
//! - A session step reads [`SCIOTTE_WALK_STEP_MONTHS`] in one request,
//!   bounded by [`SCIOTTE_STEP_TIMEOUT`], under the scraper's own deadline.
//! - An API step reads a year at a time, [`API_WALK_PAGE`] activities a
//!   page. A budget refusal pauses the walk: the claim is handed back with
//!   its cursor and its attempt, and the sweep resumes it in a later window.
//!
//! **Where it ends (B3).** At the floor: a Strava account's creation, which
//! the first API step reads from the athlete's profile, and never below
//! [`WHOLE_ACCOUNT_AFTER`]. Before it, a session step that finds nothing ends
//! the session's phase (and the walk, with no grant beside it), and an API
//! walk ends after two consecutive empty years. The row is then kept as
//! [`pierre_database::repositories::BackfillJobState::Walked`], which a later
//! depth ask reads as done, and the athlete is told once, in their personal
//! thread, how far back their history now reaches (B4).
//!
//! A session walked beside a push-fed grant (Garmin) also copies the year of
//! wellness before the grant once its activities are in ([`wellness`]).

use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Datelike, Utc};
use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Activity, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::repositories::{
    BackfillCoverage, BackfillJobKind, JobFloor, JobLease, JobProgressStep, WalkHandOff,
};
use pierre_providers::ai_scope;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::CoreFitnessProvider;
use pierre_services::training_history_read::{
    history_sources_for, history_store_keys, walk_across_pair, walk_hand_off, walk_source,
    HistorySourceRole, PairWalk,
};
use tokio::time::timeout;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use super::{
    capture_with_head, keep_backfill_lease, notify_backfill_reauth, persist_backfill_activities,
    provider_tenant_id_str, spawn_activity_backfill, warm_training_history, ActivityBackfillJob,
    BackfillFetch, MAX_BACKFILL_ATTEMPTS,
};
use crate::activity_fetch::write_through::WriteThrough;
use crate::implementations::data_helpers::historical_backfill_fetch_limit;
use crate::protocol::types::auth_required_provider;
use crate::protocol::UniversalExecutor;
use crate::runtime::{HistoryWalkCompletion, ToolRuntime, WalkTrigger};

/// What a walk copies on top of the activities: the wellness before a push grant
mod wellness;

/// The hard floor of every walk, in unix seconds: 2000-01-01T00:00:00Z,
/// older than any account a walk reads. A walk whose source names no floor
/// of its own ends before it, at the first empty step.
pub const WHOLE_ACCOUNT_AFTER: i64 = 946_684_800;

/// Months one session step reads in one request. Three years of a busy
/// athlete is a few hundred list rows, which the scraper reads well inside
/// its request deadline with the detail pass off.
pub const SCIOTTE_WALK_STEP_MONTHS: i64 = 36;

/// How long one session step may take: under the scraper service's own
/// 320 s request deadline, so the step fails here, cleanly, before the
/// service gives up on it.
pub const SCIOTTE_STEP_TIMEOUT: StdDuration = StdDuration::from_secs(300);

/// Months one API window spans.
const API_WALK_STEP_MONTHS: i64 = 12;

/// Activities one API page asks for: Strava's largest page.
pub const API_WALK_PAGE: usize = 200;

/// Consecutive empty API windows that end a walk with no floor of its own.
const EMPTY_WINDOWS_ENDING_A_WALK: u32 = 2;

/// The mean Gregorian month in seconds (365.2425 days / 12): how a step's
/// span is counted in the months the row reports.
const SECS_PER_MONTH: i64 = 2_629_746;

/// Start a walk of `provider`'s history for the user.
///
/// It reads the provider's first live, pageable source ([`walk_source`]).
/// Returns whether a walk was started; `false` when the user holds no such
/// source, or a walk of it is already owed, running, or reached the end of
/// the history, under its key or any other of its provider's
/// ([`walk_across_pair`]): a session's walk in its API phase is the grant's
/// walk too. One that reached the end holds all there is, and a new source the
/// athlete connects is a source of its own, walked on its first ask. The
/// one exception is a session walked alone while no API grant stood beside
/// it: once one does, the walk is reopened for the grant's phase below what
/// the session reached ([`walk_hand_off`]).
pub async fn start_history_walk(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    trigger: WalkTrigger,
) -> bool {
    let Some(source) = walk_source(runtime.repos(), user_id, tenant_id, provider).await else {
        info!(user_id = %user_id, provider, ?trigger, "History walk: no live source to walk");
        return false;
    };
    if let Some(answered) =
        walk_of_pair(runtime, user_id, tenant_id, &source.backend, trigger).await
    {
        return answered;
    }
    let limit = match source.role {
        HistorySourceRole::Session => historical_backfill_fetch_limit(),
        HistorySourceRole::Grant | HistorySourceRole::Pushed => API_WALK_PAGE,
    };
    let started = spawn_activity_backfill(ActivityBackfillJob {
        resources: Arc::clone(runtime),
        user_id,
        tenant_id,
        tenant_id_str: provider_tenant_id_str(tenant_id),
        provider_name: source.backend.clone(),
        query_params: ActivityQueryParams {
            after: Some(WHOLE_ACCOUNT_AFTER),
            limit: Some(limit),
            ..ActivityQueryParams::default()
        },
        // No conversation asked: the athlete's personal thread hears the end.
        pierre_conversation_id: None,
        kind: BackfillJobKind::HistoryWalk,
        turn_id: None,
        asked_message_id: None,
    })
    .await;
    info!(
        user_id = %user_id,
        backend = %source.backend,
        ?trigger,
        started,
        "History walk: asked"
    );
    started
}

/// What a walk of the pair already filed answers an ask to walk `backend`
/// with ([`walk_across_pair`]); `None` when none stands in its way and a
/// walk starts. One owed under any key is that walk; one that walked
/// `backend` to the end holds all there is, save the API phase a session
/// walked alone owes a grant that connected since.
async fn walk_of_pair(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    trigger: WalkTrigger,
) -> Option<bool> {
    match walk_across_pair(runtime.repos(), user_id, tenant_id, backend).await {
        Ok(PairWalk::Open) => None,
        Ok(PairWalk::Walked) => {
            Some(reopen_for_grant(runtime, user_id, tenant_id, backend, trigger).await)
        }
        Ok(PairWalk::Owed { key }) => {
            info!(user_id = %user_id, backend, key, ?trigger, "History walk: a walk of the pair is already owed");
            Some(false)
        }
        Ok(PairWalk::WalkedThrough) => Some(false),
        // A duplicate ask under the source's own key is refused by its row.
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History walk: the ledger could not be read");
            None
        }
    }
}

/// Owe a walk `backend`'s session ended alone the API phase of the grant now
/// live beside it ([`walk_hand_off`]); the resume sweep runs it. `false` when
/// no grant stands beside the session, the walk already ran that phase, or
/// the ledger could not be read or written.
async fn reopen_for_grant(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    trigger: WalkTrigger,
) -> bool {
    let Ok(Some(hand_off)) = hand_off_below(runtime, user_id, tenant_id, backend, 0).await else {
        return false;
    };
    match runtime
        .repos()
        .activity_backfill_jobs
        .reopen_history_walk(user_id, tenant_id, backend, &hand_off)
        .await
    {
        Ok(reopened) => {
            info!(
                user_id = %user_id,
                backend,
                grant = %hand_off.backend,
                ?trigger,
                reopened,
                "History walk: a walked session owes its grant's phase"
            );
            reopened
        }
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History walk: the walked row could not be reopened for the grant");
            false
        }
    }
}

/// Where the walk of `backend` goes on once its session found nothing more
/// ([`WalkHandOff`]): the live API grant beside it ([`walk_hand_off`]),
/// below the oldest activity any of the provider's history keys stores.
/// `Ok(None)` when `backend` is no session or no such grant is live; `Err`
/// when the store could not be read, so the hand-off is tried again rather
/// than paging the API from the head.
async fn hand_off_below(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    rows_landed: i64,
) -> Result<Option<WalkHandOff>, ()> {
    let repos = runtime.repos();
    let Some(grant) = walk_hand_off(repos, user_id, tenant_id, backend).await else {
        return Ok(None);
    };
    let before_ts = oldest_stored(runtime, user_id, tenant_id, backend)
        .await?
        .map(|oldest| oldest.timestamp());
    Ok(Some(WalkHandOff {
        backend: grant.backend,
        before_ts,
        months_done: before_ts.map_or(0, |before| months_between(Utc::now().timestamp(), before)),
        rows_landed,
        fetch_limit: i64::try_from(API_WALK_PAGE).unwrap_or(i64::MAX),
    }))
}

/// The start of the oldest activity stored under any of `backend`'s history
/// keys ([`history_store_keys`]); `Err` (logged) when a key could not be
/// read.
async fn oldest_stored(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Result<Option<DateTime<Utc>>, ()> {
    let cache = &runtime.repos().activity_cache;
    let mut oldest: Option<DateTime<Utc>> = None;
    for key in history_store_keys(backend) {
        match cache.oldest_activity_start(user_id, &tenant_id, key).await {
            Ok(start) => oldest = oldest.into_iter().chain(start).min(),
            Err(e) => {
                warn!(user_id = %user_id, key, error = %e, "History walk: the oldest stored activity could not be read");
                return Err(());
            }
        }
    }
    Ok(oldest)
}

/// What one step of a walk came to.
#[derive(Debug)]
enum Step {
    /// Rows landed and more history lies below `step.before_ts`.
    Walked(JobProgressStep),
    /// The walk reached the end of the history with this last step.
    Ended(JobProgressStep),
    /// A request budget refused the step: the walk resumes later, from the
    /// same cursor.
    Paused,
    /// The source's credential is dead.
    AuthRequired,
    /// A transient failure: nothing of the step is recorded.
    Failed,
}

/// Run one step of the walk `job` is, and settle its row; a walk the ledger
/// could not record walks to its end in this run instead.
#[instrument(
    skip_all,
    name = "history_walk",
    fields(user_id = %job.user_id, tenant_id = %job.tenant_id, provider = %job.provider_name)
)]
pub(super) async fn run_history_walk(job: &ActivityBackfillJob, recorded: Option<(JobLease, i64)>) {
    let executor =
        UniversalExecutor::new(job.resources.clone()).with_scopes(OAuthScope::self_grant());
    let Some((lease, attempts)) = recorded else {
        walk_unrecorded(job, &executor).await;
        return;
    };
    let renewal = keep_backfill_lease(job, &lease);
    let attempt_started_at = Utc::now();
    let step = walk_step(job, &executor, &job.query_params).await;
    settle_step(job, &lease, attempts, step, attempt_started_at).await;
    drop(renewal);
}

/// What `step` does to the walk's row.
async fn settle_step(
    job: &ActivityBackfillJob,
    lease: &JobLease,
    attempts: i64,
    step: Step,
    attempt_started_at: DateTime<Utc>,
) {
    let ledger = &job.resources.repos().activity_backfill_jobs;
    match step {
        Step::Walked(step) => {
            record_walk_coverage(job, step.before_ts, false).await;
            log_write(
                "a step landed; the next reads below it",
                ledger.record_job_progress(lease, step).await,
                job,
            );
        }
        Step::Ended(step) => {
            let below = hand_off_below(
                &job.resources,
                job.user_id,
                job.tenant_id,
                &job.provider_name,
                step.rows_landed,
            )
            .await;
            match below {
                Ok(Some(hand_off)) => hand_off_to_grant(job, lease, step, &hand_off).await,
                Ok(None) => end_walk(job, lease, attempts, step).await,
                Err(()) => log_write(
                    "the store could not be read for the hand-off; the sweep retries the step",
                    Ok(true),
                    job,
                ),
            }
        }
        Step::Paused => log_write(
            "a request budget refused the step; it resumes in a later window",
            ledger.release_backfill_job_claim(lease).await,
            job,
        ),
        Step::AuthRequired => {
            // The walked source is the athlete's own connection: flagged and
            // told as every background read does (carnet#1494).
            notify_backfill_reauth(job, attempt_started_at).await;
            log_write(
                "the source's credential is dead; the walk ends incomplete",
                ledger.end_backfill_job_incomplete(lease).await,
                job,
            );
        }
        Step::Failed if attempts >= MAX_BACKFILL_ATTEMPTS => log_write(
            "failed on its last attempt; the walk ends incomplete",
            ledger.end_backfill_job_incomplete(lease).await,
            job,
        ),
        Step::Failed => log_write(
            "a step failed; the sweep retries it once the lease lapses",
            Ok(true),
            job,
        ),
    }
}

/// Move the walk to its API phase once the session's last `step` found
/// nothing more: what the session reached is recorded as its coverage, and
/// the row reads the grant from now on, below the oldest stored row.
async fn hand_off_to_grant(
    job: &ActivityBackfillJob,
    lease: &JobLease,
    step: JobProgressStep,
    hand_off: &WalkHandOff,
) {
    record_walk_coverage(job, step.before_ts, false).await;
    log_write(
        "the session found nothing more; the grant pages below its oldest row",
        job.resources
            .repos()
            .activity_backfill_jobs
            .hand_off_history_walk(lease, hand_off)
            .await,
        job,
    );
}

/// End the walk on its last `step`: the wellness before a push grant first,
/// then the row kept as walked, the coverage, and the one message. A wellness
/// copy that failed with attempts left leaves the row to the sweep, which
/// re-runs the last step and copies again.
async fn end_walk(
    job: &ActivityBackfillJob,
    lease: &JobLease,
    attempts: i64,
    step: JobProgressStep,
) {
    if !wellness::copy_before_grant(job).await && attempts < MAX_BACKFILL_ATTEMPTS {
        log_write(
            "the wellness copy failed; the sweep re-runs the last step",
            Ok(true),
            job,
        );
        return;
    }
    let ended = job
        .resources
        .repos()
        .activity_backfill_jobs
        .end_history_walk(lease, step)
        .await;
    let landed = match ended {
        Ok(Some(progress)) => progress.rows_landed,
        other => {
            log_write(
                "reached the end of the history",
                other.map(|ended| ended.is_some()),
                job,
            );
            return;
        }
    };
    log_write("reached the end of the history", Ok(true), job);
    record_walk_coverage(job, step.before_ts, true).await;
    announce_walked(job, landed).await;
}

/// Log how writing a step's settlement to the row went: `Ok(false)` is a
/// row this lease no longer holds.
fn log_write(what: &str, written: AppResult<bool>, job: &ActivityBackfillJob) {
    let (user_id, provider) = (job.user_id, job.provider_name.as_str());
    match written {
        Ok(true) => info!(user_id = %user_id, provider, what, "History walk: step settled"),
        Ok(false) => {
            warn!(user_id = %user_id, provider, what, "History walk: the row is no longer this run's");
        }
        Err(e) => {
            warn!(user_id = %user_id, provider, what, error = %e, "History walk: the row could not be written");
        }
    }
}

/// Walk to the end in this run, for a walk whose row could not be written:
/// the cursor is kept in memory, and a step that pauses or fails stops the
/// walk until the next ask.
async fn walk_unrecorded(job: &ActivityBackfillJob, executor: &UniversalExecutor) {
    let mut window = job.query_params.clone();
    let mut landed: i64 = 0;
    let mut grant_phase: Option<ActivityBackfillJob> = None;
    loop {
        let phase = grant_phase.as_ref().unwrap_or(job);
        match walk_step(phase, executor, &window).await {
            Step::Walked(step) => {
                landed = landed.saturating_add(step.rows_landed);
                window.before = Some(step.before_ts);
                if let Some(floor) = step.floor {
                    window.after = Some(floor.after_ts);
                }
            }
            Step::Ended(step) => {
                landed = landed.saturating_add(step.rows_landed);
                let below = hand_off_below(
                    &phase.resources,
                    phase.user_id,
                    phase.tenant_id,
                    &phase.provider_name,
                    0,
                )
                .await;
                if let Ok(Some(hand_off)) = below {
                    record_walk_coverage(phase, step.before_ts, false).await;
                    window = ActivityQueryParams {
                        before: hand_off.before_ts,
                        after: None,
                        limit: Some(API_WALK_PAGE),
                        offset: None,
                    };
                    grant_phase = Some(phase_reading(job, hand_off.backend, window.clone()));
                    continue;
                }
                wellness::copy_before_grant(phase).await;
                record_walk_coverage(phase, step.before_ts, true).await;
                announce_walked(phase, landed).await;
                return;
            }
            other => {
                warn!(user_id = %job.user_id, provider = %phase.provider_name, step = ?other, "History walk: an unrecorded walk stopped; the next ask walks again");
                return;
            }
        }
    }
}

/// `job` reading `backend` over `window`: the walk's API phase.
fn phase_reading(
    job: &ActivityBackfillJob,
    backend: String,
    window: ActivityQueryParams,
) -> ActivityBackfillJob {
    ActivityBackfillJob {
        resources: Arc::clone(&job.resources),
        user_id: job.user_id,
        tenant_id: job.tenant_id,
        tenant_id_str: job.tenant_id_str.clone(),
        provider_name: backend,
        query_params: window,
        pierre_conversation_id: None,
        kind: BackfillJobKind::HistoryWalk,
        turn_id: None,
        asked_message_id: None,
    }
}

/// What the walked source is to the athlete.
fn source_role(backend: &str) -> HistorySourceRole {
    history_sources_for(backend)
        .into_iter()
        .find(|(source, _)| *source == backend)
        .map_or(HistorySourceRole::Grant, |(_, role)| role)
}

/// Read and write one step of the walk below `window.before` (now, for the
/// first step), down to no lower than `window.after`, the floor.
async fn walk_step(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    window: &ActivityQueryParams,
) -> Step {
    let floor = window
        .after
        .unwrap_or(WHOLE_ACCOUNT_AFTER)
        .max(WHOLE_ACCOUNT_AFTER);
    let top = window.before.unwrap_or_else(|| Utc::now().timestamp());
    if top <= floor {
        // A step resumed at the floor: the history is all in.
        return Step::Ended(no_step(top));
    }
    let tenant = job.tenant_id_str.as_deref();
    let provider = match executor
        .auth_service
        .create_exact_provider(&job.provider_name, job.user_id, tenant)
        .await
    {
        Ok(provider) => provider,
        Err(response) => {
            let auth_required = auth_required_provider(&response).is_some();
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = ?response.error, auth_required, "History walk: the source could not be authenticated");
            return if auth_required {
                Step::AuthRequired
            } else {
                Step::Failed
            };
        }
    };
    let role = source_role(&job.provider_name);
    let cursor = Cursor {
        top,
        floor,
        first: window.before.is_none(),
        // The head, or an API phase whose floor is still to learn (a
        // hand-off clears it): the grant's profile names it.
        learn_floor: window.before.is_none() || window.after.is_none(),
        limit: match role {
            HistorySourceRole::Session => window.limit.unwrap_or(API_WALK_PAGE),
            HistorySourceRole::Grant | HistorySourceRole::Pushed => API_WALK_PAGE,
        },
    };
    match role {
        HistorySourceRole::Session => session_step(job, executor, provider.as_ref(), cursor).await,
        HistorySourceRole::Grant => api_step(job, executor, provider.as_ref(), cursor).await,
        HistorySourceRole::Pushed => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, "History walk: a push holds no history to page");
            Step::Failed
        }
    }
}

/// Where a step starts.
#[derive(Debug, Clone, Copy)]
struct Cursor {
    /// The newest instant the step reads (unix seconds).
    top: i64,
    /// The oldest instant the walk reads to.
    floor: i64,
    /// Whether this is the walk's first step, which reads the list head.
    first: bool,
    /// Whether the step reads the athlete's profile for the walk's floor:
    /// on the head, and on an API phase's first step below the session.
    learn_floor: bool,
    /// Activities one request asks for.
    limit: usize,
}

impl Cursor {
    /// The `before` this step's first request names: none on the first step,
    /// which reads the head of the list.
    fn before(self) -> Option<i64> {
        (!self.first).then_some(self.top)
    }
}

/// A step that read nothing and moved nothing, ending at `before_ts`.
const fn no_step(before_ts: i64) -> JobProgressStep {
    JobProgressStep {
        before_ts,
        months_walked: 0,
        rows_landed: 0,
        secs_per_unit: None,
        floor: None,
    }
}

/// Whole months between two instants.
fn months_between(top: i64, bottom: i64) -> i64 {
    top.saturating_sub(bottom).max(0) / SECS_PER_MONTH
}

/// One session step: [`SCIOTTE_WALK_STEP_MONTHS`] below the cursor in one
/// request. Finding nothing there ends the walk.
async fn session_step(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    provider: &dyn CoreFitnessProvider,
    cursor: Cursor,
) -> Step {
    let bottom = cursor
        .top
        .saturating_sub(SCIOTTE_WALK_STEP_MONTHS * SECS_PER_MONTH)
        .max(cursor.floor);
    let read = ActivityQueryParams {
        after: Some(bottom),
        before: cursor.before(),
        limit: Some(cursor.limit),
        offset: None,
    };
    let started = Instant::now();
    let Ok(fetched) = timeout(
        SCIOTTE_STEP_TIMEOUT,
        ai_scope::unfiltered(provider.get_activities_with_params(&read)),
    )
    .await
    else {
        warn!(user_id = %job.user_id, provider = %job.provider_name, "History walk: a session step outlived its timeout");
        return Step::Failed;
    };
    let activities = match fetched {
        Ok(activities) => activities,
        Err(e) => return failed_read(job, &e),
    };
    if activities.is_empty() {
        let mut step = no_step(bottom);
        step.months_walked = months_between(cursor.top, bottom);
        return Step::Ended(step);
    }
    let span = StepSpan {
        read: &read,
        top: cursor.top,
        bottom,
        cursor,
        started,
    };
    land(job, executor, provider, activities, span).await
}

/// One API step: a year at a time below the cursor, until a window lands
/// rows, the floor is reached, or two consecutive windows hold nothing.
/// The first step learns the floor from the athlete's profile.
async fn api_step(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    provider: &dyn CoreFitnessProvider,
    mut cursor: Cursor,
) -> Step {
    let learned = if cursor.learn_floor {
        match account_floor(job, provider).await {
            Ok(learned) => learned,
            Err(()) => return Step::Paused,
        }
    } else {
        None
    };
    if let Some(floor) = learned {
        cursor.floor = floor.after_ts.max(cursor.floor);
    }
    let started = Instant::now();
    let walk_top = cursor.top;
    let mut empty_windows = 0;
    loop {
        let bottom = cursor
            .top
            .saturating_sub(API_WALK_STEP_MONTHS * SECS_PER_MONTH)
            .max(cursor.floor);
        let read = ActivityQueryParams {
            after: Some(bottom),
            before: cursor.before(),
            limit: Some(cursor.limit),
            offset: None,
        };
        let activities =
            match ai_scope::unfiltered(provider.get_activities_with_params(&read)).await {
                Ok(activities) => activities,
                Err(e) if e.code == ErrorCode::ExternalRateLimited => return Step::Paused,
                Err(e) => return failed_read(job, &e),
            };
        if !activities.is_empty() {
            let span = StepSpan {
                read: &read,
                top: walk_top,
                bottom,
                cursor,
                started,
            };
            return with_floor(
                land(job, executor, provider, activities, span).await,
                learned,
            );
        }
        empty_windows += 1;
        cursor.top = bottom;
        cursor.first = false;
        if bottom <= cursor.floor || empty_windows >= EMPTY_WINDOWS_ENDING_A_WALK {
            let mut step = no_step(bottom);
            step.months_walked = months_between(walk_top, bottom);
            step.floor = learned;
            return Step::Ended(step);
        }
    }
}

/// `step` carrying the floor the walk learned on it.
fn with_floor(step: Step, learned: Option<JobFloor>) -> Step {
    match step {
        Step::Walked(mut step) => {
            step.floor = learned;
            Step::Walked(step)
        }
        Step::Ended(mut step) => {
            step.floor = learned;
            Step::Ended(step)
        }
        other => other,
    }
}

/// The floor the athlete's profile names: a Strava account's creation, and
/// the months from now down to it; [`WHOLE_ACCOUNT_AFTER`] with no length
/// when it names none, so the walk ends on its empty years and the profile
/// is not read again. `Ok(None)` when the profile cannot be read for any
/// reason but a spent budget, which is `Err`.
async fn account_floor(
    job: &ActivityBackfillJob,
    provider: &dyn CoreFitnessProvider,
) -> Result<Option<JobFloor>, ()> {
    match ai_scope::unfiltered(provider.get_athlete()).await {
        Ok(athlete) => {
            let created = athlete
                .created_at
                .map(|created| created.timestamp().max(WHOLE_ACCOUNT_AFTER));
            Ok(Some(JobFloor {
                after_ts: created.unwrap_or(WHOLE_ACCOUNT_AFTER),
                months_total: created
                    .map(|after_ts| months_between(Utc::now().timestamp(), after_ts)),
            }))
        }
        Err(e) if e.code == ErrorCode::ExternalRateLimited => Err(()),
        Err(e) => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "History walk: the athlete's profile could not be read; walking to the empty years");
            Ok(None)
        }
    }
}

/// A read that failed: a dead credential, or a transient failure.
fn failed_read(job: &ActivityBackfillJob, error: &AppError) -> Step {
    let auth_required = error.provider_auth_required_provider().is_some();
    warn!(user_id = %job.user_id, provider = %job.provider_name, error = %error, auth_required, "History walk: a step's read failed");
    if auth_required {
        Step::AuthRequired
    } else {
        Step::Failed
    }
}

/// The span one step read.
#[derive(Debug, Clone, Copy)]
struct StepSpan<'a> {
    /// The request that returned the rows.
    read: &'a ActivityQueryParams,
    /// The newest instant the step covers (unix seconds).
    top: i64,
    /// The oldest instant the request covered.
    bottom: i64,
    /// Where the step started.
    cursor: Cursor,
    /// When the step started, for its seconds per month.
    started: Instant,
}

/// Write a step's `activities` through the store, and say where the next
/// step starts: below the oldest activity when the request filled its limit
/// (more lies in its window), else below the window.
async fn land(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    provider: &dyn CoreFitnessProvider,
    activities: Vec<Activity>,
    span: StepSpan<'_>,
) -> Step {
    let fetched = activities.len();
    let activities = match capture_with_head(job, provider, activities) {
        BackfillFetch::Activities(activities) => activities,
        BackfillFetch::AuthRequired => return Step::AuthRequired,
        BackfillFetch::Failed => return Step::Failed,
    };
    let oldest = activities
        .iter()
        .map(|activity| activity.start_date().timestamp())
        .min()
        .unwrap_or(span.bottom);
    let write = WriteThrough {
        read: span.read,
        // A walk copies the past: it lands no new session.
        announces_sessions: false,
    };
    let Some(landed) = persist_backfill_activities(job, executor, &activities, write).await else {
        return Step::Failed;
    };
    warm_training_history(job).await;
    let next = if fetched >= span.cursor.limit && oldest < span.top {
        oldest
    } else {
        span.bottom
    };
    let months = months_between(span.top, next);
    let step = JobProgressStep {
        before_ts: next,
        months_walked: months,
        rows_landed: i64::try_from(landed).unwrap_or(i64::MAX),
        secs_per_unit: secs_per_month(span.started, months),
        floor: None,
    };
    if next <= span.cursor.floor {
        Step::Ended(step)
    } else {
        Step::Walked(step)
    }
}

/// The seconds one month of a step took, or `None` for a step that covered
/// less than a month, which measures nothing.
fn secs_per_month(started: Instant, months: i64) -> Option<f64> {
    let months = i32::try_from(months).ok().filter(|months| *months > 0)?;
    Some(started.elapsed().as_secs_f64() / f64::from(months))
}

/// Record how deep the walk has reached under its key, so the historical
/// gate serves the walked windows from the store; `hit_feed_end` once the
/// walk reached the end of the history.
async fn record_walk_coverage(job: &ActivityBackfillJob, reached_ts: i64, hit_feed_end: bool) {
    let coverage = BackfillCoverage {
        oldest_reached_ts: reached_ts,
        hit_feed_end,
        capture_version: current_capture_version(&job.provider_name),
    };
    if let Err(e) = job
        .resources
        .repos()
        .activity_cache
        .upsert_backfill_coverage(job.user_id, &job.tenant_id, &job.provider_name, coverage)
        .await
    {
        warn!(user_id = %job.user_id, error = %e, "History walk: coverage could not be recorded");
    }
}

/// Tell the athlete the walk reached the end of their history, naming the
/// oldest year any of its history keys now holds and the `landed`
/// activities the walk wrote. A walk that brought nothing in says nothing.
async fn announce_walked(job: &ActivityBackfillJob, landed: i64) {
    let Some(notifier) = job.resources.backfill_notifier() else {
        return;
    };
    let count = usize::try_from(landed).unwrap_or(0);
    if count == 0 {
        info!(user_id = %job.user_id, provider = %job.provider_name, "History walk: ended with nothing new; nothing to announce");
        return;
    }
    let Ok(Some(oldest)) = oldest_stored(
        &job.resources,
        job.user_id,
        job.tenant_id,
        &job.provider_name,
    )
    .await
    else {
        return;
    };
    info!(
        user_id = %job.user_id,
        provider = %job.provider_name,
        oldest_year = oldest.year(),
        count,
        "History walk: reached the end of the history"
    );
    notifier
        .push_history_walk_complete(HistoryWalkCompletion {
            user_id: job.user_id,
            tenant_id: job.tenant_id,
            provider: &job.provider_name,
            oldest,
            activity_count: count,
        })
        .await;
}
