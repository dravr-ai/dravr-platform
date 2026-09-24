// ABOUTME: Tests for GuidedWindow
// ABOUTME: Window open, supersede, re-run and completion per guided flow; malformed input degrades

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::guided_window::GuidedWindow;
use pierre_core::models::GuidedFlow;
use serde_json::json;

const START: &str = "2026-07-28T12:00:00+00:00";
const END: &str = "2026-07-28T12:20:00+00:00";

#[test]
fn a_first_run_has_no_window_to_supersede() {
    assert!(GuidedWindow::last(None, GuidedFlow::Season).is_none());
    assert!(GuidedWindow::last(Some(&json!({})), GuidedFlow::Season).is_none());
    assert!(GuidedWindow::last(
        Some(&json!({ "nutrition": { "carbs": 60 } })),
        GuidedFlow::Calibration
    )
    .is_none());
}

/// The window as `(start, end)` RFC3339 strings, for assertions.
fn stamps(profile: &serde_json::Value, flow: GuidedFlow) -> Option<(String, Option<String>)> {
    GuidedWindow::last(Some(profile), flow).map(|w| {
        (
            w.started_at.to_rfc3339(),
            w.completed_at.map(|d| d.to_rfc3339()),
        )
    })
}

#[test]
fn a_recorded_window_round_trips_and_completion_closes_it() {
    let started = GuidedWindow::record_start(None, GuidedFlow::Season, START);
    assert_eq!(
        stamps(&started, GuidedFlow::Season),
        Some((START.to_owned(), None)),
        "a running walk has an open end"
    );

    let done = GuidedWindow::record_completion(Some(started), GuidedFlow::Season, END);
    assert_eq!(
        stamps(&done, GuidedFlow::Season),
        Some((START.to_owned(), Some(END.to_owned()))),
        "completion keeps the start and closes the end"
    );
}

#[test]
fn the_flows_keep_separate_windows_and_the_rest_of_the_profile() {
    // `upsert_profile` replaces the whole document: nutrition and
    // equipment must survive, and one flow's stamp must not touch the
    // other's.
    let existing = json!({
        "nutrition": { "carbs_per_hour": 60 },
        "calibration": { "last_started_at": START, "last_completed_at": END },
    });
    let merged = GuidedWindow::record_start(Some(existing), GuidedFlow::Season, END);
    assert_eq!(merged["nutrition"]["carbs_per_hour"], 60);
    assert_eq!(
        stamps(&merged, GuidedFlow::Calibration),
        Some((START.to_owned(), Some(END.to_owned()))),
        "the other flow's window is untouched"
    );
    assert_eq!(
        stamps(&merged, GuidedFlow::Season),
        Some((END.to_owned(), None))
    );
}

#[test]
fn a_re_run_reopens_the_window() {
    let first = GuidedWindow::record_start(None, GuidedFlow::Calibration, START);
    let done = GuidedWindow::record_completion(Some(first), GuidedFlow::Calibration, END);
    let second = GuidedWindow::record_start(Some(done), GuidedFlow::Calibration, END);
    assert_eq!(
        stamps(&second, GuidedFlow::Calibration),
        Some((END.to_owned(), None)),
        "the next re-run supersedes the latest run, and a new run is open until it completes"
    );
}

#[test]
fn a_non_object_profile_and_a_malformed_stamp_degrade_safely() {
    let merged = GuidedWindow::record_start(Some(json!(["unexpected"])), GuidedFlow::Season, START);
    assert!(GuidedWindow::last(Some(&merged), GuidedFlow::Season).is_some());
    let bad = json!({ "season": { "last_started_at": "not a timestamp" } });
    assert!(GuidedWindow::last(Some(&bad), GuidedFlow::Season).is_none());
    let half = json!({ "season": { "last_started_at": START, "last_completed_at": "nope" } });
    assert_eq!(
        stamps(&half, GuidedFlow::Season),
        Some((START.to_owned(), None)),
        "a malformed end reads as open, the safe direction"
    );
}

#[test]
fn only_the_fixed_list_flows_keep_a_window() {
    assert_eq!(GuidedFlow::Pillars.profile_key(), None);
    assert_eq!(GuidedFlow::Intake.profile_key(), None);
    assert_eq!(GuidedFlow::Calibration.profile_key(), Some("calibration"));
    assert_eq!(GuidedFlow::Season.profile_key(), Some("season"));
}
