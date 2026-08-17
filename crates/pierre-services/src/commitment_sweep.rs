// ABOUTME: Commitment sweep — counts due athlete commitments against real activity data and reports the verdict
// ABOUTME: Hourly background tick; labels met/partial/missed, defers on stale data, holds on a closed delivery route
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Commitment sweep
//!
//! The athlete says "three easy runs this week", the coach confirms it through
//! a tool call, and this is the half that checks. Every tick does two passes
//! over [`CommitmentRepository`]:
//!
//! 1. **Sweep** — open commitments whose window has closed. Count the matching
//!    activities the athlete actually recorded, then label the row met, partial
//!    or missed. Partial is a first-class verdict: two of three is the most
//!    common real result and the only one worth a conversation.
//! 2. **Report** — labeled verdicts that have not reached the athlete yet, one
//!    per athlete per day, handed to a [`CommitmentReporter`] that owns the
//!    channel policy.
//!
//! Two guards decide when *not* to speak, because the failure that destroys
//! trust here is telling someone they missed a run they actually did:
//!
//! - **Freshness.** The activity cache is warmed write-through by chat and tool
//!   fetches; nothing back-fills it on a schedule. An athlete who promised
//!   something and then never opened the app has an empty window that means "we
//!   do not know", not "they did nothing". A shortfall is only labeled once the
//!   cache has been refreshed past the window's end — otherwise the row defers
//!   and retries, and expires unlabeled after [`SWEEP_STALENESS_SECS`] rather
//!   than guessing.
//! - **Cadence.** One verdict per athlete per [`REPORT_CADENCE_SECS`]. A message
//!   for every missed commitment is a reason to mute the coach; the value is in
//!   being noticed, not in being scolded.
//!
//! The sweep never composes athlete-facing prose. It hands the reporter a
//! [`Commitment`] whose numbers came from the evaluator, and the reporter builds
//! the message from counts and the sanitized sport slug. An activity titled with
//! an injection payload can move a number; it can never author a sentence the
//! athlete receives.

use std::env;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, SportType, TenantId};
use pierre_database::repositories::SweptVerdict;
use pierre_database::RepositoryRegistry;
use pierre_memory::commitments::{Commitment, CommitmentOutcome};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Env var controlling how often the sweep scans for due commitments.
pub const COMMITMENT_SWEEP_INTERVAL_ENV_VAR: &str = "PIERRE_COMMITMENT_SWEEP_INTERVAL_SECS";

/// Default scan cadence — hourly. A commitment window is days long, so an
/// hourly tick is timely without being chatty.
const DEFAULT_COMMITMENT_SWEEP_INTERVAL_SECS: u64 = 3600;

/// How many rows each pass processes per tick.
pub const DEFAULT_BATCH_SIZE: i64 = 50;

/// Upper bound on activities pulled for one window. A window is at most 30 days,
/// so this never truncates a real working set.
const ACTIVITY_SCAN_LIMIT: i64 = 200;

/// How long a commitment may keep deferring before it is closed unlabeled.
///
/// The due scan is oldest-first and batch-capped, so without a backstop a row
/// whose data never arrives would head-of-line block every newer commitment.
///
/// LIMITATION(registre#32): `SWEEP_STALENESS_SECS` closes out a commitment the
/// activity cache never caught up with, and nothing warms that cache on a
/// schedule — so an athlete who promises something and then never opens a chat
/// gets no verdict at all, even if they held the promise.
pub const SWEEP_STALENESS_SECS: i64 = 7 * 24 * 3600;

/// How long a labeled verdict may wait for a delivery route before it is
/// dropped. "You missed two runs" is worth saying on Monday and embarrassing a
/// fortnight later.
pub const REPORT_STALENESS_SECS: i64 = 7 * 24 * 3600;

/// Minimum gap between two commitment verdicts reaching the same athlete.
pub const REPORT_CADENCE_SECS: i64 = 24 * 3600;

/// Two activity starts closer together than this are treated as one session.
///
/// An athlete with two providers connected has the same run cached twice — once
/// from Strava, once from Garmin — under different provider rows. Counting rows
/// would report three runs for a week that held two.
pub const DUPLICATE_SESSION_WINDOW_SECS: i64 = 600;

/// What a sweep concluded from the counted sessions.
///
/// Pure and total. A zero target cannot reach here through the tool, which
/// clamps to at least one, but if a corrupt row ever carries one it reads as met
/// — telling an athlete they failed a promise that asked for nothing is the one
/// outcome this feature must never produce.
#[must_use]
pub fn commitment_outcome(completed: u32, target: u32) -> CommitmentOutcome {
    if target == 0 || completed >= target {
        CommitmentOutcome::Met
    } else if completed == 0 {
        CommitmentOutcome::Missed
    } else {
        CommitmentOutcome::Partial
    }
}

/// Count distinct sessions from activity start times, collapsing near-duplicates.
///
/// `starts` need not be sorted. Two starts within [`DUPLICATE_SESSION_WINDOW_SECS`]
/// of each other count once; a morning and an evening run on the same day count
/// twice.
#[must_use]
pub fn count_sessions(starts: &[DateTime<Utc>]) -> u32 {
    let mut sorted: Vec<i64> = starts.iter().map(DateTime::timestamp).collect();
    sorted.sort_unstable();

    let mut counted: u32 = 0;
    let mut last: Option<i64> = None;
    for ts in sorted {
        if last.is_some_and(|prev| ts - prev < DUPLICATE_SESSION_WINDOW_SECS) {
            continue;
        }
        counted = counted.saturating_add(1);
        last = Some(ts);
    }
    counted
}

/// Whether the activity cache is fresh enough to read a shortfall as real.
///
/// `last_sync` is the most recent write-through into the cache across every
/// provider. A sync that predates the window's close means the athlete may well
/// have done the work and we simply have not fetched it.
#[must_use]
pub fn data_covers_window(last_sync: Option<DateTime<Utc>>, window_end: DateTime<Utc>) -> bool {
    last_sync.is_some_and(|synced| synced >= window_end)
}

/// Counted outcome of a single sweep tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    /// Due commitments examined this tick.
    pub scanned: usize,
    /// Commitments that got a verdict.
    pub labeled: usize,
    /// Commitments left open because the activity data had not caught up.
    pub deferred: usize,
    /// Commitments closed without a verdict, or verdicts dropped as stale.
    pub expired: usize,
    /// Verdicts delivered to an athlete.
    pub reported: usize,
    /// Verdicts held back by the cadence cap or a closed delivery route.
    pub held: usize,
    /// Per-row failures — each is logged and skipped, never fatal to the tick.
    pub errors: usize,
}

/// Delivers a commitment verdict back to the athlete.
///
/// Implemented in `pierre-server`, where the messaging rail and the push stack
/// both live; this crate cannot reach either. The implementation owns the
/// per-channel policy — Telegram and Slack take an unsolicited message any
/// time, `WhatsApp` and `Messenger` only inside their re-engagement window —
/// composes the message from the commitment's counts, never from stored text.
#[async_trait]
pub trait CommitmentReporter: Send + Sync {
    /// Deliver the verdict. Returns the route label it went out by
    /// (`"telegram"`, `"push"`, ...) or `None` when no route was open, in which
    /// case the sweep holds the verdict and retries on the next tick.
    async fn report(&self, commitment: &Commitment) -> Option<String>;
}

/// Run one sweep + report pass.
///
/// `now` is a parameter rather than a `Utc::now()` call so a test can place a
/// window in the past without sleeping.
///
/// # Errors
/// Returns an error only when a scan itself fails. Per-row failures increment
/// [`SweepOutcome::errors`] and the pass continues.
pub async fn tick(
    repos: &RepositoryRegistry,
    reporter: Option<&dyn CommitmentReporter>,
    now: DateTime<Utc>,
    batch_size: i64,
) -> AppResult<SweepOutcome> {
    let mut outcome = SweepOutcome::default();
    sweep_due(repos, now, batch_size, &mut outcome).await?;
    report_labeled(repos, reporter, now, batch_size, &mut outcome).await?;
    Ok(outcome)
}

/// Pass 1 — count due commitments and record their verdicts.
async fn sweep_due(
    repos: &RepositoryRegistry,
    now: DateTime<Utc>,
    batch_size: i64,
    outcome: &mut SweepOutcome,
) -> AppResult<()> {
    let due = repos
        .commitments
        .due_commitments(now.timestamp(), batch_size)
        .await?;
    outcome.scanned = due.len();

    for commitment in &due {
        let Some((user_id, tenant_id)) = parse_ids(commitment) else {
            close_out(repos, commitment, outcome).await;
            continue;
        };
        sweep_one(repos, commitment, user_id, &tenant_id, now, outcome).await;
    }
    Ok(())
}

/// Count the matching sessions inside a commitment's window.
///
/// `None` on a read failure, which the caller treats as "try again next tick"
/// rather than as zero — an unreadable cache is not evidence of anything.
async fn count_matching_sessions(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    user_id: Uuid,
    tenant_id: &TenantId,
) -> Option<u32> {
    let activities = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            tenant_id,
            None,
            commitment.window_start,
            commitment.window_end,
            ACTIVITY_SCAN_LIMIT,
        )
        .await
        .inspect_err(|e| {
            warn!(commitment_id = %commitment.id, error = %e, "activity read failed; will retry");
        })
        .ok()?;

    let want = commitment
        .sport
        .as_deref()
        .map(SportType::from_internal_string);
    let starts: Vec<DateTime<Utc>> = activities
        .iter()
        .filter(|a| want.as_ref().is_none_or(|w| a.sport_type() == w))
        .map(Activity::start_date)
        .collect();
    Some(count_sessions(&starts))
}

/// What to do with a commitment whose count came in under its target.
enum Shortfall {
    /// The cache has caught up past the window, so the shortfall is real.
    Believable,
    /// The data has not caught up yet — try again next tick.
    Defer,
    /// It never caught up and the row is past the staleness horizon.
    GiveUp,
    /// The freshness read itself failed.
    Unreadable,
}

/// Decide whether an under-target count is evidence or ignorance.
async fn classify_shortfall(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    user_id: Uuid,
    tenant_id: &TenantId,
    now: DateTime<Utc>,
) -> Shortfall {
    let last_sync = match repos
        .activity_cache
        .latest_activity_sync_any(user_id, tenant_id)
        .await
    {
        Ok(sync) => sync,
        Err(e) => {
            warn!(commitment_id = %commitment.id, error = %e, "sync freshness read failed; will retry");
            return Shortfall::Unreadable;
        }
    };
    if data_covers_window(last_sync, commitment.window_end) {
        Shortfall::Believable
    } else if now.timestamp() - commitment.window_end.timestamp() > SWEEP_STALENESS_SECS {
        Shortfall::GiveUp
    } else {
        Shortfall::Defer
    }
}

/// Write the verdict and announce it.
async fn write_verdict(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    completed: u32,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
) {
    let verdict = commitment_outcome(completed, commitment.target_sessions);
    let written = repos
        .commitments
        .record_commitment_verdict(&SweptVerdict {
            tenant_id: &commitment.tenant_id,
            commitment_id: &commitment.id,
            outcome: verdict,
            completed_sessions: completed,
            at: now,
        })
        .await;

    match written {
        Ok(true) => {
            outcome.labeled += 1;
            info!(
                target: "notify",
                event = "commitment.swept",
                tenant_id = %commitment.tenant_id,
                user_id = %commitment.user_id,
                outcome = verdict.as_str(),
                completed = completed,
                target_sessions = commitment.target_sessions,
                "counted a due athlete commitment"
            );
        }
        // Another tick already claimed the row — the conditional UPDATE is what
        // makes that a no-op instead of a double count.
        Ok(false) => debug!(commitment_id = %commitment.id, "commitment already swept"),
        Err(e) => {
            error!(commitment_id = %commitment.id, error = %e, "failed to record commitment verdict");
            outcome.errors += 1;
        }
    }
}

/// Count one commitment and write its verdict, or leave it open to retry.
async fn sweep_one(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    user_id: Uuid,
    tenant_id: &TenantId,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
) {
    let Some(completed) = count_matching_sessions(repos, commitment, user_id, tenant_id).await
    else {
        outcome.errors += 1;
        return;
    };

    // A met target needs no freshness guard: the sessions are already here. Only
    // a shortfall has to be distinguished from a cache that has not caught up.
    if completed < commitment.target_sessions {
        match classify_shortfall(repos, commitment, user_id, tenant_id, now).await {
            Shortfall::Believable => {}
            Shortfall::Defer => {
                outcome.deferred += 1;
                return;
            }
            Shortfall::GiveUp => {
                debug!(commitment_id = %commitment.id, "commitment data never caught up; expiring");
                close_out(repos, commitment, outcome).await;
                return;
            }
            Shortfall::Unreadable => {
                outcome.errors += 1;
                return;
            }
        }
    }

    write_verdict(repos, commitment, completed, now, outcome).await;
}

/// Pass 2 — deliver verdicts that have not reached their athlete yet.
async fn report_labeled(
    repos: &RepositoryRegistry,
    reporter: Option<&dyn CommitmentReporter>,
    now: DateTime<Utc>,
    batch_size: i64,
    outcome: &mut SweepOutcome,
) -> AppResult<()> {
    let pending = repos.commitments.unreported_commitments(batch_size).await?;
    if pending.is_empty() {
        return Ok(());
    }

    // Without a reporter installed (a build with no messaging rail) the sweep
    // still labels and still ages verdicts out, so the queue cannot grow without
    // bound while nothing can deliver.
    let Some(reporter) = reporter else {
        for commitment in &pending {
            age_out_or_hold(repos, commitment, now, outcome).await;
        }
        return Ok(());
    };

    // One verdict per athlete per tick as well as per day — two commitments
    // closing the same hour must not arrive as two messages.
    let mut spoken_to: Vec<String> = Vec::new();

    for commitment in &pending {
        if is_verdict_stale(commitment, now) {
            close_out(repos, commitment, outcome).await;
            continue;
        }
        if spoken_to.contains(&commitment.user_id) {
            outcome.held += 1;
            continue;
        }
        if deliver_one(repos, reporter, commitment, now, outcome).await {
            // Recorded the moment the message goes out, not after the mark
            // lands: the athlete has now heard from us whatever the write does
            // next, and a failed mark must not let the next row message them
            // again inside the same tick.
            spoken_to.push(commitment.user_id.clone());
        }
    }
    Ok(())
}

/// Close out a verdict too old to be worth saying, or leave it queued.
async fn age_out_or_hold(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
) {
    if is_verdict_stale(commitment, now) {
        close_out(repos, commitment, outcome).await;
    } else {
        outcome.held += 1;
    }
}

/// Try to deliver one verdict. Returns whether a message actually went out.
async fn deliver_one(
    repos: &RepositoryRegistry,
    reporter: &dyn CommitmentReporter,
    commitment: &Commitment,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
) -> bool {
    match cadence_allows(repos, commitment, now).await {
        Ok(true) => {}
        Ok(false) => {
            outcome.held += 1;
            return false;
        }
        Err(e) => {
            warn!(commitment_id = %commitment.id, error = %e, "cadence read failed; holding verdict");
            outcome.errors += 1;
            return false;
        }
    }

    let Some(route) = reporter.report(commitment).await else {
        // No open route today — hold the verdict rather than burning it. The
        // Meta channels both close outside their re-engagement window, and both
        // reopen the moment the athlete speaks again.
        outcome.held += 1;
        return false;
    };

    mark_and_announce(repos, commitment, &route, now, outcome).await;
    true
}

/// Record that the verdict was delivered, and say so on the notify channel.
///
/// Send-then-mark is deliberately at-least-once. Marking first would make a
/// failed send a verdict the athlete never hears and the sweep never retries;
/// this way the rare failed mark costs a repeat rather than a silence.
async fn mark_and_announce(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    route: &str,
    now: DateTime<Utc>,
    outcome: &mut SweepOutcome,
) {
    match repos
        .commitments
        .mark_commitment_reported(&commitment.tenant_id, &commitment.id, now)
        .await
    {
        Ok(true) => {
            outcome.reported += 1;
            info!(
                target: "notify",
                event = "commitment.reported",
                tenant_id = %commitment.tenant_id,
                user_id = %commitment.user_id,
                outcome = commitment.outcome.map_or("unknown", CommitmentOutcome::as_str),
                route = %route,
                "delivered a commitment verdict"
            );
        }
        Ok(false) => debug!(commitment_id = %commitment.id, "commitment verdict already reported"),
        Err(e) => {
            error!(commitment_id = %commitment.id, error = %e, "failed to mark commitment reported");
            outcome.errors += 1;
        }
    }
}

/// Whether this athlete has heard from the sweep too recently to hear again.
async fn cadence_allows(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    let last = repos
        .commitments
        .last_commitment_report(&commitment.tenant_id, &commitment.user_id)
        .await?;
    Ok(last.is_none_or(|at| now.timestamp() - at.timestamp() >= REPORT_CADENCE_SECS))
}

/// Whether a labeled verdict has aged past the point of being worth saying.
fn is_verdict_stale(commitment: &Commitment, now: DateTime<Utc>) -> bool {
    commitment
        .swept_at
        .is_some_and(|at| now.timestamp() - at.timestamp() > REPORT_STALENESS_SECS)
}

/// Close a commitment out without a verdict reaching the athlete.
async fn close_out(
    repos: &RepositoryRegistry,
    commitment: &Commitment,
    outcome: &mut SweepOutcome,
) {
    match repos
        .commitments
        .expire_commitment(&commitment.tenant_id, &commitment.id)
        .await
    {
        Ok(_) => outcome.expired += 1,
        Err(e) => {
            error!(commitment_id = %commitment.id, error = %e, "failed to expire commitment");
            outcome.errors += 1;
        }
    }
}

/// Recover the typed ids a data read needs. A row whose ids are not UUIDs can
/// never be swept, so it is closed out rather than retried forever.
fn parse_ids(commitment: &Commitment) -> Option<(Uuid, TenantId)> {
    if let (Ok(user_id), Ok(tenant_id)) = (
        Uuid::parse_str(&commitment.user_id),
        TenantId::parse_str(&commitment.tenant_id),
    ) {
        Some((user_id, tenant_id))
    } else {
        warn!(commitment_id = %commitment.id, "commitment has non-UUID tenant/user; expiring");
        None
    }
}

/// Spawn the background sweep loop.
///
/// Fire-and-forget, like every sibling scheduler: the task owns its own error
/// handling, a panicking tick is caught so the daemon survives it, and the first
/// immediate tick is skipped so a restart does not slam the database.
pub fn spawn_commitment_sweep(
    repos: Arc<RepositoryRegistry>,
    reporter: Option<Arc<dyn CommitmentReporter>>,
) {
    let interval_secs = env::var(COMMITMENT_SWEEP_INTERVAL_ENV_VAR)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_COMMITMENT_SWEEP_INTERVAL_SECS);

    debug!(interval_secs, "starting commitment sweep");

    tokio::spawn(async move {
        // `interval(0)` panics, so a misconfigured env var clamps to one second.
        let mut ticker = interval(Duration::from_secs(interval_secs.max(1)));
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            let pass = tick(&repos, reporter.as_deref(), Utc::now(), DEFAULT_BATCH_SIZE);
            match AssertUnwindSafe(pass).catch_unwind().await {
                Ok(Ok(outcome)) => {
                    if outcome.labeled > 0 || outcome.reported > 0 {
                        debug!(
                            scanned = outcome.scanned,
                            labeled = outcome.labeled,
                            deferred = outcome.deferred,
                            expired = outcome.expired,
                            reported = outcome.reported,
                            held = outcome.held,
                            errors = outcome.errors,
                            "commitment sweep tick complete"
                        );
                    }
                }
                Ok(Err(e)) => {
                    error!(error = %e, "commitment sweep tick errored — retrying next interval");
                }
                Err(_) => error!("commitment sweep tick panicked; continuing (see stderr)"),
            }
        }
    });
}
