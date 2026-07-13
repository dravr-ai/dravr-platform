// ABOUTME: Regression test for the Sciotte per-user active-login dedup guard
// ABOUTME: Pins that a concurrent login from the same user is rejected, released on drop, per-user
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Active-login dedup regression test.
//!
//! One slow Sciotte login (vision-mode grind, or a number-match phone-tap poll)
//! can legitimately hold the pod's single Chrome slot for the full
//! `DRAVR_SCIOTTE_LOGIN_TIMEOUT`. While it runs, a client retrying `/login`
//! previously acquired the saturated limiter, waited the full acquire timeout,
//! and shed a 503 on every retry — one stuck login became a self-inflicted 503
//! storm (the 2026-07-11 rev 00687 incident).
//!
//! [`try_begin_active_login`] closes the gap between the parked-flow dedup
//! (which only covers OTP/2FA continuations) and the active `credential_login`
//! phase. These tests pin its contract without launching Chrome. The distinct
//! random user ids also keep the tests independent when run in parallel.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![cfg(feature = "provider-sciotte")]

use pierre_routes_auth::try_begin_active_login;
use uuid::Uuid;

#[test]
fn second_concurrent_login_for_same_user_is_deduped() {
    let user = Uuid::new_v4();

    let first = try_begin_active_login(user);
    assert!(first.is_some(), "first login for a user must be admitted");

    // While the first is in flight a concurrent login from the same user is
    // rejected — this is what collapses a retry storm into a single 409 instead
    // of a pile of acquire-timeout 503s.
    let second = try_begin_active_login(user);
    assert!(
        second.is_none(),
        "a concurrent login for the same user must be deduped while one is active"
    );

    // The guard is RAII: once the first login completes (guard dropped) the user
    // may log in again.
    drop(first);
    let after_release = try_begin_active_login(user);
    assert!(
        after_release.is_some(),
        "after the active login completes, the user may log in again"
    );
    drop(after_release);
}

#[test]
fn distinct_users_are_not_blocked_by_each_other() {
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();

    let alice_guard = try_begin_active_login(alice);
    let bob_guard = try_begin_active_login(bob);

    // Per-user isolation is the anti-regression guarantee: dedup only ever
    // rejects a *duplicate* login from the *same* user, never a distinct user
    // and never a single login. Capping/serialising is emphatically not what
    // this does.
    assert!(alice_guard.is_some(), "Alice's login must be admitted");
    assert!(
        bob_guard.is_some(),
        "Bob's login must not be blocked by Alice's concurrent login"
    );

    drop(alice_guard);
    drop(bob_guard);
}
