// ABOUTME: Unit coverage for the get_activities window-coverage framing helpers
// ABOUTME: coverage note, window bounds and totals, and how much of a bounded window is fetched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests the LLM-facing coverage framing that stops the agent from anchoring on
//! the oldest activity in a truncated slice ("depuis le 21 août") instead of the
//! true window total + span ("552 en 2024, voici les 200 récentes").

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::activity_fetch::{activity_date_span, activity_window_totals};
use pierre_tool_runtime::implementations::data_helpers::{
    activity_coverage_note, historical_backfill_fetch_limit, pagination_has_more,
    window_fetch_limit, HISTORICAL_WINDOW_READ_LIMIT,
};

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
    assert!(
        text.contains("full count"),
        "note anchors on the full count: {text}"
    );
    // The shown slice follows the caller's sort_by (distance_desc, date_asc, …),
    // so it is NOT necessarily the most recent — the note must not claim it is.
    assert!(
        !text.contains("most recent"),
        "note must stay accurate under any sort order: {text}"
    );
    // Below the read cap the count is exact, so no lower-bound hedging.
    assert!(
        !text.contains("at least"),
        "an uncapped window states an exact count: {text}"
    );
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
    let text = note["note"].as_str().unwrap();
    assert!(text.contains("552"));
    // Sort-order-agnostic: the count-only note must not claim "most recent" either.
    assert!(
        !text.contains("most recent"),
        "count-only note stays accurate under any sort order: {text}"
    );
}

#[test]
fn coverage_note_frames_count_as_lower_bound_when_read_is_capped() {
    // A window that fills HISTORICAL_WINDOW_READ_LIMIT was truncated by the
    // durable read cap, so window_total is a LOWER BOUND, not the true total.
    let span = ("2019-01-01".to_owned(), "2024-12-28".to_owned());
    let note = activity_coverage_note(Some(HISTORICAL_WINDOW_READ_LIMIT), 200, Some(&span))
        .expect("a capped, truncated window must emit a coverage note");

    // The numeric key stays the exact figure read; only the prose hedges.
    assert_eq!(note["window_total"], HISTORICAL_WINDOW_READ_LIMIT);
    let text = note["note"].as_str().unwrap();
    let lower_bound = format!("at least {HISTORICAL_WINDOW_READ_LIMIT}");
    assert!(
        text.contains(lower_bound.as_str()),
        "a capped read must be framed as a lower bound: {text}"
    );
}

#[test]
fn coverage_note_count_only_is_lower_bound_when_read_is_capped() {
    // Same lower-bound framing on the span-less (count-only) branch.
    let note = activity_coverage_note(Some(HISTORICAL_WINDOW_READ_LIMIT), 200, None)
        .expect("a capped, truncated window still emits a count-only note");
    let text = note["note"].as_str().unwrap();
    let lower_bound = format!("at least {HISTORICAL_WINDOW_READ_LIMIT}");
    assert!(
        text.contains(lower_bound.as_str()),
        "a capped count-only read must be framed as a lower bound: {text}"
    );
}

#[test]
fn date_span_spans_oldest_to_newest_regardless_of_order() {
    // Deliberately out of order: newest, oldest, middle.
    let activities = vec![
        act_on("a", 2024, 12, 28),
        act_on("b", 2024, 1, 15),
        act_on("c", 2024, 6, 1),
    ];
    let (oldest, newest) = activity_date_span(&activities, None).expect("non-empty span");
    assert_eq!(oldest, "2024-01-15");
    assert_eq!(newest, "2024-12-28");
}

#[test]
fn date_span_is_none_for_empty_slice() {
    assert!(activity_date_span(&[], None).is_none());
}

// carnet#874: "compare my 2011 cycling with this year" read 2011 as its last
// three December rides. The provider was asked for the display limit (20) of
// every sport, newest first; the ride filter left three, and `has_more` said
// there was nothing else.

/// 2011-01-01T00:00:00Z and 2012-01-01T00:00:00Z.
const YEAR_2011: (i64, i64) = (1_293_840_000, 1_325_376_000);

#[test]
fn a_window_bounded_on_both_ends_is_fetched_whole() {
    let (after, before) = YEAR_2011;
    let fetched = window_fetch_limit(20, Some(after), Some(before), None);
    assert_eq!(fetched, historical_backfill_fetch_limit());
    assert!(fetched > 20, "the whole year, not the display limit");
    assert_eq!(
        window_fetch_limit(20, Some(after), Some(before), Some(0)),
        fetched
    );
}

#[test]
fn an_open_or_paged_window_keeps_the_display_limit() {
    let (after, before) = YEAR_2011;
    assert_eq!(window_fetch_limit(20, None, None, None), 20);
    assert_eq!(window_fetch_limit(20, Some(after), None, None), 20);
    assert_eq!(window_fetch_limit(20, None, Some(before), None), 20);
    assert_eq!(
        window_fetch_limit(20, Some(after), Some(before), Some(40)),
        20
    );
}

#[test]
fn more_is_reported_when_the_window_holds_more_than_is_shown() {
    // 63 rides in the window, 20 shown.
    assert!(pagination_has_more(20, 63, false));
    // Three rides left of a full 20-activity page: the page may continue.
    assert!(pagination_has_more(3, 3, true));
    // The whole window shown, and the provider stopped short of its page.
    assert!(!pagination_has_more(3, 3, false));
}

#[test]
fn window_totals_sum_the_whole_window() {
    // Twelve rides with distance and climb, plus one that reports neither:
    // it counts and adds its hour, and adds no distance or climb.
    let mut window: Vec<Activity> = (1..=12)
        .map(|month| {
            ActivityBuilder::new(
                format!("ride-{month}"),
                format!("ride-{month}"),
                SportType::Ride,
                Utc.with_ymd_and_hms(2011, month, 15, 12, 0, 0)
                    .single()
                    .unwrap(),
                3600,
                "strava".to_owned(),
            )
            .distance_meters(10_000.0)
            .elevation_gain(125.0)
            .build()
        })
        .collect();
    window.push(
        ActivityBuilder::new(
            "trainer".to_owned(),
            "trainer".to_owned(),
            SportType::Ride,
            Utc.with_ymd_and_hms(2011, 12, 20, 12, 0, 0)
                .single()
                .unwrap(),
            3600,
            "strava".to_owned(),
        )
        .build(),
    );
    let totals = activity_window_totals(&window);
    assert_eq!(totals.count, 13);
    assert!((totals.distance_km - 120.0).abs() < f64::EPSILON);
    assert!((totals.duration_hours - 13.0).abs() < f64::EPSILON);
    assert!((totals.elevation_gain_m - 1500.0).abs() < f64::EPSILON);
    assert_eq!(
        totals.to_json(),
        serde_json::json!({
            "count": 13,
            "distance_km": 120.0,
            "duration_hours": 13.0,
            "elevation_gain_m": 1500.0,
        })
    );
}
