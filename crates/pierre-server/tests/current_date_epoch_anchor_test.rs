// ABOUTME: Pins that the {{CURRENT_DATE}} anchor carries the literal current Unix epoch
// ABOUTME: Regression for 2026-07-24: a coach miscomputed before=<unix-now> a year early
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The `{{CURRENT_DATE}}` prompt anchor must carry the current Unix timestamp
//! as a literal integer.
//!
//! 2026-07-24 live incident: the coach had the human-readable date
//! (`2026-07-24`) in its prompt but converted it to `before=1753362000`
//! (2025-07-24, a year early) — human-date→epoch is exactly the arithmetic
//! LLMs botch — so the scrape returned year-old activities. The fix carries
//! the exact epoch so the model copies it instead of computing one.

use chrono::Utc;
use pierre_chat_pipeline::stages::prompt_assembly::format_current_date;

/// Parse the epoch to the right of a `= ` on the line containing `label`.
fn epoch_for(rendered: &str, label: &str) -> i64 {
    let line = rendered
        .lines()
        .find(|l| l.contains(label))
        .unwrap_or_else(|| panic!("no line for {label:?} in:\n{rendered}"));
    line.rsplit('=')
        .next()
        .and_then(|tok| tok.trim().parse::<i64>().ok())
        .unwrap_or_else(|| panic!("no epoch on {label:?} line: {line:?}"))
}

#[test]
fn anchor_carries_a_current_unix_epoch() {
    let before = Utc::now().timestamp();
    let rendered = format_current_date(Some("America/Toronto"));
    let after = Utc::now().timestamp();

    assert!(rendered.contains("America/Toronto"), "got: {rendered}");
    assert!(
        rendered.contains(&Utc::now().format("%Y").to_string()),
        "anchor must show the current year; got: {rendered}"
    );

    let now = epoch_for(&rendered, "now =");
    assert!(
        (before..=after + 1).contains(&now),
        "`now` epoch must be current: {now} not in [{before}, {after}]; got: {rendered}"
    );
}

#[test]
fn anchor_carries_the_common_window_boundaries_ordered() {
    let rendered = format_current_date(Some("America/Toronto"));
    let now = epoch_for(&rendered, "now =");
    let today0 = epoch_for(&rendered, "start of today");
    let yesterday0 = epoch_for(&rendered, "start of yesterday");
    let week0 = epoch_for(&rendered, "start of this week");
    let month0 = epoch_for(&rendered, "start of this month");

    // Ordering the coach relies on: week/month start ≤ today start ≤ now, and
    // yesterday is exactly one day before today's local midnight.
    assert!(today0 <= now, "today0 {today0} must be ≤ now {now}");
    assert!(week0 <= today0, "week0 {week0} must be ≤ today0 {today0}");
    assert!(
        month0 <= today0,
        "month0 {month0} must be ≤ today0 {today0}"
    );
    assert_eq!(
        yesterday0,
        today0 - 86_400,
        "yesterday must be today − 24h (barring a rare midnight DST shift)"
    );
    // The boundaries are all within the current year, not a year off — the
    // exact failure mode this anchor prevents.
    let year_ago = now - 366 * 86_400;
    for (name, e) in [
        ("today", today0),
        ("yesterday", yesterday0),
        ("week", week0),
        ("month", month0),
    ] {
        assert!(e > year_ago, "{name} boundary {e} is more than a year old");
    }
}

#[test]
fn anchor_instructs_to_copy_not_compute_epochs() {
    let rendered = format_current_date(None);
    assert!(
        rendered.to_lowercase().contains("unix epochs"),
        "anchor must label the epoch table; got: {rendered}"
    );
    assert!(
        rendered.contains("never convert a date to an epoch"),
        "anchor must tell the model not to convert dates to epochs; got: {rendered}"
    );
    // No-timezone fallback still labels UTC.
    assert!(rendered.contains("UTC"), "got: {rendered}");
}
