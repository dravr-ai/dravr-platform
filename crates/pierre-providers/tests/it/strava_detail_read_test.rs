// ABOUTME: Pins that a Strava detail read carries the athlete's description, Perceived Exertion and recording device onto the Activity
// ABOUTME: The same fields intervals.icu fills, so the agent reads notes and the Garmin attribution provider-agnostically
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava detail read tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pierre_core::constants::oauth_providers::activity_attribution;
use pierre_providers::strava_provider::StravaProvider;
use pierre_providers::strava_types::DetailedActivityResponse;
use serde_json::{json, Value};

fn detail_with(description: Option<&str>, device_name: Option<&str>) -> DetailedActivityResponse {
    serde_json::from_value(json!({
        "id": 4242,
        "name": "Tempo run",
        "type": "Run",
        "sport_type": "Run",
        "start_date": "2026-09-20T10:00:00Z",
        "elapsed_time": 2400,
        "distance": 8000.0,
        "description": description,
        "device_name": device_name
    }))
    .unwrap()
}

fn detail(description: Option<&str>) -> DetailedActivityResponse {
    detail_with(description, None)
}

#[test]
fn detail_read_carries_the_athletes_description() {
    let activity = StravaProvider::convert_detailed_strava_activity(
        detail(Some("Calf tight from km 5, eased off")),
        None,
    )
    .unwrap();
    assert_eq!(
        activity.description(),
        Some("Calf tight from km 5, eased off")
    );
}

#[test]
fn a_blank_or_absent_description_is_no_description() {
    for description in [None, Some(""), Some("  \n ")] {
        let activity =
            StravaProvider::convert_detailed_strava_activity(detail(description), None).unwrap();
        assert!(activity.description().is_none(), "{description:?}");
    }
}

/// A Garmin recording Strava relays is attributed to its Garmin model
/// (carnet#521); Strava names the device on its detail read only.
#[test]
fn detail_read_carries_the_recording_device() {
    let activity = StravaProvider::convert_detailed_strava_activity(
        detail_with(None, Some("Garmin Forerunner 965")),
        None,
    )
    .unwrap();
    assert_eq!(activity.device_name(), Some("Garmin Forerunner 965"));
    assert_eq!(
        activity_attribution(activity.source(), activity.device_name()).as_deref(),
        Some("Garmin Forerunner 965")
    );

    for blank in [None, Some(""), Some("   ")] {
        let activity =
            StravaProvider::convert_detailed_strava_activity(detail_with(None, blank), None)
                .unwrap();
        assert!(activity.device_name().is_none(), "{blank:?}");
    }
}

/// A detail payload in the shape `GET /activities/{id}` answers for a run the
/// athlete rated, with `perceived_exertion` set to `rating` — or the key left
/// out when `rating` is `None`. `prefer_perceived_exertion` rides along as
/// Strava sends it; it never decides whether the rating is kept.
fn rated_detail(rating: Option<Value>) -> DetailedActivityResponse {
    let mut payload = json!({
        "resource_state": 3,
        "id": 16_093_224_118_u64,
        "name": "Du coup, les buttes",
        "type": "Run",
        "sport_type": "Run",
        "start_date": "2026-10-15T05:30:42Z",
        "start_date_local": "2026-10-15T07:30:42Z",
        "elapsed_time": 3534,
        "moving_time": 3198,
        "distance": 9714.8,
        "total_elevation_gain": 128.0,
        "average_speed": 3.038,
        "max_speed": 7.333,
        "has_heartrate": true,
        "average_heartrate": 135.1,
        "max_heartrate": 156.0,
        "suffer_score": 94.0,
        "calories": 698.0,
        "description": null,
        "device_name": "Garmin Forerunner 965",
        "prefer_perceived_exertion": false,
        "map": { "id": "a16093224118", "summary_polyline": "", "resource_state": 3 }
    });
    if let (Some(payload), Some(rating)) = (payload.as_object_mut(), rating) {
        payload.insert("perceived_exertion".to_owned(), rating);
    }
    serde_json::from_value(payload).unwrap()
}

/// Strava keeps the athlete's 1–10 slider value on the detail read only
/// (carnet#1033); it crosses as the CR-10 rating whether Strava sends it as an
/// integer or a float.
#[test]
fn detail_read_carries_the_athletes_perceived_exertion() {
    for (rating, expected) in [(json!(4), 4.0), (json!(6.0), 6.0), (json!(10), 10.0)] {
        let activity =
            StravaProvider::convert_detailed_strava_activity(rated_detail(Some(rating)), None)
                .unwrap();
        assert_eq!(activity.perceived_exertion(), Some(expected));
        // Relative Effort is Strava's heart-rate score, never the rating.
        assert_eq!(activity.suffer_score(), Some(94));
    }
}

#[test]
fn the_rating_is_kept_whether_or_not_strava_prefers_it_for_relative_effort() {
    for prefer in [true, false] {
        let detail: DetailedActivityResponse = serde_json::from_value(json!({
            "id": 1,
            "name": "Run",
            "type": "Run",
            "start_date": "2026-10-15T05:30:42Z",
            "perceived_exertion": 7,
            "prefer_perceived_exertion": prefer
        }))
        .unwrap();
        let activity = StravaProvider::convert_detailed_strava_activity(detail, None).unwrap();
        assert_eq!(activity.perceived_exertion(), Some(7.0), "prefer={prefer}");
    }
}

#[test]
fn an_unrated_or_off_scale_detail_has_no_perceived_exertion() {
    for rating in [None, Some(Value::Null), Some(json!(0)), Some(json!(11))] {
        let activity =
            StravaProvider::convert_detailed_strava_activity(rated_detail(rating.clone()), None)
                .unwrap();
        assert_eq!(activity.perceived_exertion(), None, "{rating:?}");
    }
}
