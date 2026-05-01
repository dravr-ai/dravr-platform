// ABOUTME: Unit tests for the Sciotte backpressure limiter (FIFO fairness, queue-full, timeout, try_acquire)
// ABOUTME: Runs in isolation from the HTTP routes to keep feedback fast and deterministic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pierre_providers::sciotte_limiter::{
    LimiterConfig, LimiterConfigError, LimiterError, SciotteLimiter, ENV_MAX_CONCURRENT,
    ENV_WATCHDOG_INTERVAL_SECS,
};
use tokio::sync::Mutex;
use tokio::time;

fn tiny_config() -> LimiterConfig {
    LimiterConfig {
        max_concurrent: 1,
        max_queue_depth: 2,
        acquire_timeout: Duration::from_millis(100),
        parked_permit_ttl: Duration::from_secs(10),
        watchdog_interval: Duration::from_secs(1),
        retry_after_hint: Duration::from_millis(500),
        closed_retry_after: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn acquire_releases_slot_on_drop() {
    let limiter = SciotteLimiter::new(tiny_config());
    assert_eq!(limiter.available_permits(), 1);

    let permit = limiter.acquire().await.expect("first acquire ok");
    assert_eq!(limiter.available_permits(), 0);
    assert_eq!(limiter.depth(), 1);

    drop(permit);
    assert_eq!(limiter.available_permits(), 1);
    assert_eq!(limiter.depth(), 0);
}

#[tokio::test]
async fn queue_full_rejects_immediately() {
    let limiter = SciotteLimiter::new(LimiterConfig {
        max_concurrent: 1,
        max_queue_depth: 1,
        acquire_timeout: Duration::from_secs(1),
        parked_permit_ttl: Duration::from_secs(1),
        watchdog_interval: Duration::from_secs(1),
        retry_after_hint: Duration::from_secs(3),
        closed_retry_after: Duration::from_secs(10),
    });
    let held = limiter.acquire().await.expect("first acquire ok");
    assert_eq!(limiter.depth(), 1);

    let rejected = limiter.acquire().await;
    assert!(
        matches!(rejected, Err(LimiterError::QueueFull { .. })),
        "expected QueueFull, got {rejected:?}"
    );
    drop(held);
}

#[tokio::test]
async fn acquire_timeout_emits_error() {
    let limiter = SciotteLimiter::new(tiny_config());
    let held = limiter.acquire().await.expect("first acquire ok");

    let waiter_start = Instant::now();
    let result = limiter.acquire().await;
    let elapsed = waiter_start.elapsed();

    assert!(
        matches!(result, Err(LimiterError::AcquireTimeout { .. })),
        "expected AcquireTimeout, got {result:?}"
    );
    assert!(elapsed >= Duration::from_millis(80));
    drop(held);
}

#[tokio::test]
async fn try_acquire_does_not_block() {
    let limiter = SciotteLimiter::new(tiny_config());
    let held = limiter.acquire().await.expect("first ok");

    let attempt = limiter.try_acquire();
    assert!(
        matches!(attempt, Err(LimiterError::NoCapacity { .. })),
        "try_acquire should return NoCapacity when the pool is saturated, got {attempt:?}"
    );
    drop(held);

    let ok = limiter.try_acquire();
    assert!(ok.is_ok());
}

#[test]
fn limiter_error_carries_configured_retry_after_hint() {
    // Each error variant embeds the caller-supplied retry_after Duration;
    // there is no crate-level default. The HTTP layer reads `retry_after_secs()`
    // which simply returns the embedded value.
    let err = LimiterError::QueueFull {
        depth: 9,
        max: 8,
        retry_after: Duration::from_secs(7),
    };
    assert_eq!(err.retry_after_secs(), 7);

    let err = LimiterError::AcquireTimeout {
        timeout: Duration::from_secs(3),
        retry_after: Duration::from_secs(4),
    };
    assert_eq!(err.retry_after_secs(), 4);

    let err = LimiterError::NoCapacity {
        retry_after: Duration::from_secs(2),
    };
    assert_eq!(err.retry_after_secs(), 2);

    let err = LimiterError::Closed {
        retry_after: Duration::from_mins(1),
    };
    assert_eq!(err.retry_after_secs(), 60);
}

#[test]
fn limiter_config_error_shapes_are_user_readable() {
    // `LimiterConfig::from_env()` is exercised at binary startup, not in
    // integration tests — mutating the process environment is racy under
    // `cargo test`'s parallel harness. This test instead asserts the error
    // display shapes, which is what operators actually see in server logs
    // when a container boots with bad config.
    let missing = LimiterConfigError::Missing(ENV_MAX_CONCURRENT);
    assert!(missing.to_string().contains(ENV_MAX_CONCURRENT));

    let zero = LimiterConfigError::ZeroValue(ENV_WATCHDOG_INTERVAL_SECS);
    assert!(zero.to_string().contains(ENV_WATCHDOG_INTERVAL_SECS));
    assert!(zero.to_string().contains("greater than zero"));

    let inverted = LimiterConfigError::QueueSmallerThanConcurrency {
        concurrent: 4,
        queue: 2,
    };
    assert!(inverted.to_string().contains("max_queue_depth (2)"));
    assert!(inverted.to_string().contains("max_concurrent (4)"));
}

#[tokio::test]
async fn fifo_waiter_order_is_preserved() {
    let limiter = SciotteLimiter::new(LimiterConfig {
        max_concurrent: 1,
        max_queue_depth: 16,
        acquire_timeout: Duration::from_secs(5),
        parked_permit_ttl: Duration::from_secs(10),
        watchdog_interval: Duration::from_secs(1),
        retry_after_hint: Duration::from_secs(2),
        closed_retry_after: Duration::from_secs(5),
    });

    let head = limiter.acquire().await.expect("head ok");
    let order = Arc::new(Mutex::new(Vec::<u32>::new()));

    let mut handles = Vec::new();
    for idx in 1_u32..=3 {
        let l = Arc::clone(&limiter);
        let o = Arc::clone(&order);
        let h = tokio::spawn(async move {
            let p = l.acquire().await.expect("acquire ok");
            o.lock().await.push(idx);
            drop(p);
        });
        handles.push(h);
        time::sleep(Duration::from_millis(20)).await;
    }

    drop(head);
    for h in handles {
        h.await.expect("task joined");
    }

    let observed = order.lock().await.clone();
    assert_eq!(observed, vec![1, 2, 3], "FIFO violated: {observed:?}");
}

#[tokio::test]
async fn watchdog_drives_tick_callback() {
    let limiter = SciotteLimiter::new(LimiterConfig {
        max_concurrent: 4,
        max_queue_depth: 8,
        acquire_timeout: Duration::from_millis(100),
        parked_permit_ttl: Duration::from_millis(50),
        watchdog_interval: Duration::from_millis(25),
        retry_after_hint: Duration::from_millis(500),
        closed_retry_after: Duration::from_secs(5),
    });

    let tick_count = Arc::new(AtomicUsize::new(0));
    let observed_ttl = Arc::new(Mutex::new(Duration::ZERO));

    let tc = Arc::clone(&tick_count);
    let ot = Arc::clone(&observed_ttl);
    let handle = limiter.spawn_watchdog(move |ttl| {
        let tc = Arc::clone(&tc);
        let ot = Arc::clone(&ot);
        async move {
            tc.fetch_add(1, Ordering::Relaxed);
            *ot.lock().await = ttl;
            0
        }
    });

    time::sleep(Duration::from_millis(120)).await;
    handle.abort();

    let observed_count = tick_count.load(Ordering::Relaxed);
    assert!(
        observed_count >= 2,
        "watchdog should have ticked at least twice, got {observed_count}"
    );
    assert_eq!(*observed_ttl.lock().await, Duration::from_millis(50));
}
