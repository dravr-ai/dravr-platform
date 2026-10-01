// ABOUTME: A shutdown signal ends the server: drain, return, and runtime teardown within the budget
// ABOUTME: A background sync caught mid-tick is cancelled rather than awaited, so the process exits

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The SIGTERM handler drained the in-flight turns and returned, but the
//! handler had replaced the signal's default action and nothing else ended
//! the process: it kept serving, the contremaitre poll started its next sync,
//! and only the platform's SIGKILL stopped it. The property worth a test is
//! the whole exit path the binary runs — the server future returns once the
//! drain is done, and the runtime then shuts down without waiting on a
//! background worker that is mid-tick in blocking work.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use common::create_test_server_resources;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::services::turn_lifecycle::{
    run_to_exit, serve_until_drained, RUNTIME_SHUTDOWN_GRACE, SHUTDOWN_DRAIN_BUDGET,
};
use pierre_services::periodic::spawn_periodic;
use tokio::net::TcpStream;
use tokio::runtime::Builder;
use tokio::sync::oneshot;
use tokio::task::spawn_blocking;
use tokio::time::sleep;

/// How long the stand-in sync blocks: far past every shutdown bound, so a
/// runtime that waited on it would blow the assertion by a wide margin.
const BLOCKING_SYNC: Duration = Duration::from_secs(60);

/// A port nothing is listening on, for the server to bind.
fn free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_drained_server_exits_within_the_budget_while_a_sync_is_mid_tick() {
    let runtime = Builder::new_multi_thread().enable_all().build().unwrap();
    let sync_running = Arc::new(AtomicBool::new(false));
    let signalled_at = Arc::new(OnceLock::<Instant>::new());

    let running = Arc::clone(&sync_running);
    let signalled = Arc::clone(&signalled_at);
    let probe = Arc::clone(&sync_running);
    let result = run_to_exit(runtime, async move {
        let resources = create_test_server_resources().await.unwrap();
        let turns = Arc::clone(&resources.common.turns);
        let port = free_port();

        // A periodic worker whose tick is a long blocking sync — the shape of
        // the contremaitre poll caught mid-fetch when the signal lands.
        let _worker = spawn_periodic(
            "shutdown probe",
            Duration::from_millis(10),
            Arc::clone(&resources.common.repos.worker_runs),
            move || {
                let running = Arc::clone(&running);
                async move {
                    spawn_blocking(move || {
                        running.store(true, Ordering::SeqCst);
                        thread::sleep(BLOCKING_SYNC);
                    })
                    .await
                    .unwrap();
                    Ok(())
                }
            },
        );

        // Signal once the server answers and the sync is under way.
        let (stop, stopped) = oneshot::channel::<()>();
        tokio::spawn(async move {
            while !probe.load(Ordering::SeqCst)
                || TcpStream::connect(("127.0.0.1", port)).await.is_err()
            {
                sleep(Duration::from_millis(20)).await;
            }
            signalled.set(Instant::now()).unwrap();
            stop.send(()).unwrap();
        });

        let server = ProviderToolRouter::new(resources);
        serve_until_drained(
            server.run(port),
            async {
                stopped.await.unwrap();
            },
            &turns,
        )
        .await
    });

    let exited_after = signalled_at
        .get()
        .expect("the shutdown signal was sent")
        .elapsed();
    assert!(result.is_ok(), "a drained server returns Ok: {result:?}");
    assert!(
        sync_running.load(Ordering::SeqCst),
        "the background sync was mid-tick when the signal landed"
    );
    assert!(
        exited_after <= SHUTDOWN_DRAIN_BUDGET + RUNTIME_SHUTDOWN_GRACE,
        "the server and its runtime were gone {exited_after:?} after the signal; the budget is {:?}",
        SHUTDOWN_DRAIN_BUDGET + RUNTIME_SHUTDOWN_GRACE
    );
}

/// The server's own notification service — built by production wiring, not
/// by the test — spawns every trigger dispatch onto the drain's tracker, so a
/// notification fired just before SIGTERM is awaited like a turn. That the
/// drain then waits for such a dispatch before `serve_until_drained` returns
/// is pinned next to the drain, in `turn_lifecycle`'s unit tests.
#[cfg(feature = "client-notifications")]
mod notification_drain {
    use pierre_notifications::triggers::trigger_agent_message;
    use pierre_notifications::TenantId as CommTenantId;
    use uuid::Uuid;

    use crate::common::create_test_server_resources;

    /// The trigger's dispatch is counted the moment the trigger returns. On
    /// the single-threaded test runtime nothing else runs between the two
    /// reads, so the difference is that one dispatch.
    #[tokio::test]
    async fn the_server_notification_service_dispatches_on_the_drain_tracker() {
        let resources = create_test_server_resources().await.unwrap();
        let service = resources
            .common
            .notification_service
            .as_ref()
            .expect("the server builds a notification service");
        let turns = &resources.common.turns;

        let before = turns.len();
        trigger_agent_message(
            service,
            Uuid::new_v4(),
            CommTenantId(Uuid::new_v4()),
            "conversation-tracked",
            "Coach",
        );
        assert_eq!(
            turns.len(),
            before + 1,
            "the trigger's dispatch is tracked by the drain, not spawned bare"
        );
    }
}
