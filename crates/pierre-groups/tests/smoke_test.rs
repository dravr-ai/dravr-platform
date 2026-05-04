// ABOUTME: Smoke test for pierre-groups' GroupAggregationStrategy
// ABOUTME: Verifies LiveAggregation handles the empty-snapshot edge case
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::time::Duration;

use pierre_groups::strategies::aggregation::{GroupAggregationStrategy, LiveAggregation};

#[test]
fn live_aggregation_with_no_snapshots_yields_zero_members() {
    let strategy = LiveAggregation;
    let stats = strategy.aggregate_stats(&[]);

    assert_eq!(stats.total_members, 0);
    assert_eq!(stats.active_members, 0);
    assert_eq!(stats.flagged_members, 0);
    assert_eq!(stats.avg_weekly_volume_km, 0.0);
    assert!(stats.avg_ctl.is_none());
}

#[test]
fn live_aggregation_staleness_tolerance_is_zero_for_live_strategy() {
    let strategy = LiveAggregation;
    // "Live" by name — must serve fresh, so tolerance is zero or tiny.
    assert_eq!(
        strategy.staleness_tolerance(),
        Duration::ZERO,
        "LiveAggregation must not tolerate stale data"
    );
}
