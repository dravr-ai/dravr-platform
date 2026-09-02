// ABOUTME: Pins the stored-data date-range clamp — default window, inverted refusal, year cap
// ABOUTME: The underlying queries carry no SQL LIMIT, so this clamp is the read bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![cfg(feature = "tools-data")]

use chrono::{Duration, Utc};
use pierre_tool_runtime::implementations::stored_data::parse_date_range;
use serde_json::json;

#[test]
fn omitted_range_defaults_to_the_last_thirty_days() {
    let (start, end) = parse_date_range(&json!({})).unwrap();
    let span = end - start;
    assert!(
        span >= Duration::days(29) && span <= Duration::days(31),
        "got {span}"
    );
    assert!((Utc::now() - end).num_seconds().abs() < 5);
}

#[test]
fn an_inverted_range_is_refused() {
    let args = json!({
        "start": "2026-06-01T00:00:00Z",
        "end": "2026-01-01T00:00:00Z",
    });
    let err = parse_date_range(&args).unwrap_err();
    assert!(err.to_string().contains("after"), "got: {err}");
}

#[test]
fn a_span_wider_than_a_year_is_clipped_to_the_most_recent_year() {
    let args = json!({
        "start": "2020-01-01T00:00:00Z",
        "end": "2026-01-01T00:00:00Z",
    });
    let (start, end) = parse_date_range(&args).unwrap();
    assert_eq!(end.to_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(end - start, Duration::days(366), "clipped to the cap");
}
