// ABOUTME: Carries MCP task cancels and input between server replicas over Postgres LISTEN/NOTIFY
// ABOUTME: Installs tronc's TaskSignalBus on PostgreSQL; a SQLite deployment is one process and needs none
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cross-replica task signals.
//!
//! A task's worker runs on the replica that minted it, but `tasks/cancel` and
//! `tasks/update` land on whichever replica the load balancer picks. The task
//! row is shared through `mcp_tasks`; the worker's cancellation token and its
//! pending input request are not. tronc's
//! [`TaskSignalBus`](dravr_tronc::mcp::tasks::TaskSignalBus) is the seam that
//! closes that gap, and Postgres — the store every replica already shares —
//! is its carrier: the manager answering the request publishes with
//! `pg_notify`, and each replica's listener hands the signal to its own
//! manager's [`TaskManager::deliver`], which acts only where the run lives.
//!
//! The listener costs one pooled connection for the life of the process, so it
//! starts with the first task this process mints rather than at boot: a
//! replica that never ran a task has nothing a signal could reach. It is
//! listening before that task's id exists, so no signal for it can be missed.
//!
//! Delivery is best-effort. A notification sent while a listener reconnects is
//! lost, which is why the settle follower still polls the store as a backstop.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dravr_tronc::mcp::tasks::{TaskError, TaskManager, TaskSignalBus};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use tokio::time::timeout;
use tracing::warn;

/// How long a task create waits for the listener to subscribe. Starting it
/// takes one connection and one `LISTEN`; a pool that cannot hand one out
/// this fast is not one to make the create wait on, so it goes ahead and the
/// next create tries again.
const LISTEN_START_BUDGET: Duration = Duration::from_secs(2);

#[cfg(feature = "postgresql")]
pub use postgres::{PgTaskSignalBus, TASK_SIGNAL_CHANNEL};

/// A [`TaskSignalBus`] whose receiving half the host starts on demand.
#[async_trait]
pub trait ListeningSignalBus: TaskSignalBus {
    /// Start handing this carrier's signals to `manager`, once per process;
    /// later calls return at once.
    ///
    /// # Errors
    ///
    /// When the subscription cannot be made; the next call tries again.
    async fn ensure_listening(&self, manager: &Arc<TaskManager>) -> Result<(), TaskError>;
}

/// The task-signal carrier for one deployment's database, if it needs one.
pub struct TaskSignals {
    /// Shared with the [`TaskManager`] it is installed on, which publishes
    /// through it, while this half starts its listener.
    bus: Option<Arc<dyn ListeningSignalBus>>,
}

impl TaskSignals {
    /// The carrier `database` supports: Postgres `LISTEN/NOTIFY`, or none for
    /// SQLite, where the one process holds every run and the manager reaches
    /// them directly.
    #[must_use]
    pub fn for_database(database: &Database) -> Self {
        match database.backend() {
            DatabaseBackend::SQLite(_) => Self { bus: None },
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => Self {
                bus: Some(Arc::new(PgTaskSignalBus::new(db.pool().clone()))),
            },
        }
    }

    /// Install the carrier, if any, on a manager being built.
    #[must_use]
    pub fn install(&self, manager: TaskManager) -> TaskManager {
        match &self.bus {
            Some(bus) => manager.with_signal_bus(Arc::clone(bus) as Arc<dyn TaskSignalBus>),
            None => manager,
        }
    }

    /// Make sure this process is listening before it mints a task.
    ///
    /// A listener that cannot start is logged, not returned: the task still
    /// runs, a cancel answered here still reaches it, and the store backstop
    /// still relays one answered elsewhere, only later.
    pub async fn ensure_listening(&self, manager: &Arc<TaskManager>) {
        let Some(bus) = &self.bus else {
            return;
        };
        let failure = match timeout(LISTEN_START_BUDGET, bus.ensure_listening(manager)).await {
            Ok(Ok(())) => return,
            Ok(Err(e)) => e.to_string(),
            Err(_elapsed) => format!("not subscribed within {LISTEN_START_BUDGET:?}"),
        };
        warn!(
            error = %failure,
            "MCP task signal listener did not start; cross-replica cancels fall back to the store poll"
        );
    }
}

#[cfg(feature = "postgresql")]
mod postgres {
    use std::sync::{Arc, Weak};
    use std::time::Duration;

    use async_trait::async_trait;
    use dravr_tronc::mcp::tasks::{TaskError, TaskManager, TaskSignal, TaskSignalBus};
    use sqlx::postgres::PgListener;
    use sqlx::{Pool, Postgres};
    use tokio::sync::OnceCell;
    use tokio::time::sleep;
    use tokio_util::sync::CancellationToken;
    use tracing::{debug, error, warn};

    use super::ListeningSignalBus;

    /// The `LISTEN` channel every replica's listener subscribes to.
    pub const TASK_SIGNAL_CHANNEL: &str = "mcp_task_signals";

    /// Postgres refuses a `NOTIFY` payload of 8000 bytes or more.
    const MAX_PAYLOAD_BYTES: usize = 7_999;

    /// Pause before receiving again after the listener's connection failed,
    /// so a database that is down is not retried in a tight loop.
    const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);

    /// tronc's [`TaskSignalBus`] over Postgres `LISTEN/NOTIFY`.
    pub struct PgTaskSignalBus {
        pool: Pool<Postgres>,
        /// Set once this process's listener is subscribed.
        listening: OnceCell<()>,
        /// Fired when the bus drops, ending its listener: the relay would
        /// otherwise hold its connection until the next notification.
        closed: CancellationToken,
    }

    impl Drop for PgTaskSignalBus {
        fn drop(&mut self) {
            self.closed.cancel();
        }
    }

    impl PgTaskSignalBus {
        /// A carrier over `pool`; nothing listens until
        /// [`ListeningSignalBus::ensure_listening`].
        #[must_use]
        pub fn new(pool: Pool<Postgres>) -> Self {
            Self {
                pool,
                listening: OnceCell::new(),
                closed: CancellationToken::new(),
            }
        }
    }

    #[async_trait]
    impl ListeningSignalBus for PgTaskSignalBus {
        /// Subscribe this process to [`TASK_SIGNAL_CHANNEL`], relaying every
        /// signal to `manager` from a spawned task that ends with it.
        async fn ensure_listening(&self, manager: &Arc<TaskManager>) -> Result<(), TaskError> {
            self.listening
                .get_or_try_init(|| async {
                    let mut listener = PgListener::connect_with(&self.pool).await?;
                    listener.listen(TASK_SIGNAL_CHANNEL).await?;
                    let relay = tokio::spawn(relay_signals(
                        listener,
                        Arc::downgrade(manager),
                        self.closed.clone(),
                    ));
                    // The relay returns only once the bus, the manager or the
                    // pool is gone; a panic would end it silently, leaving only the
                    // store backstop, so it is logged.
                    tokio::spawn(async move {
                        if let Err(e) = relay.await {
                            error!(
                                error = %e,
                                is_panic = e.is_panic(),
                                "MCP task signal listener died; cross-replica cancels fall back to the store poll"
                            );
                        }
                    });
                    Ok::<(), sqlx::Error>(())
                })
                .await
                .map_err(|e| TaskError::Store(format!("task signal LISTEN failed: {e}")))?;
            Ok(())
        }
    }

    #[async_trait]
    impl TaskSignalBus for PgTaskSignalBus {
        async fn publish(&self, signal: TaskSignal) -> Result<(), TaskError> {
            let payload = serde_json::to_string(&signal)
                .map_err(|e| TaskError::Store(format!("task signal did not serialize: {e}")))?;
            if payload.len() > MAX_PAYLOAD_BYTES {
                return Err(TaskError::Store(format!(
                    "task signal is {} bytes; NOTIFY carries at most {MAX_PAYLOAD_BYTES}",
                    payload.len()
                )));
            }
            // Acquired apart from the statement, so a pool that cannot open a
            // connection is a refusal, never mistaken for an unknown outcome.
            let mut connection = self.pool.acquire().await.map_err(|e| {
                TaskError::Store(format!("task signal NOTIFY had no connection: {e}"))
            })?;
            let sent = sqlx::query("SELECT pg_notify($1, $2)")
                .bind(TASK_SIGNAL_CHANNEL)
                .bind(payload)
                .execute(&mut *connection)
                .await;
            match sent {
                Ok(_) => Ok(()),
                // The server rejected the statement and rolled it back: no
                // notification was queued.
                Err(e @ sqlx::Error::Database(_)) => {
                    Err(TaskError::Store(format!("task signal NOTIFY failed: {e}")))
                }
                // The connection failed with the statement possibly sent, so
                // the notification may have gone out. Delivery is at most once
                // either way; `Err` here would fail a task whose operation
                // may already hold its input.
                Err(e) => {
                    warn!(error = %e, "MCP task signal NOTIFY outcome unknown");
                    Ok(())
                }
            }
        }
    }

    /// Hand each notification to the manager until the bus, the manager or
    /// the pool is gone. `PgListener` reconnects on the receive after a
    /// failure and re-subscribes; what was sent in between is lost.
    async fn relay_signals(
        mut listener: PgListener,
        manager: Weak<TaskManager>,
        closed: CancellationToken,
    ) {
        loop {
            let received = tokio::select! {
                () = closed.cancelled() => return,
                received = listener.recv() => received,
            };
            let Some(manager) = manager.upgrade() else {
                return;
            };
            match received {
                Ok(notification) => deliver(&manager, notification.payload()),
                Err(sqlx::Error::PoolClosed) => return,
                Err(e) => {
                    warn!(error = %e, "MCP task signal listener lost its connection; reconnecting");
                    sleep(RECONNECT_BACKOFF).await;
                }
            }
        }
    }

    /// Decode one notification and hand it to the manager, which acts only if
    /// the task's run lives in this process.
    fn deliver(manager: &TaskManager, payload: &str) {
        match serde_json::from_str::<TaskSignal>(payload) {
            Ok(signal) => {
                let task_id = signal.task_id().clone();
                if manager.deliver(signal) {
                    debug!(task_id = %task_id, "Task signal delivered to its run");
                }
            }
            Err(e) => warn!(error = %e, "Unreadable MCP task signal ignored"),
        }
    }
}
