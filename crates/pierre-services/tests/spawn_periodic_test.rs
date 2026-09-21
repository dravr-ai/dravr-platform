// ABOUTME: spawn_periodic must tick when the ledger says it is due, tick once across instances, survive a bad pass, and stop on abort
// ABOUTME: Every background sweep shares this loop, so a regression here silently kills or duplicates all of them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Two of the seven hand-written copies this helper replaced caught a panicking
//! pass; the other five let one bad tick kill the worker for the life of the
//! process, silently. And every copy restarted its countdown at boot, so on a
//! scale-to-zero service a weekly worker never ticked (carnet#459). Those are
//! the properties worth a test: not that the loop ticks, but that it keeps
//! ticking after the two ways a tick can end badly, that it resumes the
//! schedule the ledger records, and that two instances run a due tick once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::{WorkerRun, WorkerRunRepository};
use pierre_services::periodic::spawn_periodic;
use tokio::time::sleep;

const PERIOD: Duration = Duration::from_millis(20);

async fn ledger() -> Arc<dyn WorkerRunRepository> {
    let db = create_test_db().await.expect("test db");
    Arc::clone(&db.repositories().worker_runs)
}

/// Wait until the worker's ledger row satisfies `settled`, or give up after
/// `limit`, returning the last row read.
///
/// A tick counter moves *inside* the tick, before the ledger write that
/// follows it, so a test that reads the ledger the moment the counter moves
/// races that write. Worse, aborting the worker across it is not safe on the
/// factory database: the pool holds ONE in-memory connection, and a statement
/// future dropped mid-await fails sqlx's release ping, so the pool closes the
/// connection and opens a fresh, empty `sqlite::memory:` — the next read
/// answers "no such table" (seen 1 run in 40 locally, and once in CI). A test
/// that asserts on the ledger waits for the row to settle and never aborts a
/// worker that may be mid-statement; a sleeping worker dies with the runtime.
async fn wait_for_ledger(
    ledger: &Arc<dyn WorkerRunRepository>,
    name: &str,
    limit: Duration,
    settled: impl Fn(&WorkerRun) -> bool,
) -> Option<WorkerRun> {
    let deadline = Instant::now() + limit;
    loop {
        let row = ledger.get_worker_run(name).await.unwrap();
        if row.as_ref().is_some_and(&settled) || Instant::now() >= deadline {
            return row;
        }
        sleep(Duration::from_millis(10)).await;
    }
}

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

fn counting_tick(counter: &Arc<AtomicUsize>) -> impl FnMut() -> CountFut + Send + 'static {
    let counter = Arc::clone(counter);
    move || {
        let counter = Arc::clone(&counter);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

type CountFut = Pin<Box<dyn Future<Output = AppResult<()>> + Send>>;

#[tokio::test]
async fn a_worker_with_no_record_waits_one_period_before_its_first_tick() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();

    let handle = spawn_periodic(
        "first-tick probe",
        Duration::from_millis(200),
        ledger().await,
        counting_tick(&ticks),
    );

    sleep(Duration::from_millis(60)).await;
    assert_eq!(
        ticks.load(Ordering::SeqCst),
        0,
        "with no ledger record the worker waits the (capped) period — a restart \
         must not slam every worker's sweep at once"
    );

    let seen = wait_for(&ticks, 1, Duration::from_secs(2)).await;
    assert!(seen >= 1, "the worker still ticks after the wait");
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "the first tick landed a full period in"
    );
    handle.abort();
}

#[tokio::test]
async fn a_worker_resumes_the_remainder_of_its_period_from_the_ledger() {
    let ledger = ledger().await;
    // The last tick finished 800ms ago on some other instance; with a 1s
    // period this one is due in ~200ms, not a full second after boot.
    let seeded = Utc::now().timestamp_millis() - 800;
    ledger
        .finish_worker_run("resume probe", seeded)
        .await
        .unwrap();

    let ticks = Arc::new(AtomicUsize::new(0));
    let started = Instant::now();
    let handle = spawn_periodic(
        "resume probe",
        Duration::from_secs(1),
        Arc::clone(&ledger),
        counting_tick(&ticks),
    );

    let seen = wait_for(&ticks, 1, Duration::from_millis(700)).await;
    assert_eq!(
        seen, 1,
        "the first tick lands when the recorded period elapses, not one period after boot"
    );
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "ticked {:?} in — the ledger's remainder was honoured",
        started.elapsed()
    );

    // The finish is written after the tick returns; wait for it rather than
    // read across it, and leave the worker asleep — see `wait_for_ledger`.
    let run = wait_for_ledger(&ledger, "resume probe", Duration::from_secs(2), |row| {
        row.last_run_at_ms > seeded
    })
    .await
    .unwrap();
    assert!(
        run.last_run_at_ms > seeded,
        "a finished tick stamps last_run_at_ms past the seeded value"
    );
    assert_eq!(run.leased_until_ms, 0, "a finished tick releases its lease");
    drop(handle);
}

#[tokio::test]
async fn two_instances_sharing_a_worker_run_each_due_tick_once() {
    let ledger = ledger().await;
    let ticks = Arc::new(AtomicUsize::new(0));
    let period = Duration::from_millis(300);

    // Two loops with the same name on the same ledger — two instances of the
    // service. Each period is due once; the claim decides who runs it.
    let a = spawn_periodic(
        "shared probe",
        period,
        Arc::clone(&ledger),
        counting_tick(&ticks),
    );
    let b = spawn_periodic(
        "shared probe",
        period,
        Arc::clone(&ledger),
        counting_tick(&ticks),
    );

    sleep(Duration::from_millis(1_500)).await;
    let seen = ticks.load(Ordering::SeqCst);
    a.abort();
    b.abort();

    // Five periods fit in the window; two uncoordinated loops would produce
    // about ten ticks. One ledger produces at most one per period, plus the
    // boundary tick the window may straddle.
    assert!(
        (1..=6).contains(&seen),
        "two instances ran the shared worker {seen} time(s) in five periods — the ledger must serialise them to one tick per period"
    );
}

#[tokio::test]
async fn a_failed_tick_leaves_the_ledger_unstamped_for_the_next_instance() {
    let ledger = ledger().await;
    let ticks = Arc::new(AtomicUsize::new(0));
    // Due in ~100ms on a 10s period, so the failure's hold (one period) is
    // still visible when the ledger is read back.
    let last_run = Utc::now().timestamp_millis() - 9_900;
    ledger
        .finish_worker_run("failing probe", last_run)
        .await
        .unwrap();

    let counter = Arc::clone(&ticks);
    let period = Duration::from_secs(10);
    let handle = spawn_periodic("failing probe", period, Arc::clone(&ledger), move || {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(AppError::internal("this pass always fails"))
        }
    });

    let seen = wait_for(&ticks, 1, Duration::from_secs(2)).await;
    assert!(seen >= 1, "the failing tick ran");

    // The hold is written after the tick fails; wait for it rather than abort
    // across it, and leave the worker asleep — see `wait_for_ledger`. The
    // claim before the tick already holds the row for the 15-minute lease,
    // so the defer is recognised by its shorter, one-period hold.
    let run = wait_for_ledger(&ledger, "failing probe", Duration::from_secs(2), |row| {
        let remaining = row.leased_until_ms - Utc::now().timestamp_millis();
        remaining > 0 && remaining <= 10_000
    })
    .await
    .unwrap();
    drop(handle);
    assert_eq!(
        run.last_run_at_ms, last_run,
        "a failed tick must not count as a run — the stamp stays where the last success left it"
    );
    let now = Utc::now().timestamp_millis();
    assert!(
        run.leased_until_ms > now && run.leased_until_ms <= now + 10_000,
        "the failure holds the worker for one period ({} ms from now), so the retry is next interval on whichever instance is alive",
        run.leased_until_ms - now
    );
}

#[tokio::test]
async fn an_erroring_then_panicking_tick_does_not_stop_the_worker() {
    let ticks = Arc::new(AtomicUsize::new(0));

    let counter = Arc::clone(&ticks);
    let handle = spawn_periodic("resilience probe", PERIOD, ledger().await, move || {
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

    let handle = spawn_periodic("abort probe", PERIOD, ledger().await, counting_tick(&ticks));

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
