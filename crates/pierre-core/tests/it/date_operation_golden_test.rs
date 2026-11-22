// ABOUTME: Pins the remaining date operations production uses: truncation, Sunday weeks, std durations
// ABOUTME: Recorded from the date engine in use on 2026-11-17 so a later engine gives the same answers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Operation goldens for dates (carnet#1620, gate 8, second table).
//!
//! `date_arithmetic_golden_test` pins days, months, zones and ISO weeks. These
//! are the other operations production code calls, each recorded by running:
//!
//! | Operation | Used by |
//! |---|---|
//! | cut an instant to microseconds or seconds | `next_setting_stamp`, a subscription's `last_event_at`, the usage stamp (`pierre-database`), coach attention |
//! | the Sunday that starts a week | the usage counters' weekly period (`pierre-services`) |
//! | the first day of a month | the monthly request window (`pierre-database`), the turn clock |
//! | seconds since local midnight, the zoneless local date-time | the check-in schedule (`pierre-server`) |
//! | an instant plus a lifetime, refusing an overflow | admin token expiry (`pierre-core`), and six other sites |
//! | a `std` duration into a signed one and back | sweeps, schedulers, the turn ceiling |
//! | every day from a date on | the training-history rollup (`pierre-services`) |
//! | a difference in whole milliseconds | latency and age arithmetic everywhere |
//!
//! Two of them have a public production function in `pierre-core`, and are
//! asserted through it: token expiry through
//! `CreateAdminTokenRequest::expires_at`, the check-in schedule through
//! `CheckInCadence::due_at`. Two more are asserted through their repository in
//! `pierre-database` (`date_stamp_golden_sqlite_test`). The call sites in
//! `pierre-services` and `pierre-server` are not reachable from these crates,
//! so their rows go through this file's own helper and state what the call
//! site computes today.
//!
//! Tables hold strings and integers only. Everything that names the date
//! engine lives in [`engine`].

use pierre_core::admin::models::CreateAdminTokenRequest;
use pierre_core::models::CheckInCadence;

/// Everything in this file that names the date engine.
mod engine {
    use std::iter;
    use std::time::Duration as StdDuration;

    use dravr_tronc::dates::{CivilDate, Elapsed, Zone};

    pub(super) use dravr_tronc::dates::{CivilDateTime, Timestamp as Instant};

    const STAMP: &str = "%Y-%m-%dT%H:%M:%S%.9f";

    /// A UTC instant from `2026-11-17T10:00:05.123456789`, any fraction or none.
    pub(super) fn instant(text: &str) -> Instant {
        CivilDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
            .unwrap()
            .and_utc()
            .unwrap()
    }

    /// The instant at nanosecond precision.
    pub(super) fn show(value: Instant) -> String {
        value.format(STAMP).unwrap()
    }

    fn date(text: &str) -> CivilDate {
        CivilDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    /// The instant cut to microseconds and to whole seconds.
    pub(super) fn truncated(text: &str) -> [String; 2] {
        let value = instant(text);
        [show(value.trunc_subsecs(6)), show(value.trunc_subsecs(0))]
    }

    /// Days since Sunday, the Sunday that starts the week, and the days until
    /// the week resets (seven on a Sunday), as the usage counters compute them.
    pub(super) fn sunday_week(text: &str) -> (u32, String, u32) {
        let day = date(text);
        let since_sunday = day.weekday().num_days_from_sunday();
        let sunday = day - Elapsed::days(i64::from(since_sunday));
        (since_sunday, sunday.to_string(), 7 - since_sunday)
    }

    /// The date moved to a day of its own month.
    pub(super) fn on_day(text: &str, day: u32) -> String {
        date(text)
            .with_day(day)
            .map_or_else(|| "none".to_owned(), |moved| moved.to_string())
    }

    /// The zoneless date-time an instant shows in a zone.
    pub(super) fn local_clock(value: Instant, zone: &str) -> CivilDateTime {
        zone.parse::<Zone>().unwrap().civil_at(value)
    }

    /// Seconds since the zone's wall-clock midnight, the instant that many
    /// seconds earlier, and the zoneless local date-time.
    pub(super) fn since_local_midnight(text: &str, zone: &str) -> (u32, String, String) {
        let value = instant(text);
        let local = zone.parse::<Zone>().unwrap().civil_at(value);
        let seconds = local.time().num_seconds_from_midnight();
        (
            seconds,
            show(value - Elapsed::seconds(i64::from(seconds))),
            local.format(STAMP).unwrap(),
        )
    }

    /// The instant plus and minus a number of days, each refusing an overflow.
    pub(super) fn checked_days(text: &str, days: i64) -> [String; 2] {
        let value = instant(text);
        let Some(step) = Elapsed::try_days(days) else {
            return ["no such duration".to_owned(), "no such duration".to_owned()];
        };
        [value.checked_add(step), value.checked_sub(step)]
            .map(|moved| moved.map_or_else(|| "none".to_owned(), show))
    }

    /// A `std` duration as a signed one: its whole seconds and the nanoseconds
    /// past them.
    pub(super) fn from_std(seconds: u64, nanos: u32) -> String {
        Elapsed::from_std(StdDuration::new(seconds, nanos)).map_or_else(
            |_| "rejected".to_owned(),
            |delta| format!("{}s+{}ns", delta.num_seconds(), delta.subsec_nanos()),
        )
    }

    /// A signed duration of that many milliseconds as a `std` one.
    pub(super) fn to_std(millis: i64) -> String {
        Elapsed::milliseconds(millis).to_std().map_or_else(
            |_| "rejected".to_owned(),
            |std| format!("{}s+{}ns", std.as_secs(), std.subsec_nanos()),
        )
    }

    /// Every day from `from` up to and including `to`.
    pub(super) fn days_through(from: &str, to: &str) -> Vec<String> {
        let last = date(to);
        iter::successors(Some(date(from)), |day| day.succ())
            .take_while(|day| *day <= last)
            .map(|day| day.to_string())
            .collect()
    }

    /// `later - earlier` in whole seconds, milliseconds and microseconds, and
    /// in nanoseconds.
    pub(super) fn difference(later: &str, earlier: &str) -> [i64; 4] {
        let elapsed = instant(later).signed_duration_since(instant(earlier));
        [
            elapsed.num_seconds(),
            elapsed.num_milliseconds(),
            elapsed.num_microseconds().unwrap(),
            elapsed.num_nanoseconds().unwrap(),
        ]
    }
}

use engine::{instant, show};

/// Cutting never rounds, and before 1970 it still cuts toward the earlier
/// instant: the digits are dropped, the value is not moved toward zero.
#[test]
fn cutting_an_instant_to_microseconds_or_seconds_drops_digits() {
    // instant, [cut to microseconds, cut to seconds]
    let rows = [
        (
            "2026-11-17T10:00:05",
            [
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            [
                "2026-11-17T10:00:05.123456000",
                "2026-11-17T10:00:05.000000000",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456",
            [
                "2026-11-17T10:00:05.123456000",
                "2026-11-17T10:00:05.000000000",
            ],
        ),
        (
            "2026-11-17T10:00:05.000000999",
            [
                "2026-11-17T10:00:05.000000000",
                "2026-11-17T10:00:05.000000000",
            ],
        ),
        (
            "2026-11-17T23:59:59.999999999",
            [
                "2026-11-17T23:59:59.999999000",
                "2026-11-17T23:59:59.000000000",
            ],
        ),
        (
            "1969-07-20T20:17:40.999999999",
            [
                "1969-07-20T20:17:40.999999000",
                "1969-07-20T20:17:40.000000000",
            ],
        ),
        (
            "1969-12-31T23:59:59.000000001",
            [
                "1969-12-31T23:59:59.000000000",
                "1969-12-31T23:59:59.000000000",
            ],
        ),
        (
            "2016-12-31T23:59:60.500000001",
            [
                "2016-12-31T23:59:59.500000000",
                "2016-12-31T23:59:59.000000000",
            ],
        ),
    ];
    let actual: Vec<(&str, [String; 2])> = rows
        .iter()
        .map(|(at, _)| (*at, engine::truncated(at)))
        .collect();
    let wanted: Vec<(&str, [String; 2])> = rows
        .iter()
        .map(|(at, cut)| (*at, cut.map(str::to_owned)))
        .collect();
    assert_eq!(actual, wanted, "microsecond and second cuts per instant");
}

/// The usage counters' week starts on Sunday, not on the Monday every other
/// week in the platform starts on.
#[test]
fn a_sunday_based_week_starts_on_the_most_recent_sunday() {
    // date, days since Sunday, the week's Sunday, days until the reset
    let rows = [
        ("2026-12-27", 0, "2026-12-27", 7),
        ("2026-12-28", 1, "2026-12-27", 6),
        ("2026-12-29", 2, "2026-12-27", 5),
        ("2026-12-30", 3, "2026-12-27", 4),
        ("2026-12-31", 4, "2026-12-27", 3),
        ("2027-01-01", 5, "2026-12-27", 2),
        ("2027-01-02", 6, "2026-12-27", 1),
        ("2027-01-03", 0, "2027-01-03", 7),
        ("2024-02-29", 4, "2024-02-25", 3),
        ("2024-03-02", 6, "2024-02-25", 1),
    ];
    let actual: Vec<(&str, u32, String, u32)> = rows
        .iter()
        .map(|(day, ..)| {
            let (since, sunday, until) = engine::sunday_week(day);
            (*day, since, sunday, until)
        })
        .collect();
    let wanted: Vec<(&str, u32, String, u32)> = rows
        .iter()
        .map(|(day, since, sunday, until)| (*day, *since, (*sunday).to_owned(), *until))
        .collect();
    assert_eq!(
        actual, wanted,
        "days since Sunday, the Sunday and days to the reset per date"
    );
}

/// Moving to day 1 always works; moving to a day the month lacks gives
/// nothing rather than clamping.
#[test]
fn a_date_moves_to_a_day_of_its_own_month_or_to_nothing() {
    // date, day, result
    let rows = [
        ("2026-01-31", 1, "2026-01-01"),
        ("2024-02-29", 1, "2024-02-01"),
        ("2026-12-31", 1, "2026-12-01"),
        ("2026-03-01", 1, "2026-03-01"),
        ("2026-04-15", 30, "2026-04-30"),
        ("2026-04-15", 31, "none"),
        ("2026-02-15", 29, "none"),
        ("2024-02-15", 29, "2024-02-29"),
        ("2026-04-15", 0, "none"),
        ("2026-04-15", 32, "none"),
    ];
    let actual: Vec<(&str, u32, String)> = rows
        .iter()
        .map(|(day, to, _)| (*day, *to, engine::on_day(day, *to)))
        .collect();
    let wanted: Vec<(&str, u32, String)> = rows
        .iter()
        .map(|(day, to, moved)| (*day, *to, (*moved).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "the date moved to a day of its own month");
}

/// Seconds since midnight read the wall clock, so on a day the clock changes
/// "now minus that many seconds" is not the local midnight: it is an hour
/// before it in spring and an hour after it in autumn.
#[test]
fn seconds_since_local_midnight_read_the_wall_clock() {
    // zone, instant, seconds since midnight, the instant that many seconds
    // earlier, the zoneless local date-time
    let rows = [
        (
            "America/Toronto",
            "2026-11-17T15:00:05.250",
            36_005,
            "2026-11-17T05:00:00.250000000",
            "2026-11-17T10:00:05.250000000",
        ),
        (
            "America/Toronto",
            "2026-11-17T05:00:00",
            0,
            "2026-11-17T05:00:00.000000000",
            "2026-11-17T00:00:00.000000000",
        ),
        (
            "America/Toronto",
            "2026-11-17T04:59:59.999",
            86_399,
            "2026-11-16T05:00:00.999000000",
            "2026-11-16T23:59:59.999000000",
        ),
        // Spring 2025: 03:30 EDT is two and a half hours after midnight EST.
        (
            "America/Toronto",
            "2025-03-09T07:30:00",
            12_600,
            "2025-03-09T04:00:00.000000000",
            "2025-03-09T03:30:00.000000000",
        ),
        // Autumn 2025: the second 01:30, EST, is two and a half hours after
        // midnight EDT.
        (
            "America/Toronto",
            "2025-11-02T05:30:00",
            5400,
            "2025-11-02T04:00:00.000000000",
            "2025-11-02T01:30:00.000000000",
        ),
        (
            "America/Toronto",
            "2025-11-02T06:30:00",
            5400,
            "2025-11-02T05:00:00.000000000",
            "2025-11-02T01:30:00.000000000",
        ),
        (
            "Asia/Kolkata",
            "2026-11-17T20:45:05.120",
            8105,
            "2026-11-17T18:30:00.120000000",
            "2026-11-18T02:15:05.120000000",
        ),
        (
            "Etc/UTC",
            "2026-11-17T23:59:59.999999999",
            86_399,
            "2026-11-17T00:00:00.999999999",
            "2026-11-17T23:59:59.999999999",
        ),
    ];
    let actual: Vec<(&str, &str, u32, String, String)> = rows
        .iter()
        .map(|(zone, at, ..)| {
            let (seconds, since, local) = engine::since_local_midnight(at, zone);
            (*zone, *at, seconds, since, local)
        })
        .collect();
    let wanted: Vec<(&str, &str, u32, String, String)> = rows
        .iter()
        .map(|(zone, at, seconds, since, local)| {
            (
                *zone,
                *at,
                *seconds,
                (*since).to_owned(),
                (*local).to_owned(),
            )
        })
        .collect();
    assert_eq!(
        actual, wanted,
        "seconds since local midnight per zone and instant"
    );
}

/// The check-in schedule, through the production rule: a cadence is due on
/// its weekdays inside the morning window of the athlete's own wall clock.
#[test]
fn a_check_in_is_due_on_the_athlete_s_wall_clock() {
    // zone, instant, due for [daily, three per week, weekly, off]
    let rows = [
        // Monday 2026-11-16 in Toronto, across the morning.
        ("America/Toronto", "2026-11-16T11:59:59", [0, 0, 0, 0]),
        ("America/Toronto", "2026-11-16T12:00:00", [0, 0, 0, 0]),
        ("America/Toronto", "2026-11-16T13:00:00", [1, 1, 1, 0]),
        ("America/Toronto", "2026-11-16T14:00:00", [1, 1, 1, 0]),
        ("America/Toronto", "2026-11-16T15:00:00", [1, 1, 1, 0]),
        ("America/Toronto", "2026-11-16T16:00:00", [0, 0, 0, 0]),
        // Tuesday and Wednesday at the same wall-clock hour.
        ("America/Toronto", "2026-11-17T13:00:00", [1, 0, 0, 0]),
        ("America/Toronto", "2026-11-18T13:00:00", [1, 1, 0, 0]),
        // The same instants are another weekday and hour in Kolkata.
        ("Asia/Kolkata", "2026-11-16T02:30:00", [1, 1, 1, 0]),
        ("Asia/Kolkata", "2026-11-15T20:30:00", [0, 0, 0, 0]),
        ("Asia/Kolkata", "2026-11-16T13:00:00", [0, 0, 0, 0]),
    ];
    let cadences = [
        CheckInCadence::Daily,
        CheckInCadence::ThreePerWeek,
        CheckInCadence::Weekly,
        CheckInCadence::Off,
    ];
    let actual: Vec<(&str, &str, [u8; 4])> = rows
        .iter()
        .map(|(zone, at, _)| {
            let local = engine::local_clock(instant(at), zone);
            (
                *zone,
                *at,
                cadences.map(|cadence| u8::from(cadence.due_at(local))),
            )
        })
        .collect();
    assert_eq!(actual, rows.to_vec(), "cadences due per zone and instant");
}

/// An instant plus or minus a lifetime either lands inside the calendar or is
/// refused; it never wraps.
#[test]
fn adding_a_lifetime_refuses_an_overflow() {
    // instant, days, [plus, minus]
    let rows = [
        (
            "2026-11-17T10:00:05.123456789",
            0,
            [
                "2026-11-17T10:00:05.123456789",
                "2026-11-17T10:00:05.123456789",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            1,
            [
                "2026-11-18T10:00:05.123456789",
                "2026-11-16T10:00:05.123456789",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            365,
            [
                "2027-11-17T10:00:05.123456789",
                "2025-11-17T10:00:05.123456789",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            36_500,
            [
                "2126-10-24T10:00:05.123456789",
                "1926-12-12T10:00:05.123456789",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            -1,
            [
                "2026-11-16T10:00:05.123456789",
                "2026-11-18T10:00:05.123456789",
            ],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            95_000_000,
            ["none", "none"],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            96_000_000,
            ["none", "none"],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            97_000_000,
            ["none", "none"],
        ),
        (
            "2026-11-17T10:00:05.123456789",
            i64::MAX,
            ["no such duration", "no such duration"],
        ),
    ];
    let actual: Vec<(&str, i64, [String; 2])> = rows
        .iter()
        .map(|(at, days, _)| (*at, *days, engine::checked_days(at, *days)))
        .collect();
    let wanted: Vec<(&str, i64, [String; 2])> = rows
        .iter()
        .map(|(at, days, moved)| (*at, *days, moved.map(str::to_owned)))
        .collect();
    assert_eq!(
        actual, wanted,
        "plus and minus a lifetime of days per instant"
    );
}

/// The same rule through production: when an admin token minted now expires.
#[test]
fn an_admin_token_lifetime_ends_exactly_that_many_days_later_or_is_refused() {
    let now = instant("2026-11-17T10:00:05.123456789");
    // lifetime in days, verdict
    let rows = [
        (None, "never"),
        (Some(0), "never"),
        (Some(1), "2026-11-18T10:00:05.123456789"),
        (Some(30), "2026-12-17T10:00:05.123456789"),
        (Some(365), "2027-11-17T10:00:05.123456789"),
        (Some(36_500), "2126-10-24T10:00:05.123456789"),
        (Some(95_000_000), "refused"),
        (Some(96_000_000), "refused"),
        (Some(u64::MAX), "refused"),
    ];
    let actual: Vec<(String, String)> = rows
        .iter()
        .map(|(days, _)| {
            let mut request = CreateAdminTokenRequest::new("golden".to_owned());
            request.expires_in_days = *days;
            let verdict = match request.expires_at(now) {
                Ok(None) => "never".to_owned(),
                Ok(Some(at)) => show(at),
                Err(_) => "refused".to_owned(),
            };
            (format!("{days:?}"), verdict)
        })
        .collect();
    let wanted: Vec<(String, String)> = rows
        .iter()
        .map(|(days, verdict)| (format!("{days:?}"), (*verdict).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "admin token expiry per lifetime");
}

/// A `std` duration becomes a signed one unless it is longer than the signed
/// one can hold; a negative signed duration has no `std` form.
#[test]
fn std_durations_convert_both_ways_within_range() {
    // seconds, nanoseconds, as a signed duration
    let into = [
        (0, 0, "0s+0ns"),
        (1, 500_000_000, "1s+500000000ns"),
        (86_400, 0, "86400s+0ns"),
        (0, 999_999_999, "0s+999999999ns"),
        (
            9_223_372_036_854_775,
            807_000_000,
            "9223372036854775s+807000000ns",
        ),
        (9_223_372_036_854_775, 807_000_001, "rejected"),
        (9_223_372_036_854_776, 0, "rejected"),
        (u64::MAX, 0, "rejected"),
    ];
    let actual: Vec<(u64, u32, String)> = into
        .iter()
        .map(|(seconds, nanos, _)| (*seconds, *nanos, engine::from_std(*seconds, *nanos)))
        .collect();
    let wanted: Vec<(u64, u32, String)> = into
        .iter()
        .map(|(seconds, nanos, verdict)| (*seconds, *nanos, (*verdict).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "std duration into a signed one");

    // milliseconds, as a std duration
    let back = [
        (0, "0s+0ns"),
        (1, "0s+1000000ns"),
        (1_500, "1s+500000000ns"),
        (86_400_000, "86400s+0ns"),
        (-1, "rejected"),
        (-1_500, "rejected"),
    ];
    let actual: Vec<(i64, String)> = back
        .iter()
        .map(|(millis, _)| (*millis, engine::to_std(*millis)))
        .collect();
    let wanted: Vec<(i64, String)> = back
        .iter()
        .map(|(millis, verdict)| (*millis, (*verdict).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "signed milliseconds into a std duration");
}

/// Walking days is inclusive of both ends, crosses month and year ends, and
/// gives nothing when the end is before the start.
#[test]
fn every_day_from_a_date_through_another() {
    // from, to, the days joined by a space
    let rows = [
        (
            "2024-02-27",
            "2024-03-01",
            "2024-02-27 2024-02-28 2024-02-29 2024-03-01",
        ),
        (
            "2026-02-27",
            "2026-03-01",
            "2026-02-27 2026-02-28 2026-03-01",
        ),
        (
            "2026-12-30",
            "2027-01-02",
            "2026-12-30 2026-12-31 2027-01-01 2027-01-02",
        ),
        ("2026-11-17", "2026-11-17", "2026-11-17"),
        ("2026-11-17", "2026-11-16", ""),
    ];
    let actual: Vec<(&str, &str, String)> = rows
        .iter()
        .map(|(from, to, _)| (*from, *to, engine::days_through(from, to).join(" ")))
        .collect();
    let wanted: Vec<(&str, &str, String)> = rows
        .iter()
        .map(|(from, to, days)| (*from, *to, (*days).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "days walked from one date through another");
}

/// A difference in whole units truncates toward zero below the millisecond
/// too: 1.9999 ms is 1, and -1.9999 ms is -1, not -2.
#[test]
fn a_difference_truncates_below_the_millisecond() {
    // later, earlier, [seconds, milliseconds, microseconds, nanoseconds]
    let rows = [
        (
            "2026-11-17T10:00:05.001999900",
            "2026-11-17T10:00:05",
            [0, 1, 1999, 1_999_900],
        ),
        (
            "2026-11-17T10:00:05",
            "2026-11-17T10:00:05.001999900",
            [0, -1, -1999, -1_999_900],
        ),
        (
            "2026-11-17T10:00:05.000999999",
            "2026-11-17T10:00:05",
            [0, 0, 999, 999_999],
        ),
        (
            "2026-11-17T10:00:05",
            "2026-11-17T10:00:05.000999999",
            [0, 0, -999, -999_999],
        ),
        (
            "2026-11-17T10:00:06.999999999",
            "2026-11-17T10:00:05",
            [1, 1999, 1_999_999, 1_999_999_999],
        ),
        (
            "2026-11-17T10:00:05",
            "2026-11-17T10:00:06.999999999",
            [-1, -1999, -1_999_999, -1_999_999_999],
        ),
        (
            "2026-11-17T10:00:05.000000001",
            "2026-11-17T10:00:05",
            [0, 0, 0, 1],
        ),
        (
            "1970-01-01T00:00:00.000000500",
            "1969-12-31T23:59:59.999999500",
            [0, 0, 1, 1000],
        ),
    ];
    let actual: Vec<(&str, &str, [i64; 4])> = rows
        .iter()
        .map(|(later, earlier, _)| (*later, *earlier, engine::difference(later, earlier)))
        .collect();
    assert_eq!(actual, rows.to_vec(), "whole units of each difference");
}
