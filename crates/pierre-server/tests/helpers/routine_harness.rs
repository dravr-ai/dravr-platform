// ABOUTME: A Cloud Tasks queue stand-in for the routine engine: records each wake and delivers it at its own time
// ABOUTME: Tests reconcile, then run the clock forward; every wake due by then is delivered in order, as Cloud Tasks would

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_server::services::routine_engine::{RoutineEngine, WakeOutcome};
use pierre_mcp_server::services::routine_waker::{RoutineWaker, Wake};

/// Queues wakes in memory. A wake already pending under the same task id is
/// not queued twice, as Cloud Tasks refuses a second task of one name.
#[derive(Default)]
pub(crate) struct QueuedWakes {
    pending: Mutex<Vec<Wake>>,
    queued: Mutex<Vec<Wake>>,
    refuse: AtomicUsize,
}

impl QueuedWakes {
    /// Refuse the next `n` queues, as a Cloud Tasks outage would.
    pub(crate) fn refuse_next(&self, n: usize) {
        self.refuse.store(n, Ordering::SeqCst);
    }

    /// Every wake ever queued, in order (duplicates of a pending one excluded).
    pub(crate) fn queued(&self) -> Vec<Wake> {
        self.queued.lock().unwrap().clone()
    }

    /// The wakes still waiting to be delivered.
    pub(crate) fn pending(&self) -> Vec<Wake> {
        self.pending.lock().unwrap().clone()
    }

    /// Take the earliest pending wake due at or before `until`.
    pub(crate) fn take_due(&self, until: Timestamp) -> Option<Wake> {
        let mut pending = self.pending.lock().unwrap();
        let (index, _) = pending
            .iter()
            .enumerate()
            .filter(|(_, wake)| wake.fire_at().is_some_and(|at| at <= until))
            .min_by_key(|(_, wake)| wake.fire_at_ms())?;
        Some(pending.remove(index))
    }
}

#[async_trait]
impl RoutineWaker for QueuedWakes {
    async fn queue(&self, wake: Wake) -> AppResult<()> {
        if self
            .refuse
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(AppError::internal("Cloud Tasks unreachable (test)"));
        }
        let mut pending = self.pending.lock().unwrap();
        if pending
            .iter()
            .all(|queued| queued.task_id() != wake.task_id())
        {
            pending.push(wake);
            self.queued.lock().unwrap().push(wake);
        }
        Ok(())
    }
}

/// Deliver every wake due by `until`, each at its own time, earliest first —
/// including the wakes those deliveries queue. Returns what each did.
pub(crate) async fn run_until(
    engine: &RoutineEngine,
    wakes: &QueuedWakes,
    until: Timestamp,
) -> Vec<(Wake, WakeOutcome)> {
    let mut delivered = Vec::new();
    while let Some(wake) = wakes.take_due(until) {
        let at = wake.fire_at().unwrap();
        let outcome = engine.handle(wake, at).await.unwrap();
        delivered.push((wake, outcome));
    }
    delivered
}

/// The routine wakes among `delivered` (the system jobs left out).
pub(crate) fn routine_deliveries(delivered: &[(Wake, WakeOutcome)]) -> Vec<(Wake, WakeOutcome)> {
    delivered
        .iter()
        .filter(|(wake, _)| matches!(wake, Wake::Routine { .. }))
        .copied()
        .collect()
}
