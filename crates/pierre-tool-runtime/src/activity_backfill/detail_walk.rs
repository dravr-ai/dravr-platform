// ABOUTME: The detail walk: one durable job per athlete that reads the segment efforts of a window's stored activities
// ABOUTME: Newest first, ten activities a step, through a backend that reads them, within its own day budget; told once when it ends

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detail walks (carnet#1483).
//!
//! A segment's history is the efforts of the athlete's stored activities,
//! and only a detail read of an activity carries its efforts. The live path
//! details the last days through the session, which reads none. A detail
//! walk reads them for a window: a [`BackfillJobKind::DetailWalk`] job,
//! recorded before it runs, leased while it runs, resumed by the sweep.
//!
//! **Which backend.** Any backend the athlete holds live, grant or session,
//! whose descriptor declares segment efforts on a detail read
//! ([`segment_detail_backend`]); today only the Strava grant does. It is read
//! exactly as named, and the walk's row is filed under it, so a purge of it
//! deletes the walk with the rest of its data.
//!
//! **One step per run.** A step reads the stored activities of the window
//! below its row's cursor (`before_ts`), newest first, under the keys its
//! backend's history is held under ([`history_store_keys`]), never the
//! athlete's own export. It takes whole seconds of starts at a time, so the
//! session's and the grant's copies of one activity, which share it, are
//! never split by a page's edge: a full page's oldest second is left to the
//! next read. It matches the walk's sport ([`filter_activities_by_sport_type`]),
//! counts the copies of one activity as one ([`native_activity_key`]), and
//! reads up to [`DETAIL_WALK_STEP_ACTIVITIES`] whose efforts no copy holds,
//! each through the backend in the walk's own day budget ([`in_detail_walk`]).
//! What it read is stored beside its row ([`store_detail_from_live_grant`]):
//! the grant's answer under the grant's key, its copy written first, through
//! the walk's owed-write guard ([`owed_write`]), when only the session listed
//! the activity. The cursor then moves to the oldest second it finished, and
//! the row records the step ([`JobProgress`]): activities read, activities
//! stored, the seconds one took.
//!
//! **What stops a step.** An activity the backend cannot read (gone, or any
//! other answer but the ones below) is skipped: the cursor moves past it, and
//! it is counted read, never stored. A spent budget pauses the walk until its
//! window ends; a step that read some first records them, and the next claim
//! is the one that pauses. A dead credential ends the walk incomplete. A
//! failed store retries the step a lease later, its attempt counted.
//!
//! **Where it ends.** When its cursor passes the window's floor (`after_ts`).
//! The row is kept as walked with its window and counts, and the athlete is
//! told once, in their personal thread, how many activities had their
//! efforts read, when that is any
//! ([`BackfillNotifier::push_detail_walk_complete`](crate::runtime::BackfillNotifier::push_detail_walk_complete)).

use std::slice;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::constants::oauth::providers::STRAVA_ARCHIVE;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::coverage::Pending;
use pierre_core::models::{Activity, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::repositories::{
    BackfillJobKind, BackfillJobState, CachedActivityRow, CachedRowsQuery, DetailWalkEnd,
    DetailWalkRow, JobLease, JobProgress, JobProgressStep, WalkWiden,
};
use pierre_providers::ai_scope;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::native_activity_key;
use pierre_providers::field_support::ActivityFields;
use pierre_providers::request_budget::{in_detail_walk, DETAIL_WALK_DAILY_CALLS};
use pierre_providers::CoreFitnessProvider;
use pierre_services::training_history_read::{history_source_order, history_store_keys};
use tracing::{info, instrument, warn};
use uuid::Uuid;

use super::history_walk::retry_at_ms;
use super::owed_write::{land_owed, Landed};
use super::{
    enqueue, keep_backfill_lease, lease_ms, notify_backfill_reauth, now_ms, provider_tenant_id_str,
    ActivityBackfillJob, Enqueued, MAX_BACKFILL_ATTEMPTS,
};
use crate::activity_detail_store::{store_detail_from_live_grant, DetailKept};
use crate::activity_fetch::write_through::WriteThrough;
use crate::implementations::fitness_support::filter_activities_by_sport_type;
use crate::protocol::types::auth_required_provider;
use crate::protocol::UniversalExecutor;
use crate::runtime::{DetailWalkCompletion, ToolRuntime};

/// Activities whose efforts one step reads, at most: the whole seconds it
/// takes stop once they hold this many.
pub const DETAIL_WALK_STEP_ACTIVITIES: usize = 10;

/// Stored rows one read of the window takes.
const WALK_PAGE_ROWS: i64 = 200;

/// Reads of the window one step makes looking for activities to read before
/// it records where it got to.
const WALK_PAGES_PER_STEP: usize = 20;

/// Seconds in a day.
const SECS_PER_DAY: f64 = 86_400.0;

/// The backend a detail walk of `provider`'s history reads for the athlete.
///
/// The first live one among the backends holding that history, grant or
/// session, whose descriptor declares segment efforts on a detail read.
/// `None` when the athlete holds none: no walk can read them.
pub async fn segment_detail_backend(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> Option<String> {
    let registry = runtime.provider_registry();
    history_source_order(runtime.repos(), user_id, tenant_id, provider)
        .await
        .into_iter()
        .filter(|source| source.live)
        .find(|source| {
            registry
                .get_descriptor(&source.backend)
                .is_some_and(|descriptor| {
                    descriptor
                        .activity_fields()
                        .detail_only
                        .contains(ActivityFields::SEGMENT_EFFORTS)
                })
        })
        .map(|source| source.backend)
}

/// The window a detail walk is asked to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailWalkAsk<'ask> {
    /// The window's floor (unix seconds), included.
    pub after_ts: i64,
    /// The window's top (unix seconds), excluded.
    pub before_ts: i64,
    /// The sport the ask names; `None` for every sport.
    pub sport: Option<&'ask str>,
}

/// What asking for a detail walk came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailWalkAsked {
    /// A walk was recorded and started, counting this many activities to read.
    Started {
        /// Activities of the window whose efforts no copy holds.
        units_total: i64,
    },
    /// A walk was already owed: its floor was lowered to the ask's, and its
    /// total recounted.
    Widened,
    /// Every activity of the window has its efforts read: nothing to walk.
    NothingUnread,
    /// The store or the ledger could not be read or written (logged).
    NotRecorded,
}

/// Ask for a detail walk of `ask`'s window through `backend` for the
/// athlete.
///
/// A walk owed for the backend is widened to the ask's floor
/// ([`WalkWiden`]); its cursor stays where its steps moved it, and a newer
/// top is the next walk's once this one ends. With none owed, a walk is
/// recorded over the window, counted at the activities in it whose efforts
/// no copy holds, unless there are none.
pub async fn ask_detail_walk(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    ask: DetailWalkAsk<'_>,
) -> DetailWalkAsked {
    let athlete = Athlete {
        runtime,
        user_id,
        tenant_id,
    };
    match owed_walk(&athlete, backend).await {
        Ok(Some(row)) => return widen(&athlete, backend, &row, ask).await,
        Ok(None) => {}
        Err(()) => return DetailWalkAsked::NotRecorded,
    }
    let keys = walk_keys(backend);
    let Ok(units_total) = athlete
        .unread_between(&keys, ask.after_ts, ask.before_ts, ask.sport)
        .await
    else {
        return DetailWalkAsked::NotRecorded;
    };
    if units_total == 0 {
        return DetailWalkAsked::NothingUnread;
    }
    let job = ActivityBackfillJob {
        resources: Arc::clone(runtime),
        user_id,
        tenant_id,
        tenant_id_str: provider_tenant_id_str(tenant_id),
        provider_name: backend.to_owned(),
        query_params: ActivityQueryParams {
            after: Some(ask.after_ts),
            before: Some(ask.before_ts),
            limit: None,
            offset: None,
        },
        // No conversation waits on it: the athlete's personal thread hears
        // the end.
        pierre_conversation_id: None,
        kind: BackfillJobKind::DetailWalk,
        turn_id: None,
        asked_message_id: None,
        walk_sport: ask.sport.map(str::to_owned),
    };
    let progress = JobProgress {
        units_total: Some(units_total),
        ..JobProgress::default()
    };
    if matches!(enqueue(job, None, progress).await, Enqueued::Started) {
        info!(user_id = %user_id, backend, units_total, "Detail walk: asked");
        return DetailWalkAsked::Started { units_total };
    }
    // Recorded by another ask between the read and the record: this one
    // widens it.
    match owed_walk(&athlete, backend).await {
        Ok(Some(row)) => widen(&athlete, backend, &row, ask).await,
        _ => DetailWalkAsked::NotRecorded,
    }
}

/// The detail walk owed for `backend`, if one is; `Err` (logged) when the
/// ledger could not be read.
async fn owed_walk(athlete: &Athlete<'_>, backend: &str) -> Result<Option<DetailWalkRow>, ()> {
    match athlete
        .runtime
        .repos()
        .activity_backfill_jobs
        .detail_walk_row(athlete.user_id, athlete.tenant_id, backend)
        .await
    {
        Ok(row) => Ok(row.filter(|row| row.state == BackfillJobState::Owed)),
        Err(e) => {
            warn!(user_id = %athlete.user_id, backend, error = %e, "Detail walk: the ledger could not be read");
            Err(())
        }
    }
}

/// Widen the walk owed for `backend` (`row`) to `ask`'s floor, its total
/// recounted at what it has read plus what is unread below its cursor in
/// the widened window.
async fn widen(
    athlete: &Athlete<'_>,
    backend: &str,
    row: &DetailWalkRow,
    ask: DetailWalkAsk<'_>,
) -> DetailWalkAsked {
    let floor = row
        .after_ts
        .map_or(ask.after_ts, |after| after.min(ask.after_ts));
    let cursor = row.before_ts.unwrap_or(ask.before_ts);
    let sport = if row.walk_sport.as_deref() == ask.sport {
        ask.sport
    } else {
        None
    };
    let keys = walk_keys(backend);
    let Ok(units_left) = athlete.unread_between(&keys, floor, cursor, sport).await else {
        return DetailWalkAsked::NotRecorded;
    };
    let widen = WalkWiden {
        after_ts: ask.after_ts,
        walk_sport: ask.sport.map(str::to_owned),
        units_left,
    };
    match athlete
        .runtime
        .repos()
        .activity_backfill_jobs
        .widen_detail_walk(athlete.user_id, athlete.tenant_id, backend, &widen)
        .await
    {
        Ok(true) => DetailWalkAsked::Widened,
        Ok(false) => {
            info!(user_id = %athlete.user_id, backend, "Detail walk: the owed walk ended before the ask could widen it");
            DetailWalkAsked::NotRecorded
        }
        Err(e) => {
            warn!(user_id = %athlete.user_id, backend, error = %e, "Detail walk: the owed walk could not be widened");
            DetailWalkAsked::NotRecorded
        }
    }
}

/// The detail walk still owed for `backend`'s window, with its progress
/// ([`Pending::DetailWalkRunning`]), or `None` when none is owed.
///
/// Its ETA is the activities left at the slower of the pace its steps
/// measured and the one its budget allows: one activity a call, at most
/// [`DETAIL_WALK_DAILY_CALLS`] calls a day.
///
/// # Errors
///
/// Returns [`AppError`] when the ledger cannot be read.
pub async fn detail_walk_pending(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> AppResult<Option<Pending>> {
    let row = runtime
        .repos()
        .activity_backfill_jobs
        .detail_walk_row(user_id, tenant_id, backend)
        .await?;
    Ok(row
        .filter(|row| row.state == BackfillJobState::Owed)
        .map(|row| detail_walk_running(&row.progress)))
}

/// A running detail walk's [`Pending`] from its recorded progress
/// ([`Pending::detail_walk_running`]).
#[must_use]
pub fn detail_walk_running(progress: &JobProgress) -> Pending {
    let units = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
    let budget_pace = SECS_PER_DAY / f64::from(DETAIL_WALK_DAILY_CALLS);
    let pace = progress
        .secs_per_unit
        .map_or(budget_pace, |measured| measured.max(budget_pace));
    Pending::detail_walk_running(
        units(progress.units_done),
        progress.units_total.map(units),
        Some(pace),
    )
}

/// The keys a walk through `backend` reads candidates under: every key its
/// history is held under, never the athlete's own Strava export, which no
/// API detail is ever written beside.
fn walk_keys(backend: &str) -> Vec<&str> {
    history_store_keys(backend)
        .into_iter()
        .filter(|key| *key != STRAVA_ARCHIVE)
        .collect()
}

/// The athlete a walk reads for, and the runtime it reads through.
struct Athlete<'run> {
    runtime: &'run Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
}

/// One activity of a second of the window: its copies under the walk's keys.
#[derive(Debug)]
struct Unread {
    copies: Vec<CachedActivityRow>,
}

/// The activities started within one second of the window whose efforts no
/// copy holds.
#[derive(Debug)]
struct Second {
    /// The second (unix seconds).
    at: i64,
    /// Its activities to read.
    unread: Vec<Unread>,
}

/// One read of the window's stored rows below `top`: the seconds it holds
/// whole, newest first, and whether it reached the window's floor.
#[derive(Debug)]
struct Page {
    seconds: Vec<Second>,
    reached_floor: bool,
}

impl Athlete<'_> {
    /// Read the athlete's rows under `keys` started in `[floor, top)`, at
    /// most [`WALK_PAGE_ROWS`], as whole seconds: a full page's oldest second
    /// may hold more rows past its edge, so it is left to the next read,
    /// unless it is the page's only one.
    async fn page(
        &self,
        keys: &[&str],
        floor: i64,
        top: i64,
        sport: Option<&str>,
    ) -> AppResult<Page> {
        let (Some(start), Some(end)) = (
            Utc.timestamp_opt(floor, 0).single(),
            Utc.timestamp_opt(top, 0).single(),
        ) else {
            return Err(AppError::invalid_input(format!(
                "a detail walk window [{floor}, {top}) is out of range"
            )));
        };
        // `top` is excluded: the read's end is inclusive.
        let end: DateTime<Utc> = end - Duration::nanoseconds(1);
        if end < start {
            return Ok(Page {
                seconds: Vec::new(),
                reached_floor: true,
            });
        }
        let rows = self
            .runtime
            .repos()
            .activity_cache
            .get_cached_activity_rows(
                self.user_id,
                &self.tenant_id,
                &CachedRowsQuery::window(start, end, WALK_PAGE_ROWS).under_keys(keys),
            )
            .await?;
        let full = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= WALK_PAGE_ROWS;
        let mut seconds = by_second(rows, sport);
        if full && seconds.len() > 1 {
            seconds.pop();
        }
        Ok(Page {
            seconds,
            reached_floor: !full,
        })
    }

    /// How many activities started in `[floor, top)` under `keys`, of
    /// `sport`, no copy of which holds its efforts.
    async fn unread_between(
        &self,
        keys: &[&str],
        floor: i64,
        top: i64,
        sport: Option<&str>,
    ) -> Result<i64, ()> {
        let mut unread: i64 = 0;
        let mut top = top;
        loop {
            let page = self.page(keys, floor, top, sport).await.map_err(|e| {
                warn!(user_id = %self.user_id, error = %e, "Detail walk: the store could not be read to count the window");
            })?;
            for second in &page.seconds {
                unread = unread.saturating_add(i64::try_from(second.unread.len()).unwrap_or(0));
            }
            match page.seconds.last() {
                Some(oldest) if !page.reached_floor => top = oldest.at,
                _ => return Ok(unread),
            }
        }
    }
}

/// `rows`, newest first, as the seconds they started in, each holding its
/// activities of `sport` whose efforts no copy holds: the copies of one
/// activity are one ([`native_activity_key`]).
fn by_second(rows: Vec<CachedActivityRow>, sport: Option<&str>) -> Vec<Second> {
    let mut seconds: Vec<(i64, Vec<CachedActivityRow>)> = Vec::new();
    for row in rows {
        let at = row.activity.start_date().timestamp();
        match seconds.last_mut() {
            Some((last, rows)) if *last == at => rows.push(row),
            _ => seconds.push((at, vec![row])),
        }
    }
    seconds
        .into_iter()
        .map(|(at, rows)| Second {
            at,
            unread: unread_of(rows, sport),
        })
        .collect()
}

/// The service and its own activity id the copies of one activity share.
type NativeKey = (String, String);

/// The activities among one second's `rows` of `sport` no copy of which
/// holds its efforts.
fn unread_of(rows: Vec<CachedActivityRow>, sport: Option<&str>) -> Vec<Unread> {
    let mut activities: Vec<(Option<NativeKey>, Vec<CachedActivityRow>)> = Vec::new();
    for row in rows {
        if filter_activities_by_sport_type(vec![row.activity.clone()], sport).is_empty() {
            continue;
        }
        let key = native_activity_key(&row.activity)
            .map(|(service, id)| (service.to_owned(), id.to_owned()));
        match activities
            .iter_mut()
            .find(|(seen, _)| key.is_some() && *seen == key)
        {
            Some((_, copies)) => copies.push(row),
            None => activities.push((key, vec![row])),
        }
    }
    activities
        .into_iter()
        .filter(|(_, copies)| !copies.iter().any(|copy| copy.segment_read))
        .map(|(_, copies)| Unread { copies })
        .collect()
}

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
pub(super) async fn run_detail_walk(job: &ActivityBackfillJob, recorded: Option<(JobLease, i64)>) {
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
#[derive(Debug, Clone, Copy)]
struct Progress {
    walked_to: i64,
    units_done: i64,
    rows_landed: i64,
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
    let Ok(taken) = take_seconds(&athlete, &keys, floor, cursor, job.walk_sport.as_deref()).await
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
                Read::Skipped => progress.units_done += 1,
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
    }
    if taken.reached_floor {
        progress.walked_to = floor;
    }
    Step::Walked {
        step: progress.step(),
        ended: taken.reached_floor,
    }
}

/// The whole seconds a step takes below its cursor.
#[derive(Debug)]
struct Taken {
    seconds: Vec<Second>,
    /// Whether the seconds taken are every one down to the floor.
    reached_floor: bool,
}

impl Taken {
    /// Where the cursor moves once every second taken is finished.
    fn walked_to(&self, floor: i64, cursor: i64) -> i64 {
        if self.reached_floor {
            floor
        } else {
            self.seconds.last().map_or(cursor, |oldest| oldest.at)
        }
    }
}

/// Take whole seconds of the window below `cursor`, newest first, until
/// they hold [`DETAIL_WALK_STEP_ACTIVITIES`] activities to read, the floor
/// is reached, or [`WALK_PAGES_PER_STEP`] reads found too few. A second whose
/// activities would carry the step past its count is left to the next step,
/// unless the step holds none yet.
async fn take_seconds(
    athlete: &Athlete<'_>,
    keys: &[&str],
    floor: i64,
    cursor: i64,
    sport: Option<&str>,
) -> Result<Taken, ()> {
    let mut taken = Taken {
        seconds: Vec::new(),
        reached_floor: false,
    };
    let mut unread = 0_usize;
    let mut top = cursor;
    for _ in 0..WALK_PAGES_PER_STEP {
        let page = athlete.page(keys, floor, top, sport).await.map_err(|e| {
            warn!(user_id = %athlete.user_id, error = %e, "Detail walk: the store could not be read for the step");
        })?;
        for second in page.seconds {
            if unread >= DETAIL_WALK_STEP_ACTIVITIES && !second.unread.is_empty() {
                return Ok(taken);
            }
            unread += second.unread.len();
            top = second.at;
            taken.seconds.push(second);
        }
        if page.reached_floor {
            taken.reached_floor = true;
            return Ok(taken);
        }
    }
    Ok(taken)
}

/// What reading one activity's detail came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    /// Its detail is stored beside its row.
    Stored,
    /// The backend could not read it, or no row stands to store it beside:
    /// it is walked past.
    Skipped,
    /// A spent budget refused the read.
    Paused(Option<i64>),
    /// The backend's credential is dead.
    AuthRequired,
    /// The store failed.
    Failed,
    /// The step's row is no longer its own.
    Stopped,
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
        let detailed = match fetched {
            Ok(detailed) => detailed,
            Err(e) if e.code == ErrorCode::ExternalRateLimited => {
                info!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "Detail walk: a budget refused the read; the walk pauses");
                return Read::Paused(retry_at_ms(&e));
            }
            Err(e) if e.provider_auth_required_provider().is_some() => {
                warn!(user_id = %job.user_id, provider = %job.provider_name, error = %e, "Detail walk: the backend's credential is dead");
                return Read::AuthRequired;
            }
            Err(e) => {
                info!(user_id = %job.user_id, provider = %job.provider_name, activity_id = %id, error = %e, "Detail walk: the backend could not read an activity; walked past");
                return Read::Skipped;
            }
        };
        self.store(&listed.provider, &detailed).await
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_database::repositories::CachedActivityRow;

    use pierre_core::models::coverage::Pending;
    use pierre_database::repositories::JobProgress;

    use super::{by_second, detail_walk_running, Second};

    /// The unread activities of each second, by the first copy's id.
    fn unread_ids(seconds: &[Second]) -> HashMap<i64, Vec<String>> {
        seconds
            .iter()
            .map(|second| {
                let ids = second
                    .unread
                    .iter()
                    .filter_map(|unread| unread.copies.first())
                    .map(|copy| copy.activity.id().to_owned())
                    .collect();
                (second.at, ids)
            })
            .collect()
    }

    /// A row of `id` under `key`; a session's capture names Strava as its
    /// source, as sciotte's does.
    fn row(
        key: &str,
        id: &str,
        start: i64,
        sport: SportType,
        segment_read: bool,
    ) -> CachedActivityRow {
        let started = Utc.timestamp_opt(start, 0).single().expect("a start");
        let activity = ActivityBuilder::new(id, "Ride", sport, started, 3_600, key);
        let activity = if key == "sciotte" {
            activity.source("strava")
        } else {
            activity
        };
        CachedActivityRow {
            provider: key.to_owned(),
            activity: activity.build(),
            route: None,
            detail_read: segment_read,
            segment_read,
        }
    }

    #[test]
    fn a_walk_goes_no_faster_than_its_budget_allows() {
        let progress = |measured: f64| JobProgress {
            units_total: Some(60),
            units_done: 10,
            rows_landed: 8,
            secs_per_unit: Some(measured),
        };
        assert_eq!(
            detail_walk_running(&progress(5.0)),
            Pending::DetailWalkRunning {
                units_done: 10,
                units_total: Some(60),
                eta_secs: Some(50 * 1_728),
            },
            "bug: 50 activities left at 50 a day is a day, however fast a step measured"
        );
        assert_eq!(
            detail_walk_running(&progress(3_000.0)),
            Pending::DetailWalkRunning {
                units_done: 10,
                units_total: Some(60),
                eta_secs: Some(50 * 3_000),
            },
            "bug: a measured pace slower than the budget's is the walk's"
        );
    }

    #[test]
    fn the_copies_of_one_activity_are_read_once_and_any_read_copy_reads_it() {
        let rows = vec![
            row("strava", "101", 2_000, SportType::Ride, false),
            row("sciotte", "101", 2_000, SportType::Ride, false),
            row("strava", "102", 1_000, SportType::Ride, true),
            row("sciotte", "102", 1_000, SportType::Ride, false),
            row("sciotte", "103", 1_000, SportType::Run, false),
        ];
        let seconds = by_second(rows, Some("ride"));
        let ids = unread_ids(&seconds);
        assert_eq!(
            ids.get(&2_000).map(Vec::len),
            Some(1),
            "bug: the session's and the grant's copies of one ride are read twice: {ids:?}"
        );
        assert_eq!(
            ids.get(&1_000),
            Some(&Vec::new()),
            "bug: a ride a copy of which holds its efforts, or a run in a ride walk, is read: {ids:?}"
        );
    }
}
