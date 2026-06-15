// ABOUTME: Tests the historical-backfill gate predicate that routes inline-fetch vs background backfill
// ABOUTME: A deep `after` must route to backfill (never an inline scrape); a recent `after` fetches inline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Gate-predicate tests for [`is_historical_backfill_window`].
//!
//! The predicate is the safety-critical decision: a deep historical `after`
//! must route to the bounded background backfill, never an inline provider
//! scrape (the known sciotte timeout outage). These tests assert the routing
//! at the wall-clock boundaries against the default 90-day threshold (the
//! `PIERRE_HISTORICAL_BACKFILL_THRESHOLD_DAYS` env is unset in this binary).

use chrono::{Duration, Utc};
use pierre_tool_runtime::activity_backfill::is_historical_backfill_window;

#[test]
fn deep_historical_after_routes_to_backfill() {
    // A three-year-old lower bound is far past any sane inline-scrape threshold.
    let three_years_ago = (Utc::now() - Duration::days(365 * 3)).timestamp();
    assert!(
        is_historical_backfill_window(three_years_ago),
        "a 3-year-old `after` must route to background backfill, not an inline scrape"
    );
}

#[test]
fn recent_after_fetches_inline() {
    // Yesterday is comfortably inside the rolling window — fetch inline.
    let yesterday = (Utc::now() - Duration::days(1)).timestamp();
    assert!(
        !is_historical_backfill_window(yesterday),
        "a 1-day-old `after` is recent and must fetch inline"
    );
}

#[test]
fn within_default_threshold_fetches_inline() {
    // 30 days back is well inside the default 90-day threshold.
    let thirty_days_ago = (Utc::now() - Duration::days(30)).timestamp();
    assert!(
        !is_historical_backfill_window(thirty_days_ago),
        "30 days is within the default 90-day window and must fetch inline"
    );
}

#[test]
fn well_past_default_threshold_routes_to_backfill() {
    // 180 days back is double the default 90-day threshold — backfill.
    let one_eighty_days_ago = (Utc::now() - Duration::days(180)).timestamp();
    assert!(
        is_historical_backfill_window(one_eighty_days_ago),
        "180 days exceeds the default 90-day window and must route to backfill"
    );
}
