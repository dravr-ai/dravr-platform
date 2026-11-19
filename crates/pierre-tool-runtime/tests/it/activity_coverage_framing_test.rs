// ABOUTME: Unit coverage for the get_activities window-coverage framing helpers
// ABOUTME: Coverage and its note, window bounds and totals, and how much of a bounded window is fetched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests the LLM-facing coverage framing that stops the agent from anchoring on
//! the oldest activity in a truncated slice ("depuis le 21 août") instead of the
//! true window total + span ("552 en 2024, voici les 200 récentes").

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::NaiveDate;
use pierre_core::models::coverage::{Completeness, IncompleteReason, WindowTotals};
use pierre_tool_runtime::implementations::data_helpers::{
    activity_coverage, historical_backfill_fetch_limit, pagination_has_more, window_fetch_limit,
    AskedWindow, WindowCount, HISTORICAL_WINDOW_READ_LIMIT,
};
use serde_json::Value;

fn day(year: i32, month: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, d).unwrap()
}

/// The `coverage` sidecar as the model reads it, totals left unsummed, for
/// a list that read the whole window it counts.
fn coverage_json(
    window_total: Option<usize>,
    returned: usize,
    span: Option<(NaiveDate, NaiveDate)>,
    asked: AskedWindow,
    completeness: Completeness,
) -> Option<Value> {
    let count = WindowCount {
        total: window_total,
        fetched: window_total.unwrap_or(returned),
        span,
        ..WindowCount::default()
    };
    activity_coverage(count, returned, asked, completeness)
        .map(|coverage| serde_json::to_value(coverage).unwrap())
}

#[test]
fn coverage_present_when_window_exceeds_returned() {
    let span = (day(2024, 1, 15), day(2024, 12, 28));
    let coverage = coverage_json(
        Some(552),
        200,
        Some(span),
        AskedWindow::default(),
        Completeness::Complete,
    )
    .expect("a truncated window must emit its Coverage");

    assert_eq!(coverage["window"]["total"], 552, "bug: {coverage}");
    assert_eq!(coverage["window"]["returned"], 200, "bug: {coverage}");
    assert_eq!(
        coverage["window"]["oldest"], "2024-01-15",
        "bug: {coverage}"
    );
    assert_eq!(
        coverage["window"]["newest"], "2024-12-28",
        "bug: {coverage}"
    );
    // The note must steer the model toward the full count, not the shown slice.
    let text = coverage["note"].as_str().unwrap();
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
fn coverage_absent_when_whole_window_returned() {
    let span = (day(2024, 1, 15), day(2024, 12, 28));
    // returned == total → nothing was hidden, so no framing.
    assert!(
        coverage_json(
            Some(200),
            200,
            Some(span),
            AskedWindow::default(),
            Completeness::Complete
        )
        .is_none(),
        "bug: a whole, complete, undated window needs no coverage"
    );
    // returned > total can't happen, but treat it as "nothing hidden" too.
    assert!(
        coverage_json(
            Some(150),
            200,
            Some(span),
            AskedWindow::default(),
            Completeness::Complete
        )
        .is_none(),
        "bug: more returned than the total hides nothing"
    );
}

#[test]
fn coverage_absent_when_window_total_unknown() {
    let span = (day(2024, 1, 15), day(2024, 12, 28));
    // The cached-snapshot path passes None — no coverage sidecar.
    assert!(
        coverage_json(
            None,
            200,
            Some(span),
            AskedWindow::default(),
            Completeness::Complete
        )
        .is_none(),
        "bug: a complete window with no known total carries no coverage"
    );
}

#[test]
fn coverage_is_count_only_without_a_span() {
    let coverage = coverage_json(
        Some(552),
        200,
        None,
        AskedWindow::default(),
        Completeness::Complete,
    )
    .expect("a truncated window still emits a count-only Coverage");
    assert_eq!(coverage["window"]["total"], 552, "bug: {coverage}");
    assert_eq!(coverage["window"]["returned"], 200, "bug: {coverage}");
    assert!(
        coverage["window"].get("oldest").is_none(),
        "no span fields without dates"
    );
    let text = coverage["note"].as_str().unwrap();
    assert!(text.contains("552"));
    // Sort-order-agnostic: the count-only note must not claim "most recent" either.
    assert!(
        !text.contains("most recent"),
        "count-only note stays accurate under any sort order: {text}"
    );
}

#[test]
fn coverage_frames_count_as_lower_bound_when_read_is_capped() {
    // A window that fills HISTORICAL_WINDOW_READ_LIMIT was truncated by the
    // durable read cap, so window_total is a LOWER BOUND, not the true total.
    let span = (day(2019, 1, 1), day(2024, 12, 28));
    let coverage = coverage_json(
        Some(HISTORICAL_WINDOW_READ_LIMIT),
        200,
        Some(span),
        AskedWindow::default(),
        Completeness::Complete,
    )
    .expect("a capped, truncated window must emit its Coverage");

    // The numeric key stays the exact figure read; the bound and the prose hedge.
    assert_eq!(
        coverage["window"]["total"], HISTORICAL_WINDOW_READ_LIMIT,
        "bug: {coverage}"
    );
    assert_eq!(
        coverage["window"]["total_is_lower_bound"], true,
        "bug: {coverage}"
    );
    let text = coverage["note"].as_str().unwrap();
    let lower_bound = format!("at least {HISTORICAL_WINDOW_READ_LIMIT}");
    assert!(
        text.contains(lower_bound.as_str()),
        "a capped read must be framed as a lower bound: {text}"
    );
}

#[test]
fn coverage_count_only_is_lower_bound_when_read_is_capped() {
    // Same lower-bound framing on the span-less (count-only) branch.
    let coverage = coverage_json(
        Some(HISTORICAL_WINDOW_READ_LIMIT),
        200,
        None,
        AskedWindow::default(),
        Completeness::Complete,
    )
    .expect("a capped, truncated window still emits a count-only Coverage");
    let text = coverage["note"].as_str().unwrap();
    let lower_bound = format!("at least {HISTORICAL_WINDOW_READ_LIMIT}");
    assert!(
        text.contains(lower_bound.as_str()),
        "a capped count-only read must be framed as a lower bound: {text}"
    );
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

/// The replay's Q3: a 151-row cached window served while the live read ran
/// on, for a question back to 2020. It must say it is incomplete, and why.
#[test]
fn a_window_served_while_the_live_read_runs_is_incomplete() {
    let span = (day(2026, 7, 21), day(2026, 11, 9));
    let note = coverage_json(
        Some(43),
        43,
        Some(span),
        AskedWindow {
            after: Some(1_577_836_800),
            before: None,
        },
        Completeness::Incomplete(IncompleteReason::LiveReadRunning),
    )
    .expect("bug: an incomplete window always carries coverage");
    assert_eq!(note["complete"], false, "bug: {note}");
    assert_eq!(
        note["incomplete_reason"], "live_read_running",
        "bug: {note}"
    );
    assert_eq!(note["window"]["asked_from"], "2020-01-01", "bug: {note}");
    assert!(
        note["note"]
            .as_str()
            .unwrap()
            .contains("Do not state a count"),
        "bug: the note must forbid stating counts: {note}"
    );
}

/// A complete window that answers a dated ask says so.
#[test]
fn a_complete_dated_window_says_complete() {
    let span = (day(2020, 1, 3), day(2026, 11, 9));
    let note = coverage_json(
        Some(639),
        400,
        Some(span),
        AskedWindow {
            after: Some(1_577_836_800),
            before: None,
        },
        Completeness::Complete,
    )
    .expect("bug: a truncated window carries coverage");
    assert_eq!(note["complete"], true, "bug: {note}");
    assert!(note.get("incomplete_reason").is_none(), "bug: {note}");
}

/// The whole window's totals ride in the window, and the note sends the
/// model to them for a comparison (carnet#874).
#[test]
fn window_totals_ride_in_the_window_and_the_note_names_them() {
    let count = WindowCount {
        total: Some(2),
        fetched: 2,
        span: Some((day(2011, 3, 1), day(2011, 9, 1))),
        totals: Some(WindowTotals {
            count: 2,
            distance_km: 20.0,
            duration_seconds: 7_200,
            elevation_gain_m: 0.0,
        }),
        unstored: 0,
    };
    let coverage = activity_coverage(count, 1, AskedWindow::default(), Completeness::Complete)
        .expect("a truncated window carries coverage");
    let note = serde_json::to_value(coverage).unwrap();
    assert_eq!(note["window"]["totals"]["count"], 2, "bug: {note}");
    assert_eq!(note["window"]["totals"]["distance_km"], 20.0, "bug: {note}");
    assert!(
        note["note"].as_str().unwrap().contains("use window.totals"),
        "bug: the note must send a comparison to the totals: {note}"
    );
}

/// A shown row the store does not hold is left out of the store's count,
/// and the coverage names it even where nothing else would carry one: a
/// complete, undated window whose count is no larger than what is shown.
#[test]
fn a_shown_row_the_store_lacks_is_named_and_left_out_of_the_count() {
    let count = WindowCount {
        total: Some(0),
        fetched: 1,
        unstored: 1,
        ..WindowCount::default()
    };
    let coverage = activity_coverage(count, 1, AskedWindow::default(), Completeness::Complete)
        .map(|coverage| serde_json::to_value(coverage).unwrap())
        .expect("bug: a shown row the store lacks carries no coverage");
    assert_eq!(coverage["window"]["unstored"], 1, "bug: {coverage}");
    assert_eq!(coverage["window"]["total"], 0, "bug: {coverage}");
    let note = coverage["note"].as_str().unwrap();
    assert!(
        note.contains("1 activity shown is not in the store yet"),
        "bug: the note does not say the shown row is left out: {note}"
    );
    assert_eq!(
        coverage["complete"], false,
        "bug: a count that leaves out a shown row reads complete: {coverage}"
    );
    assert_eq!(
        coverage["incomplete_reason"], "unstored_rows",
        "bug: the incomplete reason does not name the unstored rows: {coverage}"
    );
    assert!(
        note.contains("This window is INCOMPLETE."),
        "bug: the note does not say the window is incomplete: {note}"
    );
}

/// A window already short for another reason keeps that reason when a shown
/// row is unstored too: a running live read stays the reason, and the note
/// still names the unstored row.
#[test]
fn an_unstored_row_keeps_a_window_already_short_for_another_reason() {
    let count = WindowCount {
        total: Some(0),
        fetched: 1,
        unstored: 1,
        ..WindowCount::default()
    };
    let coverage = activity_coverage(
        count,
        1,
        AskedWindow::default(),
        Completeness::Incomplete(IncompleteReason::LiveReadRunning),
    )
    .map(|coverage| serde_json::to_value(coverage).unwrap())
    .expect("bug: an incomplete window carries no coverage");
    assert_eq!(
        coverage["incomplete_reason"], "live_read_running",
        "bug: an unstored row replaced the window's own reason: {coverage}"
    );
    assert!(
        coverage["note"]
            .as_str()
            .unwrap()
            .contains("1 activity shown is not in the store yet"),
        "bug: the note does not name the unstored row: {coverage}"
    );
}

/// The store's count is not capped: a window it counts past the read cap is
/// an exact count when the list itself read less than the cap. Only a read
/// that filled the cap is framed as a lower bound.
#[test]
fn a_store_count_past_the_read_cap_is_exact_when_the_read_was_not_capped() {
    let count = WindowCount {
        total: Some(HISTORICAL_WINDOW_READ_LIMIT + 100),
        fetched: 200,
        ..WindowCount::default()
    };
    let coverage = activity_coverage(count, 200, AskedWindow::default(), Completeness::Complete)
        .map(|coverage| serde_json::to_value(coverage).unwrap())
        .expect("bug: a truncated window carries no coverage");
    assert!(
        coverage["window"].get("total_is_lower_bound").is_none(),
        "bug: an uncapped read's store count is framed as a lower bound: {coverage}"
    );
    let text = coverage["note"].as_str().unwrap();
    assert!(
        !text.contains("at least"),
        "bug: an exact store count is hedged as a lower bound: {text}"
    );
}
