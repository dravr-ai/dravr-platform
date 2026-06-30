// ABOUTME: Unit coverage for the get_activities window-coverage framing helpers
// ABOUTME: activity_coverage_note (truncation sidecar) + activity_date_span (window bounds)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests the LLM-facing coverage framing that stops the coach from anchoring on
//! the oldest activity in a truncated slice ("depuis le 21 août") instead of the
//! true window total + span ("552 en 2024, voici les 200 récentes").

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::implementations::data::{activity_coverage_note, activity_date_span};

fn act_on(id: &str, year: i32, month: u32, day: u32) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        id.to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0)
            .single()
            .unwrap(),
        3600,
        "strava".to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

#[test]
fn coverage_note_present_when_window_exceeds_returned() {
    let span = ("2024-01-15".to_owned(), "2024-12-28".to_owned());
    let note = activity_coverage_note(Some(552), 200, Some(&span))
        .expect("a truncated window must emit a coverage note");

    assert_eq!(note["window_total"], 552);
    assert_eq!(note["returned"], 200);
    assert_eq!(note["window_oldest"], "2024-01-15");
    assert_eq!(note["window_newest"], "2024-12-28");
    // The note must steer the model toward the full count, not the shown slice.
    let text = note["note"].as_str().unwrap();
    assert!(text.contains("552"), "note cites the full total: {text}");
    assert!(text.contains("200"), "note cites the shown count: {text}");
}

#[test]
fn coverage_note_absent_when_whole_window_returned() {
    let span = ("2024-01-15".to_owned(), "2024-12-28".to_owned());
    // returned == total → nothing was hidden, so no framing.
    assert!(activity_coverage_note(Some(200), 200, Some(&span)).is_none());
    // returned > total can't happen, but treat it as "nothing hidden" too.
    assert!(activity_coverage_note(Some(150), 200, Some(&span)).is_none());
}

#[test]
fn coverage_note_absent_when_window_total_unknown() {
    let span = ("2024-01-15".to_owned(), "2024-12-28".to_owned());
    // The cached-snapshot path passes None — no coverage sidecar.
    assert!(activity_coverage_note(None, 200, Some(&span)).is_none());
}

#[test]
fn coverage_note_is_count_only_without_a_span() {
    let note = activity_coverage_note(Some(552), 200, None)
        .expect("a truncated window still emits a count-only note");
    assert_eq!(note["window_total"], 552);
    assert_eq!(note["returned"], 200);
    assert!(
        note.get("window_oldest").is_none(),
        "no span fields without dates"
    );
    assert!(note["note"].as_str().unwrap().contains("552"));
}

#[test]
fn date_span_spans_oldest_to_newest_regardless_of_order() {
    // Deliberately out of order: newest, oldest, middle.
    let activities = vec![
        act_on("a", 2024, 12, 28),
        act_on("b", 2024, 1, 15),
        act_on("c", 2024, 6, 1),
    ];
    let (oldest, newest) = activity_date_span(&activities).expect("non-empty span");
    assert_eq!(oldest, "2024-01-15");
    assert_eq!(newest, "2024-12-28");
}

#[test]
fn date_span_is_none_for_empty_slice() {
    assert!(activity_date_span(&[]).is_none());
}
