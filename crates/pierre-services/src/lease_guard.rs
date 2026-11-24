// ABOUTME: The one keep-alive guard for a leased row: renews on a fixed cadence until dropped
// ABOUTME: A failed renewal is logged and retried; a refused one is logged once and ends them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keeping a leased row alive for as long as its run is.
//!
//! A run that holds a row under a lease (a backfill job, a memory extraction,
//! a resumable turn, a bundle build) renews that lease while it works, so a
//! sweep only takes the row over once the run is really gone. [`keep_lease`]
//! is that renewal and [`LeaseGuard`] is what stops it: when the run ends,
//! fails or panics the guard drops, the renewals stop, and the lease is left
//! to lapse.

use std::future::Future;
use std::time::Duration;

use pierre_core::errors::AppResult;
use tokio::task::AbortHandle;
use tokio::time::sleep;
use tracing::warn;

/// The renewals of one lease; they stop when this is dropped.
#[derive(Debug)]
#[must_use = "the renewals stop as soon as the guard is dropped"]
pub struct LeaseGuard(AbortHandle);

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Renew a lease every `every` until the returned guard drops.
///
/// `name` says which lease this is and `row` which row it holds; both go on
/// each line logged here. `renew` extends the lease once and answers whether
/// the row is still this run's. The first renewal comes one `every` after the
/// call, so a lease must outlast a few of them.
///
/// - A renewal that fails (`Err`) is logged and tried again at the next one.
/// - A renewal that is refused (`Ok(false)`) means the row is gone or another
///   runner holds it: that is logged once and the renewals stop. The run is
///   not interrupted; whether its work still counts is the caller's to decide
///   when it settles the row.
///
/// # Panics
///
/// Panics when called outside a Tokio runtime: the renewals are a spawned task.
pub fn keep_lease<Renew, Renewal>(
    name: &'static str,
    row: impl Into<String>,
    every: Duration,
    mut renew: Renew,
) -> LeaseGuard
where
    Renew: FnMut() -> Renewal + Send + 'static,
    Renewal: Future<Output = AppResult<bool>> + Send + 'static,
{
    let row = row.into();
    let renewals = tokio::spawn(async move {
        loop {
            sleep(every).await;
            match renew().await {
                Ok(true) => {}
                Ok(false) => {
                    warn!(
                        lease = name,
                        row = %row,
                        "Lease is no longer this run's; its renewals stop and the run finishes"
                    );
                    return;
                }
                Err(error) => {
                    warn!(
                        lease = name,
                        row = %row,
                        error = %error,
                        "Lease could not be renewed; trying again at the next renewal"
                    );
                }
            }
        }
    });
    LeaseGuard(renewals.abort_handle())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use pierre_core::errors::AppError;
    use tokio::time::timeout;

    use super::*;

    /// Short enough to keep the tests quick, long enough to tell two
    /// renewals apart.
    const EVERY: Duration = Duration::from_millis(10);

    /// Long enough for several renewals that must not happen.
    const SEVERAL: Duration = Duration::from_millis(100);

    /// Keep a lease whose `n`th renewal answers `answer(n)`, counting from 1.
    fn counted_lease(answer: fn(u32) -> AppResult<bool>) -> (LeaseGuard, Arc<AtomicU32>) {
        let renewals = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&renewals);
        let guard = keep_lease("test lease", "row-1", EVERY, move || {
            let nth = counted.fetch_add(1, Ordering::SeqCst) + 1;
            async move { answer(nth) }
        });
        (guard, renewals)
    }

    /// Wait until `renewals` reaches `wanted`.
    async fn reach(renewals: &AtomicU32, wanted: u32) {
        let reached = async {
            while renewals.load(Ordering::SeqCst) < wanted {
                sleep(Duration::from_millis(1)).await;
            }
        };
        timeout(Duration::from_secs(5), reached)
            .await
            .expect("bug: the lease was not renewed in time");
    }

    #[tokio::test]
    async fn the_lease_is_renewed_on_its_cadence_and_not_before() {
        let (guard, renewals) = counted_lease(|_| Ok(true));
        assert_eq!(
            renewals.load(Ordering::SeqCst),
            0,
            "the first renewal waits one cadence"
        );
        reach(&renewals, 3).await;
        drop(guard);
    }

    #[tokio::test]
    async fn dropping_the_guard_stops_the_renewals() {
        let (guard, renewals) = counted_lease(|_| Ok(true));
        reach(&renewals, 1).await;
        drop(guard);
        let at_drop = renewals.load(Ordering::SeqCst);
        sleep(SEVERAL).await;
        assert_eq!(
            renewals.load(Ordering::SeqCst),
            at_drop,
            "a dropped guard's lease kept being renewed"
        );
    }

    #[tokio::test]
    async fn a_failed_renewal_is_tried_again() {
        let (guard, renewals) = counted_lease(|nth| {
            if nth == 1 {
                Err(AppError::internal("the ledger was unreachable"))
            } else {
                Ok(true)
            }
        });
        reach(&renewals, 3).await;
        drop(guard);
    }

    #[tokio::test]
    async fn a_refused_renewal_ends_the_renewals() {
        let (guard, renewals) = counted_lease(|nth| Ok(nth < 2));
        reach(&renewals, 2).await;
        sleep(SEVERAL).await;
        assert_eq!(
            renewals.load(Ordering::SeqCst),
            2,
            "a lease another runner holds was renewed again"
        );
        drop(guard);
    }

    #[tokio::test]
    async fn a_run_that_panics_stops_its_renewals() {
        let (guard, renewals) = counted_lease(|_| Ok(true));
        reach(&renewals, 1).await;
        let run = tokio::spawn(async move {
            let _guard = guard;
            panic!("the run died");
        });
        assert!(
            run.await.unwrap_err().is_panic(),
            "bug: the test run was meant to panic"
        );
        let at_panic = renewals.load(Ordering::SeqCst);
        sleep(SEVERAL).await;
        assert_eq!(
            renewals.load(Ordering::SeqCst),
            at_panic,
            "a dead run's lease kept being renewed"
        );
    }
}
