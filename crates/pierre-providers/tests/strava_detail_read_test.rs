// ABOUTME: Pins that a Strava detail read carries the athlete's description and the recording device onto the Activity
// ABOUTME: The same fields intervals.icu fills, so the agent reads notes and the Garmin attribution provider-agnostically
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava detail read tests.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pierre_core::constants::oauth_providers::activity_attribution;
use pierre_providers::strava_provider::StravaProvider;
use pierre_providers::strava_types::DetailedActivityResponse;
use serde_json::json;

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
