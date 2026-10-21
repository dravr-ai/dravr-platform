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

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::implementations::activity_summary::{
    detail_json, summary_json, ActivitySummary,
};

fn ride() -> Activity {
    ActivityBuilder::new(
        "a1",
        "Tempo ride",
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 9, 20, 17, 0, 0).unwrap(),
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
    assert!(text.contains("12.8"), "{text}");
    assert!(text.contains("6.3"), "{text}");
    assert!(!text.contains("12.80000"), "summary widened an f32: {text}");
    assert!(!text.contains("6.30000"), "summary widened an f32: {text}");
}

#[test]
fn a_detail_row_reads_12_8_not_its_f64_widening() {
    let text = detail_json(&[ride()]).unwrap().to_string();
    assert!(text.contains("12.8"), "{text}");
    assert!(!text.contains("12.80000"), "detail widened an f32: {text}");
    assert!(!text.contains("6.30000"), "detail widened an f32: {text}");
}
