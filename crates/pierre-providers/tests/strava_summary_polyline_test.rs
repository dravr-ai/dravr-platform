// ABOUTME: Pins that Strava's map.summary_polyline reaches the Activity exactly as Strava encoded it
// ABOUTME: An empty polyline (indoor, trainer, manual) reads as no route rather than an empty route
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava summary polyline mapping tests.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pierre_providers::strava_provider::StravaProvider;
use pierre_providers::strava_types::DetailedActivityResponse;
use serde_json::{json, Value};

/// The Google polyline reference encoding of (38.5,-120.2), (40.7,-120.95),
/// (43.252,-126.453) — the shape Strava's `summary_polyline` takes.
const REFERENCE_POLYLINE: &str = "_p~iF~ps|U_ulLnnqC_mqNvxq`@";

fn activity_with_map(map: &Value) -> DetailedActivityResponse {
    serde_json::from_value(json!({
        "id": 9001,
        "name": "Hill loop",
        "type": "Run",
        "sport_type": "TrailRun",
        "start_date": "2026-09-20T07:00:00Z",
        "elapsed_time": 3000,
        "distance": 9500.0,
        "start_latlng": [45.5, -73.6],
        "map": map
    }))
    .unwrap()
}

#[test]
fn the_summary_polyline_is_carried_byte_for_byte() {
    let detail = activity_with_map(&json!({
        "id": "a9001",
        "summary_polyline": REFERENCE_POLYLINE,
        "resource_state": 2
    }));
    let activity = StravaProvider::convert_detailed_strava_activity(detail, None).unwrap();
    assert_eq!(activity.summary_polyline(), Some(REFERENCE_POLYLINE));
    assert_eq!(activity.start_latitude(), Some(45.5));
}

#[test]
fn an_indoor_activitys_empty_polyline_is_no_route() {
    for map in [
        json!({ "id": "a9001", "summary_polyline": "" }),
        json!({ "id": "a9001", "summary_polyline": null }),
        json!({ "id": "a9001" }),
        Value::Null,
    ] {
        let activity =
            StravaProvider::convert_detailed_strava_activity(activity_with_map(&map), None)
                .unwrap();
        assert!(
            activity.summary_polyline().is_none(),
            "{map} must read as no route"
        );
    }
}
