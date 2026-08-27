// ABOUTME: Tracks the detached background turns a webhook starts, so shutdown can drain them
// ABOUTME: Holds the TaskTracker every messaging turn is spawned into plus the drain signal it watches

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! In-flight turn lifecycle.
//!
//! A messaging webhook answers HTTP 200 the moment it has persisted the
//! inbound message, and the LLM turn it started keeps running afterwards.
//! That is deliberate — Telegram retries a webhook that takes seconds to
//! answer — but it means the turn is invisible to everything that reasons
//! about the process being busy. Cloud Run counts in-flight *requests*, so
//! from its side the instance went idle in the same second the athlete asked
//! their question, and any rollout or scaledown is free to take it.
//!
//! On 2026-08-26 one did: a group chart ask reached the tool loop, produced
//! 1542 characters of answer, opened a second session to write the final
//! reply, and the instance was drained mid-retry. The athlete is still
//! looking at the "génération de la réponse…" placeholder, because the
//! placeholder is only ever *edited* into the finished reply and a turn that
//! dies never edits anything.
//!
//! [`InFlightTurns`] is what the process knows about those turns:
//!
//! - every turn is spawned through it, so `len()` is the real count of work
//!   the instance would lose if it died right now;
//! - `drain` spends the SIGTERM grace window awaiting them instead of
//!   sleeping through it;
//! - when the grace runs out, `drain_token` fires and each turn still
//!   running gets the chance to close its placeholder honestly rather than
//!   leave it open forever.
//!
//! The token is a deadline signal, not a kill switch: nothing here aborts a
//! turn. A turn that ignores it simply dies with the process, exactly as it
//! did before — the tracker only ever adds chances to finish.

use std::future::Future;
#[cfg(unix)]
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use pierre_services::server_lifecycle;
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
#[cfg(unix)]
use tokio::time::sleep;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{info, warn};

/// What one [`InFlightTurns::drain`] spent its grace window on.
///
/// Logged verbatim on shutdown. The counts are the difference between "the
/// deploy was clean" and "two athletes lost their answer", which is not
/// visible from anywhere else — the turns leave no request trace behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainReport {
    /// Turns still running when SIGTERM arrived.
    pub in_flight_at_signal: usize,
    /// Turns still running after the grace window, i.e. the ones the drain
    /// signal was raised for.
    pub signalled: usize,
    /// Turns still running after the signal window too. These die with the
    /// process; each one is an athlete holding an open placeholder.
    pub abandoned: usize,
    /// Wall clock the whole drain consumed.
    pub elapsed: Duration,
}

impl DrainReport {
    /// Whether every tracked turn reached its own end.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.abandoned == 0
    }
}

/// The background turns this process is responsible for finishing.
///
/// Cloned handles share one tracker: `Arc<InFlightTurns>` on the server
/// context is the only instance, and the webhook route, the dispatcher and
/// the signal handler all reach the same counters through it.
pub struct InFlightTurns {
    tracker: TaskTracker,
    drain: CancellationToken,
}

impl InFlightTurns {
    /// An empty tracker accepting turns.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tracker: TaskTracker::new(),
            drain: CancellationToken::new(),
        }
    }

    /// Spawn a turn onto the runtime under this tracker.
    ///
    /// The `JoinHandle` is deliberately not returned: a turn has no result
    /// worth awaiting at the call site (it delivers its own reply, and its
    /// panic boundary lives inside `run_guarded`). What the caller gains by
    /// going through here is that the turn is now *countable* — shutdown can
    /// see it and wait for it.
    ///
    /// Spawning during a drain is allowed and tracked. Cloud Run stops
    /// routing to a draining instance, so this is the rare webhook already
    /// in flight when the signal landed; running it inside the tracker gives
    /// it the remaining grace window, where refusing it would lose the
    /// message outright.
    pub fn spawn<F>(&self, turn: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn(turn);
    }

    /// The signal a turn watches to learn the process is going away.
    ///
    /// Cancelled once the grace window has elapsed with turns still running.
    /// A turn holding one of these is expected to stop what it is doing and
    /// close its status placeholder — see
    /// `messaging_ingress::turn_guard::run_bounded`.
    #[must_use]
    pub fn drain_token(&self) -> CancellationToken {
        self.drain.clone()
    }

    /// How many turns are running right now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracker.len()
    }

    /// Whether no turn is running.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracker.is_empty()
    }

    /// Spend the shutdown grace window finishing turns.
    ///
    /// Two windows, because a turn has two different things left to do:
    ///
    /// 1. `grace` — await the turns as they are. Most of a turn's wall clock
    ///    is one LLM call, so a turn that started seconds ago usually
    ///    finishes here and the athlete never learns the instance changed.
    /// 2. `signal_window` — raise [`Self::drain_token`] for whatever is
    ///    left. Those turns give up their answer, but they are still alive
    ///    and can spend a second closing their placeholder with an honest
    ///    message. This window only has to cover one channel API edit.
    ///
    /// Both windows are bounded because the caller's own deadline is:
    /// Cloud Run sends SIGKILL when the termination grace period expires,
    /// and a drain that overruns it is indistinguishable from no drain at
    /// all. Returns as soon as the turns are done, so an idle instance pays
    /// nothing.
    pub async fn drain(&self, grace: Duration, signal_window: Duration) -> DrainReport {
        let started = Instant::now();
        let in_flight_at_signal = self.tracker.len();

        // Closing lets `wait()` return once the tracker empties. It does not
        // refuse later spawns — see `spawn`.
        self.tracker.close();

        if timeout(grace, self.tracker.wait()).await.is_ok() {
            return DrainReport {
                in_flight_at_signal,
                signalled: 0,
                abandoned: 0,
                elapsed: started.elapsed(),
            };
        }

        let signalled = self.tracker.len();
        warn!(
            in_flight = signalled,
            grace_secs = grace.as_secs(),
            "shutdown grace elapsed with turns still running; signalling drain"
        );
        self.drain.cancel();

        let _ = timeout(signal_window, self.tracker.wait()).await;
        let abandoned = self.tracker.len();

        DrainReport {
            in_flight_at_signal,
            signalled,
            abandoned,
            elapsed: started.elapsed(),
        }
    }
}

impl Default for InFlightTurns {
    fn default() -> Self {
        Self::new()
    }
}

/// Log one drain's outcome at the severity its worst case deserves.
///
/// An abandoned turn is an athlete who asked a question and will never be
/// told anything, so it is an ERROR even though the process is exiting
/// normally — the alternative is a shutdown that reports success while
/// dropping work.
fn log_drain(report: &DrainReport) {
    if report.is_clean() {
        info!(
            in_flight_at_signal = report.in_flight_at_signal,
            signalled = report.signalled,
            elapsed_ms = report.elapsed.as_millis(),
            "in-flight turns drained before shutdown"
        );
    } else {
        tracing::error!(
            in_flight_at_signal = report.in_flight_at_signal,
            signalled = report.signalled,
            abandoned = report.abandoned,
            elapsed_ms = report.elapsed.as_millis(),
            "shutdown abandoned in-flight turns; their placeholders stay open"
        );
    }
}

/// How long the shutdown drain awaits in-flight turns as they are.
///
/// Most of a messaging turn's wall clock is one LLM call, so a turn that
/// started seconds before the signal usually lands its reply inside this
/// window and the athlete never learns the instance changed.
#[cfg(unix)]
const TURN_DRAIN_GRACE: Duration = Duration::from_secs(5);

/// How long turns get, after the drain signal, to close their placeholders.
///
/// They have given up their answer by this point; all that is left is one
/// channel API edit each, replacing the "thinking…" placeholder with the
/// notice that the answer is not coming.
#[cfg(unix)]
const TURN_DRAIN_SIGNAL_WINDOW: Duration = Duration::from_secs(2);

/// Floor on the whole shutdown path, so the operator "stopping" notice has
/// time to leave the process.
///
/// It is an async Slack post with no one left to await it. An instance with no
/// turns in flight drains instantly and would otherwise exit before the notice
/// went out — which is what the bare 3s sleep in the composition root used to
/// buy.
#[cfg(unix)]
const SHUTDOWN_NOTICE_FLUSH: Duration = Duration::from_secs(3);

/// Drain in-flight messaging turns on SIGTERM.
///
/// Unix-only: Cloud Run (Linux) delivers SIGTERM on scaledown and redeploy, and
/// `tokio::signal::unix` does not exist on Windows, which the cross-platform
/// build still compiles the server binary for.
///
/// Cloud Run counts in-flight *requests*, and a messaging turn is not one: its
/// webhook answered 200 before the turn began. So the instance reads as idle
/// while it is working, and a rollout or scaledown may terminate it mid-turn —
/// on 2026-08-26 one did, and that athlete's placeholder is still open
/// (registre#109). This is where the grace window gets spent on the turns
/// instead of on a sleep.
///
/// The whole budget (5s + 2s, floored at 3s for the notice) fits inside Cloud
/// Run's ~10s default termination grace: a drain that overran it would be
/// killed partway and leave exactly the placeholders it exists to close.
#[cfg(unix)]
pub fn spawn_sigterm_drain(turns: Arc<InFlightTurns>) {
    tokio::spawn(async move {
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
                server_lifecycle::notify_stopping();
                let report = turns
                    .drain(TURN_DRAIN_GRACE, TURN_DRAIN_SIGNAL_WINDOW)
                    .await;
                log_drain(&report);
                if let Some(remaining) = SHUTDOWN_NOTICE_FLUSH.checked_sub(report.elapsed) {
                    sleep(remaining).await;
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to install SIGTERM handler for shutdown notification");
            }
        }
    });
}
