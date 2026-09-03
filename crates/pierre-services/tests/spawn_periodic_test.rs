// ABOUTME: spawn_periodic must skip the first tick, survive an error and a panic, and stop on abort
// ABOUTME: Seven workers share this loop, so a regression here silently kills every background sweep
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Two of the seven hand-written copies this helper replaced caught a panicking
//! pass; the other five let one bad tick kill the worker for the life of the
//! process, silently. That is the property worth a test: not that the loop
//! ticks, but that it keeps ticking after the two ways a tick can end badly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pierre_core::errors::AppError;
use pierre_services::periodic::spawn_periodic;
use tokio::time::sleep;

const PERIOD: Duration = Duration::from_millis(20);

/// Wait until `counter` reaches `target`, or give up after `limit`.
async fn wait_for(counter: &Arc<AtomicUsize>, target: usize, limit: Duration) -> usize {
    let deadline = Instant::now() + limit;
    loop {
        let seen = counter.load(Ordering::SeqCst);
        if seen >= target || Instant::now() >= deadline {
            return seen;
        }
        sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn the_first_tick_waits_one_full_period() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();

    let counter = Arc::clone(&ticks);
    let handle = spawn_periodic("first-tick probe", Duration::from_millis(200), move || {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });

    sleep(Duration::from_millis(60)).await;
    assert_eq!(
        ticks.load(Ordering::SeqCst),
        0,
        "the immediate first tick must be consumed — a restart must not slam \
         every worker's sweep at once"
    );

    let seen = wait_for(&ticks, 1, Duration::from_secs(2)).await;
    assert!(seen >= 1, "the worker still ticks after the skipped one");
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "the first real tick landed a full period in"
    );
    handle.abort();
}

#[tokio::test]
async fn an_erroring_then_panicking_tick_does_not_stop_the_worker() {
    let ticks = Arc::new(AtomicUsize::new(0));

    let counter = Arc::clone(&ticks);
    let handle = spawn_periodic("resilience probe", PERIOD, move || {
        let counter = Arc::clone(&counter);
        async move {
            // Pass 1 fails, pass 2 panics, and every later pass succeeds. A
            // worker that dies on either never reaches 4.
            match counter.fetch_add(1, Ordering::SeqCst) {
                0 => Err(AppError::internal("first pass fails")),
                1 => panic!("second pass panics"),
                _ => Ok(()),
            }
        }
    });

    let seen = wait_for(&ticks, 4, Duration::from_secs(5)).await;
    assert!(
        seen >= 4,
        "the worker must survive both a returned error and a panic; it ticked {seen} time(s)"
    );
    handle.abort();
}

#[tokio::test]
async fn aborting_the_handle_stops_the_worker() {
    let ticks = Arc::new(AtomicUsize::new(0));

    let counter = Arc::clone(&ticks);
    let handle = spawn_periodic("abort probe", PERIOD, move || {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });

    wait_for(&ticks, 2, Duration::from_secs(5)).await;
    handle.abort();
    let at_abort = ticks.load(Ordering::SeqCst);

    sleep(PERIOD * 5).await;
    assert_eq!(
        ticks.load(Ordering::SeqCst),
        at_abort,
        "an aborted worker runs no further ticks"
    );
}
