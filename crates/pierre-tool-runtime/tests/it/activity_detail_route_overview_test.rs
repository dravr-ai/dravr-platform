// ABOUTME: Pins that mode=detailed never hands the model a provider's encoded route overview (summary_polyline) or raw GPS track
// ABOUTME: The untrimmed line starts at the athlete's door; geometry leaves the server only as a trimmed route track
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TimeSeriesData};
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

#[test]
fn detail_mode_names_the_service_a_scraped_row_was_read_from() {
    // A Garmin connection signed in with the athlete's username and password
    // from Dravr stamps its rows with the mirror backend; the model reads the
    // service instead.
    let scraped = ActivityBuilder::new(
        "g1",
        "Hill loop",
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 9, 20, 7, 0, 0).unwrap(),
        3_000,
        "sciotte",
    )
    .source("garmin")
    .build();
    let value = detail_json(&[scraped, outdoor_run()]).unwrap();
    let rows = value.as_array().unwrap();
    assert_eq!(rows[0]["provider"], "garmin");
    assert_eq!(rows[1]["provider"], "strava");
    assert!(
        !value.to_string().to_lowercase().contains("sciotte"),
        "the backend reached the model: {value}"
    );
}

/// A run whose detail read carried its track, the way a sciotte (Garmin,
/// TrainingPeaks) detail page does: every sampled point, door to door.
fn run_with_track() -> Activity {
    ActivityBuilder::new(
        "g42",
        "Hill loop",
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 9, 20, 7, 0, 0).unwrap(),
        3_000,
        "strava",
    )
    .start_latitude(38.5)
    .start_longitude(-120.2)
    .city("Placerville".to_owned())
    .time_series_data_opt(Some(TimeSeriesData {
        timestamps: vec![0, 1, 2],
        heart_rate: Some(vec![Some(120), Some(131), Some(140)]),
        power: None,
        cadence: None,
        speed: None,
        altitude: Some(vec![Some(610.0), Some(611.5), Some(613.0)]),
        temperature: None,
        gps_coordinates: Some(vec![
            Some((38.512_345, -120.212_345)),
            Some((38.512_399, -120.212_401)),
            None,
        ]),
        distance: None,
    }))
    .build()
}

#[test]
fn detail_mode_leaves_the_raw_gps_track_out() {
    // The premise: the activity carries the track, so its absence below is
    // detail_json's doing.
    let raw = serde_json::to_value(run_with_track()).unwrap();
    assert!(
        raw["time_series_data"]["gps_coordinates"].is_array(),
        "bug in the fixture: the activity must carry its track for this test to mean anything"
    );

    let value = detail_json(&[run_with_track()]).unwrap();
    let text = value.to_string();
    assert!(
        !text.contains("gps_coordinates"),
        "bug: the raw track reached the model: {text}"
    );
    assert!(
        !text.contains("38.512345"),
        "bug: a track point leaked: {text}"
    );

    let row = only_row(&value);
    assert_eq!(
        row["start_latitude"], 38.5,
        "bug: where the activity started must stay: {row:?}"
    );
    assert_eq!(
        row["start_longitude"], -120.2,
        "bug: where the activity started must stay: {row:?}"
    );
    assert_eq!(
        row["city"], "Placerville",
        "bug: the activity's city must stay: {row:?}"
    );
    let streams = &row["time_series_data"];
    assert_eq!(
        streams["heart_rate"][2], 140,
        "bug: the non-GPS streams must stay: {streams}"
    );
    assert_eq!(
        streams["altitude"][0], 610.0,
        "bug: the non-GPS streams must stay: {streams}"
    );
}
