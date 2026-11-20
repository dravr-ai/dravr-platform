// ABOUTME: The tag walk: one durable job per athlete that pages Strava's own activity list for the tags and gear id the newest page alone carries
// ABOUTME: Page by page through the session, within the detail walk's daily pools and burst, filling rows already stored and nothing else

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tag walks (carnet#1622).
//!
//! Strava's training list carries an activity's tags and gear id on its rows,
//! but the training-log read the platform keeps the store fresh with sees
//! only the newest ones. The provider's filtered list reaches past them
//! ([`FitnessProvider::search_activities`](pierre_providers::core::FitnessProvider::search_activities)):
//! a tag walk reads it page by page from the first and fills what each row
//! carries into the activity the store already holds, so `query_activities`
//! can ask the whole history for a tag. It backfills; it is not a search
//! tool, and it creates no row.
//!
//! **One page a call.** The read names its page and bounds its rows to
//! [`TAG_WALK_PAGE_ROWS`], one page of the list, so a call is one page of the
//! provider and the cursor is the number of pages done, kept in the row's
//! `months_done` (the columns keep their first names; the unit is the job
//! kind's). A page the scraper says it did not read whole is retried at the
//! same page, never counted.
//!
//! **Budget.** Each page is admitted against the windows the detail walk
//! reads in: the 125 a day every athlete's session walk shares, the
//! athlete's 50 a day and the burst of ten pages in fifteen minutes
//! ([`session_walk_window`]). A refusal pauses the walk to the end of the
//! refusing window; a day that leaves the walk no call ends it incomplete.
//! Only a session the limiter counts is walked: any other backend is not
//! asked.
//!
//! **Where it ends.** On a page that lists no activity, on one the store
//! holds none of (the list has gone past the stored history), or at
//! [`TAG_WALK_MAX_PAGES`]. The row is kept as walked, and the walk is not
//! asked again for the connection (it is asked only once the connection's
//! history walk, when it has one, has ended, so the store holds the whole
//! history the list is read against); a dead session ends it incomplete, and
//! the next ask records it again. It is asked once a connection's head
//! refresh succeeds ([`ask_tag_walk`]): by the nightly capture sweep, and by
//! a chat turn's live read of the session's list head, both through
//! `activity_backfill::after_refresh`.
//!
//! **What it writes.** Through
//! [`ActivityCacheRepository::amend_listed_reads`](pierre_database::repositories::ActivityCacheRepository::amend_listed_reads)
//! alone, under every key the connection's history is held under
//! ([`history_store_keys`]): tags and gear of rows that exist.
//! A row without tags on the page never erases tags the store holds.

use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::provider_context::ProviderActivitySearch;
use pierre_core::models::Activity;
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::repositories::{
    AmendedReads, BackfillJobKind, BackfillJobState, JobLease, JobProgress, JobProgressStep,
};
use pierre_providers::ai_scope;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::request_budget::{in_detail_walk, RequestBudget};
use pierre_providers::CoreFitnessProvider;
use pierre_services::training_history_read::history_store_keys;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use super::detail_walk::session_walk_window;
use super::pause::paused_by;
use super::{
    enqueue, keep_backfill_lease, lease_ms, notify_backfill_reauth, now_ms, provider_tenant_id_str,
    ActivityBackfillJob, Enqueued, MAX_BACKFILL_ATTEMPTS,
};
use crate::protocol::types::auth_required_provider;
use crate::protocol::UniversalExecutor;
use crate::runtime::ToolRuntime;

/// Rows one list page holds, as the read bounds itself to: the provider's
/// training list pages by twenty.
pub(crate) const TAG_WALK_PAGE_ROWS: usize = 20;

/// Pages one step reads before it records itself and lets go of the row.
const TAG_WALK_STEP_PAGES: u32 = 5;

/// The most pages a walk reads: 8,000 activities, past which the list holds
/// no stored history of an athlete's.
pub(crate) const TAG_WALK_MAX_PAGES: i64 = 400;

/// What asking for a tag walk came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagWalkAsked {
    /// A walk was recorded and started.
    Started,
    /// The connection has one owed or walked, or it is already running: no
    /// second is recorded.
    Standing,
    /// The athlete's history walk through the backend has not ended (a
    /// connection with no history walk row has nothing walking: its history
    /// came from the first sync or an import, and is asked): the
    /// store does not hold the whole history yet, so a list that ran past it
    /// would end the walk too early. Asked again by a later sweep.
    HistoryUnfinished,
    /// The backend is not a session the limiter counts, or the ledger could
    /// not say: nothing was asked.
    NotAsked,
}

/// Ask for the tag walk of `backend`'s session for the athlete, once.
///
/// A connection whose walk is owed or ended walked is left alone; one that
/// ended incomplete (its session died, its budget left no call) is recorded
/// again. Asked after a head refresh that succeeded, so the session is live:
/// by the capture sweep, and by a chat turn's live read of the list head.
pub async fn ask_tag_walk(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> TagWalkAsked {
    if session_walk_window(runtime, user_id, tenant_id, backend)
        .await
        .is_none()
    {
        return TagWalkAsked::NotAsked;
    }
    if let Some(not_asked) = held_back(runtime, user_id, tenant_id, backend).await {
        return not_asked;
    }
    let job = ActivityBackfillJob {
        resources: Arc::clone(runtime),
        user_id,
        tenant_id,
        tenant_id_str: provider_tenant_id_str(tenant_id),
        provider_name: backend.to_owned(),
        query_params: ActivityQueryParams {
            after: None,
            before: None,
            limit: None,
            offset: None,
        },
        pierre_conversation_id: None,
        kind: BackfillJobKind::TagWalk,
        turn_id: None,
        asked_message_id: None,
        walk_sport: None,
    };
    match enqueue(job, None, JobProgress::default()).await {
        Enqueued::Started => {
            info!(user_id = %user_id, backend, "Tag walk: asked");
            TagWalkAsked::Started
        }
        Enqueued::InFlightHere(..) | Enqueued::Owed(..) => TagWalkAsked::Standing,
        Enqueued::Unrecorded => TagWalkAsked::NotAsked,
    }
}

/// Why no walk is recorded for the connection now, when none is: its
/// history walk has not ended, or a tag walk already stands.
async fn held_back(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Option<TagWalkAsked> {
    let ledger = &runtime.repos().activity_backfill_jobs;
    match ledger.history_walk_row(user_id, tenant_id, backend).await {
        // No history walk row: the history came from the first sync or an
        // import, and nothing is walking it; only a walk still owed holds the
        // tag walk back.
        Ok(None) => {}
        Ok(Some(history)) if history.state == BackfillJobState::Walked => {}
        Ok(Some(_)) => return Some(TagWalkAsked::HistoryUnfinished),
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "Tag walk: the history walk could not be read; nothing is asked");
            return Some(TagWalkAsked::NotAsked);
        }
    }
    match ledger
        .backfill_job_state(user_id, tenant_id, backend, BackfillJobKind::TagWalk)
        .await
    {
        Ok(Some(BackfillJobState::Owed | BackfillJobState::Walked)) => Some(TagWalkAsked::Standing),
        Ok(_) => None,
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "Tag walk: the ledger could not be read; nothing is asked");
            Some(TagWalkAsked::NotAsked)
        }
    }
}

/// What one step of a tag walk came to.
#[derive(Debug)]
enum Step {
    /// The step read its pages; `ended` once the list ran out.
    Walked { step: JobProgressStep, ended: bool },
    /// A spent budget stopped the step, until the refusing window ends
    /// (unix ms).
    Paused {
        step: JobProgressStep,
        until_ms: i64,
    },
    /// The session is dead.
    AuthRequired,
    /// The budget leaves the walk no call a day.
    NoBudget,
    /// The provider, the store or the ledger failed before the list ran out.
    Failed { step: JobProgressStep },
}

/// Run one step of the tag walk `job` is, and settle its row. A walk the
/// ledger could not record reads nothing: no step writes without its row.
#[instrument(
    skip_all,
    name = "tag_walk",
    fields(user_id = %job.user_id, tenant_id = %job.tenant_id, provider = %job.provider_name)
)]
pub(super) async fn run_tag_walk(job: &ActivityBackfillJob, recorded: Option<(JobLease, i64)>) {
    let Some((lease, attempts)) = recorded else {
        warn!(user_id = %job.user_id, provider = %job.provider_name, "Tag walk: the walk could not be recorded; nothing is read until it is asked again");
        return;
    };
    let renewal = keep_backfill_lease(job, &lease);
    let attempt_started_at = Utc::now();
    let step = walk_step(job).await;
    settle(job, &lease, attempts, step, attempt_started_at).await;
    drop(renewal);
}

/// The step as the row records it: `pages` read, `filled` rows amended.
fn recorded_step(
    job: &ActivityBackfillJob,
    pages: i64,
    filled: i64,
    began: Instant,
) -> JobProgressStep {
    let secs_per_unit = i32::try_from(pages)
        .ok()
        .filter(|done| *done > 0)
        .map(|done| began.elapsed().as_secs_f64() / f64::from(done));
    JobProgressStep {
        before_ts: job.query_params.before.unwrap_or(0),
        units_done: pages,
        rows_landed: filled,
        secs_per_unit,
        floor: None,
    }
}

/// Read up to [`TAG_WALK_STEP_PAGES`] pages from where the row stands.
async fn walk_step(job: &ActivityBackfillJob) -> Step {
    let began = Instant::now();
    let none_read = || recorded_step(job, 0, 0, began);
    let ledger = &job.resources.repos().activity_backfill_jobs;
    let done = match ledger
        .job_progress(
            job.user_id,
            job.tenant_id,
            &job.provider_name,
            BackfillJobKind::TagWalk,
        )
        .await
    {
        Ok(progress) => progress.map_or(0, |progress| progress.units_done),
        Err(e) => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "Tag walk: the row's progress could not be read");
            return Step::Failed { step: none_read() };
        }
    };
    if done >= TAG_WALK_MAX_PAGES {
        return Step::Walked {
            step: none_read(),
            ended: true,
        };
    }
    let Some((budget, daily_calls)) = session_walk_window(
        &job.resources,
        job.user_id,
        job.tenant_id,
        &job.provider_name,
    )
    .await
    else {
        return Step::NoBudget;
    };
    if daily_calls == Some(0) {
        return Step::NoBudget;
    }
    let executor =
        UniversalExecutor::new(job.resources.clone()).with_scopes(OAuthScope::self_grant());
    let provider = match executor
        .auth_service
        .create_exact_provider(
            &job.provider_name,
            job.user_id,
            job.tenant_id_str.as_deref(),
        )
        .await
    {
        Ok(provider) => provider,
        Err(response) => {
            let auth_required = auth_required_provider(&response).is_some();
            warn!(user_id = %job.user_id, provider = %job.provider_name, error = ?response.error, auth_required, "Tag walk: the session could not be authenticated");
            return if auth_required {
                Step::AuthRequired
            } else {
                Step::Failed { step: none_read() }
            };
        }
    };
    let reader = PageReader {
        job,
        provider: provider.as_ref(),
        budget: &budget,
    };
    let (mut pages, mut filled) = (0_i64, 0_i64);
    for offset in 0..TAG_WALK_STEP_PAGES {
        let page = done + i64::from(offset) + 1;
        if page > TAG_WALK_MAX_PAGES {
            return Step::Walked {
                step: recorded_step(job, pages, filled, began),
                ended: true,
            };
        }
        match reader.read(page).await {
            Read::Filled { filled: n } => {
                pages += 1;
                filled += n;
            }
            Read::End { filled: n } => {
                return Step::Walked {
                    step: recorded_step(job, pages + 1, filled + n, began),
                    ended: true,
                };
            }
            Read::Paused(until_ms) => {
                return Step::Paused {
                    step: recorded_step(job, pages, filled, began),
                    until_ms,
                };
            }
            Read::AuthRequired => return Step::AuthRequired,
            Read::Failed => {
                return Step::Failed {
                    step: recorded_step(job, pages, filled, began),
                };
            }
        }
    }
    Step::Walked {
        step: recorded_step(job, pages, filled, began),
        ended: false,
    }
}

/// What reading one page came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    /// The page was read and its rows amended; the list goes on.
    Filled { filled: i64 },
    /// The list ran out on this page: it was empty, or held no stored row.
    End { filled: i64 },
    /// A spent budget refused the read, until the refusing window ends.
    Paused(i64),
    /// The session is dead.
    AuthRequired,
    /// The provider or the store failed; the page is read again.
    Failed,
}

/// What a step reads through.
struct PageReader<'step> {
    job: &'step ActivityBackfillJob,
    provider: &'step dyn CoreFitnessProvider,
    budget: &'step RequestBudget,
}

impl PageReader<'_> {
    /// Read list page `page` and fill what its rows carry into the store.
    async fn read(&self, page: i64) -> Read {
        let job = self.job;
        let Ok(number) = u32::try_from(page) else {
            return Read::End { filled: 0 };
        };
        let search = ProviderActivitySearch {
            page: Some(number),
            limit: Some(TAG_WALK_PAGE_ROWS),
        };
        let read = in_detail_walk(job.user_id, async {
            self.budget.admit(&job.provider_name).await?;
            ai_scope::unfiltered(self.provider.search_activities(&search)).await
        })
        .await;
        let listed = match read {
            Ok(listed) => listed,
            Err(e) => return read_failed(job, page, &e),
        };
        // A walk that stopped short says nothing about the rows it missed:
        // the page is read again, never counted.
        if !listed.complete {
            warn!(user_id = %job.user_id, provider = %job.provider_name, page, "Tag walk: the scraper did not read the page whole; it is read again");
            return Read::Failed;
        }
        if listed.activities.is_empty() {
            return Read::End { filled: 0 };
        }
        let amended = amend_across_keys(job, &listed.activities).await;
        match amended {
            Ok(amended) => {
                let filled = i64::try_from(amended.filled).unwrap_or(i64::MAX);
                if amended.present == 0 {
                    Read::End { filled }
                } else {
                    Read::Filled { filled }
                }
            }
            Err(e) => {
                warn!(user_id = %job.user_id, provider = %job.provider_name, page, error = %e, "Tag walk: the page's reads could not be stored");
                Read::Failed
            }
        }
    }
}

/// Fill `activities`' tags and gear into the rows stored for them.
///
/// The history the list is read against is held under every key of the
/// connection's history ([`history_store_keys`]: the session's and the Strava
/// grant's), not the session's alone: a page's runs may sit under the grant,
/// and a page whose runs sit under another key is not past the stored
/// history. An archive import's key is not among them, so its rows never
/// take the session's reads.
async fn amend_across_keys(
    job: &ActivityBackfillJob,
    activities: &[Activity],
) -> AppResult<AmendedReads> {
    let activity_cache = &job.resources.repos().activity_cache;
    let mut amended = AmendedReads::default();
    for key in history_store_keys(&job.provider_name) {
        let under_key = activity_cache
            .amend_listed_reads(job.user_id, &job.tenant_id, key, activities)
            .await?;
        amended.present += under_key.present;
        amended.filled += under_key.filled;
    }
    Ok(amended)
}

/// What a failed page read comes to: a spent budget or a provider rate limit
/// pauses the walk, a dead credential ends it, anything else retries it.
fn read_failed(job: &ActivityBackfillJob, page: i64, e: &AppError) -> Read {
    paused_by(e).map_or_else(
        || {
            if e.provider_auth_required_provider().is_some() {
                Read::AuthRequired
            } else {
                warn!(user_id = %job.user_id, provider = %job.provider_name, page, error = %e, "Tag walk: the page could not be read");
                Read::Failed
            }
        },
        |until_ms| {
            info!(user_id = %job.user_id, provider = %job.provider_name, page, error = %e, "Tag walk: a rate limit paused the walk");
            Read::Paused(until_ms)
        },
    )
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
        // A walk that ran its list out is kept, released, as walked. The
        // statement is the history walk's: it adds the step to the row's
        // progress and marks the row walked, whatever its kind.
        Step::Walked { step, ended: true } => log_write(
            "the list ran out; the walk is kept as walked",
            ledger
                .end_history_walk(lease, step)
                .await
                .map(|ended| ended.is_some()),
            job,
        ),
        Step::Walked { step, ended: false } => {
            record(job, lease, "a step landed; the next reads on", step).await;
        }
        Step::Paused { step, .. } if step.units_done > 0 => {
            let what = "a budget refused the step part-way; what it read is recorded";
            record(job, lease, what, step).await;
        }
        Step::Paused { until_ms, .. } => log_write(
            "a budget refused the step; it resumes once the window ends",
            ledger
                .release_backfill_job_claim(lease, Some(until_ms))
                .await,
            job,
        ),
        Step::AuthRequired => {
            notify_backfill_reauth(job, attempt_started_at).await;
            log_write(
                "the session is dead; the walk ends incomplete",
                ledger.end_backfill_job_incomplete(lease).await,
                job,
            );
        }
        Step::NoBudget => log_write(
            "the session's budget leaves the walk no call; the walk ends incomplete",
            ledger.end_backfill_job_incomplete(lease).await,
            job,
        ),
        Step::Failed { step } if step.units_done > 0 => {
            let what = "a step failed part-way; what it read is recorded";
            record(job, lease, what, step).await;
        }
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
    }
}

/// Record `step` on the row `lease` holds, releasing it for the next one.
async fn record(job: &ActivityBackfillJob, lease: &JobLease, what: &str, step: JobProgressStep) {
    let recorded = job
        .resources
        .repos()
        .activity_backfill_jobs
        .record_job_progress(lease, step)
        .await;
    log_write(what, recorded, job);
}

/// Log how writing a step's settlement to the row went.
fn log_write(what: &str, written: AppResult<bool>, job: &ActivityBackfillJob) {
    let (user_id, provider) = (job.user_id, job.provider_name.as_str());
    match written {
        Ok(true) => info!(user_id = %user_id, provider, what, "Tag walk: step settled"),
        Ok(false) => {
            warn!(user_id = %user_id, provider, what, "Tag walk: the row is no longer this run's");
        }
        Err(e) => {
            warn!(user_id = %user_id, provider, what, error = %e, "Tag walk: the row could not be written");
        }
    }
}
