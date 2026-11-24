// ABOUTME: Pins the exact text every date type produces: hand formatters, strftime patterns, JSON
// ABOUTME: Recorded from the date engine in use on 2026-11-17 so a later engine must reproduce it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Output goldens for dates (carnet#1620, gates 1 and 2).
//!
//! Three writers put an instant into text and they do not agree with each
//! other, which is why each is pinned separately:
//!
//! - the hand formatter (`to_rfc3339`) ends a UTC value in `+00:00`. It is the
//!   text stored in every `SQLite` timestamp column, where rows are compared as
//!   text, so one row in another shape breaks ordering;
//! - the fixed-width formatters end in `Z` and cut the fraction, never round;
//! - the JSON serializer ends in `Z`.
//!
//! All of them pad the fraction to exactly 3, 6 or 9 digits. `…:05.100` never
//! prints as `…:05.1`, and the rows with trailing zeros exist to catch a writer
//! that trims.
//!
//! Every expected value was recorded by running the test, not derived from
//! documentation. The assertions hold strings only: everything that names the
//! date engine lives in [`engine`], the one place a change of engine edits.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use pierre_core::models::{SleepStage, SleepStageType};
use pierre_core::pagination::StoreCursor;
use serde::{Deserialize, Serialize};

/// Everything in this file that names the date engine.
mod engine {
    use dravr_tronc::dates::SecondsFormat;

    pub(super) use dravr_tronc::dates::{
        CivilDate, CivilDateTime, CivilTime, Timestamp as Instant, Weekday,
    };

    /// A calendar date.
    pub(super) fn date(year: i32, month: u32, day: u32) -> CivilDate {
        CivilDate::from_ymd(year, month, day).unwrap()
    }

    /// A time of day with a nanosecond fraction.
    pub(super) fn time(hour: u32, minute: u32, second: u32, nanos: u32) -> CivilTime {
        CivilTime::from_hms_nano(hour, minute, second, nanos).unwrap()
    }

    /// A date and a time of day with no zone.
    pub(super) fn civil((year, month, day): (i32, u32, u32), at: CivilTime) -> CivilDateTime {
        date(year, month, day).and_time(at)
    }

    /// The UTC instant at that date and time of day.
    pub(super) fn instant(ymd: (i32, u32, u32), at: CivilTime) -> Instant {
        civil(ymd, at).and_utc().unwrap()
    }

    /// Monday is 0.
    pub(super) fn weekday(days_from_monday: u8) -> Weekday {
        Weekday::ALL[usize::from(days_from_monday)]
    }

    /// The hand formatter: what every `.to_rfc3339()` call site writes.
    pub(super) fn rfc3339(value: Instant) -> String {
        value.to_rfc3339()
    }

    /// Fixed width, no fraction, `Z`.
    pub(super) fn rfc3339_seconds(value: Instant) -> String {
        value.to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    /// Fixed width, three fraction digits, `Z`.
    pub(super) fn rfc3339_millis(value: Instant) -> String {
        value.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// Fixed width, six fraction digits, `Z`.
    pub(super) fn rfc3339_micros(value: Instant) -> String {
        value.to_rfc3339_opts(SecondsFormat::Micros, true)
    }

    /// An instant through a strftime pattern.
    pub(super) fn format_instant(value: Instant, pattern: &str) -> String {
        value.format(pattern).unwrap()
    }

    /// A calendar date through a strftime pattern.
    pub(super) fn format_date(value: CivilDate, pattern: &str) -> String {
        value.format(pattern).unwrap()
    }

    /// A calendar date as it prints.
    pub(super) fn show_date(value: CivilDate) -> String {
        value.to_string()
    }

    /// Nanoseconds since the epoch, to compare two instants as plain integers.
    pub(super) fn epoch_nanos(value: Instant) -> i128 {
        i128::from(value.epoch_seconds()) * 1_000_000_000 + i128::from(value.subsec_nanos())
    }
}

use engine::{CivilDate, CivilDateTime, CivilTime, Instant, Weekday};

/// One row of the value table: a label, the date, the time of day, the fraction.
struct Sample {
    label: &'static str,
    ymd: (i32, u32, u32),
    hms: (u32, u32, u32),
    nanos: u32,
}

impl Sample {
    fn time(&self) -> CivilTime {
        let (hour, minute, second) = self.hms;
        engine::time(hour, minute, second, self.nanos)
    }

    fn instant(&self) -> Instant {
        engine::instant(self.ymd, self.time())
    }

    fn civil(&self) -> CivilDateTime {
        engine::civil(self.ymd, self.time())
    }
}

const fn sample(
    label: &'static str,
    ymd: (i32, u32, u32),
    hms: (u32, u32, u32),
    nanos: u32,
) -> Sample {
    Sample {
        label,
        ymd,
        hms,
        nanos,
    }
}

/// The fixed value table every golden below walks, in this order.
const SAMPLES: [Sample; 11] = [
    sample("whole second", (2026, 11, 17), (10, 0, 5), 0),
    sample(".100", (2026, 11, 17), (10, 0, 5), 100_000_000),
    sample(".120", (2026, 11, 17), (10, 0, 5), 120_000_000),
    sample(".123", (2026, 11, 17), (10, 0, 5), 123_000_000),
    sample(".123400", (2026, 11, 17), (10, 0, 5), 123_400_000),
    sample(".123456", (2026, 11, 17), (10, 0, 5), 123_456_000),
    sample(".123456789", (2026, 11, 17), (10, 0, 5), 123_456_789),
    sample(".000001", (2026, 11, 17), (10, 0, 5), 1_000),
    sample(".999999999", (2026, 11, 17), (23, 59, 59), 999_999_999),
    sample("before 1970", (1969, 7, 20), (20, 17, 40), 250_000_000),
    sample("leap day", (2024, 2, 29), (23, 59, 59), 999_000_000),
];

/// Walks the value table through one writer and compares every row.
fn assert_table(writer: &str, write: impl Fn(&Sample) -> String, expected: [&str; 11]) {
    let actual: Vec<(&str, String)> = SAMPLES.iter().map(|row| (row.label, write(row))).collect();
    let wanted: Vec<(&str, String)> = SAMPLES
        .iter()
        .zip(expected)
        .map(|(row, text)| (row.label, text.to_owned()))
        .collect();
    assert_eq!(actual, wanted, "{writer} changed its output");
}

/// What the hand formatter writes for every row of the value table.
const HAND_FORMATTED: [&str; 11] = [
    "2026-11-17T10:00:05+00:00",
    "2026-11-17T10:00:05.100+00:00",
    "2026-11-17T10:00:05.120+00:00",
    "2026-11-17T10:00:05.123+00:00",
    "2026-11-17T10:00:05.123400+00:00",
    "2026-11-17T10:00:05.123456+00:00",
    "2026-11-17T10:00:05.123456789+00:00",
    "2026-11-17T10:00:05.000001+00:00",
    "2026-11-17T23:59:59.999999999+00:00",
    "1969-07-20T20:17:40.250+00:00",
    "2024-02-29T23:59:59.999+00:00",
];

#[test]
fn the_hand_formatter_pads_the_fraction_and_ends_in_plus_zero() {
    assert_table(
        "to_rfc3339",
        |row| engine::rfc3339(row.instant()),
        HAND_FORMATTED,
    );
}

/// The same text reached through production: a store cursor writes its
/// timestamp with the hand formatter, and `SQLite` compares that text against
/// the column it came from.
#[test]
fn a_store_cursor_writes_its_timestamp_with_the_hand_formatter() {
    assert_table(
        "StoreCursor::encode",
        |row| {
            let cursor = StoreCursor::newest("agent-1".to_owned(), Some(row.instant())).encode();
            let text = String::from_utf8(URL_SAFE_NO_PAD.decode(cursor.as_str()).unwrap()).unwrap();
            text.strip_prefix("newest|")
                .and_then(|rest| rest.strip_suffix("|agent-1"))
                .unwrap()
                .to_owned()
        },
        HAND_FORMATTED,
    );
}

#[test]
fn the_seconds_formatter_drops_the_fraction_without_rounding() {
    assert_table(
        "to_rfc3339_opts(Secs, Z)",
        |row| engine::rfc3339_seconds(row.instant()),
        [
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T10:00:05Z",
            "2026-11-17T23:59:59Z",
            "1969-07-20T20:17:40Z",
            "2024-02-29T23:59:59Z",
        ],
    );
}

#[test]
fn the_millisecond_formatter_always_writes_three_digits() {
    assert_table(
        "to_rfc3339_opts(Millis, Z)",
        |row| engine::rfc3339_millis(row.instant()),
        [
            "2026-11-17T10:00:05.000Z",
            "2026-11-17T10:00:05.100Z",
            "2026-11-17T10:00:05.120Z",
            "2026-11-17T10:00:05.123Z",
            "2026-11-17T10:00:05.123Z",
            "2026-11-17T10:00:05.123Z",
            "2026-11-17T10:00:05.123Z",
            "2026-11-17T10:00:05.000Z",
            "2026-11-17T23:59:59.999Z",
            "1969-07-20T20:17:40.250Z",
            "2024-02-29T23:59:59.999Z",
        ],
    );
}

#[test]
fn the_microsecond_formatter_always_writes_six_digits() {
    assert_table(
        "to_rfc3339_opts(Micros, Z)",
        |row| engine::rfc3339_micros(row.instant()),
        [
            "2026-11-17T10:00:05.000000Z",
            "2026-11-17T10:00:05.100000Z",
            "2026-11-17T10:00:05.120000Z",
            "2026-11-17T10:00:05.123000Z",
            "2026-11-17T10:00:05.123400Z",
            "2026-11-17T10:00:05.123456Z",
            "2026-11-17T10:00:05.123456Z",
            "2026-11-17T10:00:05.000001Z",
            "2026-11-17T23:59:59.999999Z",
            "1969-07-20T20:17:40.250000Z",
            "2024-02-29T23:59:59.999000Z",
        ],
    );
}

/// Every strftime pattern passed to `.format(…)` on an instant anywhere in the
/// repository: ten of the twenty are used by production code, the others only
/// by tests. On a Thursday with a single-digit day, month, and afternoon hour so the
/// unpadded and twelve-hour directives show.
#[test]
fn every_strftime_pattern_in_use_keeps_its_output() {
    let thursday = engine::instant((2026, 3, 5), engine::time(15, 7, 9, 123_000_000));
    let expected = [
        ("%Y-%m-%d", "2026-03-05"),
        ("%Y-%m-%d %H:%M UTC", "2026-03-05 15:07 UTC"),
        ("%Y%m%d_%H%M%S", "20260305_150709"),
        ("%H:%M", "15:07"),
        ("%Y-%m-%dT%H:%M:%S", "2026-03-05T15:07:09"),
        ("%Y-%m-%dT%H:%M:%SZ", "2026-03-05T15:07:09Z"),
        ("%Y-%m-%dT%H:%M:%S%.3fZ", "2026-03-05T15:07:09.123Z"),
        ("%Y-%m-%d %H:%M", "2026-03-05 15:07"),
        ("%Y-%m-%d %H:%M:%S UTC", "2026-03-05 15:07:09 UTC"),
        ("%Y", "2026"),
        (
            "%Y-%m-%d, started %H:%M UTC",
            "2026-03-05, started 15:07 UTC",
        ),
        ("%Y-%m-%d %H:%M:%S +0200", "2026-03-05 15:07:09 +0200"),
        ("%b %-d, %Y, %-I:%M:%S %p", "Mar 5, 2026, 3:07:09 PM"),
        ("%Y-%m", "2026-03"),
        ("%A", "Thursday"),
        ("%A %Y-%m-%d", "Thursday 2026-03-05"),
        (
            "%A %Y-%m-%d, started %H:%M UTC",
            "Thursday 2026-03-05, started 15:07 UTC",
        ),
        ("%a %b %-d", "Thu Mar 5"),
        ("%a %-d %b", "Thu 5 Mar"),
        ("%-d/%-m/%Y", "5/3/2026"),
    ];

    let actual: Vec<(&str, String)> = expected
        .iter()
        .map(|(pattern, _)| (*pattern, engine::format_instant(thursday, pattern)))
        .collect();
    let wanted: Vec<(&str, String)> = expected
        .iter()
        .map(|(pattern, text)| (*pattern, (*text).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "the text each strftime pattern writes");
}

/// `%.3f` cuts the fraction, it does not round: `.9996` stays `.999`.
#[test]
fn the_fixed_fraction_directive_truncates() {
    let late = engine::instant((2026, 3, 5), engine::time(15, 7, 9, 999_600_000));
    assert_eq!(
        engine::format_instant(late, "%Y-%m-%dT%H:%M:%S%.3fZ"),
        "2026-03-05T15:07:09.999Z",
        "%.3f rounded .9996 instead of cutting it"
    );
}

/// The date patterns that are applied to a calendar date with no time.
#[test]
fn a_calendar_date_formats_the_same_through_a_pattern_and_through_display() {
    let rows = [
        (
            (2026, 3, 5),
            "2026-03-05",
            "Thursday",
            "Thu Mar 5",
            "5/3/2026",
        ),
        (
            (2024, 2, 29),
            "2024-02-29",
            "Thursday",
            "Thu Feb 29",
            "29/2/2024",
        ),
        (
            (1969, 7, 20),
            "1969-07-20",
            "Sunday",
            "Sun Jul 20",
            "20/7/1969",
        ),
        (
            (2026, 12, 31),
            "2026-12-31",
            "Thursday",
            "Thu Dec 31",
            "31/12/2026",
        ),
    ];
    for ((year, month, day), iso, name, short, slashed) in rows {
        let date = engine::date(year, month, day);
        assert_eq!(
            engine::format_date(date, "%Y-%m-%d"),
            iso,
            "%Y-%m-%d of {iso}"
        );
        assert_eq!(engine::show_date(date), iso, "Display is the ISO form too");
        assert_eq!(engine::format_date(date, "%A"), name, "%A of {iso}");
        assert_eq!(
            engine::format_date(date, "%a %b %-d"),
            short,
            "%a %b %-d of {iso}"
        );
        assert_eq!(
            engine::format_date(date, "%-d/%-m/%Y"),
            slashed,
            "%-d/%-m/%Y of {iso}"
        );
    }
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

/// What the JSON serializer writes for every row of the value table.
const JSON_INSTANTS: [&str; 11] = [
    r#""2026-11-17T10:00:05Z""#,
    r#""2026-11-17T10:00:05.100Z""#,
    r#""2026-11-17T10:00:05.120Z""#,
    r#""2026-11-17T10:00:05.123Z""#,
    r#""2026-11-17T10:00:05.123400Z""#,
    r#""2026-11-17T10:00:05.123456Z""#,
    r#""2026-11-17T10:00:05.123456789Z""#,
    r#""2026-11-17T10:00:05.000001Z""#,
    r#""2026-11-17T23:59:59.999999999Z""#,
    r#""1969-07-20T20:17:40.250Z""#,
    r#""2024-02-29T23:59:59.999Z""#,
];

#[test]
fn an_instant_serializes_with_a_padded_fraction_and_a_z() {
    assert_table(
        "serde on an instant",
        |row| json(&row.instant()),
        JSON_INSTANTS,
    );
}

/// The same text reached through a production model: a sleep stage's start is
/// an instant field written by the derived serializer.
#[test]
fn a_production_model_writes_an_instant_field_the_same_way() {
    assert_table(
        "SleepStage.start_time",
        |row| {
            let stage = SleepStage {
                stage_type: SleepStageType::Deep,
                start_time: row.instant(),
                duration_minutes: 64,
            };
            let document: serde_json::Value = serde_json::from_str(&json(&stage)).unwrap();
            document["start_time"].to_string()
        },
        JSON_INSTANTS,
    );
}

#[test]
fn an_optional_instant_serializes_as_the_instant_or_null() {
    assert_table(
        "serde on an optional instant",
        |row| json(&Some(row.instant())),
        JSON_INSTANTS,
    );
    assert_eq!(json(&None::<Instant>), "null", "an absent optional instant");
}

#[test]
fn a_calendar_date_serializes_as_year_month_day() {
    assert_table(
        "serde on a calendar date",
        |row| {
            let (year, month, day) = row.ymd;
            json(&engine::date(year, month, day))
        },
        [
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""2026-11-17""#,
            r#""1969-07-20""#,
            r#""2024-02-29""#,
        ],
    );
}

#[test]
fn a_time_of_day_serializes_with_a_padded_fraction_and_no_zone() {
    assert_table(
        "serde on a time of day",
        |row| json(&row.time()),
        [
            r#""10:00:05""#,
            r#""10:00:05.100""#,
            r#""10:00:05.120""#,
            r#""10:00:05.123""#,
            r#""10:00:05.123400""#,
            r#""10:00:05.123456""#,
            r#""10:00:05.123456789""#,
            r#""10:00:05.000001""#,
            r#""23:59:59.999999999""#,
            r#""20:17:40.250""#,
            r#""23:59:59.999""#,
        ],
    );
}

#[test]
fn a_zoneless_date_time_serializes_with_a_t_and_no_offset() {
    assert_table(
        "serde on a zoneless date-time",
        |row| json(&row.civil()),
        [
            r#""2026-11-17T10:00:05""#,
            r#""2026-11-17T10:00:05.100""#,
            r#""2026-11-17T10:00:05.120""#,
            r#""2026-11-17T10:00:05.123""#,
            r#""2026-11-17T10:00:05.123400""#,
            r#""2026-11-17T10:00:05.123456""#,
            r#""2026-11-17T10:00:05.123456789""#,
            r#""2026-11-17T10:00:05.000001""#,
            r#""2026-11-17T23:59:59.999999999""#,
            r#""1969-07-20T20:17:40.250""#,
            r#""2024-02-29T23:59:59.999""#,
        ],
    );
}

#[test]
fn a_weekday_serializes_as_its_three_letter_english_name() {
    let actual: Vec<String> = (0..7).map(|day| json(&engine::weekday(day))).collect();
    assert_eq!(
        actual,
        [r#""Mon""#, r#""Tue""#, r#""Wed""#, r#""Thu""#, r#""Fri""#, r#""Sat""#, r#""Sun""#],
        "weekdays from Monday"
    );
}

/// A record carrying one field of each family, the shape a wire type has.
#[derive(Serialize, Deserialize)]
struct Record {
    at: Instant,
    seen: Option<Instant>,
    never: Option<Instant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    omitted: Option<Instant>,
    day: CivilDate,
    time: CivilTime,
    local: CivilDateTime,
    weekday: Weekday,
    days: Vec<CivilDate>,
}

const RECORD_JSON: &str = concat!(
    r#"{"at":"2026-11-17T10:00:05.100Z","seen":"2024-02-29T23:59:59.999Z","never":null,"#,
    r#""day":"2024-02-29","time":"10:00:05.120","local":"2026-11-17T10:00:05.123400","#,
    r#""weekday":"Tue","days":["1969-07-20","2026-12-31"]}"#,
);

fn record() -> Record {
    Record {
        at: SAMPLES[1].instant(),
        seen: Some(SAMPLES[10].instant()),
        never: None,
        omitted: None,
        day: engine::date(2024, 2, 29),
        time: SAMPLES[2].time(),
        local: SAMPLES[4].civil(),
        weekday: engine::weekday(1),
        days: vec![engine::date(1969, 7, 20), engine::date(2026, 12, 31)],
    }
}

#[test]
fn a_record_with_every_date_family_serializes_to_the_recorded_bytes() {
    assert_eq!(
        json(&record()),
        RECORD_JSON,
        "the bytes a record of every date family writes"
    );
}

/// What the serializer wrote must read back as the same value and write the
/// same bytes again: a stored document survives a load and a save unchanged.
#[test]
fn the_recorded_bytes_read_back_and_write_out_unchanged() {
    let read: Record = serde_json::from_str(RECORD_JSON).unwrap();

    assert_eq!(
        engine::epoch_nanos(read.at),
        engine::epoch_nanos(SAMPLES[1].instant()),
        "the instant read back"
    );
    assert_eq!(
        read.seen.map(engine::rfc3339),
        Some("2024-02-29T23:59:59.999+00:00".to_owned()),
        "the optional instant read back"
    );
    assert!(read.never.is_none(), "null read back as an instant");
    assert!(
        read.omitted.is_none(),
        "a missing optional field read back as an instant"
    );
    assert_eq!(
        engine::show_date(read.day),
        "2024-02-29",
        "the calendar date read back"
    );
    assert_eq!(json(&read), RECORD_JSON, "the bytes written after a read");
}

/// Each instant in the table survives JSON at full nanosecond precision.
#[test]
fn an_instant_round_trips_through_json_at_nanosecond_precision() {
    for row in &SAMPLES {
        let back: Instant = serde_json::from_str(&json(&row.instant())).unwrap();
        assert_eq!(
            engine::epoch_nanos(back),
            engine::epoch_nanos(row.instant()),
            "{} lost precision through JSON",
            row.label
        );
    }
}
