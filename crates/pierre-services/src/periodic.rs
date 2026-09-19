// ABOUTME: One tick loop for every background worker — due per the ledger, claimed, run, recorded, keep going
// ABOUTME: Replaces seven hand-written copies that had already drifted on panic handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Periodic background workers.
//!
//! Seven workers each spelled the same loop: build a `tokio::time::interval`,
//! consume the immediate first tick so a restart does not slam the database
//! before warm-up, then tick forever, logging a failed pass and retrying on the
//! next one. Copied by hand, they drifted where it mattered — two of the seven
//! caught a panicking pass and kept running, the other five let one bad tick
//! kill the worker silently for the life of the process.
//!
//! [`spawn_periodic`] is that loop, once. Each worker keeps its own `tick`
//! function, its own interval constant and its own outcome logging; what it
//! stops owning is the scaffolding around them.
//!
//! # The schedule belongs to the worker, not the process
//!
//! The first version waited one full `period` after boot before its first
//! tick, and restarted that countdown on every boot. On a scale-to-zero
//! service whose instances rarely live an hour, a worker with a period of a
//! day or a week therefore never ticked at all: the weekly digests were dead
//! features and the hourly sweeps ran only inside a sustained-traffic hour
//! with no deploy (carnet#459).
//!
//! The loop now reads the [`WorkerRunRepository`] ledger: a fresh instance
//! waits only until the worker is *due* — one period after the last tick
//! that finished, wherever it ran — and an overdue worker ticks shortly
//! after boot. Two instances booting together each ask the ledger for the
//! tick and one of them gets it; a tick that dies leaves the last-run stamp
//! untouched, so the next instance after the lease retries it rather than
//! waiting out another period.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::FutureExt as _;
use pierre_core::errors::AppResult;
use pierre_database::repositories::WorkerRunRepository;
use tokio::task::AbortHandle;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// The longest a worker waits for its first tick when the ledger says it is
/// due (or has never run). Short enough that a weekly digest three days
/// overdue lands on the instance that booted, long enough that nine workers
/// do not all fan out in the same second a fresh instance is still warming
/// up. A period shorter than this caps the wait at the period.
const FIRST_TICK_CAP: Duration = Duration::from_secs(30);

/// How long a claimed tick is held before another instance may take it
/// over. A sweep that is still running past this is treated as dead —
/// every tick here is idempotent, so the cost of a wrong guess is one
/// duplicate pass, and the cost of a long lease on a dead instance would be
/// a worker that nobody runs for a week. A tick that fails holds the worker
/// for one period or this, whichever is shorter: a one-minute sweep still
/// retries next minute, an hourly one within a quarter hour.
const TICK_LEASE: Duration = Duration::from_mins(15);

/// Run `tick` on a fixed period, in a spawned task, for as long as the
/// process lives — resuming the schedule the ledger records rather than
/// restarting it.
///
/// The first tick fires when the worker is due: `period` after the last
/// finished tick on record, or within [`FIRST_TICK_CAP`] when there is no
/// record or the worker is overdue. Every tick is claimed in the ledger
/// before it runs, so across instances a due tick runs once. A tick that
/// returns an error is logged and retried, and a tick that *panics* is
/// caught, logged and retried too — the worker outliving one bad pass is
/// the whole point of a best-effort sweep. Neither outcome stamps the
/// last-run time; both defer the worker by the shorter of `period` and
/// [`TICK_LEASE`], so the retry is next interval for a short worker and
/// within the lease for a long one, on whichever instance is alive then.
///
/// `name` appears on every line this loop logs and keys the ledger row, so
/// a worker is identifiable in production without reading the call site.
///
/// The returned [`AbortHandle`] stops the worker. Most callers discard it:
/// the workers are best-effort and a restart re-arms them.
pub fn spawn_periodic<F, Fut>(
    name: &'static str,
    period: Duration,
    ledger: Arc<dyn WorkerRunRepository>,
    mut tick: F,
) -> AbortHandle
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = AppResult<()>> + Send + 'static,
{
    // A zero period would spin; two callers read theirs from the
    // environment, so a misconfigured `=0` clamps to a second rather than
    // taking the process down at boot.
    let period = if period.is_zero() {
        Duration::from_secs(1)
    } else {
        period
    };
    let period_ms = i64::try_from(period.as_millis()).unwrap_or(i64::MAX);
    let lease_ms = i64::try_from(TICK_LEASE.as_millis()).unwrap_or(i64::MAX);
    let retry_hold_ms = period_ms.min(lease_ms);

    let handle = tokio::spawn(async move {
        info!(
            worker = name,
            interval_secs = period.as_secs(),
            "periodic worker started"
        );

        loop {
            sleep(wait_until_due(name, period, period_ms, ledger.as_ref()).await).await;

            let now = now_ms();
            match ledger
                .claim_worker_run(name, period_ms, now, lease_ms)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    debug!(
                        worker = name,
                        "tick not due or held elsewhere; re-reading the ledger"
                    );
                    continue;
                }
                Err(e) => {
                    // The ledger is unreachable; running unclaimed keeps the
                    // sweep alive (as before the ledger existed) at the cost
                    // of a possible duplicate pass on another instance.
                    warn!(worker = name, error = %e, "worker ledger claim failed; ticking unclaimed");
                }
            }

            match AssertUnwindSafe(tick()).catch_unwind().await {
                Ok(Ok(())) => {
                    if let Err(e) = ledger.finish_worker_run(name, now_ms()).await {
                        warn!(worker = name, error = %e, "worker ledger finish failed; the tick will repeat after its lease");
                    }
                    continue;
                }
                Ok(Err(e)) => {
                    error!(worker = name, error = %e, "tick errored — retrying next interval ({name})");
                }
                // The default panic hook has already printed the payload and
                // location to stderr; this line says which worker survived it.
                Err(_) => error!(
                    worker = name,
                    "tick panicked; continuing (see stderr) ({name})"
                ),
            }
            if let Err(e) = ledger
                .defer_worker_run(name, now_ms().saturating_add(retry_hold_ms))
                .await
            {
                warn!(worker = name, error = %e, "worker ledger defer failed; the retry waits out the lease");
            }
        }
    });

    handle.abort_handle()
}

/// How long to sleep before the next tick attempt, from the ledger.
///
/// Due, or never run: the capped first wait. Not yet due, or held by a
/// running or deferred tick: exactly until the later of the two ends — a
/// losing instance sleeps through a sibling's lease instead of polling the
/// claim. Ledger unreadable: the capped wait, so a database blip costs one
/// early claim attempt rather than a dead worker.
async fn wait_until_due(
    name: &'static str,
    period: Duration,
    period_ms: i64,
    ledger: &dyn WorkerRunRepository,
) -> Duration {
    let capped = period.min(FIRST_TICK_CAP);
    match ledger.get_worker_run(name).await {
        Ok(Some(run)) => {
            let now = now_ms();
            let due_at = if run.last_run_at_ms > 0 {
                run.last_run_at_ms.saturating_add(period_ms)
            } else {
                now
            };
            let remaining = due_at.max(run.leased_until_ms).saturating_sub(now);
            if remaining <= 0 {
                capped
            } else {
                Duration::from_millis(u64::try_from(remaining).unwrap_or(u64::MAX))
            }
        }
        Ok(None) => capped,
        Err(e) => {
            warn!(worker = name, error = %e, "worker ledger read failed; waiting the capped first interval");
            capped
        }
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
