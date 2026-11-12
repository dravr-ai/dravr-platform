// ABOUTME: Pins that Strava's moving time, workout tag, trainer and commute flags reach the Activity
// ABOUTME: Elapsed time stays the duration; moving time beside it is what shows a long stop
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava activity fields the list endpoint already returns (carnet#1479).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pierre_providers::strava_provider::StravaProvider;
use pierre_providers::strava_types::DetailedActivityResponse;
use serde_json::{json, Value};

fn activity(extra: &Value) -> DetailedActivityResponse {
    let mut payload = json!({
        "id": 16_001,
        "name": "Bûcheronner !",
        "type": "Workout",
        "sport_type": "Workout",
        "start_date": "2026-11-07T13:00:00Z",
        "elapsed_time": 13_320,
        "distance": 2_100.0
    });
    for (key, value) in extra.as_object().unwrap() {
        payload[key] = value.clone();
    }
    serde_json::from_value(payload).unwrap()
}

#[test]
fn moving_time_sits_beside_the_elapsed_duration() {
    let mapped = StravaProvider::convert_detailed_strava_activity(
        activity(&json!({ "moving_time": 4_080 })),
        None,
    )
    .unwrap();
    assert_eq!(
        mapped.duration_seconds(),
        13_320,
        "duration stays elapsed time"
    );
    assert_eq!(
        mapped.moving_time_seconds(),
        Some(4_080),
        "bug: moving time dropped"
    );
}

#[test]
fn workout_tag_trainer_and_commute_are_carried() {
    let mapped = StravaProvider::convert_detailed_strava_activity(
        activity(&json!({ "workout_type": 2, "trainer": true, "commute": false })),
        None,
    )
    .unwrap();
    assert_eq!(mapped.workout_type(), Some(2), "bug: workout tag dropped");
    assert_eq!(mapped.trainer(), Some(true), "bug: trainer flag dropped");
    assert_eq!(mapped.commute(), Some(false), "bug: commute flag dropped");
}

#[test]
fn absent_fields_stay_absent() {
    let mapped =
        StravaProvider::convert_detailed_strava_activity(activity(&json!({})), None).unwrap();
    assert_eq!(
        mapped.moving_time_seconds(),
        None,
        "bug: moving time invented"
    );
    assert_eq!(mapped.workout_type(), None, "bug: workout tag invented");
    assert_eq!(mapped.trainer(), None, "bug: trainer flag invented");
    assert_eq!(mapped.commute(), None, "bug: commute flag invented");
}

#[test]
fn a_zero_moving_time_is_no_moving_time() {
    let mapped = StravaProvider::convert_detailed_strava_activity(
        activity(&json!({ "moving_time": 0 })),
        None,
    )
    .unwrap();
    assert_eq!(
        mapped.moving_time_seconds(),
        None,
        "bug: a zero moving time reads as an activity that never moved"
    );
}
