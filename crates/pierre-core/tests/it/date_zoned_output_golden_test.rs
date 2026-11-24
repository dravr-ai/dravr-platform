// ABOUTME: Pins the text an instant produces when it is shown in a zone other than UTC
// ABOUTME: Recorded from the date engine in use on 2026-11-17: offsets, fractions, prompt stamps
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Zoned output goldens for dates (carnet#1620, gates 1 and 2).
//!
//! `date_output_golden_test` walks UTC values only. Three production writers
//! put an instant into text after moving it to the athlete's zone, and none of
//! them ends in `+00:00` or `Z`:
//!
//! - `start_date_local` on the activity tools is
//!   `.with_timezone(&zone).to_rfc3339()` (`pierre-tool-runtime`,
//!   `fitness_support.rs`): the athlete's wall clock, the padded fraction, and
//!   the zone's offset at that instant;
//! - the session-merge note in the activity list is
//!   `.with_timezone(&zone).format("%Y-%m-%d %H:%M")`
//!   (`activity_list_render.rs`), pinned through that function in
//!   `pierre-tool-runtime`'s `date_tool_output_golden_test`;
//! - the prompt stamps (`2026-09-01 lun 18:30`) are written by
//!   `pierre_core::civil_time`, and are asserted through those functions here.
//!
//! `start_date_local` is assembled inside a tool handler that no public
//! function of these crates reaches without a server, so its rows go through
//! this file's own helper. They state what the call site writes today.
//!
//! Zones and dates are chosen so the zone database in use (2025b) and the one
//! that replaces it (2026f) agree on every row: Toronto and Kolkata are not
//! among the zones that differ.

use pierre_core::civil_time::{
    format_civil_day, format_clock_stamp, format_local_day, format_local_stamp, local_date,
    parse_zone,
};

/// Everything in this file that names the date engine.
mod engine {
    use std::borrow::Borrow;

    use dravr_tronc::dates::CivilDateTime;

    pub(super) use dravr_tronc::dates::Timestamp as Instant;

    /// A UTC instant from `2026-11-17T15:00:05.120`, with any fraction or none.
    pub(super) fn instant(text: &str) -> Instant {
        CivilDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
            .unwrap()
            .and_utc()
            .unwrap()
    }

    pub(super) use dravr_tronc::dates::Zone;

    /// A zone from its IANA name, held for the whole run so the tables can
    /// pass it on by copy as they passed the zone type it replaced.
    pub(super) fn zone(name: &str) -> &'static Zone {
        Box::leak(Box::new(name.parse().unwrap()))
    }

    /// The hand formatter on a zoned value: what `start_date_local` carries.
    pub(super) fn zoned_rfc3339(value: Instant, zone: impl Borrow<Zone>) -> String {
        zone.borrow()
            .format(value, "%Y-%m-%dT%H:%M:%S%.f%:z")
            .unwrap()
    }

    /// The minute pattern on a zoned value: the session-merge note's window.
    pub(super) fn zoned_minutes(value: Instant, zone: impl Borrow<Zone>) -> String {
        zone.borrow().format(value, "%Y-%m-%d %H:%M").unwrap()
    }

    /// The JSON serializer on a value that keeps a fixed offset.
    pub(super) fn fixed_offset_json(value: Instant, seconds_east: i32) -> String {
        let offset = Zone::fixed(seconds_east).unwrap();
        let pattern = if seconds_east == 0 {
            "%Y-%m-%dT%H:%M:%S%.fZ"
        } else {
            "%Y-%m-%dT%H:%M:%S%.f%:z"
        };
        serde_json::to_string(&offset.format(value, pattern).unwrap()).unwrap()
    }
}

use engine::{instant, zone};

/// zone, UTC instant
const ROWS: [(&str, &str); 10] = [
    // Toronto in winter (-05:00), with every fraction width and with none.
    ("America/Toronto", "2026-11-17T15:00:05"),
    ("America/Toronto", "2026-11-17T15:00:05.100"),
    ("America/Toronto", "2026-11-17T15:00:05.120"),
    ("America/Toronto", "2026-11-17T15:00:05.123456"),
    ("America/Toronto", "2026-11-17T15:00:05.123456789"),
    // Toronto in summer (-04:00), on the evening before the UTC date.
    ("America/Toronto", "2026-08-28T02:59:03"),
    ("America/Toronto", "2026-08-28T02:59:03.250"),
    // Kolkata (+05:30), on the morning after the UTC date.
    ("Asia/Kolkata", "2026-11-17T20:45:05"),
    ("Asia/Kolkata", "2026-11-17T20:45:05.120"),
    ("Asia/Kolkata", "2026-11-17T20:45:05.123456789"),
];

/// Walks the rows through one writer and compares the whole table at once.
fn assert_rows(writer: &str, write: impl Fn(&str, &str) -> String, expected: [&str; 10]) {
    let actual: Vec<(&str, &str, String)> = ROWS
        .iter()
        .map(|(zone, at)| (*zone, *at, write(zone, at)))
        .collect();
    let wanted: Vec<(&str, &str, String)> = ROWS
        .iter()
        .zip(expected)
        .map(|((zone, at), text)| (*zone, *at, text.to_owned()))
        .collect();
    assert_eq!(actual, wanted, "{writer} changed its output");
}

/// The fraction is padded exactly as in UTC, and the offset is the zone's own
/// at that instant, with a colon.
#[test]
fn the_hand_formatter_on_a_zoned_value_ends_in_the_zone_s_offset() {
    assert_rows(
        "with_timezone + to_rfc3339",
        |name, at| engine::zoned_rfc3339(instant(at), zone(name)),
        [
            "2026-11-17T10:00:05-05:00",
            "2026-11-17T10:00:05.100-05:00",
            "2026-11-17T10:00:05.120-05:00",
            "2026-11-17T10:00:05.123456-05:00",
            "2026-11-17T10:00:05.123456789-05:00",
            "2026-08-27T22:59:03-04:00",
            "2026-08-27T22:59:03.250-04:00",
            "2026-11-18T02:15:05+05:30",
            "2026-11-18T02:15:05.120+05:30",
            "2026-11-18T02:15:05.123456789+05:30",
        ],
    );
}

/// The minute pattern drops the seconds and the fraction without rounding and
/// writes no offset at all.
#[test]
fn the_minute_pattern_on_a_zoned_value_is_the_wall_clock_alone() {
    assert_rows(
        "with_timezone + format(%Y-%m-%d %H:%M)",
        |name, at| engine::zoned_minutes(instant(at), zone(name)),
        [
            "2026-11-17 10:00",
            "2026-11-17 10:00",
            "2026-11-17 10:00",
            "2026-11-17 10:00",
            "2026-11-17 10:00",
            "2026-08-27 22:59",
            "2026-08-27 22:59",
            "2026-11-18 02:15",
            "2026-11-18 02:15",
            "2026-11-18 02:15",
        ],
    );
}

/// A value that keeps a fixed offset serializes with that offset, never
/// converted to `Z`. Terra payloads and stored goals hold such values.
#[test]
fn json_on_a_fixed_offset_value_keeps_the_offset() {
    // UTC instant, seconds east, JSON
    let rows = [
        (
            "2026-11-17T15:00:05",
            -18_000,
            r#""2026-11-17T10:00:05-05:00""#,
        ),
        (
            "2026-11-17T15:00:05.120",
            -18_000,
            r#""2026-11-17T10:00:05.120-05:00""#,
        ),
        (
            "2026-11-17T20:45:05.123456789",
            19_800,
            r#""2026-11-18T02:15:05.123456789+05:30""#,
        ),
        (
            "2026-11-17T15:00:05.100",
            0,
            r#""2026-11-17T15:00:05.100Z""#,
        ),
        (
            "2026-11-17T15:00:05",
            20_700,
            r#""2026-11-17T20:45:05+05:45""#,
        ),
    ];
    for (at, seconds_east, json) in rows {
        assert_eq!(
            engine::fixed_offset_json(instant(at), seconds_east),
            json,
            "{at} at {seconds_east}"
        );
    }
}

/// The prompt stamps, through the production functions that write them. Each
/// row: zone, instant, locale, then the clock stamp, the local stamp and the
/// day-only stamp.
#[test]
fn the_prompt_stamps_show_the_athlete_s_wall_clock_and_weekday() {
    let rows = [
        (
            "America/Toronto",
            "2026-08-28T02:59:03",
            "fr",
            "2026-08-27 jeu 22:59",
            "2026-08-27 jeu 22:59",
            "2026-08-27 jeu",
        ),
        (
            "America/Toronto",
            "2026-11-17T15:00:05.999",
            "en",
            "2026-11-17 Tue 10:00",
            "2026-11-17 Tue 10:00",
            "2026-11-17 Tue",
        ),
        (
            "Asia/Kolkata",
            "2026-11-17T20:45:05.120",
            "en",
            "2026-11-18 Wed 02:15",
            "2026-11-18 Wed 02:15",
            "2026-11-18 Wed",
        ),
        // Exactly midnight UTC is a date-only provider row: the clock stamp
        // still converts it, the other two keep the date it was given.
        (
            "America/Toronto",
            "2026-11-17T00:00:00",
            "en",
            "2026-11-16 Mon 19:00",
            "2026-11-17 Tue 00:00",
            "2026-11-17 Tue",
        ),
        (
            "Asia/Kolkata",
            "2026-11-17T00:00:00",
            "fr",
            "2026-11-17 mar 05:30",
            "2026-11-17 mar 00:00",
            "2026-11-17 mar",
        ),
    ];

    let actual: Vec<[String; 3]> = rows
        .iter()
        .map(|(name, at, locale, ..)| {
            let zone = parse_zone(Some(name)).unwrap();
            let at = instant(at);
            [
                format_clock_stamp(at, &zone, locale),
                format_local_stamp(at, &zone, locale),
                format_local_day(at, &zone, locale),
            ]
        })
        .collect();
    let wanted: Vec<[String; 3]> = rows
        .iter()
        .map(|(_, _, _, clock, local, day)| {
            [(*clock).to_owned(), (*local).to_owned(), (*day).to_owned()]
        })
        .collect();
    assert_eq!(
        actual, wanted,
        "the clock, local and day-only stamps of each row"
    );
}

/// The day-only stamp for a date already on the athlete's calendar is the
/// same text the instant gives through `local_date`.
#[test]
fn a_calendar_date_stamps_as_its_date_and_weekday() {
    let toronto = parse_zone(Some("America/Toronto")).unwrap();
    let evening = instant("2026-08-28T02:59:03");
    assert_eq!(
        format_civil_day(local_date(evening, &toronto), "fr"),
        "2026-08-27 jeu",
        "the French day-only stamp of a calendar date"
    );
    assert_eq!(
        format_civil_day(local_date(evening, &toronto), "en"),
        "2026-08-27 Thu",
        "the English day-only stamp of a calendar date"
    );
}
