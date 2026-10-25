// ABOUTME: Serialises the it-binary tests that run a guarded fallback chain, each on a reset CHAIN_GUARD
// ABOUTME: The breaker and GitHub budget are one per process, so one test's faults would reroute another's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `CHAIN_GUARD` holds the chain's circuit breaker and the Copilot token's
//! GitHub budget, one instance per process. Every request through
//! `EmbacleProvider::chain` reads it, and a primary fault counts on it: three
//! faults from any tests in this binary open the circuit for a minute, and a
//! test that pushes the budget low reroutes every Copilot primary after it.
//! A test that runs a chain therefore holds this lock for its whole body and
//! starts from a guard put back to fail-open: circuit closed, no failures
//! counted, budget above threshold.

use std::time::UNIX_EPOCH;

use embacle::quota_http::GithubRateLimit;
use pierre_llm::chain_guard::{CHAIN_GUARD, GITHUB_BUDGET_THRESHOLD};
use tokio::sync::{Mutex, MutexGuard};

static CHAIN_GUARD_USERS: Mutex<()> = Mutex::const_new(());

/// Waits for every other chain test to finish, then resets `CHAIN_GUARD`.
/// Keep the returned guard alive until the test ends.
pub(crate) async fn fresh_chain_guard() -> MutexGuard<'static, ()> {
    let held = CHAIN_GUARD_USERS.lock().await;
    CHAIN_GUARD.record_primary_success();
    let healthy = GithubRateLimit {
        remaining: GITHUB_BUDGET_THRESHOLD * 10,
        limit: GITHUB_BUDGET_THRESHOLD * 10,
        used: 0,
        resets_at: UNIX_EPOCH,
    };
    CHAIN_GUARD.record_github_headroom(&Ok(healthy));
    held
}
