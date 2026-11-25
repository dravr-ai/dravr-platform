// ABOUTME: Pins that activity rows reach the reader with each f32 at its own precision
// ABOUTME: Summary and detail mode both widened f32 to f64, showing 12.8 °C as 12.800000190734863
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity list is the most-read tool result, and both of its shapes
//! carried `f32` values through `serde_json::to_value`, which widens them:
//! summary mode's backfilled `temperature` and `perceived_exertion`, and detail
//! mode's cageux `Activity` (temperature, humidity, wind, altitude, blood oxygen …).
//! carnet#532.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_tronc::dates::Timestamp;
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_test_support::numbers::mentions_number;
use pierre_tool_runtime::implementations::activity_summary::{
    detail_json, summary_json, ActivitySummary,
};

/// How an `f32` reads once widened to `f64`: the number a fixed reply must not
/// carry. Matched as a whole number, so a clock's digits cannot stand for it.
fn widened(value: f32) -> String {
    f64::from(value).to_string()
}

fn ride() -> Activity {
    ActivityBuilder::new(
        "a1",
        "Tempo ride",
        SportType::Ride,
        Timestamp::parse_from_rfc3339("2026-09-20T17:00:00Z").unwrap(),
        3_600,
        "strava",
    )
    .temperature(12.8)
    .perceived_exertion(6.3)
    .build()
}

#[test]
fn a_summary_row_reads_12_8_not_its_f64_widening() {
    let text = summary_json(&[ActivitySummary::from(&ride())])
        .unwrap()
        .to_string();
    assert!(mentions_number(&text, "12.8"), "{text}");
    assert!(mentions_number(&text, "6.3"), "{text}");
    assert!(
        !mentions_number(&text, &widened(12.8)),
        "summary widened an f32: {text}"
    );
    assert!(
        !mentions_number(&text, &widened(6.3)),
        "summary widened an f32: {text}"
    );
}

#[test]
fn a_detail_row_reads_12_8_not_its_f64_widening() {
    let text = detail_json(&[ride()]).unwrap().to_string();
    assert!(mentions_number(&text, "12.8"), "{text}");
    assert!(
        !mentions_number(&text, &widened(12.8)),
        "detail widened an f32: {text}"
    );
    assert!(
        !mentions_number(&text, &widened(6.3)),
        "detail widened an f32: {text}"
    );
}
