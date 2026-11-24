// ABOUTME: Pins which date strings the platform accepts, and the value each one means, on both text parsers
// ABOUTME: Recorded from the date engine in use on 2026-11-17 so a later engine gives the same verdicts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Inbound goldens for dates (carnet#1620, gate 7, the two text parsers).
//!
//! What we emit is one shape; what we accept is many, and nobody chose the
//! list. It is whatever the parser tolerates, and clients, stored documents
//! and provider payloads already rely on it. Two parsers are in play and they
//! disagree:
//!
//! - **JSON into an instant field** is lenient: a space for the `T`, an offset
//!   without its colon and unpadded digits all pass;
//! - **the strict parser** (`parse_from_rfc3339`) takes RFC 3339 only.
//!
//! Each row carries the verdict recorded by running it: `rejected`, or the
//! instant the text means, written at nanosecond precision in UTC. A row that
//! changes verdict after an engine change is either a regression or a
//! tightening somebody reviewed and listed, never a side effect.
//!
//! The third inbound path, `SQLite` text into an instant, is pinned in
//! `pierre-database` (`date_decode_golden_sqlite_test`), next to the
//! database. All three paths answer the one input list in
//! `date_inbound_inputs.rs`.
//!
//! Two tables are also asked of production code, so a call site translated
//! differently from the helper turns red: the JSON rules through a model's
//! instant field, the strict parser through `StoreCursor::decode`.
//!
//! Everything that names the date engine lives in [`engine`].

use crate::date_inbound_inputs::INBOUND_INPUTS;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use pierre_core::models::SleepStage;
use pierre_core::pagination::{Cursor, StoreCursor, StoreSortOrder};

/// Everything in this file that names the date engine.
mod engine {
    use dravr_tronc::dates::{CivilDate, CivilTime, Timestamp, Weekday};

    const REJECTED: &str = "rejected";

    /// The instant at nanosecond precision in UTC, the shape every verdict is
    /// written in.
    pub(super) fn canonical(value: Timestamp) -> String {
        value.format("%Y-%m-%dT%H:%M:%S%.9fZ").unwrap()
    }

    /// The lenient path: the text as a JSON string into an instant field.
    pub(super) fn json_instant(text: &str) -> String {
        let quoted = serde_json::Value::String(text.to_owned()).to_string();
        serde_json::from_str::<Timestamp>(&quoted).map_or_else(|_| REJECTED.to_owned(), canonical)
    }

    /// Any JSON document into an instant field: a number, `null`, an object.
    pub(super) fn json_document_instant(document: &str) -> String {
        serde_json::from_str::<Timestamp>(document).map_or_else(|_| REJECTED.to_owned(), canonical)
    }

    /// The strict path: `parse_from_rfc3339`, read as a UTC instant.
    pub(super) fn strict_instant(text: &str) -> String {
        Timestamp::parse_from_rfc3339(text).map_or_else(|_| REJECTED.to_owned(), canonical)
    }

    /// The text as a JSON string into a calendar-date field.
    pub(super) fn json_date(text: &str) -> String {
        let quoted = serde_json::Value::String(text.to_owned()).to_string();
        serde_json::from_str::<CivilDate>(&quoted)
            .map_or_else(|_| REJECTED.to_owned(), |date| date.to_string())
    }

    /// The hand parser for a calendar date: `parse_from_str` with `%Y-%m-%d`.
    pub(super) fn pattern_date(text: &str) -> String {
        CivilDate::parse_from_str(text, "%Y-%m-%d")
            .map_or_else(|_| REJECTED.to_owned(), |date| date.to_string())
    }

    /// The text as a JSON string into a time-of-day field.
    pub(super) fn json_time(text: &str) -> String {
        let quoted = serde_json::Value::String(text.to_owned()).to_string();
        serde_json::from_str::<CivilTime>(&quoted).map_or_else(
            |_| REJECTED.to_owned(),
            |time| time.format("%H:%M:%S%.9f").unwrap(),
        )
    }

    /// The text as a JSON string into a weekday field, as days from Monday.
    pub(super) fn json_weekday(text: &str) -> String {
        let quoted = serde_json::Value::String(text.to_owned()).to_string();
        serde_json::from_str::<Weekday>(&quoted).map_or_else(
            |_| REJECTED.to_owned(),
            |day| day.num_days_from_monday().to_string(),
        )
    }
}

/// A verdict table that must name exactly the shared inputs, in their order,
/// so no path can skip a row the others answer.
fn assert_shared_verdicts(parser: &str, parse: fn(&str) -> String, expected: &[(&str, &str)]) {
    let named: Vec<&str> = expected.iter().map(|(input, _)| *input).collect();
    assert_eq!(
        named, INBOUND_INPUTS,
        "{parser} does not answer the shared inputs"
    );
    assert_verdicts(parser, parse, expected);
}

/// A sleep stage document with `text` as its start, read into the production
/// model.
fn sleep_stage_start(text: &str) -> String {
    let document = serde_json::json!({
        "stage_type": "deep",
        "start_time": text,
        "duration_minutes": 64,
    });
    serde_json::from_value::<SleepStage>(document).map_or_else(
        |_| REJECTED.to_owned(),
        |stage| engine::canonical(stage.start_time),
    )
}

/// A store cursor whose timestamp is `text`, decoded by production. An empty
/// timestamp is a cursor with no date, which reads as `rejected` here: no
/// instant came out of it.
fn store_cursor_instant(text: &str) -> String {
    let raw = URL_SAFE_NO_PAD.encode(format!("newest|{text}|agent-1"));
    StoreCursor::decode(&Cursor::from_string(raw), StoreSortOrder::Newest)
        .and_then(|cursor| cursor.published_at)
        .map_or_else(|| REJECTED.to_owned(), engine::canonical)
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

const TEN: &str = "2026-11-17T10:00:05.000000000Z";
const REJECTED: &str = "rejected";

/// What JSON into an instant answers for every shared input.
const JSON_VERDICTS: [(&str, &str); 54] = [
    // The two shapes we emit.
    ("2026-11-17T10:00:05Z", TEN),
    ("2026-11-17T10:00:05+00:00", TEN),
    // Offsets.
    ("2026-11-17T05:00:05-05:00", TEN),
    ("2026-11-17T15:30:05+05:30", TEN),
    ("2026-11-17T10:00:05-00:00", TEN),
    ("2026-11-17T10:00:05+0000", TEN),
    ("2026-11-17T05:00:05-0500", TEN),
    ("2026-11-17T10:00:05+00", REJECTED),
    ("2026-11-17T10:00:05 +00:00", TEN),
    // Case and separator.
    ("2026-11-17t10:00:05z", TEN),
    ("2026-11-17T10:00:05z", TEN),
    ("2026-11-17t10:00:05Z", TEN),
    ("2026-11-17 10:00:05Z", TEN),
    ("2026-11-17 10:00:05+00:00", TEN),
    ("2026-11-17  10:00:05Z", TEN),
    ("2026-11-17_10:00:05Z", REJECTED),
    // A zone name instead of an offset.
    ("2026-11-17 10:00:05 UTC", TEN),
    ("2026-11-17T10:00:05 UTC", TEN),
    ("2026-11-17T10:00:05UTC", TEN),
    ("2026-11-17T10:00:05 utc", TEN),
    ("2026-11-17T10:00:05 GMT", REJECTED),
    ("2026-11-17T05:00:05 EST", REJECTED),
    // Unpadded digits.
    ("2026-1-7T9:0:5Z", "2026-01-07T09:00:05.000000000Z"),
    ("2026-11-17T9:00:05Z", "2026-11-17T09:00:05.000000000Z"),
    // Missing parts.
    ("2026-11-17T10:00:05", REJECTED),
    ("2026-11-17 10:00:05", REJECTED),
    ("2026-11-17T10:00Z", REJECTED),
    ("2026-11-17", REJECTED),
    ("2026-11-17Z", REJECTED),
    ("", REJECTED),
    // One to ten fraction digits, and the other separators.
    ("2026-11-17T10:00:05.1Z", "2026-11-17T10:00:05.100000000Z"),
    ("2026-11-17T10:00:05.12Z", "2026-11-17T10:00:05.120000000Z"),
    ("2026-11-17T10:00:05.123Z", "2026-11-17T10:00:05.123000000Z"),
    (
        "2026-11-17T10:00:05.1234Z",
        "2026-11-17T10:00:05.123400000Z",
    ),
    (
        "2026-11-17T10:00:05.12345Z",
        "2026-11-17T10:00:05.123450000Z",
    ),
    (
        "2026-11-17T10:00:05.123456Z",
        "2026-11-17T10:00:05.123456000Z",
    ),
    (
        "2026-11-17T10:00:05.1234567Z",
        "2026-11-17T10:00:05.123456700Z",
    ),
    (
        "2026-11-17T10:00:05.12345678Z",
        "2026-11-17T10:00:05.123456780Z",
    ),
    (
        "2026-11-17T10:00:05.123456789Z",
        "2026-11-17T10:00:05.123456789Z",
    ),
    (
        "2026-11-17T10:00:05.1234567891Z",
        "2026-11-17T10:00:05.123456789Z",
    ),
    (
        "2026-11-17T10:00:05.100+00:00",
        "2026-11-17T10:00:05.100000000Z",
    ),
    ("2026-11-17T10:00:05,123Z", REJECTED),
    ("2026-11-17T10:00:05.Z", REJECTED),
    // A leap second, and values that are not on the calendar.
    ("2016-12-31T23:59:60Z", "2016-12-31T23:59:59.000000000Z"),
    ("2026-11-17T10:00:60Z", "2026-11-17T10:00:59.000000000Z"),
    ("2026-11-17T24:00:00Z", REJECTED),
    ("2026-02-30T10:00:05Z", REJECTED),
    ("2025-02-29T10:00:05Z", REJECTED),
    ("2026-13-01T10:00:05Z", REJECTED),
    // Surrounding whitespace and other formats.
    (" 2026-11-17T10:00:05Z", TEN),
    ("2026-11-17T10:00:05Z ", TEN),
    ("20261117T100005Z", REJECTED),
    ("Tue, 17 Nov 2026 10:00:05 GMT", REJECTED),
    ("1794909605", REJECTED),
];

#[test]
fn json_into_an_instant_accepts_the_relaxed_shapes() {
    assert_shared_verdicts("JSON into an instant", engine::json_instant, &JSON_VERDICTS);
}

/// The same rules reached through a production model: a sleep stage's start
/// is an instant field read by the derived deserializer.
#[test]
fn a_production_model_reads_an_instant_field_by_the_same_rules() {
    assert_shared_verdicts("SleepStage.start_time", sleep_stage_start, &JSON_VERDICTS);
}

#[test]
fn json_into_an_instant_takes_a_string_and_nothing_else() {
    assert_verdicts(
        "a JSON document into an instant",
        engine::json_document_instant,
        &[
            (r#""2026-11-17T10:00:05Z""#, TEN),
            ("1794909605", REJECTED),
            ("1794909605000", REJECTED),
            ("1794909605.5", REJECTED),
            ("null", REJECTED),
            (r#"{"at":"2026-11-17T10:00:05Z"}"#, REJECTED),
        ],
    );
}

/// What the strict parser answers for every shared input.
const STRICT_VERDICTS: [(&str, &str); 54] = [
    ("2026-11-17T10:00:05Z", TEN),
    ("2026-11-17T10:00:05+00:00", TEN),
    ("2026-11-17T05:00:05-05:00", TEN),
    ("2026-11-17T15:30:05+05:30", TEN),
    ("2026-11-17T10:00:05-00:00", TEN),
    ("2026-11-17T10:00:05+0000", REJECTED),
    ("2026-11-17T05:00:05-0500", REJECTED),
    ("2026-11-17T10:00:05+00", REJECTED),
    ("2026-11-17T10:00:05 +00:00", REJECTED),
    ("2026-11-17t10:00:05z", TEN),
    ("2026-11-17T10:00:05z", TEN),
    ("2026-11-17t10:00:05Z", TEN),
    ("2026-11-17 10:00:05Z", TEN),
    ("2026-11-17 10:00:05+00:00", TEN),
    ("2026-11-17  10:00:05Z", REJECTED),
    ("2026-11-17_10:00:05Z", REJECTED),
    ("2026-11-17 10:00:05 UTC", REJECTED),
    ("2026-11-17T10:00:05 UTC", REJECTED),
    ("2026-11-17T10:00:05UTC", REJECTED),
    ("2026-11-17T10:00:05 utc", REJECTED),
    ("2026-11-17T10:00:05 GMT", REJECTED),
    ("2026-11-17T05:00:05 EST", REJECTED),
    ("2026-1-7T9:0:5Z", REJECTED),
    ("2026-11-17T9:00:05Z", REJECTED),
    ("2026-11-17T10:00:05", REJECTED),
    ("2026-11-17 10:00:05", REJECTED),
    ("2026-11-17T10:00Z", REJECTED),
    ("2026-11-17", REJECTED),
    ("2026-11-17Z", REJECTED),
    ("", REJECTED),
    ("2026-11-17T10:00:05.1Z", "2026-11-17T10:00:05.100000000Z"),
    ("2026-11-17T10:00:05.12Z", "2026-11-17T10:00:05.120000000Z"),
    ("2026-11-17T10:00:05.123Z", "2026-11-17T10:00:05.123000000Z"),
    (
        "2026-11-17T10:00:05.1234Z",
        "2026-11-17T10:00:05.123400000Z",
    ),
    (
        "2026-11-17T10:00:05.12345Z",
        "2026-11-17T10:00:05.123450000Z",
    ),
    (
        "2026-11-17T10:00:05.123456Z",
        "2026-11-17T10:00:05.123456000Z",
    ),
    (
        "2026-11-17T10:00:05.1234567Z",
        "2026-11-17T10:00:05.123456700Z",
    ),
    (
        "2026-11-17T10:00:05.12345678Z",
        "2026-11-17T10:00:05.123456780Z",
    ),
    (
        "2026-11-17T10:00:05.123456789Z",
        "2026-11-17T10:00:05.123456789Z",
    ),
    (
        "2026-11-17T10:00:05.1234567891Z",
        "2026-11-17T10:00:05.123456789Z",
    ),
    (
        "2026-11-17T10:00:05.100+00:00",
        "2026-11-17T10:00:05.100000000Z",
    ),
    ("2026-11-17T10:00:05,123Z", REJECTED),
    ("2026-11-17T10:00:05.Z", REJECTED),
    ("2016-12-31T23:59:60Z", "2016-12-31T23:59:59.000000000Z"),
    ("2026-11-17T10:00:60Z", "2026-11-17T10:00:59.000000000Z"),
    ("2026-11-17T24:00:00Z", REJECTED),
    ("2026-02-30T10:00:05Z", REJECTED),
    ("2025-02-29T10:00:05Z", REJECTED),
    ("2026-13-01T10:00:05Z", REJECTED),
    (" 2026-11-17T10:00:05Z", REJECTED),
    ("2026-11-17T10:00:05Z ", REJECTED),
    ("20261117T100005Z", REJECTED),
    ("Tue, 17 Nov 2026 10:00:05 GMT", REJECTED),
    ("1794909605", REJECTED),
];

#[test]
fn the_strict_parser_takes_rfc_3339_only() {
    assert_shared_verdicts(
        "parse_from_rfc3339",
        engine::strict_instant,
        &STRICT_VERDICTS,
    );
}

/// The same parser reached through production: a store cursor carries its
/// timestamp as RFC 3339 text and `StoreCursor::decode` reads it back.
#[test]
fn a_store_cursor_reads_its_timestamp_with_the_strict_parser() {
    assert_shared_verdicts(
        "StoreCursor::decode",
        store_cursor_instant,
        &STRICT_VERDICTS,
    );
}

/// A two-digit year is read as the year 26, not 2026, and surrounding spaces
/// pass: both are recorded as they are, not endorsed.
#[test]
fn json_into_a_calendar_date_accepts_year_month_day() {
    assert_verdicts(
        "JSON into a calendar date",
        engine::json_date,
        &[
            ("2026-11-17", "2026-11-17"),
            ("2024-02-29", "2024-02-29"),
            ("2025-02-29", REJECTED),
            ("2026-02-30", REJECTED),
            ("2026-1-7", "2026-01-07"),
            ("2026-01-07", "2026-01-07"),
            ("26-11-17", "0026-11-17"),
            ("2026/11/17", REJECTED),
            ("20261117", REJECTED),
            ("2026-11-17T00:00:00", REJECTED),
            ("2026-11-17T00:00:00Z", REJECTED),
            ("2026-11-17 ", "2026-11-17"),
            (" 2026-11-17", "2026-11-17"),
            ("2026-11", REJECTED),
            ("+2026-11-17", "2026-11-17"),
            ("", REJECTED),
        ],
    );
}

/// The pattern parser differs from JSON on one row: a trailing space.
#[test]
fn the_pattern_parser_for_a_calendar_date_gives_its_own_verdicts() {
    assert_verdicts(
        "parse_from_str with %Y-%m-%d",
        engine::pattern_date,
        &[
            ("2026-11-17", "2026-11-17"),
            ("2024-02-29", "2024-02-29"),
            ("2025-02-29", REJECTED),
            ("2026-02-30", REJECTED),
            ("2026-1-7", "2026-01-07"),
            ("2026-01-07", "2026-01-07"),
            ("26-11-17", "0026-11-17"),
            ("2026/11/17", REJECTED),
            ("20261117", REJECTED),
            ("2026-11-17T00:00:00", REJECTED),
            ("2026-11-17T00:00:00Z", REJECTED),
            ("2026-11-17 ", REJECTED),
            (" 2026-11-17", "2026-11-17"),
            ("2026-11", REJECTED),
            ("+2026-11-17", "2026-11-17"),
            ("", REJECTED),
        ],
    );
}

#[test]
fn json_into_a_time_of_day_accepts_these_shapes() {
    assert_verdicts(
        "JSON into a time of day",
        engine::json_time,
        &[
            ("10:00:05", "10:00:05.000000000"),
            ("10:00", "10:00:00.000000000"),
            ("10:00:05.1", "10:00:05.100000000"),
            ("10:00:05.123456789", "10:00:05.123456789"),
            ("7:05:03", "07:05:03.000000000"),
            ("7:5:3", "07:05:03.000000000"),
            ("23:59:60", "23:59:59.000000000"),
            ("24:00:00", REJECTED),
            ("10", REJECTED),
            ("10:00:05Z", REJECTED),
            ("10:00 AM", REJECTED),
            ("T10:00:05", REJECTED),
            ("", REJECTED),
        ],
    );
}

#[test]
fn json_into_a_weekday_accepts_english_names_in_any_case() {
    assert_verdicts(
        "JSON into a weekday",
        engine::json_weekday,
        &[
            ("Mon", "0"),
            ("Tue", "1"),
            ("Wed", "2"),
            ("Thu", "3"),
            ("Fri", "4"),
            ("Sat", "5"),
            ("Sun", "6"),
            ("mon", "0"),
            ("MON", "0"),
            ("Monday", "0"),
            ("monday", "0"),
            ("Tues", REJECTED),
            ("Thur", REJECTED),
            ("Mo", REJECTED),
            ("1", REJECTED),
            ("lundi", REJECTED),
            ("", REJECTED),
        ],
    );
}
