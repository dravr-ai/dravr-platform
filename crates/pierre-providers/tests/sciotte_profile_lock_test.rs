// ABOUTME: Unit tests for the Sciotte per-session profile lock (Chrome SingletonLock serialization)
// ABOUTME: Verifies same-session scrapes serialize while distinct sessions run concurrently
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte profile-lock unit tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pierre_providers::sciotte_provider::ProfileLockRegistry;
use tokio::time;

#[tokio::test]
async fn same_session_serializes() {
    let registry = Arc::new(ProfileLockRegistry::new());
    let acquired = Arc::new(AtomicBool::new(false));

    // Hold the lock for "session-a".
    let guard = registry.acquire("session-a").await;

    let registry_clone = Arc::clone(&registry);
    let acquired_clone = Arc::clone(&acquired);
    let handle = tokio::spawn(async move {
        let _g = registry_clone.acquire("session-a").await;
        acquired_clone.store(true, Ordering::SeqCst);
    });

    // The spawned task must stay blocked while the first guard is held, no
    // matter how long it has had to run.
    time::sleep(Duration::from_millis(50)).await;
    assert!(
        !acquired.load(Ordering::SeqCst),
        "second acquire for the same session must block while the first guard is held"
    );

    // Releasing the guard lets the waiter proceed — this is what stops two
    // Chromes launching on the same profile dir and colliding on SingletonLock.
    drop(guard);
    handle.await.unwrap();
    assert!(
        acquired.load(Ordering::SeqCst),
        "second acquire must proceed once the first guard is dropped"
    );
}

#[tokio::test]
async fn distinct_sessions_do_not_block() {
    let registry = ProfileLockRegistry::new();

    // Hold "session-a"...
    let _held = registry.acquire("session-a").await;

    // ...a different session has its own profile dir, so it must acquire
    // immediately rather than waiting on session-a.
    let other = time::timeout(Duration::from_millis(100), registry.acquire("session-b")).await;
    assert!(
        other.is_ok(),
        "distinct session ids must not serialize against each other"
    );
}

#[tokio::test]
async fn lock_is_reusable_after_release() {
    let registry = ProfileLockRegistry::new();

    // Sequential acquisitions of the same session each succeed once the prior
    // guard drops — the map entry is reused, not leaked into a permanent hold.
    for _ in 0..3 {
        let guard = registry.acquire("session-a").await;
        drop(guard);
    }

    let guard = time::timeout(Duration::from_millis(100), registry.acquire("session-a")).await;
    assert!(
        guard.is_ok(),
        "a released lock must be re-acquirable for the same session"
    );
}
