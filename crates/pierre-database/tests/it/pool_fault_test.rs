// ABOUTME: The PostgreSQL pool under network faults: refused, silent, reset and slow connections
// ABOUTME: Each fault fails a query within the acquire timeout, never a hang, and the pool heals after
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "postgresql")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! On 2026-07-10 the dev database ran out of connection slots, and every
//! request waited the full 30 s acquire timeout before it failed. Whether the
//! pool fails *within* its acquire timeout and comes back on its own once the
//! database does had never been tested: nothing could make a database
//! misbehave on cue. These tests put a [`FaultProxy`] between the pool and the
//! test's `PostgreSQL` and turn on the failures Cloud SQL can show through its
//! connector: refused connections (no slot left), connections that open and
//! never answer, live connections cut by a restart or failover, and a slow link.
//!
//! The faults are network faults, so these tests need a `PostgreSQL` server.
//! The PostgreSQL lane sets `PIERRE_TEST_REQUIRE_POSTGRES`, under which the
//! test database factory refuses to hand out a `SQLite` URL, so there they
//! cannot pass without having run against the proxy.

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use pierre_core::config::database::PostgresPoolConfig;
use pierre_database::backends::factory::Database;
use pierre_test_support::db::{create_test_db_url, TestDatabaseUrl};
use pierre_test_support::fault_proxy::{Fault, FaultProxy};
use tokio::time::sleep;

/// The pool's acquire timeout in these tests.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long past the acquire timeout a failed query may take before the test
/// calls it a hang: a fresh connection's TCP and TLS-less startup, plus a
/// loaded runner's scheduling.
const SLACK: Duration = Duration::from_secs(4);

/// How long a healed database may take to serve the pool again.
const RECOVERY: Duration = Duration::from_secs(10);

/// The `host:port` of a `postgres://` URL, and the URL with it replaced.
fn split_authority(url: &str) -> Option<(SocketAddr, String, String)> {
    let rest = url
        .strip_prefix("postgresql://")
        .or_else(|| url.strip_prefix("postgres://"))?;
    let scheme = &url[..url.len() - rest.len()];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let (userinfo, host_port) = authority
        .rsplit_once('@')
        .map_or(("", authority), |(user, host)| (user, host));
    let host_port = if host_port.contains(':') {
        host_port.to_owned()
    } else {
        format!("{host_port}:5432")
    };
    let upstream = host_port
        .to_socket_addrs()
        .ok()?
        .find(SocketAddr::is_ipv4)?;
    let prefix = if userinfo.is_empty() {
        scheme.to_owned()
    } else {
        format!("{scheme}{userinfo}@")
    };
    Some((upstream, prefix, rest[authority_end..].to_owned()))
}

/// A pool on the test database, connecting through a fresh fault proxy.
struct Faulty {
    db: Database,
    proxy: FaultProxy,
    _reserved: TestDatabaseUrl,
}

/// `None` when the factory handed out `SQLite`, which has no network to fault.
async fn faulty_pool() -> Option<Faulty> {
    let reserved = create_test_db_url().await.expect("test database");
    if !reserved.url.starts_with("postgres") {
        return None;
    }
    let (upstream, prefix, path) = split_authority(&reserved.url)
        .unwrap_or_else(|| panic!("a PostgreSQL test URL that cannot be routed through the proxy"));
    let proxy = FaultProxy::start(upstream).await.expect("fault proxy");
    let url = format!("{prefix}{}{path}", proxy.addr());
    let config = PostgresPoolConfig {
        max_connections: 2,
        min_connections: 0,
        acquire_timeout_secs: ACQUIRE_TIMEOUT.as_secs(),
        connection_retries: 0,
        initial_retry_delay_ms: 50,
        max_retry_delay_ms: 50,
        ..PostgresPoolConfig::default()
    };
    let db = Database::new(&url, vec![0_u8; 32], &config)
        .await
        .expect("a pool through a passing proxy opens");
    Some(Faulty {
        db,
        proxy,
        _reserved: reserved,
    })
}

/// One cheap round trip through the pool, and how long it took.
async fn probe(db: &Database) -> (bool, Duration) {
    let started = Instant::now();
    let ok = db.get_system_setting("pool-fault-probe").await.is_ok();
    (ok, started.elapsed())
}

/// Asserts a query fails, and within the acquire timeout plus slack.
async fn assert_fails_bounded(db: &Database, fault: &str) {
    let (ok, took) = probe(db).await;
    assert!(!ok, "{fault}: the query succeeded through the fault");
    assert!(
        took <= ACQUIRE_TIMEOUT + SLACK,
        "{fault}: the query took {took:?} to fail, past the {ACQUIRE_TIMEOUT:?} acquire timeout"
    );
}

/// Asserts the pool serves a query again within [`RECOVERY`].
async fn assert_recovers(db: &Database, after: &str) {
    let deadline = Instant::now() + RECOVERY;
    loop {
        if probe(db).await.0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "after {after}: the pool did not serve a query within {RECOVERY:?} of the database healing"
        );
        sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_connections_fail_within_the_acquire_timeout_and_the_pool_heals() {
    let Some(faulty) = faulty_pool().await else {
        return;
    };
    assert!(probe(&faulty.db).await.0, "the pool works before any fault");

    faulty.proxy.set(Fault::Refuse);
    faulty.proxy.sever();
    assert_fails_bounded(&faulty.db, "refused").await;

    faulty.proxy.set(Fault::Pass);
    assert_recovers(&faulty.db, "refused connections").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_database_that_accepts_and_never_answers_fails_within_the_acquire_timeout() {
    let Some(faulty) = faulty_pool().await else {
        return;
    };
    assert!(probe(&faulty.db).await.0, "the pool works before any fault");

    faulty.proxy.set(Fault::Blackhole);
    faulty.proxy.sever();
    assert_fails_bounded(&faulty.db, "blackholed").await;

    faulty.proxy.set(Fault::Pass);
    faulty.proxy.sever();
    assert_recovers(&faulty.db, "a silent database").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_connections_cut_by_a_restart_are_replaced() {
    let Some(faulty) = faulty_pool().await else {
        return;
    };
    // Fill the pool so the cut lands on connections it holds.
    let (a, b) = tokio::join!(probe(&faulty.db), probe(&faulty.db));
    assert!(a.0 && b.0, "the pool works before any fault");

    faulty.proxy.sever();
    assert_recovers(&faulty.db, "a restart").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_link_still_serves_queries() {
    let Some(faulty) = faulty_pool().await else {
        return;
    };
    faulty.proxy.set(Fault::Latency(Duration::from_millis(100)));
    faulty.proxy.sever();
    let (ok, took) = probe(&faulty.db).await;
    assert!(ok, "a slow link is slow, not down");
    assert!(
        took >= Duration::from_millis(200),
        "the latency was applied: {took:?}"
    );
}
