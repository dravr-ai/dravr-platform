// ABOUTME: Tests the historical-backfill gate predicate that routes inline-fetch vs background backfill
// ABOUTME: A deep `after` must route to backfill (never an inline session read); a recent `after` fetches inline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Gate-predicate tests for [`is_historical_backfill_window`].
//!
//! The predicate is the safety-critical decision: a deep historical `after`
//! must route to the bounded background backfill, never an inline provider
//! session read (the known session-read timeout outage). These tests assert the routing
//! at the wall-clock boundaries against the default 90-day threshold (the
//! `PIERRE_HISTORICAL_BACKFILL_THRESHOLD_DAYS` env is unset in this binary).

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_tool_runtime::activity_backfill::is_historical_backfill_window;

#[test]
fn deep_historical_after_routes_to_backfill() {
    // A three-year-old lower bound is far past any sane inline-read threshold.
    let three_years_ago = (Timestamp::now() - Elapsed::days(365 * 3)).epoch_seconds();
    assert!(
        is_historical_backfill_window(three_years_ago),
        "a 3-year-old `after` must route to background backfill, not an inline session read"
    );
}

#[test]
fn recent_after_fetches_inline() {
    // Yesterday is comfortably inside the rolling window — fetch inline.
    let yesterday = (Timestamp::now() - Elapsed::days(1)).epoch_seconds();
    assert!(
        !is_historical_backfill_window(yesterday),
        "a 1-day-old `after` is recent and must fetch inline"
    );
}

#[test]
fn within_default_threshold_fetches_inline() {
    // 30 days back is well inside the default 90-day threshold.
    let thirty_days_ago = (Timestamp::now() - Elapsed::days(30)).epoch_seconds();
    assert!(
        !is_historical_backfill_window(thirty_days_ago),
        "30 days is within the default 90-day window and must fetch inline"
    );
}

#[test]
fn well_past_default_threshold_routes_to_backfill() {
    // 180 days back is double the default 90-day threshold — backfill.
    let one_eighty_days_ago = (Timestamp::now() - Elapsed::days(180)).epoch_seconds();
    assert!(
        is_historical_backfill_window(one_eighty_days_ago),
        "180 days exceeds the default 90-day window and must route to backfill"
    );
}
