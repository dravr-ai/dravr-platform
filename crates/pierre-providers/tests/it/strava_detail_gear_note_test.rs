// ABOUTME: Pins that Strava's detail payload brings the gear and the athlete's private note onto the Activity
// ABOUTME: An empty note or a nameless gear reads as none, never as an empty value
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava detail fields read once and stored (carnet#1480).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use pierre_providers::strava_provider::StravaProvider;
use pierre_providers::strava_types::DetailedActivityResponse;
use serde_json::{json, Value};

fn detail(extra: &Value) -> DetailedActivityResponse {
    let mut payload = json!({
        "id": 16_002,
        "name": "Gai-Luron",
        "type": "Ride",
        "sport_type": "MountainBikeRide",
        "start_date": "2026-11-09T14:00:00Z",
        "elapsed_time": 5_400,
        "distance": 18_000.0
    });
    for (key, value) in extra.as_object().unwrap() {
        payload[key] = value.clone();
    }
    serde_json::from_value(payload).unwrap()
}

#[test]
fn gear_and_private_note_reach_the_activity() {
    let activity = StravaProvider::convert_detailed_strava_activity(
        detail(&json!({
            "gear": { "id": "b42", "name": " Epic EVO ", "distance": 1234.0, "primary": true },
            "private_note": "Genou sensible"
        })),
        None,
    )
    .unwrap();
    let gear = activity
        .gear()
        .expect("bug: the detail's gear must be mapped");
    assert_eq!(gear.id.as_deref(), Some("b42"), "bug: gear id");
    assert_eq!(gear.name, "Epic EVO", "bug: gear name is trimmed");
    assert_eq!(
        activity.private_note(),
        Some("Genou sensible"),
        "bug: private note"
    );
}

#[test]
fn an_empty_note_and_a_nameless_gear_read_as_none() {
    let activity = StravaProvider::convert_detailed_strava_activity(
        detail(&json!({ "gear": { "id": "b42", "name": "  " }, "private_note": " " })),
        None,
    )
    .unwrap();
    assert!(activity.gear().is_none(), "bug: a nameless gear is no gear");
    assert!(
        activity.private_note().is_none(),
        "bug: a blank note is no note"
    );
}
