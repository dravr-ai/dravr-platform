// ABOUTME: A per-instance periodic loop under a supervisor: every instance ticks, and a panicking tick is logged and respawned
// ABOUTME: The counterpart of periodic::spawn_periodic for work each process must do itself, such as reloading its keyring

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-instance loops.
//!
//! [`spawn_periodic`](crate::periodic::spawn_periodic) runs a tick once
//! across the fleet, claimed through the worker ledger. Some work belongs to
//! every process instead: reloading its own keyring, re-reading its own
//! prompt overlays. [`spawn_supervised_loop`] runs such a tick on every
//! instance, first at once and then every period, with no ledger.
//!
//! The tick runs in its own task, which a supervisor awaits: a panic is
//! logged at `ERROR` and the loop is respawned after one period. A dead loop
//! would otherwise freeze what it maintains for the life of the instance,
//! with only a stderr panic as trace.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::AbortHandle;
use tokio::time::{interval, sleep};
use tracing::error;

/// Run `tick` at once and then every `period`, in a spawned task, on this
/// instance, for as long as the process lives. A panicking tick is logged
/// under `name` and the loop respawned after one `period`.
pub fn spawn_supervised_loop<F, Fut>(name: &'static str, period: Duration, tick: F) -> AbortHandle
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let tick = Arc::new(tick);
    tokio::spawn(async move {
        loop {
            let run = Arc::clone(&tick);
            let ticking = tokio::spawn(async move {
                let mut ticker = interval(period);
                loop {
                    ticker.tick().await;
                    run().await;
                }
            });
            // The inner loop never returns normally; reaching here means the
            // task died (a panic, or a runtime-cancelled join).
            if let Err(e) = ticking.await {
                error!(
                    worker = name,
                    error = %e,
                    is_panic = e.is_panic(),
                    "{name} loop died — respawning after one period"
                );
            }
            sleep(period).await;
        }
    })
    .abort_handle()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[tokio::test]
    async fn a_panicking_tick_is_respawned_and_keeps_ticking() {
        let ticks = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&ticks);
        let handle = spawn_supervised_loop("test loop", Duration::from_millis(10), move || {
            let counted = Arc::clone(&counted);
            async move {
                let n = counted.fetch_add(1, Ordering::SeqCst);
                assert!(n != 0, "the first tick panics");
            }
        });
        for _ in 0..200 {
            if ticks.load(Ordering::SeqCst) >= 3 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        handle.abort();
        assert!(
            ticks.load(Ordering::SeqCst) >= 3,
            "the loop kept ticking after the panic"
        );
    }
}
