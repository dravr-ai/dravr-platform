// ABOUTME: The one-time history import a usr/pwd sign-in beside a Garmin grant runs, and the retirement that ends it
// ABOUTME: Rides the backfill rail on the session itself, then wellness, then drops the session without purging a row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One-time history imports (carnet#1386).
//!
//! An athlete whose Garmin grant serves their reads may still sign in with
//! their Garmin username and password: that session copies what came before
//! the grant, once. The copy is a backfill job like any other — recorded
//! before it spawns, leased while it runs, resumed by the sweep if its
//! instance dies — that differs in three ways:
//!
//! - it reads the session itself ([`crate::protocol::AuthService::create_session_provider`]),
//!   never the grant routing would pick, and stores under the session's key,
//!   which the grant's window serves ([`pierre_providers::backend_resolver::history_keys`]);
//! - it asks for the whole account ([`WHOLE_ACCOUNT_AFTER`]), so the
//!   write-through's prune window spans the whole import and cannot reach it;
//! - its end, whatever it is, retires the session
//!   ([`retire_history_session`]): copied rows stay, the session goes, and
//!   nothing ever reads, renews, retries or notifies it again. A session
//!   that expired before its history landed leaves the job row behind as
//!   incomplete, which the provider card reads; it never reaches the athlete
//!   as a reconnect.
//!
//! The wellness half runs once the activities have landed: the year of days
//! before the grant (`WELLNESS_HISTORY_DAYS`), read inside
//! [`history_import_scope`], the only sync that reads a session beside a
//! push-fed grant.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::repositories::{ActivityBackfillJobRow, HistoryImportWindow, JobLease};
use pierre_providers::backend_resolver::{mirror_backend_for, session_imports_history};
use pierre_providers::core::ActivityQueryParams;
use pierre_services::history_import::HistoryImports;
use pierre_services::provider_revocation::{retire_history_session, HistoryRetirement};
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_backfill::{
    provider_tenant_id_str, spawn_activity_backfill, ActivityBackfillJob, BackfillRunOutcome,
    MAX_BACKFILL_ATTEMPTS,
};
use crate::implementations::data_helpers::historical_backfill_fetch_limit;
use crate::runtime::ToolRuntime;
#[cfg(feature = "health-sync")]
use chrono::{Days, NaiveDate};
#[cfg(feature = "health-sync")]
use dravr_enforme::error::EnformeResult;
#[cfg(feature = "health-sync")]
use pierre_services::garmin_push;

/// The `after` a history import asks for, in unix seconds:
/// 2000-01-01T00:00:00Z, older than any Garmin Connect account.
///
/// The whole account, as `ChefFamille` decided (carnet#1386). Its depth is also
/// what keeps the import from pruning itself: a backfill prunes its key below
/// `max(retention, now - after)`, so a floor this deep prunes nothing the
/// import wrote. How many activities one run copies is bounded by the
/// operator's historical fetch limit; a run that fills it records the oldest
/// one it reached as its coverage, never the floor.
pub const WHOLE_ACCOUNT_AFTER: i64 = 946_684_800;

/// A backfill job that is a usr/pwd session's one-time history import.
pub(crate) struct HistoryImport;

/// How an import's run ends.
enum Ending {
    /// What it copied landed: the session is retired and the row removed.
    Landed,
    /// The page filled its fetch limit: the row stays owed for the page
    /// below the oldest activity reached, and the session stays for it.
    NextPage(i64),
    /// It ended before its history landed: the session is retired and the
    /// row kept as incomplete.
    Ended,
    /// A transient failure: the row stays owed and the session in place for
    /// the resume sweep's next run.
    Retry,
}

impl HistoryImport {
    /// `Some` when `job` was asked for as a history import
    /// ([`ActivityBackfillJob::history_import`]) and its session still stands
    /// beside a grant whose reads serve what it copies
    /// ([`session_imports_history`]). An import whose grant went away
    /// meanwhile runs as the ordinary backfill it then is.
    pub(crate) async fn of(job: &ActivityBackfillJob) -> Option<Self> {
        if !job.history_import {
            return None;
        }
        session_imports_history(
            job.resources.repos(),
            job.user_id,
            job.tenant_id,
            &job.provider_name,
        )
        .await
        .then_some(Self)
    }

    /// End the import's run on `outcome`: the session's wellness once its
    /// activities landed, then the session's retirement, then the job row —
    /// removed when the history landed, kept as incomplete when it did not.
    ///
    /// A page that filled its fetch limit leaves the row owed for the next
    /// page, and a transient failure with attempts left leaves it owed for
    /// the resume sweep's next run; the session stays for either. `row` is
    /// `None` for a run the ledger could not record: such a run is never
    /// resumed, so a transient failure keeps the session for the athlete's
    /// next sign-in instead of retiring it.
    pub(crate) async fn settle(
        &self,
        job: &ActivityBackfillJob,
        row: Option<(&JobLease, i64)>,
        outcome: &BackfillRunOutcome,
    ) {
        let attempts_left = row.is_none_or(|(_, attempts)| attempts < MAX_BACKFILL_ATTEMPTS);
        let ending = match ending_of(job, row, outcome) {
            Ending::Landed => match import_wellness(job).await {
                WellnessCopy::Copied => Ending::Landed,
                WellnessCopy::Failed if attempts_left => Ending::Retry,
                WellnessCopy::Failed => Ending::Ended,
            },
            other => other,
        };
        let landed = match ending {
            Ending::Retry => {
                info!(
                    user_id = %job.user_id,
                    provider = %job.provider_name,
                    recorded = row.is_some(),
                    "History import: failed; the session stays for the next run"
                );
                return;
            }
            Ending::NextPage(before_ts) => {
                if let Some((lease, _)) = row {
                    page_down(job, lease, before_ts).await;
                }
                return;
            }
            Ending::Landed => true,
            Ending::Ended => false,
        };
        retire(job, &job.provider_name).await;
        match row {
            Some((lease, _)) => record_end(job, lease, landed).await,
            None if !landed => warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                "History import: ended before its history landed, with no job row to say so"
            ),
            None => {}
        }
    }
}

/// How the run's activity copy ended.
fn ending_of(
    job: &ActivityBackfillJob,
    row: Option<(&JobLease, i64)>,
    outcome: &BackfillRunOutcome,
) -> Ending {
    match outcome {
        BackfillRunOutcome::Completed => Ending::Landed,
        BackfillRunOutcome::Filled { oldest_ts } if row.is_some() => Ending::NextPage(*oldest_ts),
        BackfillRunOutcome::Filled { .. } => {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                "History import: filled its fetch limit with no job row to page down from; ending at that depth"
            );
            Ending::Landed
        }
        BackfillRunOutcome::Failed
            if row.is_none_or(|(_, attempts)| attempts < MAX_BACKFILL_ATTEMPTS) =>
        {
            Ending::Retry
        }
        BackfillRunOutcome::AuthRequired | BackfillRunOutcome::Failed => Ending::Ended,
    }
}

/// Leave the import's row owed for the page below `before_ts`.
async fn page_down(job: &ActivityBackfillJob, lease: &JobLease, before_ts: i64) {
    let paged = job
        .resources
        .repos()
        .activity_backfill_jobs
        .page_backfill_job_down(lease, before_ts)
        .await;
    match paged {
        Ok(true) => info!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            before_ts,
            "History import: a page filled its limit; the next page reads below it"
        ),
        Ok(false) => warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            "History import: the job row is no longer this run's; not paging down"
        ),
        Err(e) => warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            error = %e,
            "History import: the next page could not be recorded; the sweep re-runs this one"
        ),
    }
}

/// Record the import's end on its job row: removed when the history landed,
/// kept as incomplete when it did not.
async fn record_end(job: &ActivityBackfillJob, lease: &JobLease, landed: bool) {
    let jobs = &job.resources.repos().activity_backfill_jobs;
    let recorded = if landed {
        jobs.finish_backfill_job(lease).await
    } else {
        jobs.end_backfill_job_incomplete(lease).await
    };
    let failure = match recorded {
        Ok(true) => {
            info!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                landed,
                "History import: ended"
            );
            return;
        }
        Ok(false) => "the job row is no longer this run's".to_owned(),
        Err(e) => e.to_string(),
    };
    warn!(
        user_id = %job.user_id,
        provider = %job.provider_name,
        error = %failure,
        "History import: its end could not be recorded"
    );
}

/// How the wellness half of an import went.
enum WellnessCopy {
    /// It was read and written, or this build or server holds no wellness
    /// sync for the provider, which leaves nothing to copy.
    Copied,
    /// The read or the write failed: what the session holds is not in yet.
    Failed,
}

/// Read the session's wellness once, over the [`WELLNESS_HISTORY_DAYS`]
/// ending the day before the grant's pushes began ([`wellness_window`]).
///
/// The windowed backfill reads each of those days once without moving the
/// incremental sync's cursor. The read never asks for a day the push holds,
/// and [`history_import_scope`] still keeps the write off every such day,
/// whose readings a whole-day write would blank. A read that left any day
/// unread is a failure, so the import's retry runs while attempts remain.
#[cfg(feature = "health-sync")]
async fn import_wellness(job: &ActivityBackfillJob) -> WellnessCopy {
    use pierre_providers::backend_resolver::user_facing_name;
    use pierre_services::health_sync::history_import_scope;

    let Some(orchestrator) = job.resources.sync_orchestrator() else {
        return WellnessCopy::Copied;
    };
    let provider = user_facing_name(&job.provider_name);
    if !orchestrator.provider_names().contains(&provider) {
        return WellnessCopy::Copied;
    }
    // Without the grant's first day no day can be told the push's: copy none.
    let Some(pushed_from) = pushed_from(job, provider).await else {
        return WellnessCopy::Failed;
    };
    // A grant made on the oldest representable day has no day before it.
    let Some((start, end)) = wellness_window(pushed_from) else {
        return WellnessCopy::Copied;
    };
    let user_id = job.user_id.to_string();
    let read = orchestrator.backfill_between(&user_id, provider, start, end);
    let copy = wellness_copy_of(
        history_import_scope(job.user_id, pushed_from, read)
            .await
            .map(|landed| {
                (
                    landed
                        .records_created
                        .saturating_add(landed.records_updated),
                    landed.records_errored,
                )
            }),
    );
    match &copy {
        Ok((written, errored)) => info!(
            user_id = %job.user_id,
            provider,
            records = written,
            errored,
            "History import: copied the session's wellness"
        ),
        Err(why) => warn!(
            user_id = %job.user_id,
            provider,
            error = %why,
            "History import: the session's wellness could not be copied"
        ),
    }
    if copy.is_ok() {
        WellnessCopy::Copied
    } else {
        WellnessCopy::Failed
    }
}

/// What the windowed backfill's `result` — the records it wrote and the
/// days it could not read — copied, or why the copy is not complete. Any
/// unread day fails the copy, whatever else landed: it is history the
/// session still holds and only a retry can bring in. A read that wrote
/// nothing cleanly found no reading on any day, or held no session the
/// provider reads.
#[cfg(feature = "health-sync")]
fn wellness_copy_of(result: EnformeResult<(u32, u32)>) -> Result<(u32, u32), String> {
    let (written, errored) = result.map_err(|e| e.to_string())?;
    if errored > 0 {
        return Err(format!(
            "{errored} day(s) could not be read ({written} records landed)"
        ));
    }
    Ok((written, errored))
}

/// No health sync in this build: no wellness to copy.
#[cfg(not(feature = "health-sync"))]
async fn import_wellness(_job: &ActivityBackfillJob) -> WellnessCopy {
    WellnessCopy::Copied
}

/// How many days of wellness before the grant a history import copies: a
/// year, as `ChefFamille` decided (carnet#1386). It is the depth a season's
/// baselines (resting heart rate, HRV, sleep) are read against, and it stays
/// inside the windowed backfill's bound
/// ([`dravr_enforme::orchestrator::backfill::MAX_BACKFILL_WINDOW_DAYS`]).
/// Older days are left on the account.
#[cfg(feature = "health-sync")]
const WELLNESS_HISTORY_DAYS: u64 = 365;

/// The inclusive day window a history import reads wellness over: the
/// [`WELLNESS_HISTORY_DAYS`] ending the day before `pushed_from`, the grant's
/// first day, which the push holds on. `None` when no day precedes it.
#[cfg(feature = "health-sync")]
fn wellness_window(pushed_from: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let end = pushed_from.checked_sub_days(Days::new(1))?;
    let start = end
        .checked_sub_days(Days::new(WELLNESS_HISTORY_DAYS - 1))
        .unwrap_or(NaiveDate::MIN);
    Some((start, end))
}

/// The first day the grant's pushes hold `provider`'s wellness: the earlier
/// of the day its connection was made and the first day a push actually
/// stored ([`garmin_push::first_pushed_wellness_day`]).
///
/// A reconnect restamps the connection's `connected_at`, so it alone could
/// move the first day later than what the push already stored, and the
/// import would then overwrite those days; the stored rows keep it at or
/// before them. `None` when the connection or the stored rows cannot be
/// read, or the connection is not there, so no day can be told apart from
/// the push's.
#[cfg(feature = "health-sync")]
async fn pushed_from(job: &ActivityBackfillJob, provider: &str) -> Option<NaiveDate> {
    let repos = job.resources.repos();
    let connected = match repos
        .provider_connections
        .get_for_user(job.user_id, Some(job.tenant_id))
        .await
    {
        Ok(connections) => connections
            .iter()
            .filter(|c| c.provider == provider && c.connection_type != ConnectionType::Delegated)
            .map(|c| c.connected_at.date_naive())
            .min(),
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History import: the grant's connection could not be read; copying no wellness");
            return None;
        }
    };
    let Some(connected) = connected else {
        warn!(user_id = %job.user_id, "History import: the grant has no connection row; copying no wellness");
        return None;
    };
    match garmin_push::first_pushed_wellness_day(repos, job.user_id, job.tenant_id).await {
        Ok(stored) => Some(stored.map_or(connected, |stored| stored.min(connected))),
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History import: the days the push holds could not be read; copying no wellness");
            None
        }
    }
}

/// Retire the session the import read. A retirement that fails is logged:
/// the session then still stands beside the grant, where nothing reads it.
async fn retire(job: &ActivityBackfillJob, mirror: &str) {
    retire_session(&job.resources, job.user_id, job.tenant_id, mirror).await;
}

/// Retire `mirror`'s session for the user, logging a retirement that fails.
async fn retire_session(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    mirror: &str,
) {
    match retire_history_session(
        runtime.repos(),
        runtime.provider_registry().sciotte_client(),
        user_id,
        tenant_id,
        mirror,
    )
    .await
    {
        Ok(HistoryRetirement::Retired | HistoryRetirement::NotAMirror) => {}
        Ok(HistoryRetirement::KeptForWellness) => info!(
            user_id = %user_id,
            provider = %mirror,
            "History import: the session stays, the only source of the athlete's wellness"
        ),
        Err(e) => warn!(
            user_id = %user_id,
            provider = %mirror,
            error = %e,
            "History import: the session could not be retired"
        ),
    }
}

/// Start `backend`'s one-time history import for the user. Returns whether
/// an import is now on file for the session; `false` when one already was.
///
/// A job already in flight or owed for the session is an ordinary backfill
/// (a chat's or the capture rail's ask, recorded before the grant): it is
/// adopted as the import instead of refused as a duplicate
/// ([`pierre_database::repositories::ActivityBackfillJobRepository::adopt_as_history_import`]),
/// so its run cannot end the session's only import at its own narrow
/// window. An ordinary run in flight with no row at all leaves the import
/// recorded beside it, for the resume sweep.
pub async fn start_history_import(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    let limit = historical_backfill_fetch_limit();
    let started = spawn_activity_backfill(ActivityBackfillJob {
        resources: Arc::clone(runtime),
        user_id,
        tenant_id,
        tenant_id_str: provider_tenant_id_str(tenant_id),
        provider_name: backend.to_owned(),
        query_params: ActivityQueryParams {
            after: Some(WHOLE_ACCOUNT_AFTER),
            limit: Some(limit),
            ..ActivityQueryParams::default()
        },
        // No conversation asked: the card, not a chat, shows how it went.
        pierre_conversation_id: None,
        history_import: true,
    })
    .await;
    if started {
        return true;
    }
    let ledger = &runtime.repos().activity_backfill_jobs;
    let window = HistoryImportWindow {
        after_ts: WHOLE_ACCOUNT_AFTER,
        fetch_limit: Some(i64::try_from(limit).unwrap_or(i64::MAX)),
    };
    let adopted = match ledger
        .adopt_as_history_import(user_id, tenant_id, backend, window)
        .await
    {
        Ok(true) => true,
        Ok(false) => ledger
            .record_backfill_job(&released_import_row(user_id, tenant_id, backend, window))
            .await
            .unwrap_or_else(|e| {
                warn!(user_id = %user_id, error = %e, "History import: could not be recorded");
                false
            }),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "History import: the job owed for the session could not be adopted");
            false
        }
    };
    info!(
        user_id = %user_id,
        backend = %backend,
        adopted,
        "History import: a job was already running or owed for the session"
    );
    adopted
}

/// A history import's row recorded beside a run that holds the pair
/// unrecorded: free, so the resume sweep takes it once that run is over.
fn released_import_row(
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    window: HistoryImportWindow,
) -> ActivityBackfillJobRow {
    ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id,
        user_id,
        provider: backend.to_owned(),
        after_ts: Some(window.after_ts),
        before_ts: None,
        fetch_limit: window.fetch_limit,
        conversation_id: None,
        created_at_ms: Utc::now().timestamp_millis(),
        leased_until_ms: 0,
        attempts: 0,
        lease_token: None,
        history_import: true,
    }
}

/// Settle the usr/pwd session standing beside a `grant` the user just connected.
///
/// A live one runs one import and is then retired; a flagged one, or one
/// whose token row is gone, is retired at once. A user with no session, or
/// one whose session the grant does not serve, is left alone.
pub async fn settle_session_beside_grant(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    grant: &str,
) {
    let Some(mirror) = mirror_backend_for(grant) else {
        return;
    };
    if !session_imports_history(runtime.repos(), user_id, tenant_id, mirror).await {
        return;
    }
    match session_beside(runtime, user_id, tenant_id, mirror).await {
        SessionBeside::None => {}
        SessionBeside::Live => {
            let started = start_history_import(runtime, user_id, tenant_id, mirror).await;
            info!(
                user_id = %user_id,
                backend = %mirror,
                started,
                "History import: a live session stood beside the new grant; importing once"
            );
        }
        SessionBeside::Dead => {
            info!(
                user_id = %user_id,
                backend = %mirror,
                "History import: a dead session stood beside the new grant; retiring it"
            );
            retire_session(runtime, user_id, tenant_id, mirror).await;
        }
    }
}

/// What stands beside a new grant under its mirror.
enum SessionBeside {
    /// No session, or none that could be read.
    None,
    /// A session whose token row and unflagged connection both stand.
    Live,
    /// Anything else: a flagged connection, or one row without the other.
    Dead,
}

/// Read the `mirror` session the user holds in the tenant.
async fn session_beside(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    mirror: &str,
) -> SessionBeside {
    let repos = runtime.repos();
    let token = matches!(
        repos
            .oauth_tokens
            .get_token(user_id, tenant_id, mirror)
            .await,
        Ok(Some(_))
    );
    let connection = match repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
    {
        Ok(connections) => connections
            .into_iter()
            .find(|c| c.provider == mirror && c.connection_type != ConnectionType::Delegated),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "History import: the session beside the new grant could not be read");
            return SessionBeside::None;
        }
    };
    match (token, connection) {
        (false, None) => SessionBeside::None,
        (true, Some(connection)) if !connection.status.requires_reauth() => SessionBeside::Live,
        _ => SessionBeside::Dead,
    }
}

/// The [`HistoryImports`] the auth routes report to, over the tool runtime.
pub struct RuntimeHistoryImports {
    runtime: Arc<dyn ToolRuntime>,
}

impl RuntimeHistoryImports {
    /// Report to `runtime`.
    #[must_use]
    pub const fn new(runtime: Arc<dyn ToolRuntime>) -> Self {
        Self { runtime }
    }
}

impl fmt::Debug for RuntimeHistoryImports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeHistoryImports")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl HistoryImports for RuntimeHistoryImports {
    async fn import_history(&self, user_id: Uuid, tenant_id: TenantId, backend: &str) -> bool {
        start_history_import(&self.runtime, user_id, tenant_id, backend).await
    }

    async fn grant_connected(&self, user_id: Uuid, tenant_id: TenantId, grant: &str) {
        settle_session_beside_grant(&self.runtime, user_id, tenant_id, grant).await;
    }
}

#[cfg(feature = "health-sync")]
#[cfg(test)]
mod tests {
    use dravr_enforme::error::EnformeError;
    use dravr_enforme::orchestrator::backfill::MAX_BACKFILL_WINDOW_DAYS;

    use super::*;

    #[test]
    fn the_wellness_window_is_the_year_ending_the_day_before_the_grant() {
        let grant = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let (start, end) = wellness_window(grant).unwrap();
        assert_eq!(
            end,
            NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
            "the grant's own day is the push's"
        );
        assert_eq!(
            start,
            NaiveDate::from_ymd_opt(2025, 11, 5).unwrap(),
            "bug: the window starts a year before the grant"
        );
        let days = (end - start).num_days() + 1;
        assert_eq!(days, 365, "bug: the window is WELLNESS_HISTORY_DAYS long");
        assert!(
            days <= MAX_BACKFILL_WINDOW_DAYS,
            "the window fits the backfill's bound"
        );
    }

    #[test]
    fn no_day_precedes_a_grant_on_the_oldest_day() {
        assert_eq!(
            wellness_window(NaiveDate::MIN),
            None,
            "bug: no day precedes the oldest representable day"
        );
    }

    #[test]
    fn an_unread_day_fails_the_copy_even_when_records_landed() {
        assert_eq!(
            wellness_copy_of(Ok((120, 0))),
            Ok((120, 0)),
            "bug: a clean read copies what it wrote"
        );
        assert_eq!(
            wellness_copy_of(Ok((0, 0))),
            Ok((0, 0)),
            "bug: a clean read of no reading is a complete copy"
        );
        assert!(
            wellness_copy_of(Ok((120, 3))).is_err(),
            "bug: an unread day fails the copy though records landed"
        );
        assert!(
            wellness_copy_of(Ok((0, 365))).is_err(),
            "bug: a read of no day fails the copy"
        );
        assert!(
            wellness_copy_of(Err(EnformeError::config("inverted"))).is_err(),
            "bug: a failed read fails the copy"
        );
    }
}
