// ABOUTME: Tests should_skip_probe — the piggyback decision that lets the periodic LLM probe
// ABOUTME: skip its billed copilot --acp round-trip when real chat traffic already proved liveness
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for the periodic-probe skip decision in
//! [`pierre_services::chat_provider_factory::should_skip_probe`]. The
//! function is pure (no I/O), so the cost-saving policy is unit-testable
//! without spawning the probe task.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pierre_services::chat_provider_factory::should_skip_probe;

const INTERVAL: Duration = Duration::from_mins(30);

#[test]
fn no_real_traffic_yet_never_skips() {
    // An idle service that has never served a turn must keep probing so
    // `/ready` reflects synthetic liveness.
    assert!(!should_skip_probe(None, INTERVAL));
}

#[test]
fn recent_real_traffic_skips_the_billed_probe() {
    assert!(should_skip_probe(Some(Duration::from_mins(1)), INTERVAL));
}

#[test]
fn stale_real_traffic_falls_back_to_probing() {
    // Last real turn is older than the interval — liveness is no longer
    // proven, so the synthetic probe must run.
    let stale = INTERVAL + Duration::from_secs(1);
    assert!(!should_skip_probe(Some(stale), INTERVAL));
}

#[test]
fn success_exactly_at_interval_still_probes() {
    // Boundary: the predicate is strictly-less-than, so a success landing
    // exactly `interval` ago does NOT skip.
    assert!(!should_skip_probe(Some(INTERVAL), INTERVAL));
}

#[tokio::test]
async fn piggyback_stamps_the_real_success_time_not_now() {
    use pierre_llm::health::LlmHealthState;

    // A real turn succeeded ~10 minutes ago; the periodic tick observes it now
    // and skips its billed probe. The recorded snapshot must reflect the real
    // success time, not `now`, or `/health/llm` overstates freshness.
    let state = LlmHealthState::new();
    let observed_ago = Duration::from_mins(10);
    let before = chrono::Utc::now();
    state
        .record_healthy_observed("copilot_headless", observed_ago)
        .await;

    let snap = state.snapshot().await;
    let checked_at = snap
        .checked_at
        .expect("a healthy snapshot carries checked_at");
    let age_secs = (before - checked_at).num_seconds();
    assert!(
        (590..=610).contains(&age_secs),
        "checked_at must trail now by ~observed_ago (600s), got {age_secs}s"
    );
}
