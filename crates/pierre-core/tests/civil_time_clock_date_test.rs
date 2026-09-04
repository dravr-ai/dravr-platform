// ABOUTME: Pins that today-on-the-athlete's-clock converts where a provider date-only row must not
// ABOUTME: Regression for registre#260 — a UTC window end dropped the current civil day
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Two functions read a UTC instant into a civil date and they disagree on
//! purpose, at exactly one instant per day.
//!
//! [`local_date`] refuses to convert midnight UTC, because a date-only provider
//! row — a Strava-mirror scrape — uses that as its sentinel for "a day, not a
//! moment", and converting it into a negative-offset zone moves the workout to
//! the previous day (registre#258).
//!
//! [`clock_date`] always converts, because a reading of the wall clock is never
//! a sentinel. Using the wrong one to bound a rollup window costs the athlete a
//! whole day of their own training: the rollup buckets each activity on their
//! civil date, so for any zone ahead of UTC their current day sits past the end
//! of a UTC-bounded window and is dropped (registre#260).

use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use pierre_core::civil_time::{clock_date, local_date};

/// 09:00 in Sydney on the 4th is 23:00 UTC on the 3rd. The athlete's "today" is
/// a day ahead of the server's, and a window ending on the server's excludes
/// the session they have already finished.
#[test]
fn a_zone_ahead_of_utc_is_already_on_the_next_civil_day() {
    let instant = Utc.with_ymd_and_hms(2026, 9, 3, 23, 0, 0).unwrap();
    let sydney: Tz = "Australia/Sydney".parse().unwrap();

    assert_eq!(
        clock_date(instant, sydney).to_string(),
        "2026-09-04",
        "23:00 UTC is 09:00 the next morning in Sydney"
    );
    assert_eq!(
        instant.date_naive().to_string(),
        "2026-09-03",
        "and the server is still on the previous day — the gap this closes"
    );
}

/// The one instant a day where the two functions must differ.
///
/// A date-only row keeps the day its provider named; the clock does not, and a
/// rollup window built from `local_date` would rewind a day for every athlete
/// ahead of UTC exactly then.
#[test]
fn at_midnight_utc_the_clock_converts_and_a_provider_row_does_not() {
    let midnight = Utc.with_ymd_and_hms(2026, 9, 4, 0, 0, 0).unwrap();
    let sydney: Tz = "Australia/Sydney".parse().unwrap();

    assert_eq!(
        local_date(midnight, sydney).to_string(),
        "2026-09-04",
        "a date-only provider row keeps the day the provider named"
    );
    assert_eq!(
        clock_date(midnight, sydney).to_string(),
        "2026-09-04",
        "and the clock reads 10:00 on the 4th in Sydney — the same day here"
    );

    // The direction where they part company: a zone BEHIND UTC.
    let toronto: Tz = "America/Toronto".parse().unwrap();
    assert_eq!(
        local_date(midnight, toronto).to_string(),
        "2026-09-04",
        "the sentinel is not converted, so the provider's day survives"
    );
    assert_eq!(
        clock_date(midnight, toronto).to_string(),
        "2026-09-03",
        "but the clock really does read 20:00 the previous evening in Toronto"
    );
}

/// Away from the sentinel the two agree, which is why the split is easy to miss.
#[test]
fn away_from_the_sentinel_the_two_agree() {
    let evening = Utc.with_ymd_and_hms(2026, 9, 2, 1, 30, 0).unwrap();
    let toronto: Tz = "America/Toronto".parse().unwrap();

    assert_eq!(local_date(evening, toronto), clock_date(evening, toronto));
    assert_eq!(clock_date(evening, toronto).to_string(), "2026-09-01");
}
