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
use chrono_tz::America::Toronto;
use pierre_chat_pipeline::stages::prompt_assembly::format_current_date;

/// Mirrors `NOW_QUANTUM_SECS` in `prompt_assembly`. Duplicated deliberately:
/// a test that reads the constant it is checking cannot catch it changing.
const QUANTUM: i64 = 300;

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
    let rendered = format_current_date(Some("America/Toronto"), "fr");
    let after = Utc::now().timestamp();

    assert!(rendered.contains("America/Toronto"), "got: {rendered}");
    assert!(
        rendered.contains(&Utc::now().format("%Y").to_string()),
        "anchor must show the current year; got: {rendered}"
    );

    // `now` is floored to the current five-minute quantum rather than carrying
    // the exact call instant — see `the_clock_is_quantized_so_the_prefix_holds`.
    // The 2026-07-24 fix this file pins is unaffected: what mattered was that
    // the model COPIES a literal epoch instead of converting a human date, and
    // the failure was a year early, not five minutes.
    let now = epoch_for(&rendered, "now =");
    assert!(
        now <= after && now > before - QUANTUM,
        "`now` must be the current quantum: {now} not within {QUANTUM}s below \
         [{before}, {after}]; got: {rendered}"
    );
}

#[test]
fn anchor_carries_the_common_window_boundaries_ordered() {
    let rendered = format_current_date(Some("America/Toronto"), "fr");
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
    let rendered = format_current_date(None, "en");
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

/// The prompt clock must not move the cache prefix on every request.
///
/// This block is interpolated as `{{CURRENT_DATE}}` into the platform contract,
/// which LEADS the system prompt. Prompt caching is a prefix match: one changed
/// byte invalidates everything after it, and the render order is tools, then
/// system, then messages. Carrying the raw `Utc::now().timestamp()` here moved
/// the front of the prefix on every single request, so no provider could reuse
/// any of the prompt behind it — including providers that cache implicitly on a
/// stable prefix, with no `cache_control` needed.
///
/// That is a cause of poor cache reuse that is entirely ours, and it is fixed
/// here. The flat zero cache reads recorded across August 2026 were a
/// measurement artifact rather than a finding — embacle parsed past the counts
/// the providers were already sending until 0.22.0 — so they neither confirm
/// nor refute this block's effect.
///
/// Flooring, not rounding up: rounding up crosses midnight and would move
/// "today" a day early at 23:58. The cost is an upper bound up to one quantum
/// stale, which is immaterial because the block itself tells the model to pass
/// no bounds for a freshness fetch — `now` is only ever a `before` for a window
/// question.
#[test]
fn the_clock_is_quantized_so_the_prefix_holds() {
    let rendered = format_current_date(Some("America/Toronto"), "fr");

    let now = epoch_for(&rendered, "now =");
    assert_eq!(
        now % QUANTUM,
        0,
        "`now` must be floored to a {QUANTUM}s boundary or the cache prefix \
         changes every request: {now}; got: {rendered}"
    );

    // The human-readable clock on the first line moves too, so it is quantized
    // with the same instant — a minute-resolution timestamp would still change
    // the prefix five times per quantum.
    // Found by shape, not by position. The first line gained a weekday between
    // the date and the time (see `the_anchor_names_the_weekday`), and a fixed
    // index silently read that word instead of the clock.
    let first = rendered.lines().next().unwrap_or_default();
    let minutes: i64 = first
        .split_whitespace()
        .find(|tok| tok.contains(':'))
        .and_then(|hm| hm.split(':').nth(1))
        .and_then(|m| m.parse().ok())
        .unwrap_or_else(|| panic!("no HH:MM on the first line: {first:?}"));
    assert_eq!(
        minutes % (QUANTUM / 60),
        0,
        "the displayed clock must share the quantized instant: {first:?}"
    );

    // The date boundaries are NOT quantized — they come from the true local
    // time, so "today" never shifts under the athlete.
    let today0 = epoch_for(&rendered, "start of today");
    assert_eq!(today0 % 60, 0, "midnight is a whole minute: {today0}");
    assert!(
        today0 <= now,
        "today's start must not be after now: {today0} > {now}"
    );
}

/// The anchor names the weekday, in the athlete's locale.
///
/// Production Telegram, 2026-09-02: the anchor rendered
/// `2026-09-02 06:41 (America/Toronto)` — a bare date and a zone *name*, never
/// a weekday. `format_current_date` computed `local.weekday()` internally for
/// the Monday boundary and threw it away. So the model derived every weekday
/// itself, for the anchor and for every activity row, and got them wrong across
/// fifteen turns until the athlete gave up.
///
/// Same argument as the epoch table this file already pins: the server knows
/// the answer, so the server says it.
#[test]
fn the_anchor_names_the_weekday() {
    use chrono::Datelike;

    let rendered = format_current_date(Some("America/Toronto"), "fr");
    let expected = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"][Utc::now()
        .with_timezone(&Toronto)
        .weekday()
        .num_days_from_monday()
        as usize];

    assert!(
        rendered.contains(expected),
        "the anchor must name today's weekday ({expected}) rather than leaving \
         the model to derive it; got: {rendered}"
    );
}

/// The weekday follows the chat locale, like every other word the coach reads.
#[test]
fn the_anchor_weekday_is_localized() {
    use chrono::Datelike;

    let idx = Utc::now()
        .with_timezone(&Toronto)
        .weekday()
        .num_days_from_monday() as usize;

    for (locale, table) in [
        ("fr", ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"]),
        ("en", ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]),
        ("de", ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]),
    ] {
        let rendered = format_current_date(Some("America/Toronto"), locale);
        assert!(
            rendered.contains(table[idx]),
            "locale {locale} must render today as {}; got: {rendered}",
            table[idx]
        );
    }
}
