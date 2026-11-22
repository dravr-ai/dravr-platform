// ABOUTME: Pins date arithmetic answers: exact days, truncating differences, month ends, zones, ISO weeks
// ABOUTME: Recorded from the date engine in use on 2026-11-17 so a later engine gives the same answers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Arithmetic goldens for dates (carnet#1620, gate 8).
//!
//! A date library can change under code that still compiles and still passes
//! tests written for the happy path. These are the answers that differ between
//! libraries, each recorded by running it:
//!
//! - a "day" added to an instant is exactly 86,400 seconds, including across a
//!   daylight saving change, where the wall clock therefore moves by an hour;
//! - the difference of two instants in whole days truncates toward zero;
//! - an instant becomes a calendar date in UTC, or in the athlete's zone, and
//!   the two disagree for hours every day;
//! - a wall-clock time can be missing (spring) or happen twice (autumn);
//! - a month added to the 31st clamps to the last day of the target month;
//! - the ISO week of a date near 1 January can belong to the other year.
//!
//! Zone cases use transitions that already happened, in zones the next zone
//! database release leaves alone, so a database update cannot move them. The
//! nine zones that release does change have their own table, marked expected
//! to change, in `date_zone_database_golden_test`.
//!
//! Tables hold strings and integers only. Everything that names the date engine
//! lives in [`engine`].

use pierre_core::civil_time::{clock_date, is_date_only, local_date, parse_zone, resolve_zone};

/// Everything in this file that names the date engine.
mod engine {
    use std::borrow::Borrow;

    use dravr_tronc::dates::{CivilDateTime, Elapsed, LocalResolution, Weekday};

    pub(super) use dravr_tronc::dates::{CivilDate, Timestamp as Instant};

    const STAMP: &str = "%Y-%m-%dT%H:%M:%S%.3f";

    /// A UTC instant from `2025-03-08T12:00:00.000`, always with milliseconds.
    pub(super) fn instant(text: &str) -> Instant {
        CivilDateTime::parse_from_str(text, STAMP)
            .unwrap()
            .and_utc()
            .unwrap()
    }

    /// The same shape back, so a table reads the way it was written.
    pub(super) fn show(value: Instant) -> String {
        value.format(STAMP).unwrap()
    }

    /// A calendar date as `2025-03-08`.
    pub(super) fn show_date(value: CivilDate) -> String {
        value.to_string()
    }

    /// A calendar date from `2025-03-08`.
    pub(super) fn date(text: &str) -> CivilDate {
        CivilDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    pub(super) use dravr_tronc::dates::Zone;

    /// A zone from its IANA name, held for the whole run so the tables can
    /// pass it on by copy as they passed the zone type it replaced.
    pub(super) fn zone(name: &str) -> &'static Zone {
        Box::leak(Box::new(name.parse().unwrap()))
    }

    /// Whether a zone name is known, and the name the zone answers to.
    pub(super) fn zone_verdict(name: &str) -> String {
        name.parse::<Zone>()
            .map_or_else(|_| "rejected".to_owned(), |zone| zone.name().to_owned())
    }

    /// Seconds from the epoch.
    pub(super) fn epoch_seconds(value: Instant) -> i64 {
        value.epoch_seconds()
    }

    /// Milliseconds from the epoch.
    pub(super) fn epoch_millis(value: Instant) -> i64 {
        value.epoch_millis()
    }

    /// The instant at that many seconds and nanoseconds from the epoch.
    pub(super) fn from_epoch(seconds: i64, nanos: u32) -> Option<Instant> {
        Instant::from_epoch_seconds_and_nanos(seconds, nanos).ok()
    }

    /// The instant at that many milliseconds from the epoch.
    pub(super) fn from_epoch_millis(millis: i64) -> Option<Instant> {
        Instant::from_epoch_millis(millis).ok()
    }

    /// The instant plus a whole number of days, negative to go back.
    pub(super) fn plus_days(value: Instant, days: i64) -> Instant {
        value + Elapsed::days(days)
    }

    /// The instant plus a whole number of weeks.
    pub(super) fn plus_weeks(value: Instant, weeks: i64) -> Instant {
        value + Elapsed::weeks(weeks)
    }

    /// The instant plus a whole number of hours.
    pub(super) fn plus_hours(value: Instant, hours: i64) -> Instant {
        value + Elapsed::hours(hours)
    }

    /// `later - earlier` in whole days, hours, minutes, seconds and milliseconds.
    pub(super) fn difference(later: Instant, earlier: Instant) -> [i64; 5] {
        let elapsed = later.signed_duration_since(earlier);
        [
            elapsed.num_days(),
            elapsed.num_hours(),
            elapsed.num_minutes(),
            elapsed.num_seconds(),
            elapsed.num_milliseconds(),
        ]
    }

    /// `later - earlier` in whole weeks.
    pub(super) fn difference_in_weeks(later: Instant, earlier: Instant) -> i64 {
        later.signed_duration_since(earlier).num_weeks()
    }

    /// The UTC calendar date of an instant.
    pub(super) fn utc_date(value: Instant) -> String {
        value.utc_date().to_string()
    }

    /// The wall clock and offset an instant shows in a zone.
    pub(super) fn wall_clock(value: Instant, zone: impl Borrow<Zone>) -> String {
        zone.borrow()
            .format(value, "%Y-%m-%d %H:%M:%S %:z %Z")
            .unwrap()
    }

    fn naive(text: &str) -> CivilDateTime {
        CivilDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// The instants a wall-clock time means in a zone: none, one, or two.
    pub(super) fn resolve_wall_clock(text: &str, zone: impl Borrow<Zone>) -> String {
        match zone.borrow().resolve(naive(text)) {
            LocalResolution::None => "none".to_owned(),
            LocalResolution::Single(only) => format!("one:{}", show(only)),
            LocalResolution::Ambiguous(first, second) => {
                format!("two:{}|{}", show(first), show(second))
            }
        }
    }

    /// The three ways call sites pick one instant: earliest, latest, single.
    pub(super) fn pick_wall_clock(text: &str, zone: impl Borrow<Zone>) -> [String; 3] {
        let resolved = zone.borrow().resolve(naive(text));
        let utc = |local: Option<Instant>| local.map_or_else(|| "none".to_owned(), show);
        [
            utc(resolved.earliest()),
            utc(resolved.latest()),
            utc(resolved.single()),
        ]
    }

    /// A date moved by whole months, negative to go back.
    pub(super) fn date_plus_months(value: CivilDate, months: i32) -> String {
        let step = months.unsigned_abs();
        let moved = if months >= 0 {
            value.checked_add_months(step)
        } else {
            value.checked_sub_months(step)
        };
        moved.map_or_else(|| "none".to_owned(), |date| date.to_string())
    }

    /// An instant moved by whole months, negative to go back: its UTC date
    /// moves and its time of day stays.
    pub(super) fn instant_plus_months(value: Instant, months: i32) -> String {
        let step = months.unsigned_abs();
        let civil = value.utc_datetime();
        let moved = if months >= 0 {
            civil.date().checked_add_months(step)
        } else {
            civil.date().checked_sub_months(step)
        };
        moved
            .and_then(|date| date.and_time(civil.time()).and_utc().ok())
            .map_or_else(|| "none".to_owned(), show)
    }

    /// A date moved by whole days, negative to go back.
    pub(super) fn date_plus_days(value: CivilDate, days: i64) -> String {
        let moved = i64::try_from(days.unsigned_abs()).ok().and_then(|step| {
            if days >= 0 {
                value.checked_add_days(step)
            } else {
                value.checked_sub_days(step)
            }
        });
        moved.map_or_else(|| "none".to_owned(), |date| date.to_string())
    }

    /// The same move written the way most call sites write it.
    pub(super) fn date_plus_elapsed_days(value: CivilDate, days: i64) -> String {
        (value + Elapsed::days(days)).to_string()
    }

    /// `later - earlier` in days between two calendar dates.
    pub(super) fn days_between(later: CivilDate, earlier: CivilDate) -> i64 {
        later.signed_duration_since(earlier).num_days()
    }

    /// Weekday name, days from Monday, day of the year.
    pub(super) fn day_facts(value: CivilDate) -> (String, u32, u32) {
        (
            value.weekday().to_string(),
            value.weekday().num_days_from_monday(),
            value.ordinal(),
        )
    }

    /// The Monday that starts the date's week, by both spellings in use.
    pub(super) fn week_start(value: CivilDate) -> (String, String) {
        let by_subtraction =
            value - Elapsed::days(i64::from(value.weekday().num_days_from_monday()));
        let by_week = value.start_of_week(Weekday::Mon);
        (by_subtraction.to_string(), by_week.to_string())
    }

    /// The ISO year and week number.
    pub(super) fn iso_week(value: CivilDate) -> (i32, u32) {
        value.iso_week()
    }

    /// Midnight UTC at the start of the date.
    pub(super) fn start_of_day(value: CivilDate) -> Instant {
        value.at_midnight().and_utc().unwrap()
    }
}

use engine::{date, instant, show, zone};

const DAY: i64 = 86_400;

/// Adding days to an instant never looks at a calendar: the result is exactly
/// n × 86,400 seconds away, so across a clock change the wall clock moves.
#[test]
fn a_day_added_to_an_instant_is_exactly_86400_seconds() {
    let toronto = zone("America/Toronto");
    // Noon UTC on the eve of the 2025 spring change, and of the autumn one.
    let rows = [
        (
            "2025-03-08T12:00:00.000",
            1,
            "2025-03-09T12:00:00.000",
            "2025-03-09 08:00:00 -04:00 EDT",
        ),
        (
            "2025-03-08T12:00:00.000",
            7,
            "2025-03-15T12:00:00.000",
            "2025-03-15 08:00:00 -04:00 EDT",
        ),
        (
            "2025-03-08T12:00:00.000",
            30,
            "2025-04-07T12:00:00.000",
            "2025-04-07 08:00:00 -04:00 EDT",
        ),
        (
            "2025-03-08T12:00:00.000",
            90,
            "2025-06-06T12:00:00.000",
            "2025-06-06 08:00:00 -04:00 EDT",
        ),
        (
            "2025-03-08T12:00:00.000",
            -1,
            "2025-03-07T12:00:00.000",
            "2025-03-07 07:00:00 -05:00 EST",
        ),
        (
            "2025-03-08T12:00:00.000",
            -7,
            "2025-03-01T12:00:00.000",
            "2025-03-01 07:00:00 -05:00 EST",
        ),
        (
            "2025-03-08T12:00:00.000",
            -30,
            "2025-02-06T12:00:00.000",
            "2025-02-06 07:00:00 -05:00 EST",
        ),
        (
            "2025-03-08T12:00:00.000",
            -90,
            "2024-12-08T12:00:00.000",
            "2024-12-08 07:00:00 -05:00 EST",
        ),
        (
            "2025-11-01T12:00:00.000",
            1,
            "2025-11-02T12:00:00.000",
            "2025-11-02 07:00:00 -05:00 EST",
        ),
        (
            "2025-11-01T12:00:00.000",
            7,
            "2025-11-08T12:00:00.000",
            "2025-11-08 07:00:00 -05:00 EST",
        ),
        (
            "2025-11-01T12:00:00.000",
            30,
            "2025-12-01T12:00:00.000",
            "2025-12-01 07:00:00 -05:00 EST",
        ),
        (
            "2025-11-01T12:00:00.000",
            90,
            "2026-01-30T12:00:00.000",
            "2026-01-30 07:00:00 -05:00 EST",
        ),
        (
            "2025-11-01T12:00:00.000",
            -1,
            "2025-10-31T12:00:00.000",
            "2025-10-31 08:00:00 -04:00 EDT",
        ),
        (
            "2025-11-01T12:00:00.000",
            -90,
            "2025-08-03T12:00:00.000",
            "2025-08-03 08:00:00 -04:00 EDT",
        ),
        (
            "2024-02-28T23:59:59.999",
            1,
            "2024-02-29T23:59:59.999",
            "2024-02-29 18:59:59 -05:00 EST",
        ),
        (
            "2024-12-31T23:59:59.999",
            1,
            "2025-01-01T23:59:59.999",
            "2025-01-01 18:59:59 -05:00 EST",
        ),
        (
            "1970-01-01T00:00:00.000",
            -1,
            "1969-12-31T00:00:00.000",
            "1969-12-30 19:00:00 -05:00 EST",
        ),
    ];

    for (start, days, expected, in_toronto) in rows {
        let from = instant(start);
        let moved = engine::plus_days(from, days);
        assert_eq!(show(moved), expected, "{start} {days:+} days");
        assert_eq!(
            engine::epoch_seconds(moved) - engine::epoch_seconds(from),
            days * DAY,
            "{start} {days:+} days is not a whole number of 86,400 s"
        );
        assert_eq!(
            engine::wall_clock(moved, toronto),
            in_toronto,
            "{start} {days:+} days, read in Toronto"
        );
    }
}

/// The wall clock before the move, to show the hour it gains or loses.
#[test]
fn across_a_clock_change_the_wall_clock_moves_by_an_hour() {
    let toronto = zone("America/Toronto");
    assert_eq!(
        engine::wall_clock(instant("2025-03-08T12:00:00.000"), toronto),
        "2025-03-08 07:00:00 -05:00 EST",
        "noon UTC on the eve of the spring change, read in Toronto"
    );
    assert_eq!(
        engine::wall_clock(instant("2025-11-01T12:00:00.000"), toronto),
        "2025-11-01 08:00:00 -04:00 EDT",
        "noon UTC on the eve of the autumn change, read in Toronto"
    );
}

#[test]
fn weeks_and_hours_are_exact_multiples_of_seconds_too() {
    let from = instant("2025-03-08T12:00:00.000");
    assert_eq!(
        show(engine::plus_weeks(from, 1)),
        "2025-03-15T12:00:00.000",
        "one week later"
    );
    assert_eq!(
        show(engine::plus_weeks(from, -2)),
        "2025-02-22T12:00:00.000",
        "two weeks earlier"
    );
    assert_eq!(
        show(engine::plus_hours(from, 24)),
        "2025-03-09T12:00:00.000",
        "24 hours later, across the spring change"
    );
    assert_eq!(
        show(engine::plus_hours(from, -36)),
        "2025-03-07T00:00:00.000",
        "36 hours earlier"
    );
    assert_eq!(
        engine::epoch_seconds(engine::plus_weeks(from, 1)) - engine::epoch_seconds(from),
        7 * DAY,
        "a week is exactly seven 86,400 s days"
    );
}

/// Whole units of a difference truncate toward zero, in both directions.
#[test]
fn a_difference_in_whole_units_truncates_toward_zero() {
    let base = "2025-03-08T12:00:00.000";
    // later, earlier, [days, hours, minutes, seconds, milliseconds]
    let rows = [
        (
            "2025-03-09T12:00:00.000",
            base,
            [1, 24, 1_440, 86_400, 86_400_000],
        ),
        (
            "2025-03-09T11:59:59.999",
            base,
            [0, 23, 1_439, 86_399, 86_399_999],
        ),
        (
            "2025-03-09T12:00:00.001",
            base,
            [1, 24, 1_440, 86_400, 86_400_001],
        ),
        (
            "2025-03-11T00:00:00.000",
            base,
            [2, 60, 3_600, 216_000, 216_000_000],
        ),
        (
            base,
            "2025-03-09T12:00:00.000",
            [-1, -24, -1_440, -86_400, -86_400_000],
        ),
        (
            base,
            "2025-03-09T11:59:59.999",
            [0, -23, -1_439, -86_399, -86_399_999],
        ),
        (
            base,
            "2025-03-09T12:00:00.001",
            [-1, -24, -1_440, -86_400, -86_400_001],
        ),
        (
            base,
            "2025-03-11T00:00:00.000",
            [-2, -60, -3_600, -216_000, -216_000_000],
        ),
        ("2025-03-08T12:00:00.999", base, [0, 0, 0, 0, 999]),
        (base, "2025-03-08T12:00:00.999", [0, 0, 0, 0, -999]),
        (base, "2025-03-08T12:00:01.500", [0, 0, 0, -1, -1_500]),
        (base, base, [0, 0, 0, 0, 0]),
        // Thirty days across the spring change: still thirty, never 29.96.
        (
            "2025-04-07T12:00:00.000",
            base,
            [30, 720, 43_200, 2_592_000, 2_592_000_000],
        ),
    ];

    for (later, earlier, expected) in rows {
        assert_eq!(
            engine::difference(instant(later), instant(earlier)),
            expected,
            "{later} - {earlier}"
        );
    }

    assert_eq!(
        engine::difference_in_weeks(instant("2025-03-22T11:59:59.999"), instant(base)),
        1,
        "a millisecond short of two weeks is one whole week"
    );
    assert_eq!(
        engine::difference_in_weeks(instant(base), instant("2025-03-22T11:59:59.999")),
        -1,
        "the same span backwards truncates toward zero to minus one week"
    );
    assert_eq!(
        engine::difference_in_weeks(instant("2025-03-15T11:59:59.999"), instant(base)),
        0,
        "a millisecond short of one week is zero whole weeks"
    );
}

#[test]
fn an_instant_reads_as_its_utc_calendar_date() {
    let rows = [
        ("2026-11-17T23:59:59.999", "2026-11-17"),
        ("2026-11-18T00:00:00.000", "2026-11-18"),
        ("2024-02-29T23:59:59.999", "2024-02-29"),
        ("2024-03-01T00:00:00.000", "2024-03-01"),
        ("2026-12-31T23:59:59.999", "2026-12-31"),
        ("1969-12-31T23:59:59.999", "1969-12-31"),
        ("1970-01-01T00:00:00.000", "1970-01-01"),
    ];
    for (at, expected) in rows {
        assert_eq!(engine::utc_date(instant(at)), expected, "{at}");
    }
}

#[test]
fn a_calendar_date_starts_at_midnight_utc() {
    assert_eq!(
        show(engine::start_of_day(date("2024-02-29"))),
        "2024-02-29T00:00:00.000",
        "a leap day starts at its own midnight UTC"
    );
    assert_eq!(
        engine::epoch_seconds(engine::start_of_day(date("1970-01-02"))),
        DAY,
        "the day after the epoch starts one day of seconds in"
    );
    assert_eq!(
        engine::epoch_seconds(engine::start_of_day(date("1969-12-31"))),
        -DAY,
        "the day before the epoch starts one day of seconds back"
    );
}

/// The athlete's local date and wall clock around clock changes, in zones that
/// change at 02:00, at midnight, by half an hour, and once by a whole day.
#[test]
fn an_instant_reads_as_the_local_date_and_wall_clock_of_a_zone() {
    // zone, instant, wall clock, the clock's date, the bucketing date
    let rows = [
        // Toronto, spring 2025: 02:00 EST becomes 03:00 EDT.
        (
            "America/Toronto",
            "2025-03-09T04:59:59.000",
            "2025-03-08 23:59:59 -05:00 EST",
            "2025-03-08",
            "2025-03-08",
        ),
        (
            "America/Toronto",
            "2025-03-09T05:00:00.000",
            "2025-03-09 00:00:00 -05:00 EST",
            "2025-03-09",
            "2025-03-09",
        ),
        (
            "America/Toronto",
            "2025-03-09T06:59:59.000",
            "2025-03-09 01:59:59 -05:00 EST",
            "2025-03-09",
            "2025-03-09",
        ),
        (
            "America/Toronto",
            "2025-03-09T07:00:00.000",
            "2025-03-09 03:00:00 -04:00 EDT",
            "2025-03-09",
            "2025-03-09",
        ),
        (
            "America/Toronto",
            "2025-03-10T03:59:59.000",
            "2025-03-09 23:59:59 -04:00 EDT",
            "2025-03-09",
            "2025-03-09",
        ),
        (
            "America/Toronto",
            "2025-03-10T04:00:00.000",
            "2025-03-10 00:00:00 -04:00 EDT",
            "2025-03-10",
            "2025-03-10",
        ),
        // Toronto, autumn 2025: 01:00 to 02:00 happens twice.
        (
            "America/Toronto",
            "2025-11-02T05:30:00.000",
            "2025-11-02 01:30:00 -04:00 EDT",
            "2025-11-02",
            "2025-11-02",
        ),
        (
            "America/Toronto",
            "2025-11-02T06:30:00.000",
            "2025-11-02 01:30:00 -05:00 EST",
            "2025-11-02",
            "2025-11-02",
        ),
        (
            "America/Toronto",
            "2025-11-03T04:59:59.000",
            "2025-11-02 23:59:59 -05:00 EST",
            "2025-11-02",
            "2025-11-02",
        ),
        (
            "America/Toronto",
            "2025-11-03T05:00:00.000",
            "2025-11-03 00:00:00 -05:00 EST",
            "2025-11-03",
            "2025-11-03",
        ),
        // Midnight UTC: the clock converts, a date-only provider row does not.
        (
            "America/Toronto",
            "2025-11-03T00:00:00.000",
            "2025-11-02 19:00:00 -05:00 EST",
            "2025-11-02",
            "2025-11-03",
        ),
        (
            "Australia/Sydney",
            "2025-11-03T00:00:00.000",
            "2025-11-03 11:00:00 +11:00 AEDT",
            "2025-11-03",
            "2025-11-03",
        ),
        // Cairo, spring 2024: the change is at midnight, so 00:00 does not exist.
        (
            "Africa/Cairo",
            "2024-04-25T21:59:59.000",
            "2024-04-25 23:59:59 +02:00 EET",
            "2024-04-25",
            "2024-04-25",
        ),
        (
            "Africa/Cairo",
            "2024-04-25T22:00:00.000",
            "2024-04-26 01:00:00 +03:00 EEST",
            "2024-04-26",
            "2024-04-26",
        ),
        // Lord Howe, autumn 2025: the clock goes back half an hour.
        (
            "Australia/Lord_Howe",
            "2025-04-05T14:59:59.000",
            "2025-04-06 01:59:59 +11:00 +11",
            "2025-04-06",
            "2025-04-06",
        ),
        (
            "Australia/Lord_Howe",
            "2025-04-05T15:00:00.000",
            "2025-04-06 01:30:00 +10:30 +1030",
            "2025-04-06",
            "2025-04-06",
        ),
        // Kolkata and Kathmandu: offsets that are not whole hours.
        (
            "Asia/Kolkata",
            "2025-06-30T18:29:59.000",
            "2025-06-30 23:59:59 +05:30 IST",
            "2025-06-30",
            "2025-06-30",
        ),
        (
            "Asia/Kolkata",
            "2025-06-30T18:30:00.000",
            "2025-07-01 00:00:00 +05:30 IST",
            "2025-07-01",
            "2025-07-01",
        ),
        (
            "Asia/Kathmandu",
            "2025-06-30T18:15:00.000",
            "2025-07-01 00:00:00 +05:45 +0545",
            "2025-07-01",
            "2025-07-01",
        ),
        // Apia, 2011: 30 December never happened there.
        (
            "Pacific/Apia",
            "2011-12-30T09:59:59.000",
            "2011-12-29 23:59:59 -10:00 -10",
            "2011-12-29",
            "2011-12-29",
        ),
        (
            "Pacific/Apia",
            "2011-12-30T10:00:00.000",
            "2011-12-31 00:00:00 +14:00 +14",
            "2011-12-31",
            "2011-12-31",
        ),
        // UTC itself.
        (
            "UTC",
            "2025-03-09T23:59:59.000",
            "2025-03-09 23:59:59 +00:00 UTC",
            "2025-03-09",
            "2025-03-09",
        ),
    ];

    for (name, at, wall, clock, bucket) in rows {
        let zone = zone(name);
        let at_instant = instant(at);
        assert_eq!(engine::wall_clock(at_instant, zone), wall, "{at} in {name}");
        assert_eq!(
            engine::show_date(clock_date(at_instant, zone)),
            clock,
            "clock date of {at} in {name}"
        );
        assert_eq!(
            engine::show_date(local_date(at_instant, zone)),
            bucket,
            "bucketing date of {at} in {name}"
        );
    }
}

#[test]
fn only_exact_midnight_utc_counts_as_a_date_only_row() {
    assert!(
        is_date_only(instant("2025-11-03T00:00:00.000")),
        "exact midnight UTC is a date-only row"
    );
    assert!(
        !is_date_only(instant("2025-11-03T00:00:00.001")),
        "a millisecond past midnight is not date-only"
    );
    assert!(
        !is_date_only(instant("2025-11-02T23:59:59.999")),
        "a millisecond before midnight is not date-only"
    );
}

/// A wall-clock time names no instant, one, or two.
#[test]
fn a_wall_clock_time_resolves_to_none_one_or_two_instants() {
    // zone, wall clock, verdict, [earliest, latest, single]
    let rows = [
        (
            "America/Toronto",
            "2025-03-09 01:59:59",
            "one:2025-03-09T06:59:59.000",
            [
                "2025-03-09T06:59:59.000",
                "2025-03-09T06:59:59.000",
                "2025-03-09T06:59:59.000",
            ],
        ),
        (
            "America/Toronto",
            "2025-03-09 02:00:00",
            "none",
            ["none", "none", "none"],
        ),
        (
            "America/Toronto",
            "2025-03-09 02:30:00",
            "none",
            ["none", "none", "none"],
        ),
        (
            "America/Toronto",
            "2025-03-09 02:59:59",
            "none",
            ["none", "none", "none"],
        ),
        (
            "America/Toronto",
            "2025-03-09 03:00:00",
            "one:2025-03-09T07:00:00.000",
            [
                "2025-03-09T07:00:00.000",
                "2025-03-09T07:00:00.000",
                "2025-03-09T07:00:00.000",
            ],
        ),
        (
            "America/Toronto",
            "2025-11-02 00:59:59",
            "one:2025-11-02T04:59:59.000",
            [
                "2025-11-02T04:59:59.000",
                "2025-11-02T04:59:59.000",
                "2025-11-02T04:59:59.000",
            ],
        ),
        (
            "America/Toronto",
            "2025-11-02 01:00:00",
            "two:2025-11-02T05:00:00.000|2025-11-02T06:00:00.000",
            ["2025-11-02T05:00:00.000", "2025-11-02T06:00:00.000", "none"],
        ),
        (
            "America/Toronto",
            "2025-11-02 01:30:00",
            "two:2025-11-02T05:30:00.000|2025-11-02T06:30:00.000",
            ["2025-11-02T05:30:00.000", "2025-11-02T06:30:00.000", "none"],
        ),
        (
            "America/Toronto",
            "2025-11-02 02:00:00",
            "one:2025-11-02T07:00:00.000",
            [
                "2025-11-02T07:00:00.000",
                "2025-11-02T07:00:00.000",
                "2025-11-02T07:00:00.000",
            ],
        ),
        // Midnight itself is missing in Cairo on the day the clock goes forward.
        (
            "Africa/Cairo",
            "2024-04-26 00:00:00",
            "none",
            ["none", "none", "none"],
        ),
        (
            "Africa/Cairo",
            "2024-04-26 01:00:00",
            "one:2024-04-25T22:00:00.000",
            [
                "2024-04-25T22:00:00.000",
                "2024-04-25T22:00:00.000",
                "2024-04-25T22:00:00.000",
            ],
        ),
        // The whole of 30 December 2011 is missing in Apia.
        (
            "Pacific/Apia",
            "2011-12-30 12:00:00",
            "none",
            ["none", "none", "none"],
        ),
        // A half-hour overlap.
        (
            "Australia/Lord_Howe",
            "2025-04-06 01:45:00",
            "two:2025-04-05T14:45:00.000|2025-04-05T15:15:00.000",
            ["2025-04-05T14:45:00.000", "2025-04-05T15:15:00.000", "none"],
        ),
        (
            "UTC",
            "2025-03-09 02:30:00",
            "one:2025-03-09T02:30:00.000",
            [
                "2025-03-09T02:30:00.000",
                "2025-03-09T02:30:00.000",
                "2025-03-09T02:30:00.000",
            ],
        ),
    ];

    for (name, wall, verdict, picks) in rows {
        let zone = zone(name);
        assert_eq!(
            engine::resolve_wall_clock(wall, zone),
            verdict,
            "{wall} in {name}"
        );
        assert_eq!(
            engine::pick_wall_clock(wall, zone),
            picks,
            "earliest, latest and single for {wall} in {name}"
        );
    }
}

/// A month added to a day the target month lacks lands on its last day.
#[test]
fn month_arithmetic_clamps_to_the_last_day_of_the_month() {
    let rows = [
        ("2025-01-31", 1, "2025-02-28"),
        ("2024-01-31", 1, "2024-02-29"),
        ("2025-01-30", 1, "2025-02-28"),
        ("2025-01-28", 1, "2025-02-28"),
        ("2025-03-31", -1, "2025-02-28"),
        ("2024-03-31", -1, "2024-02-29"),
        ("2025-02-28", 1, "2025-03-28"),
        ("2025-02-28", -1, "2025-01-28"),
        ("2025-10-31", 1, "2025-11-30"),
        ("2025-12-31", 2, "2026-02-28"),
        ("2025-12-31", 1, "2026-01-31"),
        ("2025-01-31", -1, "2024-12-31"),
        ("2025-05-31", -3, "2025-02-28"),
        ("2024-02-29", 12, "2025-02-28"),
        ("2024-02-29", -12, "2023-02-28"),
        ("2024-02-29", 48, "2028-02-29"),
        ("2024-02-29", -48, "2020-02-29"),
        ("2025-02-28", 12, "2026-02-28"),
        ("2023-02-28", 12, "2024-02-28"),
        ("2026-11-17", -24, "2024-11-17"),
        ("2026-11-17", 0, "2026-11-17"),
    ];
    for (start, months, expected) in rows {
        assert_eq!(
            engine::date_plus_months(date(start), months),
            expected,
            "{start} {months:+} months"
        );
    }
}

/// The same clamp on an instant keeps the time of day.
#[test]
fn month_arithmetic_on_an_instant_keeps_the_time_of_day() {
    let rows = [
        ("2026-03-31T10:00:05.100", -1, "2026-02-28T10:00:05.100"),
        ("2024-01-31T23:59:59.999", 1, "2024-02-29T23:59:59.999"),
        ("2024-02-29T00:00:00.000", 12, "2025-02-28T00:00:00.000"),
        ("2026-11-17T10:00:05.100", -24, "2024-11-17T10:00:05.100"),
    ];
    for (start, months, expected) in rows {
        assert_eq!(
            engine::instant_plus_months(instant(start), months),
            expected,
            "{start} {months:+} months"
        );
    }
}

#[test]
fn day_arithmetic_on_a_calendar_date_crosses_month_and_year_ends() {
    let rows = [
        ("2024-02-28", 1, "2024-02-29"),
        ("2024-02-28", 2, "2024-03-01"),
        ("2025-02-28", 1, "2025-03-01"),
        ("2024-03-01", -1, "2024-02-29"),
        ("2025-12-31", 1, "2026-01-01"),
        ("2026-01-01", -1, "2025-12-31"),
        ("2025-03-08", 90, "2025-06-06"),
        ("2025-03-08", -90, "2024-12-08"),
        ("2025-03-08", 365, "2026-03-08"),
        ("2024-01-01", 365, "2024-12-31"),
    ];
    for (start, days, expected) in rows {
        assert_eq!(
            engine::date_plus_days(date(start), days),
            expected,
            "{start} {days:+} days"
        );
        assert_eq!(
            engine::date_plus_elapsed_days(date(start), days),
            expected,
            "{start} {days:+} days, added as a length of time"
        );
    }

    assert_eq!(
        engine::days_between(date("2024-03-01"), date("2024-02-28")),
        2,
        "across a leap day"
    );
    assert_eq!(
        engine::days_between(date("2025-03-01"), date("2025-02-28")),
        1,
        "across the end of a common February"
    );
    assert_eq!(
        engine::days_between(date("2024-02-28"), date("2024-03-01")),
        -2,
        "backwards across a leap day"
    );
    assert_eq!(
        engine::days_between(date("2026-01-01"), date("2025-01-01")),
        365,
        "a common year"
    );
    assert_eq!(
        engine::days_between(date("2025-01-01"), date("2024-01-01")),
        366,
        "a leap year"
    );
}

/// Start of week and ISO week around the turn of a year.
#[test]
fn weeks_start_on_monday_and_iso_weeks_can_belong_to_the_other_year() {
    // date, weekday, days from Monday, day of year, week start, ISO (year, week)
    let rows = [
        ("2024-12-29", "Sun", 6, 364, "2024-12-23", (2024, 52)),
        ("2024-12-30", "Mon", 0, 365, "2024-12-30", (2025, 1)),
        ("2024-12-31", "Tue", 1, 366, "2024-12-30", (2025, 1)),
        ("2025-01-01", "Wed", 2, 1, "2024-12-30", (2025, 1)),
        ("2025-12-28", "Sun", 6, 362, "2025-12-22", (2025, 52)),
        ("2025-12-29", "Mon", 0, 363, "2025-12-29", (2026, 1)),
        ("2026-01-01", "Thu", 3, 1, "2025-12-29", (2026, 1)),
        ("2026-12-31", "Thu", 3, 365, "2026-12-28", (2026, 53)),
        ("2027-01-01", "Fri", 4, 1, "2026-12-28", (2026, 53)),
        ("2027-01-03", "Sun", 6, 3, "2026-12-28", (2026, 53)),
        ("2027-01-04", "Mon", 0, 4, "2027-01-04", (2027, 1)),
        ("2021-01-03", "Sun", 6, 3, "2020-12-28", (2020, 53)),
        ("2024-02-29", "Thu", 3, 60, "2024-02-26", (2024, 9)),
    ];
    for (day, weekday, from_monday, ordinal, monday, iso) in rows {
        let day_date = date(day);
        assert_eq!(
            engine::day_facts(day_date),
            (weekday.to_owned(), from_monday, ordinal),
            "{day}"
        );
        assert_eq!(
            engine::week_start(day_date),
            (monday.to_owned(), monday.to_owned()),
            "week start of {day}"
        );
        assert_eq!(engine::iso_week(day_date), iso, "ISO week of {day}");
    }
}

/// Epoch conversions, including before 1970 where a fraction rounds down.
#[test]
fn epoch_conversions_floor_before_1970() {
    // instant, seconds, milliseconds
    let rows = [
        ("1970-01-01T00:00:00.000", 0, 0),
        ("2026-11-17T10:00:05.123", 1_794_909_605, 1_794_909_605_123),
        ("1969-12-31T23:59:59.999", -1, -1),
        ("1969-12-31T23:59:58.500", -2, -1_500),
        ("1969-07-20T20:17:40.250", -14_182_940, -14_182_939_750),
    ];
    for (at, seconds, millis) in rows {
        let value = instant(at);
        assert_eq!(engine::epoch_seconds(value), seconds, "seconds of {at}");
        assert_eq!(engine::epoch_millis(value), millis, "milliseconds of {at}");
        assert_eq!(
            engine::from_epoch_millis(millis).map(show),
            Some(at.to_owned()),
            "{millis} ms back to an instant"
        );
    }

    assert_eq!(
        engine::from_epoch(-2, 500_000_000).map(show),
        Some("1969-12-31T23:59:58.500".to_owned()),
        "negative seconds plus positive nanoseconds before 1970"
    );
    assert_eq!(
        engine::from_epoch(1_794_909_605, 123_000_000).map(show),
        Some("2026-11-17T10:00:05.123".to_owned()),
        "seconds plus nanoseconds after 1970"
    );
    assert_eq!(
        engine::from_epoch(i64::MAX, 0).map(show),
        None,
        "seconds beyond the representable range give no instant"
    );
}

/// Which zone names are known. Names are matched exactly as written.
#[test]
fn zone_names_are_iana_names_matched_exactly() {
    let rows = [
        ("America/Toronto", "America/Toronto"),
        ("America/Montreal", "America/Montreal"),
        ("Europe/Paris", "Europe/Paris"),
        ("Europe/Kyiv", "Europe/Kyiv"),
        ("Europe/Kiev", "Europe/Kiev"),
        ("Asia/Calcutta", "Asia/Calcutta"),
        ("US/Eastern", "US/Eastern"),
        ("UTC", "UTC"),
        ("Etc/UTC", "Etc/UTC"),
        ("GMT", "GMT"),
        ("Etc/GMT+5", "Etc/GMT+5"),
        ("EST", "EST"),
        ("EST5EDT", "EST5EDT"),
        ("Zulu", "Zulu"),
        ("america/toronto", "rejected"),
        ("AMERICA/TORONTO", "rejected"),
        ("utc", "rejected"),
        ("Z", "rejected"),
        ("EDT", "rejected"),
        ("+05:00", "rejected"),
        ("UTC+5", "rejected"),
        ("America/Nowhere", "rejected"),
        (" America/Toronto", "rejected"),
        ("", "rejected"),
    ];
    let actual: Vec<(&str, String)> = rows
        .iter()
        .map(|(name, _)| (*name, engine::zone_verdict(name)))
        .collect();
    let wanted: Vec<(&str, String)> = rows
        .iter()
        .map(|(name, verdict)| (*name, (*verdict).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "zone name verdicts");
}

/// The two production entry points for an athlete's zone name.
#[test]
fn an_unknown_or_missing_zone_name_falls_back_to_utc() {
    let noon = instant("2025-07-01T12:00:00.000");
    let shown = |zone: engine::Zone| engine::wall_clock(noon, zone);

    assert_eq!(
        shown(resolve_zone(Some("America/Toronto"))),
        "2025-07-01 08:00:00 -04:00 EDT",
        "a known zone resolves to itself"
    );
    assert_eq!(
        shown(resolve_zone(Some("america/toronto"))),
        "2025-07-01 12:00:00 +00:00 UTC",
        "a wrongly cased zone resolves to UTC"
    );
    assert_eq!(
        shown(resolve_zone(Some(""))),
        "2025-07-01 12:00:00 +00:00 UTC",
        "an empty zone name resolves to UTC"
    );
    assert_eq!(
        shown(resolve_zone(None)),
        "2025-07-01 12:00:00 +00:00 UTC",
        "a missing zone name resolves to UTC"
    );

    assert_eq!(
        parse_zone(Some("Europe/Paris")).map(shown),
        Some("2025-07-01 14:00:00 +02:00 CEST".to_owned()),
        "parse_zone accepts a known zone"
    );
    assert_eq!(
        parse_zone(Some("Paris")).map(shown),
        None,
        "parse_zone rejects a city that is not a zone name"
    );
    assert_eq!(
        parse_zone(None).map(shown),
        None,
        "parse_zone gives nothing for a missing name"
    );
}
