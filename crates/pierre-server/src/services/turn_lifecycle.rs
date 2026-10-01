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
//!   the instance would lose if it died right now — the replies a webhook
//!   sends outside a turn (slash, intake, link prompt) included, through
//!   `messaging_ingress::outbound_send`, and every notification trigger's
//!   dispatch, through the tracker handle the server builds the
//!   notification service with;
//! - `drain` spends the SIGTERM grace window awaiting them instead of
//!   sleeping through it;
//! - when the grace runs out, `drain_token` fires and each turn still
//!   running gets the chance to hand itself to the next instance — one
//!   small write recording everything a fresh dispatch needs, placeholder
//!   id included, so the athlete's answer is delivered there through the
//!   same placeholder (`messaging_ingress::resume`, registre#126). Only a
//!   turn that has already been drained once, or whose record cannot be
//!   written, closes its placeholder with a notice instead.
//!
//! The token is a deadline signal, not a kill switch: nothing here aborts a
//! turn. A turn that ignores it simply dies with the process, exactly as it
//! did before — the tracker only ever adds chances to finish.

use std::future::{pending, Future};
use std::time::{Duration, Instant};

use pierre_core::errors::AppResult;
use pierre_services::server_lifecycle;
use tokio::runtime::Runtime;
use tokio::select;
#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{error, info, warn};

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
    /// process without recording their hand-off; each one is an athlete
    /// holding an open placeholder.
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

    /// A handle on the tracker itself, for a crate below this one that spawns
    /// work the drain must await.
    ///
    /// `pierre-notifications` cannot name this type, so its service is built
    /// with this handle and spawns every trigger dispatch onto it: a push and
    /// linked-chat message fired just before SIGTERM is counted here and
    /// awaited by [`Self::drain`] like a turn. Clones share one count.
    #[cfg(feature = "client-notifications")]
    #[must_use]
    pub(crate) fn tracker(&self) -> TaskTracker {
        self.tracker.clone()
    }

    /// The signal a turn watches to learn the process is going away.
    ///
    /// Cancelled once the grace window has elapsed with turns still running.
    /// A turn holding one of these is expected to stop what it is doing and
    /// record itself for the next instance, leaving its status placeholder
    /// for the resumed run to edit — see
    /// `messaging_ingress::turn_guard::run_bounded` and
    /// `messaging_ingress::resume`.
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
    ///    left. Those turns give up their answer on this instance, but they
    ///    are still alive and can spend the window writing the one row that
    ///    lets another instance deliver it — or, once out of attempts, one
    ///    channel API edit closing their placeholder.
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
/// An abandoned turn died before recording its hand-off, so it is an athlete
/// who asked a question and will never be told anything — an ERROR even
/// though the process is exiting normally, because the alternative is a
/// shutdown that reports success while dropping work.
fn log_drain(report: &DrainReport) {
    if report.is_clean() {
        info!(
            in_flight_at_signal = report.in_flight_at_signal,
            signalled = report.signalled,
            elapsed_ms = report.elapsed.as_millis(),
            "in-flight turns drained before shutdown"
        );
    } else {
        error!(
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
const TURN_DRAIN_GRACE: Duration = Duration::from_secs(5);

/// How long turns get, after the drain signal, to hand themselves off.
///
/// They have given up their answer on this instance by this point; what is
/// left is one database insert each recording the turn for the next
/// instance — or, for a turn already resumed once, one channel API edit
/// replacing the "thinking…" placeholder with the notice.
const TURN_DRAIN_SIGNAL_WINDOW: Duration = Duration::from_secs(2);

/// Floor on the whole shutdown path, so the operator "stopping" notice has
/// time to leave the process.
///
/// It is an async Slack post with no one left to await it. An instance with no
/// turns in flight drains instantly and would otherwise exit before the notice
/// went out.
const SHUTDOWN_NOTICE_FLUSH: Duration = Duration::from_secs(3);

/// The longest [`serve_until_drained`] runs once its shutdown signal fires:
/// both drain windows back to back, which the notice floor sits inside.
///
/// `serve_until_drained` holds itself to it: work tracked after the drain's
/// own wait returned — a reply a webhook already past its 200 queues a moment
/// later — gets what is left of this budget and no more.
///
/// The whole budget fits inside Cloud Run's ~10s default termination grace:
/// a drain that overran it would be killed partway and leave exactly the
/// placeholders it exists to close.
pub const SHUTDOWN_DRAIN_BUDGET: Duration =
    TURN_DRAIN_GRACE.saturating_add(TURN_DRAIN_SIGNAL_WINDOW);

/// How long the runtime gives its remaining tasks once the server future
/// has returned — see [`run_to_exit`].
///
/// Async tasks (the periodic workers, the contremaitre poll, open
/// connections) are cancelled at their next `.await`, which costs nothing;
/// this bounds the blocking ones (`spawn_blocking`), which a plain runtime
/// drop waits for with no limit at all.
pub const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

/// Serve until `shutdown` fires and the in-flight turns are drained, then
/// return so the process can exit.
///
/// Cloud Run counts in-flight *requests*, and a messaging turn is not one: its
/// webhook answered 200 before the turn began. So the instance reads as idle
/// while it is working, and a rollout or scaledown may terminate it mid-turn —
/// on 2026-08-26 one did, and that athlete's placeholder is still open
/// (registre#109). This is where the grace window gets spent on the turns
/// instead of on a sleep, and where a turn that cannot finish records itself
/// for the next instance to answer (registre#126).
///
/// The tracker can gain work after the drain's own wait has returned: an
/// instance idle at the signal drains at once, and a webhook already past its
/// 200 then queues its slash or intake reply through the same tracker. The
/// notice floor is followed by one more wait for that work, bounded by what is
/// left of [`SHUTDOWN_DRAIN_BUDGET`], so the reply leaves before the server
/// does instead of being cut by the runtime's teardown.
///
/// The drain is the end of the process, not a side task: once it finishes
/// the server future is dropped and this returns. Installing a SIGTERM
/// handler replaces the default action of the signal, so a drain that only
/// logged and returned left the process serving — and its periodic workers
/// ticking — until the platform's SIGKILL. `serve` returning on its own
/// (a transport error, stdin closing) ends it too, without a drain.
///
/// # Errors
///
/// Returns `serve`'s error when the transport fails before the drain has
/// finished.
pub async fn serve_until_drained<S, D>(
    serve: S,
    shutdown: D,
    turns: &InFlightTurns,
) -> AppResult<()>
where
    S: Future<Output = AppResult<()>>,
    D: Future<Output = ()>,
{
    let drained = async {
        shutdown.await;
        let signalled_at = Instant::now();
        server_lifecycle::notify_stopping();
        let report = turns
            .drain(TURN_DRAIN_GRACE, TURN_DRAIN_SIGNAL_WINDOW)
            .await;
        log_drain(&report);
        if let Some(remaining) = SHUTDOWN_NOTICE_FLUSH.checked_sub(signalled_at.elapsed()) {
            sleep(remaining).await;
        }
        let budget_left = SHUTDOWN_DRAIN_BUDGET.saturating_sub(signalled_at.elapsed());
        // An unclean report has already logged the work it is leaving behind.
        if timeout(budget_left, turns.tracker.wait()).await.is_err() && report.is_clean() {
            error!(
                still_running = turns.len(),
                budget_ms = SHUTDOWN_DRAIN_BUDGET.as_millis(),
                "work queued during the shutdown drain outlived its budget; it dies with the process"
            );
        }
    };
    select! {
        result = serve => result,
        () = drained => {
            info!("shutdown drain finished; stopping the server");
            Ok(())
        }
    }
}

/// Resolves when the platform asks the process to stop.
///
/// SIGTERM on unix: Cloud Run (Linux) delivers it on scaledown and redeploy.
/// `tokio::signal::unix` does not exist on Windows, which the cross-platform
/// build still compiles the server binary for; there this never resolves and
/// the process ends the way it always has, by being killed. The handler is
/// installed when this is first polled, so a SIGTERM that lands before the
/// server starts serving keeps the signal's default action and ends the
/// process outright.
pub async fn shutdown_signal() {
    #[cfg(unix)]
    match signal(SignalKind::terminate()) {
        Ok(mut sigterm) => {
            sigterm.recv().await;
            return;
        }
        Err(e) => {
            warn!(error = %e, "Failed to install the SIGTERM handler; shutdown will not drain");
        }
    }
    pending::<()>().await;
}

/// Drive `main` to completion on `runtime`, then shut the runtime down within
/// [`RUNTIME_SHUTDOWN_GRACE`].
///
/// Whatever `main` left spawned — a worker mid-tick, the contremaitre poll
/// mid-sync — is cancelled rather than awaited: it is best-effort background
/// work the next instance picks up, and waiting on it is what kept a drained
/// process alive.
pub fn run_to_exit<T>(runtime: Runtime, main: impl Future<Output = T>) -> T {
    let output = runtime.block_on(main);
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
    output
}

#[cfg(test)]
mod tests {
    /// The drain's notification coverage needs the notification service,
    /// which only builds with `client-notifications`.
    #[cfg(feature = "client-notifications")]
    mod notification_drain {
        use std::future::pending;
        use std::sync::{Arc, Mutex, PoisonError};
        use std::time::Duration;

        use async_trait::async_trait;
        use pierre_core::errors::AppResult;
        use pierre_database::backends::factory::{Database, DatabaseBackend};
        use pierre_notifications::triggers::trigger_agent_message;
        use pierre_notifications::{
            DispatchRequest, NotificationChannelSink, NotificationService, TenantId as CommTenantId,
        };
        use pierre_test_support::db::create_test_db;
        use tokio::sync::oneshot;
        use tokio::time::sleep;
        use uuid::Uuid;

        use super::super::{serve_until_drained, InFlightTurns, SHUTDOWN_DRAIN_BUDGET};

        /// How long the slow linked channel takes to accept a notification: past
        /// the shutdown path's three-second notice floor, so a dispatch the drain
        /// does not track is still in flight when the server future returns.
        const SLOW_CHANNEL_SEND: Duration = Duration::from_secs(4);

        /// A linked chat channel that takes [`SLOW_CHANNEL_SEND`] to accept a
        /// notification, then records whom it was for.
        #[derive(Default)]
        struct SlowChannelSink {
            delivered: Mutex<Vec<Uuid>>,
        }

        #[async_trait]
        impl NotificationChannelSink for SlowChannelSink {
            async fn deliver(&self, request: &DispatchRequest) -> usize {
                sleep(SLOW_CHANNEL_SEND).await;
                self.delivered
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(request.user_id);
                1
            }
        }

        /// The service on the test database's backend, spawning its dispatches
        /// onto `turns` — the mapping `ServerContext::create_notification_service`
        /// performs at boot.
        fn notification_service(db: &Database, turns: &InFlightTurns) -> NotificationService {
            match db.backend() {
                DatabaseBackend::SQLite(sqlite) => {
                    NotificationService::from_sqlite(sqlite.pool().clone(), turns.tracker())
                }
                #[cfg(feature = "postgresql")]
                DatabaseBackend::PostgreSQL(pg) => {
                    NotificationService::from_postgres(pg.pool().clone(), turns.tracker())
                }
            }
        }

        /// A notification trigger fires and returns; its push and linked-chat
        /// delivery run on afterwards. Spawned with a bare `tokio::spawn`, that
        /// delivery was invisible to the drain: a trigger fired a moment before
        /// SIGTERM was cut by the runtime's teardown once the server returned.
        #[tokio::test(flavor = "multi_thread")]
        async fn a_notification_fired_during_the_drain_is_delivered_before_the_server_returns() {
            assert!(
                SLOW_CHANNEL_SEND < SHUTDOWN_DRAIN_BUDGET,
                "the slow send must fit the budget the drain is allowed"
            );
            let db = create_test_db().await.unwrap();
            let user_id = Uuid::new_v4();
            let turns = Arc::new(InFlightTurns::new());
            let sink = Arc::new(SlowChannelSink::default());
            let service = Arc::new(
                notification_service(&db, &turns)
                    .with_channel_sink(Arc::clone(&sink) as Arc<dyn NotificationChannelSink>),
            );

            let (stop, stopped) = oneshot::channel::<()>();
            let draining = Arc::clone(&turns);
            let server = tokio::spawn(async move {
                serve_until_drained(
                    pending::<AppResult<()>>(),
                    async {
                        stopped.await.unwrap();
                    },
                    &draining,
                )
                .await
            });
            stop.send(()).unwrap();
            sleep(Duration::from_millis(100)).await;
            assert!(
                !server.is_finished(),
                "the drain is still under way when the notification fires"
            );

            trigger_agent_message(
                &service,
                user_id,
                CommTenantId(Uuid::new_v4()),
                "conversation-drain",
                "Coach",
            );

            server.await.unwrap().unwrap();
            assert_eq!(
                sink.delivered.lock().unwrap().as_slice(),
                [user_id],
                "the notification fired during the drain reached the linked channel before the server returned"
            );
        }
    }
}
