// ABOUTME: The wellness a history walk of a usr/pwd session beside a push-fed grant copies once its activities are in
// ABOUTME: The year of nights and readings before the grant, read in the walk's scope so no day the push holds is written

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The wellness half of a walk (carnet#1386).
//!
//! A Garmin athlete whose grant is pushed every summary from the day they
//! connected still has the nights and readings before it on the account.
//! Their usr/pwd session serves their reads, but while the push grant behind
//! it is live the push is their wellness source, so no pull reads the
//! session (carnet#1494). A walk of that session copies the
//! [`WELLNESS_HISTORY_DAYS`] before the grant once its activities are in,
//! inside [`history_walk_scope`], the only sync that reads a session beside
//! a live push-fed grant.
//!
//! Each day it copies is one read of the athlete's session, counted in the
//! background burst every other background read shares (carnet#1661). Until
//! the walk's first copy completes, its days count in the first fill's own
//! day ([`in_wellness_first_fill`],
//! `sciotte_reads.wellness_first_fill_daily.<tier>`); the row then keeps that
//! it completed, and a later copy of the walk counts in the ordinary
//! background day. A walk the ledger could not record has no row to keep it
//! on, so its copy counts as a first fill. A cap that
//! refuses a day pauses the copy until the refusing window ends, with no
//! attempt spent, and the day it stopped at is kept on the walk's row: the
//! next run copies from there, never from the window's start again.

#[cfg(not(feature = "health-sync"))]
use std::future::{ready, Ready};

use super::super::ActivityBackfillJob;
#[cfg(feature = "health-sync")]
use chrono::{Days, NaiveDate, Utc};
#[cfg(feature = "health-sync")]
use dravr_enforme::error::{EnformeError, EnformeResult};
#[cfg(feature = "health-sync")]
use pierre_core::models::ConnectionType;
use pierre_database::repositories::JobLease;
#[cfg(feature = "health-sync")]
use pierre_providers::backend_resolver::{fallback_grant, pushed_backend_for};
#[cfg(feature = "health-sync")]
use pierre_providers::request_budget::{in_background, in_wellness_first_fill};
#[cfg(feature = "health-sync")]
use pierre_services::garmin_push;
#[cfg(feature = "health-sync")]
use pierre_services::health_sync::history_walk_scope;
#[cfg(feature = "health-sync")]
use pierre_services::sciotte_health_reader::{capped_scope, CappedDay};
#[cfg(feature = "health-sync")]
use tracing::{info, warn};

#[cfg(feature = "health-sync")]
use super::super::pause::retry_at_after;
#[cfg(feature = "health-sync")]
use super::super::MAX_BACKFILL_ATTEMPTS;
#[cfg(feature = "health-sync")]
use super::log_write;

/// What a walk's wellness copy came to.
#[cfg(feature = "health-sync")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WellnessCopy {
    /// Nothing of it is left to copy: copied, or the walk has none to copy.
    Done,
    /// A read or a write failed: a retry of the walk's last step copies
    /// again.
    Failed,
    /// A session-read cap refused a day: the copy resumes from `resume_day`
    /// once the refusing window ends, at `retry_at_ms` (unix ms).
    Paused {
        resume_day: NaiveDate,
        retry_at_ms: i64,
    },
}

/// Copy the wellness before the grant at the end of the walk `job` is, and
/// say whether the walk must wait instead of ending: `None` ends it (the
/// copy is done, has nothing to copy, or failed on the walk's last
/// attempt); `Some(kept)` leaves the row to the sweep, kept the step's
/// (`false` when it no longer is).
///
/// A copy that failed with attempts left is retried a lease from now. One a
/// session-read cap paused keeps the day it stopped at on the row `lease`
/// holds and waits out the refusing window, the attempt returned.
#[cfg(feature = "health-sync")]
pub(super) async fn copy_or_wait(
    job: &ActivityBackfillJob,
    lease: &JobLease,
    attempts: i64,
) -> Option<bool> {
    match copy_before_grant(job, Some(lease)).await {
        WellnessCopy::Failed if attempts < MAX_BACKFILL_ATTEMPTS => Some(log_write(
            "the wellness copy failed; the sweep re-runs the last step",
            Ok(true),
            job,
        )),
        WellnessCopy::Done | WellnessCopy::Failed => None,
        WellnessCopy::Paused {
            resume_day,
            retry_at_ms,
        } => {
            let paused = job
                .resources
                .repos()
                .activity_backfill_jobs
                .pause_wellness_copy(lease, job.tenant_id, resume_day, retry_at_ms)
                .await;
            Some(log_write(
                "a session-read cap paused the wellness copy; it resumes from the day it stopped at",
                paused,
                job,
            ))
        }
    }
}

/// No health sync in this build: no wellness to copy, so the walk ends.
/// Answers as a ready future so the walk awaits it exactly as it awaits the
/// copy.
#[cfg(not(feature = "health-sync"))]
pub(super) fn copy_or_wait(
    _job: &ActivityBackfillJob,
    _lease: &JobLease,
    _attempts: i64,
) -> Ready<Option<bool>> {
    ready(None)
}

/// Copy the wellness before the grant at the end of a walk the ledger could
/// not record, and say whether nothing of it is left to copy. A walk with no
/// row keeps no day to resume from: a copy that failed or that a session-read
/// cap paused is left unfinished, and the next ask walks and copies again.
#[cfg(feature = "health-sync")]
pub(super) async fn copy_unrecorded(job: &ActivityBackfillJob) -> bool {
    let copy = copy_before_grant(job, None).await;
    if copy != WellnessCopy::Done {
        info!(user_id = %job.user_id, provider = %job.provider_name, ?copy, "History walk: an unrecorded walk's wellness copy did not finish; the next ask copies again");
    }
    copy == WellnessCopy::Done
}

/// No health sync in this build: no wellness to copy.
#[cfg(not(feature = "health-sync"))]
pub(super) fn copy_unrecorded(_job: &ActivityBackfillJob) -> Ready<bool> {
    ready(true)
}

/// Copy the wellness before the grant when `job` walks a session whose read
/// plan stands a live push-fed grant behind it ([`fallback_grant`]), from the
/// day an earlier run of it paused at, when the row `lease` holds keeps one.
#[cfg(feature = "health-sync")]
async fn copy_before_grant(job: &ActivityBackfillJob, lease: Option<&JobLease>) -> WellnessCopy {
    use pierre_providers::backend_resolver::user_facing_name;

    let behind = fallback_grant(
        job.resources.repos(),
        job.user_id,
        job.tenant_id,
        &job.provider_name,
    )
    .await;
    if behind.is_none_or(|grant| pushed_backend_for(&grant) != Some(grant.as_str())) {
        return WellnessCopy::Done;
    }
    let Some(orchestrator) = job.resources.sync_orchestrator() else {
        return WellnessCopy::Done;
    };
    let provider = user_facing_name(&job.provider_name);
    if !orchestrator.provider_names().contains(&provider) {
        return WellnessCopy::Done;
    }
    // Without the grant's first day no day can be told the push's: copy none.
    let Some(pushed_from) = pushed_from(job, provider).await else {
        return WellnessCopy::Failed;
    };
    // A grant made on the oldest representable day has no day before it; a
    // copy paused past a window the grant's first day moved back holds none.
    let resume_from = resume_day(job, lease).await;
    let Some((start, end)) =
        wellness_window(pushed_from).and_then(|window| resumed_window(window, resume_from))
    else {
        return WellnessCopy::Done;
    };
    let user_id = job.user_id.to_string();
    // Boxed: the windowed backfill's future is large, and the walk's step
    // future holds it across every await below.
    let read = Box::pin(orchestrator.backfill_between(&user_id, provider, start, end));
    let scoped = history_walk_scope(job.user_id, pushed_from, read);
    // The first fill's days are counted in a day of their own, still inside
    // the background burst; a copy after it, and nothing else of the walk,
    // in the background day.
    let filled = first_filled(job, lease).await;
    let (landed, capped) = if filled {
        capped_scope(in_background(scoped)).await
    } else {
        capped_scope(in_wellness_first_fill(scoped)).await
    };
    // The records it wrote, and the days it could not read.
    let landed = landed.map(|landed| {
        (
            landed
                .records_created
                .saturating_add(landed.records_updated),
            landed.records_errored,
        )
    });
    let unread = landed.as_ref().map(|(_, unread)| *unread);
    if let Some(capped) = capped {
        if let Some(resume_day) = paused_at((start, end), capped, unread) {
            info!(
                user_id = %job.user_id,
                provider,
                %resume_day,
                retry_after_secs = capped.retry_after_secs,
                "History walk: a session-read cap paused the wellness copy"
            );
            return paused(resume_day, capped);
        }
    }
    let copy = copy_outcome(job, provider, landed);
    if copy == WellnessCopy::Done && !filled {
        mark_first_filled(job, lease).await;
    }
    copy
}

/// Whether the walk `lease` holds completed its first wellness fill; `false`
/// for a walk the ledger could not record, or whose row cannot be read, so
/// its copy counts as a first fill.
#[cfg(feature = "health-sync")]
async fn first_filled(job: &ActivityBackfillJob, lease: Option<&JobLease>) -> bool {
    let Some(lease) = lease else {
        return false;
    };
    let ledger = &job.resources.repos().activity_backfill_jobs;
    match ledger.wellness_first_filled(lease, job.tenant_id).await {
        Ok(filled) => filled,
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History walk: whether the first wellness fill completed could not be read; counting the copy as one");
            false
        }
    }
}

/// Record on the row `lease` holds that the walk's first wellness fill
/// completed; a walk with no row has nowhere to keep it. A write the ledger
/// refuses is logged: the next copy counts as a first fill again.
#[cfg(feature = "health-sync")]
async fn mark_first_filled(job: &ActivityBackfillJob, lease: Option<&JobLease>) {
    let Some(lease) = lease else {
        return;
    };
    let ledger = &job.resources.repos().activity_backfill_jobs;
    let marked = ledger
        .mark_wellness_first_filled(lease, job.tenant_id, Utc::now().timestamp_millis())
        .await;
    match marked {
        Ok(true) => {
            info!(user_id = %job.user_id, provider = %job.provider_name, "History walk: the first wellness fill completed");
        }
        Ok(false) => {
            warn!(user_id = %job.user_id, provider = %job.provider_name, "History walk: the row was no longer this step's; the first wellness fill is not recorded");
        }
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History walk: the first wellness fill could not be recorded; the next copy counts as one again");
        }
    }
}

/// The copy paused at `resume_day` until the window that refused `capped`
/// ends.
#[cfg(feature = "health-sync")]
fn paused(resume_day: NaiveDate, capped: CappedDay) -> WellnessCopy {
    WellnessCopy::Paused {
        resume_day,
        retry_at_ms: retry_at_after(Some(capped.retry_after_secs)),
    }
}

/// What a copy no cap stopped came to, by what its read `landed` (the
/// records it wrote, the days it could not read), logged.
#[cfg(feature = "health-sync")]
fn copy_outcome(
    job: &ActivityBackfillJob,
    provider: &str,
    landed: EnformeResult<(u32, u32)>,
) -> WellnessCopy {
    let copy = wellness_copy_of(landed);
    match &copy {
        Ok((written, errored)) => info!(
            user_id = %job.user_id,
            provider,
            records = written,
            errored,
            "History walk: copied the session's wellness"
        ),
        Err(why) => warn!(
            user_id = %job.user_id,
            provider,
            error = %why,
            "History walk: the session's wellness could not be copied"
        ),
    }
    if copy.is_ok() {
        WellnessCopy::Done
    } else {
        WellnessCopy::Failed
    }
}

/// The day the copy of the walk `lease` holds resumes from; `None` copies
/// the whole window, as a walk the ledger could not record, or whose day
/// cannot be read, does.
#[cfg(feature = "health-sync")]
async fn resume_day(job: &ActivityBackfillJob, lease: Option<&JobLease>) -> Option<NaiveDate> {
    let ledger = &job.resources.repos().activity_backfill_jobs;
    match ledger.wellness_resume_day(lease?, job.tenant_id).await {
        Ok(day) => day,
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History walk: the wellness copy's resume day could not be read; copying the whole window");
            None
        }
    }
}

/// The part of the inclusive day `window` a copy still reads: from
/// `resume_day` when an earlier run paused there, else all of it. `None`
/// when nothing of it is left.
#[cfg(feature = "health-sync")]
fn resumed_window(
    (start, end): (NaiveDate, NaiveDate),
    resume_day: Option<NaiveDate>,
) -> Option<(NaiveDate, NaiveDate)> {
    let start = resume_day.map_or(start, |day| day.max(start));
    (start <= end).then_some((start, end))
}

/// Where a copy of `window` a session-read cap refused at `capped` resumes,
/// by what the read came to: the days it could not read (`unread`), or its
/// error. `None` when the copy did not stop on the refusal alone, and so
/// fails: a store or credential error, or a day before the refusal that
/// could not be read.
///
/// enforme reads the window oldest day first and stops at the refusal, so
/// every day from it to the window's end is unread; a read reporting more
/// unread days than that also skipped a day before it. Pausing there would
/// re-read that day on every resume, refused at the same place each time,
/// without ever spending an attempt: it fails instead, so a day that never
/// reads ends the walk within its attempts, and the stored day stands. A
/// read refused before any day landed reports no count; it resumes only
/// when the refused day is the window's first, the one case where no day
/// before it can have failed.
#[cfg(feature = "health-sync")]
fn paused_at(
    (start, end): (NaiveDate, NaiveDate),
    capped: CappedDay,
    unread: Result<u32, &EnformeError>,
) -> Option<NaiveDate> {
    match unread {
        Ok(unread) => {
            let after_refusal = (end - capped.day).num_days().saturating_add(1).max(0);
            (i64::from(unread) <= after_refusal).then_some(capped.day)
        }
        Err(EnformeError::RateLimited { .. }) => (capped.day == start).then_some(start),
        Err(_) => None,
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

/// How many days of wellness before the grant a walk copies: a year, as
/// `ChefFamille` decided (carnet#1386). It is the depth a season's baselines
/// (resting heart rate, HRV, sleep) are read against, and it stays inside
/// the windowed backfill's bound
/// ([`dravr_enforme::orchestrator::backfill::MAX_BACKFILL_WINDOW_DAYS`]).
/// Older days are left on the account.
#[cfg(feature = "health-sync")]
const WELLNESS_HISTORY_DAYS: u64 = 365;

/// The inclusive day window a walk reads wellness over: the
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
/// move the first day later than what the push already stored, and the walk
/// would then overwrite those days; the stored rows keep it at or before
/// them. `None` when the connection or the stored rows cannot be read, or
/// the connection is not there, so no day can be told apart from the push's.
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
            warn!(user_id = %job.user_id, error = %e, "History walk: the grant's connection could not be read; copying no wellness");
            return None;
        }
    };
    let Some(connected) = connected else {
        warn!(user_id = %job.user_id, "History walk: the grant has no connection row; copying no wellness");
        return None;
    };
    match garmin_push::first_pushed_wellness_day(repos, job.user_id, job.tenant_id).await {
        Ok(stored) => Some(stored.map_or(connected, |stored| stored.min(connected))),
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History walk: the days the push holds could not be read; copying no wellness");
            None
        }
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

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn a_resumed_copy_reads_from_the_day_it_stopped_at_and_never_before_its_window() {
        let window = (day(1, 1), day(12, 31));
        assert_eq!(
            resumed_window(window, None),
            Some(window),
            "bug: a copy that never paused does not read its whole window"
        );
        assert_eq!(
            resumed_window(window, Some(day(6, 15))),
            Some((day(6, 15), day(12, 31))),
            "bug: a paused copy does not resume from the day it stopped at"
        );
        assert_eq!(
            resumed_window(window, Some(day(12, 31))),
            Some((day(12, 31), day(12, 31))),
            "bug: a copy stopped at its last day does not read that day"
        );
        // The grant's first day moved: the window moved with it.
        assert_eq!(
            resumed_window(window, NaiveDate::from_ymd_opt(2025, 3, 1)),
            Some(window),
            "bug: a resume day before the window reads days the window leaves out"
        );
        assert_eq!(
            resumed_window(window, NaiveDate::from_ymd_opt(2027, 1, 1)),
            None,
            "bug: a resume day past the window reads something"
        );
    }

    #[test]
    fn a_capped_copy_resumes_at_the_refused_day_and_fails_when_a_day_before_it_was_skipped() {
        let window = (day(1, 1), day(1, 31));
        let capped = CappedDay {
            day: day(1, 21),
            retry_after_secs: 900,
        };
        // Days 21..=31 unread: the refusal's tail, nothing else.
        assert_eq!(
            paused_at(window, capped, Ok(11)),
            Some(day(1, 21)),
            "bug: a copy the cap stopped does not resume at the refused day"
        );
        // One more unread day: one before the refusal failed. Pausing would
        // re-read it on every resume at no attempt's cost; the copy fails.
        assert_eq!(
            paused_at(window, capped, Ok(12)),
            None,
            "bug: a copy that skipped a day before the refusal pauses, and loops on it"
        );
        // Refused before any day landed: it resumes only when the refused
        // day is the window's first, so no day before it failed.
        let shed = EnformeError::RateLimited {
            provider: "garmin".to_owned(),
            retry_after_secs: 900,
        };
        let at_start = CappedDay {
            day: day(1, 1),
            ..capped
        };
        assert_eq!(
            paused_at(window, at_start, Err(&shed)),
            Some(day(1, 1)),
            "bug: a copy refused on its first day does not resume there"
        );
        assert_eq!(
            paused_at(window, capped, Err(&shed)),
            None,
            "bug: a copy refused after days that all failed pauses, and loops on them"
        );
        assert_eq!(
            paused_at(window, capped, Err(&EnformeError::store("disk full"))),
            None,
            "bug: a copy a store error failed is paused instead of failed"
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
