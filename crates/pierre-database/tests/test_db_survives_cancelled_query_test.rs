// ABOUTME: Pins that a per-test database outlives a query cancelled mid-flight
// ABOUTME: Regression for the 2026-09-21 main red — "no such table: worker_runs" after a task abort
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `spawn_periodic_test::a_failed_tick_leaves_the_ledger_unstamped_for_the_next_instance`
//! went red on main twice on 2026-09-21 with `no such table: worker_runs`,
//! read straight after `handle.abort()` on a worker that was writing the
//! ledger. The `SQLite` test database was one pinned in-memory connection
//! loaded from a serialized image; the pool replaces a connection whose
//! query was cancelled mid-flight, and a replacement connection to an
//! in-memory database is an empty database, so every table was gone at once.
//!
//! This test aborts a writer at a different point in its loop 200 times, on
//! one database, and asserts the schema and the rows written before each
//! abort are still there. Against the in-memory factory the first abort that
//! landed mid-statement lost the database, within the first ten rounds on a
//! laptop and on the first on a loaded CI runner. One database rather than
//! one per round because the property is about the pool surviving a
//! cancellation, and a `PostgreSQL` clone per round cost that lane a minute.

use std::sync::Arc;
use std::time::Duration;

use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::WorkerRunRepository;
use tokio::time::sleep;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_writer_aborted_mid_query_does_not_take_the_database_with_it() {
    let db = create_test_db().await.expect("test db");
    let ledger: Arc<dyn WorkerRunRepository> = Arc::clone(&db.repositories().worker_runs);
    ledger.finish_worker_run("probe", 1).await.unwrap();

    for round in 0..200_u64 {
        let writer = {
            let ledger = Arc::clone(&ledger);
            tokio::spawn(async move {
                let mut n = 2_i64;
                loop {
                    ledger.defer_worker_run("probe", n).await.unwrap();
                    ledger.finish_worker_run("probe", n).await.unwrap();
                    n += 1;
                }
            })
        };
        // A different phase of the writer's loop each round.
        sleep(Duration::from_micros(50 + (round * 37) % 900)).await;
        writer.abort();
        let _ = writer.await;

        let run = ledger
            .get_worker_run("probe")
            .await
            .unwrap_or_else(|e| {
                panic!("round {round}: the ledger table vanished after the abort: {e}")
            })
            .expect("the probe row written before the abort is still there");
        assert!(
            run.last_run_at_ms >= 1,
            "round {round}: the row lost its stamp"
        );
    }
}
