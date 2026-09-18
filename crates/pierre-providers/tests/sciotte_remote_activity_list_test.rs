// ABOUTME: Pins how the platform decodes sciotte's /api/activities body, in particular that
// ABOUTME: head_complete is read when present and assumed true from a service that omits it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `RemoteActivityList` is the wire shape between the platform and the sciotte
//! service. `head_complete: false` is the one signal that a capture is missing
//! the list's head (carnet#151), so decoding it wrong would silently restore
//! the masking that lost activities in carnet#149; a body from a service that
//! predates the field must decode as complete, which is what every consumer
//! assumed before the field existed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pierre_providers::sciotte_remote::RemoteActivityList;

const ONE_ACTIVITY: &str = r#"{
    "id": "1",
    "name": "Morning Run",
    "sport_type": "run",
    "start_date": "2026-09-14T06:00:00Z",
    "duration_seconds": 1800,
    "provider": "strava-scraper",
    "distance_meters": 5300.0
}"#;

#[test]
fn head_complete_false_is_read_as_such() {
    let body = format!(r#"{{"count": 1, "activities": [{ONE_ACTIVITY}], "head_complete": false}}"#);
    let list: RemoteActivityList = serde_json::from_str(&body).expect("decodes");
    assert_eq!(list.activities.len(), 1);
    assert!(!list.head_complete);
}

#[test]
fn head_complete_absent_decodes_as_complete() {
    let body = format!(r#"{{"count": 1, "activities": [{ONE_ACTIVITY}]}}"#);
    let list: RemoteActivityList = serde_json::from_str(&body).expect("decodes");
    assert_eq!(list.activities.len(), 1);
    assert!(
        list.head_complete,
        "a service predating the field is trusted to have read the head"
    );
}
