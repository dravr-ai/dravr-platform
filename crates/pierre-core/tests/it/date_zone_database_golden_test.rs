// ABOUTME: Pins what the zone database answers for the zones a newer release is known to change
// ABOUTME: One table marked expected to change, reviewed, so a database update edits exactly that table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Zone database goldens (carnet#1620, gate 8).
//!
//! The zone database compiled into the date engine today is release 2025b.
//! The engine that replaces it carries 2026f, which gives different answers
//! for nine zones from 2026-09-20 on: `America/Vancouver`, `America/Edmonton`,
//! `America/Winnipeg`, `America/Inuvik`, `America/Yellowknife`,
//! `America/Rainy_River`, `America/Havana`, `Africa/Casablanca` and
//! `Africa/El_Aaiun`.
//!
//! Every other zone golden in these suites uses a zone outside that list, or
//! a date before 2026-09-20, and must come through a database update
//! unchanged. This file holds the two tables that sort the rest:
//!
//! - [`AFFECTED_ZONES_BEFORE_THE_CHANGE`]: the nine zones on dates before
//!   2026-09-20. A database update must not move these.
//! - [`EXPECTED_TO_CHANGE_REVIEWED`]: the nine zones from 2026-09-20 on, and
//!   the release name itself, as 2025b answers. A database update is expected
//!   to turn rows of this table red, and only this table; whoever updates the
//!   database edits it row by row and reviews each new answer against the
//!   release notes. A red row anywhere else is a regression.
//!
//! Everything that names the date engine lives in [`engine`].

/// Everything in this file that names the date engine.
mod engine {
    use chrono::NaiveDateTime;
    use chrono_tz::Tz;

    /// The zone database release compiled into the engine.
    pub(super) fn database_release() -> &'static str {
        chrono_tz::IANA_TZDB_VERSION
    }

    /// The wall clock and offset a UTC instant shows in a zone.
    pub(super) fn wall_clock(zone: &str, utc: &str) -> String {
        NaiveDateTime::parse_from_str(utc, "%Y-%m-%dT%H:%M:%S")
            .unwrap()
            .and_utc()
            .with_timezone(&zone.parse::<Tz>().unwrap())
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string()
    }
}

/// zone, UTC instant, wall clock and offset
type ZoneRows = [(&'static str, &'static str, &'static str)];

/// The nine zones on dates before 2026-09-20, where both releases agree.
const AFFECTED_ZONES_BEFORE_THE_CHANGE: &ZoneRows = &[
    (
        "America/Vancouver",
        "2025-01-15T12:00:00",
        "2025-01-15 04:00:00 -08:00",
    ),
    (
        "America/Vancouver",
        "2025-07-15T12:00:00",
        "2025-07-15 05:00:00 -07:00",
    ),
    (
        "America/Vancouver",
        "2026-03-15T12:00:00",
        "2026-03-15 05:00:00 -07:00",
    ),
    (
        "America/Vancouver",
        "2026-09-19T12:00:00",
        "2026-09-19 05:00:00 -07:00",
    ),
    (
        "America/Edmonton",
        "2025-01-15T12:00:00",
        "2025-01-15 05:00:00 -07:00",
    ),
    (
        "America/Edmonton",
        "2025-07-15T12:00:00",
        "2025-07-15 06:00:00 -06:00",
    ),
    (
        "America/Edmonton",
        "2026-03-15T12:00:00",
        "2026-03-15 06:00:00 -06:00",
    ),
    (
        "America/Edmonton",
        "2026-09-19T12:00:00",
        "2026-09-19 06:00:00 -06:00",
    ),
    (
        "America/Winnipeg",
        "2025-01-15T12:00:00",
        "2025-01-15 06:00:00 -06:00",
    ),
    (
        "America/Winnipeg",
        "2025-07-15T12:00:00",
        "2025-07-15 07:00:00 -05:00",
    ),
    (
        "America/Winnipeg",
        "2026-03-15T12:00:00",
        "2026-03-15 07:00:00 -05:00",
    ),
    (
        "America/Winnipeg",
        "2026-09-19T12:00:00",
        "2026-09-19 07:00:00 -05:00",
    ),
    (
        "America/Inuvik",
        "2025-01-15T12:00:00",
        "2025-01-15 05:00:00 -07:00",
    ),
    (
        "America/Inuvik",
        "2025-07-15T12:00:00",
        "2025-07-15 06:00:00 -06:00",
    ),
    (
        "America/Inuvik",
        "2026-03-15T12:00:00",
        "2026-03-15 06:00:00 -06:00",
    ),
    (
        "America/Inuvik",
        "2026-09-19T12:00:00",
        "2026-09-19 06:00:00 -06:00",
    ),
    (
        "America/Yellowknife",
        "2025-01-15T12:00:00",
        "2025-01-15 05:00:00 -07:00",
    ),
    (
        "America/Yellowknife",
        "2025-07-15T12:00:00",
        "2025-07-15 06:00:00 -06:00",
    ),
    (
        "America/Yellowknife",
        "2026-03-15T12:00:00",
        "2026-03-15 06:00:00 -06:00",
    ),
    (
        "America/Yellowknife",
        "2026-09-19T12:00:00",
        "2026-09-19 06:00:00 -06:00",
    ),
    (
        "America/Rainy_River",
        "2025-01-15T12:00:00",
        "2025-01-15 06:00:00 -06:00",
    ),
    (
        "America/Rainy_River",
        "2025-07-15T12:00:00",
        "2025-07-15 07:00:00 -05:00",
    ),
    (
        "America/Rainy_River",
        "2026-03-15T12:00:00",
        "2026-03-15 07:00:00 -05:00",
    ),
    (
        "America/Rainy_River",
        "2026-09-19T12:00:00",
        "2026-09-19 07:00:00 -05:00",
    ),
    (
        "America/Havana",
        "2025-01-15T12:00:00",
        "2025-01-15 07:00:00 -05:00",
    ),
    (
        "America/Havana",
        "2025-07-15T12:00:00",
        "2025-07-15 08:00:00 -04:00",
    ),
    (
        "America/Havana",
        "2026-03-15T12:00:00",
        "2026-03-15 08:00:00 -04:00",
    ),
    (
        "America/Havana",
        "2026-09-19T12:00:00",
        "2026-09-19 08:00:00 -04:00",
    ),
    (
        "Africa/Casablanca",
        "2025-01-15T12:00:00",
        "2025-01-15 13:00:00 +01:00",
    ),
    (
        "Africa/Casablanca",
        "2025-07-15T12:00:00",
        "2025-07-15 13:00:00 +01:00",
    ),
    (
        "Africa/Casablanca",
        "2026-03-15T12:00:00",
        "2026-03-15 12:00:00 +00:00",
    ),
    (
        "Africa/Casablanca",
        "2026-09-19T12:00:00",
        "2026-09-19 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2025-01-15T12:00:00",
        "2025-01-15 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2025-07-15T12:00:00",
        "2025-07-15 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2026-03-15T12:00:00",
        "2026-03-15 12:00:00 +00:00",
    ),
    (
        "Africa/El_Aaiun",
        "2026-09-19T12:00:00",
        "2026-09-19 13:00:00 +01:00",
    ),
];

/// EXPECTED TO CHANGE, REVIEWED: the nine zones from 2026-09-20 on, as
/// release 2025b answers. Edit this table, and no other, when the zone
/// database is updated.
const EXPECTED_TO_CHANGE_REVIEWED: &ZoneRows = &[
    (
        "America/Vancouver",
        "2026-09-20T12:00:00",
        "2026-09-20 05:00:00 -07:00",
    ),
    (
        "America/Vancouver",
        "2026-11-15T12:00:00",
        "2026-11-15 04:00:00 -08:00",
    ),
    (
        "America/Vancouver",
        "2027-01-15T12:00:00",
        "2027-01-15 04:00:00 -08:00",
    ),
    (
        "America/Vancouver",
        "2027-03-01T12:00:00",
        "2027-03-01 04:00:00 -08:00",
    ),
    (
        "America/Vancouver",
        "2027-07-15T12:00:00",
        "2027-07-15 05:00:00 -07:00",
    ),
    (
        "America/Vancouver",
        "2027-12-15T12:00:00",
        "2027-12-15 04:00:00 -08:00",
    ),
    (
        "America/Edmonton",
        "2026-09-20T12:00:00",
        "2026-09-20 06:00:00 -06:00",
    ),
    (
        "America/Edmonton",
        "2026-11-15T12:00:00",
        "2026-11-15 05:00:00 -07:00",
    ),
    (
        "America/Edmonton",
        "2027-01-15T12:00:00",
        "2027-01-15 05:00:00 -07:00",
    ),
    (
        "America/Edmonton",
        "2027-03-01T12:00:00",
        "2027-03-01 05:00:00 -07:00",
    ),
    (
        "America/Edmonton",
        "2027-07-15T12:00:00",
        "2027-07-15 06:00:00 -06:00",
    ),
    (
        "America/Edmonton",
        "2027-12-15T12:00:00",
        "2027-12-15 05:00:00 -07:00",
    ),
    (
        "America/Winnipeg",
        "2026-09-20T12:00:00",
        "2026-09-20 07:00:00 -05:00",
    ),
    (
        "America/Winnipeg",
        "2026-11-15T12:00:00",
        "2026-11-15 06:00:00 -06:00",
    ),
    (
        "America/Winnipeg",
        "2027-01-15T12:00:00",
        "2027-01-15 06:00:00 -06:00",
    ),
    (
        "America/Winnipeg",
        "2027-03-01T12:00:00",
        "2027-03-01 06:00:00 -06:00",
    ),
    (
        "America/Winnipeg",
        "2027-07-15T12:00:00",
        "2027-07-15 07:00:00 -05:00",
    ),
    (
        "America/Winnipeg",
        "2027-12-15T12:00:00",
        "2027-12-15 06:00:00 -06:00",
    ),
    (
        "America/Inuvik",
        "2026-09-20T12:00:00",
        "2026-09-20 06:00:00 -06:00",
    ),
    (
        "America/Inuvik",
        "2026-11-15T12:00:00",
        "2026-11-15 05:00:00 -07:00",
    ),
    (
        "America/Inuvik",
        "2027-01-15T12:00:00",
        "2027-01-15 05:00:00 -07:00",
    ),
    (
        "America/Inuvik",
        "2027-03-01T12:00:00",
        "2027-03-01 05:00:00 -07:00",
    ),
    (
        "America/Inuvik",
        "2027-07-15T12:00:00",
        "2027-07-15 06:00:00 -06:00",
    ),
    (
        "America/Inuvik",
        "2027-12-15T12:00:00",
        "2027-12-15 05:00:00 -07:00",
    ),
    (
        "America/Yellowknife",
        "2026-09-20T12:00:00",
        "2026-09-20 06:00:00 -06:00",
    ),
    (
        "America/Yellowknife",
        "2026-11-15T12:00:00",
        "2026-11-15 05:00:00 -07:00",
    ),
    (
        "America/Yellowknife",
        "2027-01-15T12:00:00",
        "2027-01-15 05:00:00 -07:00",
    ),
    (
        "America/Yellowknife",
        "2027-03-01T12:00:00",
        "2027-03-01 05:00:00 -07:00",
    ),
    (
        "America/Yellowknife",
        "2027-07-15T12:00:00",
        "2027-07-15 06:00:00 -06:00",
    ),
    (
        "America/Yellowknife",
        "2027-12-15T12:00:00",
        "2027-12-15 05:00:00 -07:00",
    ),
    (
        "America/Rainy_River",
        "2026-09-20T12:00:00",
        "2026-09-20 07:00:00 -05:00",
    ),
    (
        "America/Rainy_River",
        "2026-11-15T12:00:00",
        "2026-11-15 06:00:00 -06:00",
    ),
    (
        "America/Rainy_River",
        "2027-01-15T12:00:00",
        "2027-01-15 06:00:00 -06:00",
    ),
    (
        "America/Rainy_River",
        "2027-03-01T12:00:00",
        "2027-03-01 06:00:00 -06:00",
    ),
    (
        "America/Rainy_River",
        "2027-07-15T12:00:00",
        "2027-07-15 07:00:00 -05:00",
    ),
    (
        "America/Rainy_River",
        "2027-12-15T12:00:00",
        "2027-12-15 06:00:00 -06:00",
    ),
    (
        "America/Havana",
        "2026-09-20T12:00:00",
        "2026-09-20 08:00:00 -04:00",
    ),
    (
        "America/Havana",
        "2026-11-15T12:00:00",
        "2026-11-15 07:00:00 -05:00",
    ),
    (
        "America/Havana",
        "2027-01-15T12:00:00",
        "2027-01-15 07:00:00 -05:00",
    ),
    (
        "America/Havana",
        "2027-03-01T12:00:00",
        "2027-03-01 07:00:00 -05:00",
    ),
    (
        "America/Havana",
        "2027-07-15T12:00:00",
        "2027-07-15 08:00:00 -04:00",
    ),
    (
        "America/Havana",
        "2027-12-15T12:00:00",
        "2027-12-15 07:00:00 -05:00",
    ),
    (
        "Africa/Casablanca",
        "2026-09-20T12:00:00",
        "2026-09-20 13:00:00 +01:00",
    ),
    (
        "Africa/Casablanca",
        "2026-11-15T12:00:00",
        "2026-11-15 13:00:00 +01:00",
    ),
    (
        "Africa/Casablanca",
        "2027-01-15T12:00:00",
        "2027-01-15 13:00:00 +01:00",
    ),
    (
        "Africa/Casablanca",
        "2027-03-01T12:00:00",
        "2027-03-01 12:00:00 +00:00",
    ),
    (
        "Africa/Casablanca",
        "2027-07-15T12:00:00",
        "2027-07-15 13:00:00 +01:00",
    ),
    (
        "Africa/Casablanca",
        "2027-12-15T12:00:00",
        "2027-12-15 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2026-09-20T12:00:00",
        "2026-09-20 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2026-11-15T12:00:00",
        "2026-11-15 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2027-01-15T12:00:00",
        "2027-01-15 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2027-03-01T12:00:00",
        "2027-03-01 12:00:00 +00:00",
    ),
    (
        "Africa/El_Aaiun",
        "2027-07-15T12:00:00",
        "2027-07-15 13:00:00 +01:00",
    ),
    (
        "Africa/El_Aaiun",
        "2027-12-15T12:00:00",
        "2027-12-15 13:00:00 +01:00",
    ),
];

fn assert_zone_rows(table: &str, rows: &ZoneRows) {
    let actual: Vec<(&str, &str, String)> = rows
        .iter()
        .map(|(zone, utc, _)| (*zone, *utc, engine::wall_clock(zone, utc)))
        .collect();
    let wanted: Vec<(&str, &str, String)> = rows
        .iter()
        .map(|(zone, utc, clock)| (*zone, *utc, (*clock).to_owned()))
        .collect();
    assert_eq!(actual, wanted, "{table}");
}

#[test]
fn the_affected_zones_keep_their_answers_before_the_change() {
    assert_zone_rows(
        "AFFECTED_ZONES_BEFORE_THE_CHANGE",
        AFFECTED_ZONES_BEFORE_THE_CHANGE,
    );
}

/// Expected to change, reviewed: see [`EXPECTED_TO_CHANGE_REVIEWED`].
#[test]
fn expected_to_change_reviewed_the_affected_zones_after_the_change() {
    assert_zone_rows("EXPECTED_TO_CHANGE_REVIEWED", EXPECTED_TO_CHANGE_REVIEWED);
}

/// Expected to change, reviewed: the release the tables above were recorded
/// from.
#[test]
fn expected_to_change_reviewed_the_zone_database_release() {
    assert_eq!(
        engine::database_release(),
        "2025b",
        "the zone database release the tables were recorded from"
    );
}
