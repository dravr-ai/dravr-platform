// ABOUTME: Pins the hand-written date parsers and JSON into a value that keeps its offset
// ABOUTME: Recorded from the date engine in use on 2026-11-17, with leap seconds as epoch nanoseconds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Inbound goldens for the parsers `date_inbound_golden_test` does not reach
//! (carnet#1620, gate 7).
//!
//! Production reads dates with four hand-written parsers besides
//! `parse_from_rfc3339` and `%Y-%m-%d`, and reads JSON into a value that keeps
//! the offset it was written with. Each has its own leniency, recorded here by
//! running it:
//!
//! | Parser | Production call site | Asserted through production in |
//! |---|---|---|
//! | `%Y-%m-%d %H:%M:%S%.f%#z` | `parse_step_time` (`pierre-database`) | `date_stamp_golden_sqlite_test` |
//! | `%Y-%m-%d %H:%M:%S%.f` | `parse_step_time` (`pierre-database`) | `date_stamp_golden_sqlite_test` |
//! | `%Y-%m-%dT%H:%M:%S` | `parse_local_dt`, `wall_clock_in_zone` (`pierre-providers`) | `date_provider_inbound_golden_test` |
//! | `"±HH:MM".parse()` | `whoop_utc_offset` (`pierre-providers`) | `date_provider_inbound_golden_test` |
//! | JSON, offset kept | `TerraMetadata` (`pierre-providers`), stored goals | `date_provider_inbound_golden_test` |
//!
//! Those suites give the verdict of the whole production function, which
//! trims, tries RFC 3339 first, or falls back. The tables here give each
//! parser's verdict alone, on a wider list, so a replacement parser can be
//! compared one pattern at a time.
//!
//! A verdict is `rejected`, or the value read. Where an offset is read the
//! verdict also records it, as seconds east of UTC, because two inputs can
//! name the same instant and differ only in the offset kept.
//!
//! Everything that names the date engine lives in [`engine`].

use crate::date_inbound_inputs::INBOUND_INPUTS;

/// Everything in this file that names the date engine.
mod engine {
    use dravr_tronc::date_pattern;
    use dravr_tronc::dates::{CivilDateTime, Offset, Pattern, Timestamp};

    const REJECTED: &str = "rejected";

    /// An instant in UTC, to the nanosecond.
    const UTC_NANOS: Pattern<Timestamp> = date_pattern!(Timestamp, "%Y-%m-%dT%H:%M:%S%.9fZ");

    /// A wall clock, to the nanosecond.
    const ZONELESS_NANOS: Pattern<CivilDateTime> =
        date_pattern!(CivilDateTime, "%Y-%m-%dT%H:%M:%S%.9f");

    /// The instant in UTC and the offset it was written with.
    fn kept((value, offset): (Timestamp, Offset)) -> String {
        format!("{} @ {}", value.format_with(&UTC_NANOS), offset.seconds())
    }

    fn zoneless(value: CivilDateTime) -> String {
        value.format_with(&ZONELESS_NANOS)
    }

    /// The text as a JSON string into a field that keeps its offset: what
    /// serde reads, with the offset [`Timestamp::parse_with_offset`] keeps.
    pub(super) fn json_fixed_offset(text: &str) -> String {
        let quoted = serde_json::Value::String(text.to_owned()).to_string();
        serde_json::from_str::<Timestamp>(&quoted)
            .ok()
            .and_then(|_| Timestamp::parse_with_offset(text).ok())
            .map_or_else(|| REJECTED.to_owned(), kept)
    }

    /// The shape `PostgreSQL` renders a `TIMESTAMPTZ` in when cast to text.
    pub(super) fn offset_pattern(text: &str) -> String {
        Timestamp::parse_from_str_with_offset(text, "%Y-%m-%d %H:%M:%S%.f%#z")
            .map_or_else(|_| REJECTED.to_owned(), kept)
    }

    /// The shape `SQLite`'s `CURRENT_TIMESTAMP` writes, with an optional fraction.
    pub(super) fn zoneless_space_pattern(text: &str) -> String {
        CivilDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
            .map_or_else(|_| REJECTED.to_owned(), zoneless)
    }

    /// The wall clock Intervals.icu sends and takes in its queries.
    pub(super) fn zoneless_t_pattern(text: &str) -> String {
        CivilDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S")
            .map_or_else(|_| REJECTED.to_owned(), zoneless)
    }

    /// An offset on its own, as seconds east of UTC.
    pub(super) fn offset_alone(text: &str) -> String {
        text.parse::<Offset>().map_or_else(
            |_| REJECTED.to_owned(),
            |offset| offset.seconds().to_string(),
        )
    }

    /// One second-60 input as: the text it shows as, whole seconds from the
    /// epoch, nanoseconds from the epoch, the JSON it serializes to, and the
    /// hand-formatted text.
    pub(super) fn leap_second(text: &str) -> [String; 5] {
        let quoted = serde_json::Value::String(text.to_owned()).to_string();
        serde_json::from_str::<Timestamp>(&quoted).map_or_else(
            |_| [REJECTED; 5].map(str::to_owned),
            |value| {
                [
                    value.format_with(&UTC_NANOS),
                    value.epoch_seconds().to_string(),
                    i64::try_from(value.epoch_nanos())
                        .map_or_else(|_| "none".to_owned(), |nanos| nanos.to_string()),
                    serde_json::to_string(&value).unwrap(),
                    value.to_rfc3339(),
                ]
            },
        )
    }

    /// Nanoseconds from the epoch for a plain instant, to compare against.
    pub(super) fn epoch_nanos(text: &str) -> String {
        i64::try_from(Timestamp::parse_from_rfc3339(text).unwrap().epoch_nanos())
            .unwrap()
            .to_string()
    }
}

/// Runs every input through one parser and compares the whole table at once,
/// so a failure prints every verdict that moved.
fn assert_verdicts(parser: &str, parse: fn(&str) -> String, expected: &[(&str, &str)]) {
    let actual: Vec<(&str, String)> = expected
        .iter()
        .map(|(input, _)| (*input, parse(input)))
        .collect();
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(input, verdict)| (*input, (*verdict).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "{parser} changed a verdict");
}

/// JSON into a value that keeps its offset answers the shared input list with
/// the same accept-or-reject verdicts as JSON into a UTC instant, and records
/// which offset it kept. `Z`, `+00:00`, `-00:00` and `UTC` all keep zero.
#[test]
fn json_into_a_fixed_offset_value_keeps_the_offset_it_was_written_with() {
    let expected: [(&str, &str); 54] = [
        ("2026-11-17T10:00:05Z", "2026-11-17T10:00:05.000000000Z @ 0"),
        (
            "2026-11-17T10:00:05+00:00",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T05:00:05-05:00",
            "2026-11-17T10:00:05.000000000Z @ -18000",
        ),
        (
            "2026-11-17T15:30:05+05:30",
            "2026-11-17T10:00:05.000000000Z @ 19800",
        ),
        (
            "2026-11-17T10:00:05-00:00",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05+0000",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T05:00:05-0500",
            "2026-11-17T10:00:05.000000000Z @ -18000",
        ),
        ("2026-11-17T10:00:05+00", "rejected"),
        (
            "2026-11-17T10:00:05 +00:00",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        ("2026-11-17t10:00:05z", "2026-11-17T10:00:05.000000000Z @ 0"),
        ("2026-11-17T10:00:05z", "2026-11-17T10:00:05.000000000Z @ 0"),
        ("2026-11-17t10:00:05Z", "2026-11-17T10:00:05.000000000Z @ 0"),
        ("2026-11-17 10:00:05Z", "2026-11-17T10:00:05.000000000Z @ 0"),
        (
            "2026-11-17 10:00:05+00:00",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17  10:00:05Z",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        ("2026-11-17_10:00:05Z", "rejected"),
        (
            "2026-11-17 10:00:05 UTC",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05 UTC",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05UTC",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05 utc",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        ("2026-11-17T10:00:05 GMT", "rejected"),
        ("2026-11-17T05:00:05 EST", "rejected"),
        ("2026-1-7T9:0:5Z", "2026-01-07T09:00:05.000000000Z @ 0"),
        ("2026-11-17T9:00:05Z", "2026-11-17T09:00:05.000000000Z @ 0"),
        ("2026-11-17T10:00:05", "rejected"),
        ("2026-11-17 10:00:05", "rejected"),
        ("2026-11-17T10:00Z", "rejected"),
        ("2026-11-17", "rejected"),
        ("2026-11-17Z", "rejected"),
        ("", "rejected"),
        (
            "2026-11-17T10:00:05.1Z",
            "2026-11-17T10:00:05.100000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.12Z",
            "2026-11-17T10:00:05.120000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.123Z",
            "2026-11-17T10:00:05.123000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.1234Z",
            "2026-11-17T10:00:05.123400000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.12345Z",
            "2026-11-17T10:00:05.123450000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.123456Z",
            "2026-11-17T10:00:05.123456000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.1234567Z",
            "2026-11-17T10:00:05.123456700Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.12345678Z",
            "2026-11-17T10:00:05.123456780Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.123456789Z",
            "2026-11-17T10:00:05.123456789Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.1234567891Z",
            "2026-11-17T10:00:05.123456789Z @ 0",
        ),
        (
            "2026-11-17T10:00:05.100+00:00",
            "2026-11-17T10:00:05.100000000Z @ 0",
        ),
        ("2026-11-17T10:00:05,123Z", "rejected"),
        ("2026-11-17T10:00:05.Z", "rejected"),
        ("2016-12-31T23:59:60Z", "2016-12-31T23:59:59.000000000Z @ 0"),
        ("2026-11-17T10:00:60Z", "2026-11-17T10:00:59.000000000Z @ 0"),
        ("2026-11-17T24:00:00Z", "rejected"),
        ("2026-02-30T10:00:05Z", "rejected"),
        ("2025-02-29T10:00:05Z", "rejected"),
        ("2026-13-01T10:00:05Z", "rejected"),
        (
            " 2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        (
            "2026-11-17T10:00:05Z ",
            "2026-11-17T10:00:05.000000000Z @ 0",
        ),
        ("20261117T100005Z", "rejected"),
        ("Tue, 17 Nov 2026 10:00:05 GMT", "rejected"),
        ("1794909605", "rejected"),
    ];
    let named: Vec<&str> = expected.iter().map(|(input, _)| *input).collect();
    assert_eq!(
        named, INBOUND_INPUTS,
        "the rows here are the shared inbound input list, in order"
    );
    assert_verdicts(
        "JSON into a fixed-offset value",
        engine::json_fixed_offset,
        &expected,
    );
}

/// `%#z` takes an offset of hours alone, hours and minutes, with or without a
/// colon, and `Z`; the fraction may have any number of digits or be absent.
#[test]
fn the_offset_pattern_reads_what_postgresql_renders() {
    assert_verdicts(
        "%Y-%m-%d %H:%M:%S%.f%#z",
        engine::offset_pattern,
        &[
            (
                "2026-11-13 10:04:05+00",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            (
                "2026-11-13 05:04:05-05",
                "2026-11-13T10:04:05.000000000Z @ -18000",
            ),
            (
                "2026-11-13 15:34:05+05:30",
                "2026-11-13T10:04:05.000000000Z @ 19800",
            ),
            (
                "2026-11-13 10:04:05.250000+00",
                "2026-11-13T10:04:05.250000000Z @ 0",
            ),
            (
                "2026-11-13 10:04:05.25+00",
                "2026-11-13T10:04:05.250000000Z @ 0",
            ),
            (
                "2026-11-13 10:04:05.123456789+00",
                "2026-11-13T10:04:05.123456789Z @ 0",
            ),
            (
                "2026-11-13 10:04:05.1234567891+00",
                "2026-11-13T10:04:05.123456789Z @ 0",
            ),
            (
                "2026-11-13 10:04:05+0000",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            (
                "2026-11-13 10:04:05+00:00",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            (
                "2026-11-13 10:04:05-00",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            ("2026-11-13 10:04:05Z", "2026-11-13T10:04:05.000000000Z @ 0"),
            ("2026-11-13 10:04:05z", "2026-11-13T10:04:05.000000000Z @ 0"),
            (
                "2026-11-13 10:04:05 +00",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            ("2026-11-13 10:04:05", "rejected"),
            ("2026-11-13T10:04:05+00", "rejected"),
            ("2026-11-13 10:04+00", "rejected"),
            ("2026-11-13 10:04:05+5", "rejected"),
            ("2026-11-13 10:04:05+05:3", "rejected"),
            (
                "2026-11-13 10:04:60+00",
                "2026-11-13T10:04:59.000000000Z @ 0",
            ),
            ("2026-11-13 24:00:00+00", "rejected"),
            ("2026-11-13 10:04:05,250+00", "rejected"),
            (
                " 2026-11-13 10:04:05+00",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            (
                "2026-11-13 10:04:05+00 ",
                "2026-11-13T10:04:05.000000000Z @ 0",
            ),
            ("2026-11-13 10:04:05 UTC", "rejected"),
            ("2026-1-3 9:4:5+00", "2026-01-03T09:04:05.000000000Z @ 0"),
            ("2026-02-30 10:04:05+00", "rejected"),
            ("", "rejected"),
            ("yesterday", "rejected"),
        ],
    );
}

#[test]
fn the_zoneless_space_pattern_reads_what_sqlite_defaults_write() {
    assert_verdicts(
        "%Y-%m-%d %H:%M:%S%.f",
        engine::zoneless_space_pattern,
        &[
            ("2026-11-13 10:04:05", "2026-11-13T10:04:05.000000000"),
            ("2026-11-13 10:04:05.250", "2026-11-13T10:04:05.250000000"),
            (
                "2026-11-13 10:04:05.123456789",
                "2026-11-13T10:04:05.123456789",
            ),
            (
                "2026-11-13 10:04:05.1234567891",
                "2026-11-13T10:04:05.123456789",
            ),
            ("2026-11-13 10:04:05.", "rejected"),
            ("2026-11-13T10:04:05", "rejected"),
            ("2026-11-13 10:04", "rejected"),
            ("2026-11-13 10:04:05+00", "rejected"),
            ("2026-11-13 10:04:05Z", "rejected"),
            ("2026-11-13 10:04:60", "2026-11-13T10:04:59.000000000"),
            ("2026-11-13 24:00:00", "rejected"),
            ("2026-1-3 9:4:5", "2026-01-03T09:04:05.000000000"),
            (" 2026-11-13 10:04:05", "2026-11-13T10:04:05.000000000"),
            ("2026-11-13 10:04:05 ", "rejected"),
            ("2026-11-13  10:04:05", "2026-11-13T10:04:05.000000000"),
            ("2026-02-30 10:04:05", "rejected"),
            ("2026-11-13", "rejected"),
            ("", "rejected"),
        ],
    );
}

#[test]
fn the_zoneless_t_pattern_reads_a_wall_clock_to_the_second() {
    assert_verdicts(
        "%Y-%m-%dT%H:%M:%S",
        engine::zoneless_t_pattern,
        &[
            ("2026-08-30T10:00:00", "2026-08-30T10:00:00.000000000"),
            ("2026-08-30T10:00:00.5", "rejected"),
            ("2026-08-30T10:00:00Z", "rejected"),
            ("2026-08-30T10:00:00+02:00", "rejected"),
            ("2026-08-30 10:00:00", "rejected"),
            ("2026-08-30t10:00:00", "rejected"),
            ("2026-08-30T10:00", "rejected"),
            ("2026-08-30T10:00:60", "2026-08-30T10:00:59.000000000"),
            ("2026-08-30T24:00:00", "rejected"),
            ("2026-8-30T9:0:0", "2026-08-30T09:00:00.000000000"),
            (" 2026-08-30T10:00:00", "2026-08-30T10:00:00.000000000"),
            ("2026-08-30T10:00:00 ", "rejected"),
            ("2026-02-30T10:00:00", "rejected"),
            ("2026-08-30", "rejected"),
            ("", "rejected"),
        ],
    );
}

#[test]
fn an_offset_alone_parses_to_seconds_east() {
    assert_verdicts(
        "FixedOffset::from_str",
        engine::offset_alone,
        &[
            ("+00:00", "0"),
            ("-00:00", "0"),
            ("-05:00", "-18000"),
            ("+05:30", "19800"),
            ("+05:45", "20700"),
            ("+14:00", "50400"),
            ("-12:00", "-43200"),
            ("+23:59", "86340"),
            ("-23:59", "-86340"),
            ("+24:00", "rejected"),
            ("+0530", "19800"),
            ("+05", "rejected"),
            ("+5:30", "rejected"),
            ("+05:3", "rejected"),
            ("05:30", "rejected"),
            ("+05:30:15", "19800"),
            ("Z", "rejected"),
            ("z", "rejected"),
            ("UTC", "rejected"),
            (" +05:30", "rejected"),
            ("+05:30 ", "19800"),
            ("+05 30", "19800"),
            ("−05:00", "-18000"),
            ("", "rejected"),
        ],
    );
}

/// Second 60 is accepted on any minute and read as second 59 of that minute:
/// it prints as `:59`, serializes as `:59`, and counts the nanoseconds of
/// second 59 from the epoch. chrono kept second 60 as a value of its own and
/// wrote `:60` back; the date types hold no leap second (carnet#1620,
/// accepted 2026-11-18).
#[test]
fn a_leap_second_is_recorded_as_text_and_as_epoch_nanoseconds() {
    // input, [shown, epoch seconds, epoch nanoseconds, JSON, hand formatted]
    let rows: [(&str, [&str; 5]); 5] = [
        (
            "2016-12-31T23:59:60Z",
            [
                "2016-12-31T23:59:59.000000000Z",
                "1483228799",
                "1483228799000000000",
                r#""2016-12-31T23:59:59Z""#,
                "2016-12-31T23:59:59+00:00",
            ],
        ),
        (
            "2016-12-31T23:59:60.500Z",
            [
                "2016-12-31T23:59:59.500000000Z",
                "1483228799",
                "1483228799500000000",
                r#""2016-12-31T23:59:59.500Z""#,
                "2016-12-31T23:59:59.500+00:00",
            ],
        ),
        (
            "2026-11-17T10:00:60Z",
            [
                "2026-11-17T10:00:59.000000000Z",
                "1794909659",
                "1794909659000000000",
                r#""2026-11-17T10:00:59Z""#,
                "2026-11-17T10:00:59+00:00",
            ],
        ),
        (
            "2016-12-31T18:59:60-05:00",
            [
                "2016-12-31T23:59:59.000000000Z",
                "1483228799",
                "1483228799000000000",
                r#""2016-12-31T23:59:59Z""#,
                "2016-12-31T23:59:59+00:00",
            ],
        ),
        (
            "2016-12-31T23:59:59Z",
            [
                "2016-12-31T23:59:59.000000000Z",
                "1483228799",
                "1483228799000000000",
                r#""2016-12-31T23:59:59Z""#,
                "2016-12-31T23:59:59+00:00",
            ],
        ),
    ];
    let actual: Vec<(&str, [String; 5])> = rows
        .iter()
        .map(|(input, _)| (*input, engine::leap_second(input)))
        .collect();
    let wanted: Vec<(&str, [String; 5])> = rows
        .iter()
        .map(|(input, facts)| (*input, facts.map(str::to_owned)))
        .collect();
    assert_eq!(actual, wanted, "second-60 facts per input");

    // The nanosecond column is second 59's, and the half-second row is half
    // a second after it.
    assert_eq!(
        engine::epoch_nanos("2016-12-31T23:59:59Z"),
        rows[0].1[2],
        "23:59:60 counts as 23:59:59"
    );
    assert_eq!(
        engine::epoch_nanos("2016-12-31T23:59:59.5Z"),
        rows[1].1[2],
        "23:59:60.5 counts as 23:59:59.5"
    );
    assert_eq!(
        engine::epoch_nanos("2026-11-17T10:00:59Z"),
        rows[2].1[2],
        "10:00:60 counts as 10:00:59"
    );
}
