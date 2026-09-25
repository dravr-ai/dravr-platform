// ABOUTME: Pins that mode=detailed never hands the model a provider's encoded route overview (summary_polyline)
// ABOUTME: The untrimmed line starts at the athlete's door; geometry leaves the server only as a trimmed route track
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::implementations::activity_summary::detail_json;
use serde_json::Value;

/// The Google reference encoding of (38.5,-120.2), (40.7,-120.95), (43.252,-126.453).
const OVERVIEW: &str = "_p~iF~ps|U_ulLnnqC_mqNvxq`@";

fn outdoor_run() -> Activity {
    ActivityBuilder::new(
        "s77",
        "Hill loop",
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 9, 20, 7, 0, 0).unwrap(),
        3_000,
        "strava",
    )
    .distance_meters(10_200.0)
    .start_latitude(38.5)
    .start_longitude(-120.2)
    .summary_polyline(OVERVIEW.to_owned())
    .build()
}

fn only_row(value: &Value) -> &serde_json::Map<String, Value> {
    let rows = value.as_array().expect("detail_json answers with an array");
    assert_eq!(rows.len(), 1);
    rows[0].as_object().expect("each row is an object")
}

#[test]
fn the_activity_itself_still_carries_its_overview() {
    // The premise: the cached activity keeps the polyline (the Home route
    // endpoint draws from it), so leaving it out is detail_json's doing.
    let activity = outdoor_run();
    assert_eq!(activity.summary_polyline(), Some(OVERVIEW));
    let raw = serde_json::to_value(&activity).unwrap();
    assert_eq!(raw["summary_polyline"], OVERVIEW);
}

#[test]
fn detail_mode_leaves_the_route_overview_out() {
    let value = detail_json(&[outdoor_run()]).unwrap();
    let row = only_row(&value);

    assert!(
        !row.contains_key("summary_polyline"),
        "the model must never receive the provider's untrimmed route: {row:?}"
    );
    assert!(
        !value.to_string().contains(OVERVIEW),
        "the encoded line must not appear anywhere in the payload"
    );
}

#[test]
fn detail_mode_keeps_every_other_field_of_the_activity() {
    let value = detail_json(&[outdoor_run()]).unwrap();
    let row = only_row(&value);

    assert_eq!(row["id"], "s77");
    assert_eq!(row["name"], "Hill loop");
    assert_eq!(row["provider"], "strava");
    assert_eq!(row["distance_meters"], 10_200.0);
    assert_eq!(row["start_latitude"], 38.5);
    assert_eq!(row["start_longitude"], -120.2);
}
