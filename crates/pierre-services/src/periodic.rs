// ABOUTME: One tick loop for every background worker — interval, skip-first, log, keep going
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

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt as _;
use pierre_core::errors::AppResult;
use tokio::task::AbortHandle;
use tokio::time::interval;
use tracing::{error, info};

/// Run `tick` forever on a fixed period, in a spawned task.
///
/// The first tick fires one full `period` after the call, never immediately: a
/// server restart must not fan out every worker's sweep at once. A tick that
/// returns an error is logged and retried on the next period, and a tick that
/// *panics* is caught, logged and retried too — the worker outliving one bad
/// pass is the whole point of a best-effort sweep.
///
/// `name` appears on every line this loop logs, so a worker is identifiable in
/// production without reading the call site.
///
/// The returned [`AbortHandle`] stops the worker. Most callers discard it: the
/// workers are best-effort and a restart re-arms them.
pub fn spawn_periodic<F, Fut>(name: &'static str, period: Duration, mut tick: F) -> AbortHandle
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = AppResult<()>> + Send + 'static,
{
    // `interval(0)` panics, and two callers read their period from the
    // environment, so a misconfigured `=0` clamps to a second rather than
    // taking the process down at boot.
    let period = if period.is_zero() {
        Duration::from_secs(1)
    } else {
        period
    };

    let handle = tokio::spawn(async move {
        let mut ticker = interval(period);
        ticker.tick().await;
        info!(
            worker = name,
            interval_secs = period.as_secs(),
            "periodic worker started"
        );

        loop {
            ticker.tick().await;
            match AssertUnwindSafe(tick()).catch_unwind().await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    error!(worker = name, error = %e, "tick errored — retrying next interval");
                }
                // The default panic hook has already printed the payload and
                // location to stderr; this line says which worker survived it.
                Err(_) => error!(worker = name, "tick panicked; continuing (see stderr)"),
            }
        }
    });

    handle.abort_handle()
}
