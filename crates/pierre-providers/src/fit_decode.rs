// ABOUTME: The process-wide bound on concurrent FIT decodes: every decode reached from async code waits for a permit
// ABOUTME: The decoder itself is dravr-barrage's parse::fit; this keeps how many run at once in the platform
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Bounded FIT decodes
//!
//! The FIT decoder is `dravr_barrage::parse::fit` (moved out of this crate
//! with carnet#1217/#1218). Its per-file field budget
//! (`dravr_barrage::parse::fit::MAX_DECODED_FIELDS`) bounds one decode at about
//! 260 MiB; how many run together is a property of this process, so the
//! bound on that stays here and wraps every call into the decoder: uploads,
//! the upload streams backfill, Wahoo's samples and Garmin push files.

use tokio::sync::Semaphore;
use tokio::task::spawn_blocking;

use crate::errors::{AppError, AppResult};

/// How many FIT decodes may run at once in this process.
///
/// One decode at the decoder's field budget holds about 260 MiB; this bounds
/// how many are resident together, so concurrent uploads of flooded files
/// cannot add up past an instance's memory (carnet#1029): four at the cap
/// exceed the smallest backend instance deployed (prod, 1 GiB). Two at the
/// cap hold about 520 MiB, half of it. An ordinary workout file decodes in
/// milliseconds, so a third decode waits only that long.
const MAX_CONCURRENT_DECODES: usize = 2;

/// The decode permits every FIT decode in this process is admitted by.
static DECODE_PERMITS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_DECODES);

/// Run a CPU-bound FIT `decode` on the blocking pool once a decode permit is
/// free, and wait for it.
///
/// Every FIT decode reached from async code goes through here, so at most
/// [`MAX_CONCURRENT_DECODES`] run at once. The permit is held by the decode
/// itself, not by the caller: a caller dropped mid-decode (an upload whose
/// client hung up) cannot admit another decode while its own still runs.
///
/// # Errors
///
/// [`AppError::internal`] when the decode task panicked or was cancelled.
pub async fn decode_bounded<T, F>(decode: F) -> AppResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_admitted(&DECODE_PERMITS, decode).await
}

/// [`decode_bounded`] against `permits`.
async fn run_admitted<T, F>(permits: &'static Semaphore, decode: F) -> AppResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let permit = permits
        .acquire()
        .await
        .map_err(|e| AppError::internal(format!("FIT decode permits closed: {e}")))?;
    spawn_blocking(move || {
        let decoded = decode();
        drop(permit);
        decoded
    })
    .await
    .map_err(|e| AppError::internal(format!("FIT decode did not complete: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    use tokio::time;

    /// Permits of their own, so a test's count is not shared with every other
    /// decode the test binary runs at the same time.
    fn permits() -> &'static Semaphore {
        Box::leak(Box::new(Semaphore::new(MAX_CONCURRENT_DECODES)))
    }

    #[tokio::test]
    async fn a_decode_past_the_concurrent_bound_waits_for_a_permit() {
        let permits = permits();
        let held = permits
            .acquire_many(u32::try_from(MAX_CONCURRENT_DECODES).unwrap())
            .await
            .unwrap();

        let waiting = tokio::spawn(run_admitted(permits, || 1_usize));
        time::sleep(Duration::from_millis(100)).await;
        assert!(
            !waiting.is_finished(),
            "with every permit held, a decode does not start"
        );

        drop(held);
        let decoded = waiting.await.unwrap().unwrap();
        assert_eq!(decoded, 1, "it runs once a permit is free");
    }

    #[tokio::test]
    async fn a_cancelled_caller_holds_its_permit_until_its_decode_ends() {
        let permits = permits();
        let (started_tx, started) = mpsc::channel();
        let (finish, finish_rx) = mpsc::channel::<()>();
        let caller = tokio::spawn(run_admitted(permits, move || {
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
        }));
        spawn_blocking(move || started.recv().unwrap())
            .await
            .unwrap();

        caller.abort();
        assert!(
            caller.await.unwrap_err().is_cancelled(),
            "the aborted caller ends cancelled, not with a decode result"
        );
        assert_eq!(
            permits.available_permits(),
            MAX_CONCURRENT_DECODES - 1,
            "the decode still running keeps its permit after its caller is gone"
        );

        finish.send(()).unwrap();
        time::timeout(Duration::from_secs(5), async {
            while permits.available_permits() < MAX_CONCURRENT_DECODES {
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the permit comes back when the decode ends");
    }
}
