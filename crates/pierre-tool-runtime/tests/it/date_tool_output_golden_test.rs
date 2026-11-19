// ABOUTME: Pins the dates the activity list and the goal counter produce, through their public functions
// ABOUTME: Recorded from the date engine in use on 2026-11-17: zoned stamps, merge windows, offset comparison
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool-output goldens for dates (carnet#1620, gates 1 and 8).
//!
//! Two tool paths in this crate do date work that a public function reaches
//! without a server:
//!
//! - `format_activities_as_list` writes the athlete's wall clock three ways:
//!   the `[Today]` anchor, each row's stamp, and the session-merge note's
//!   window, which is `.with_timezone(&zone).format("%Y-%m-%d %H:%M")`;
//! - `activities_toward_goal` compares an activity's UTC start with the
//!   instant a goal was set, which is kept with the offset it was stored in.
//!
//! Two more are private to the crate and reached only from a tool handler,
//! which needs the server to run: `start_date_local` on the activity tools
//! (`.with_timezone(&zone).to_rfc3339()`, `fitness_support.rs`) and the stored
//! goal's `created_at` (`parse_from_rfc3339`, `goals_spec.rs`). Their output
//! is pinned through the server in `pierre-server`
//! (`date_response_activity_tools_golden_test`), and on the date engine alone
//! in `pierre-core` (`date_zoned_output_golden_test`,
//! `date_inbound_golden_test`).
//!
//! Zone rows use Toronto and Kolkata, on which the zone database in use and
//! the one that replaces it agree.
//!
//! Everything that names the date engine lives in [`engine`].

use std::collections::HashMap;

use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_tool_runtime::implementations::activity_list_render::format_activities_as_list;
use pierre_tool_runtime::implementations::goals_spec::activities_toward_goal;

/// Everything in this file that names the date engine.
mod engine {
    use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};

    pub(super) type Instant = DateTime<Utc>;
    pub(super) type OffsetInstant = DateTime<FixedOffset>;

    /// A UTC instant from `2026-11-17T15:00:05.250`, any fraction or none.
    pub(super) fn instant(text: &str) -> Instant {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
            .unwrap()
            .and_utc()
    }

    /// An instant that keeps the offset it is written with.
    pub(super) fn offset_instant(text: &str) -> OffsetInstant {
        DateTime::parse_from_rfc3339(text).unwrap()
    }
}

use engine::instant;

/// One ride recorded twice, by two providers, starting 2026-08-28 02:59:03.5
/// UTC and lasting an hour: the evening of the 27th in Toronto, the morning
/// of the 28th in Kolkata.
fn rendered(zone: Option<&str>, locale: &str) -> Vec<String> {
    let start = instant("2026-08-28T02:59:03.500");
    let recordings = vec![
        ActivityBuilder::new("s-1", "Night ride", SportType::Ride, start, 3_600, "strava")
            .distance_meters(30_000.0)
            .build(),
        ActivityBuilder::new("w-1", "Ride", SportType::Ride, start, 3_590, "whoop")
            .calories(700)
            .build(),
    ];
    let (sessions, report) = merge_duplicates(recordings, &DedupConfig::default());
    assert_eq!(sessions.len(), 1, "the two recordings are one session");
    format_activities_as_list(
        &sessions,
        &HashMap::new(),
        Some(&report),
        locale,
        zone,
        instant("2026-08-29T12:00:00"),
    )
    .lines()
    .filter(|line| line.contains("2026-08-2"))
    .map(|line| line.trim().to_owned())
    .collect()
}

/// Every line of the list that carries a date, in the athlete's zone: the
/// anchor, the merge note's window to the minute, and the row's stamp.
#[test]
fn the_activity_list_writes_every_date_on_the_athlete_s_wall_clock() {
    // zone, locale, [the anchor, the merge window, the row]
    let expected = [
        ("America/Toronto", "en", ["[Today] 2026-08-29 Sat — newest activity listed: 2026-08-27 Thu", "- session s-1; recordings: [s-1, w-1] from strava + whoop (bike ride, 2026-08-27 22:59 → 2026-08-27 23:59); took calories from whoop", "1. [bike ride] Night ride - 2026-08-27 Thu 22:59 - 30.00 km - 1:00:00 - 700 kcal"]),
        ("Asia/Kolkata", "en", ["[Today] 2026-08-29 Sat — newest activity listed: 2026-08-28 Fri (yesterday)", "- session s-1; recordings: [s-1, w-1] from strava + whoop (bike ride, 2026-08-28 08:29 → 2026-08-28 09:29); took calories from whoop", "1. [bike ride] Night ride - 2026-08-28 Fri 08:29 (yesterday) - 30.00 km - 1:00:00 - 700 kcal"]),
        ("America/Toronto", "fr", ["[Today] 2026-08-29 sam — newest activity listed: 2026-08-27 jeu", "- session s-1; recordings: [s-1, w-1] from strava + whoop (vélo, 2026-08-27 22:59 → 2026-08-27 23:59); took calories from whoop", "1. [vélo] Night ride - 2026-08-27 jeu 22:59 - 30.00 km - 1:00:00 - 700 kcal"]),
        ("Etc/UTC", "en", ["[Today] 2026-08-29 Sat — newest activity listed: 2026-08-28 Fri (yesterday)", "- session s-1; recordings: [s-1, w-1] from strava + whoop (bike ride, 2026-08-28 02:59 → 2026-08-28 03:59); took calories from whoop", "1. [bike ride] Night ride - 2026-08-28 Fri 02:59 (yesterday) - 30.00 km - 1:00:00 - 700 kcal"]),
    ];
    let actual: Vec<(&str, &str, Vec<String>)> = expected
        .iter()
        .map(|(zone, locale, _)| (*zone, *locale, rendered(Some(zone), locale)))
        .collect();
    let wanted: Vec<(&str, &str, Vec<String>)> = expected
        .iter()
        .map(|(zone, locale, lines)| (*zone, *locale, lines.map(str::to_owned).to_vec()))
        .collect();
    assert_eq!(
        actual, wanted,
        "the anchor, merge window and row stamp in each zone and locale"
    );

    // No zone on file, and a name that is not a zone, both read as UTC.
    assert_eq!(
        rendered(None, "en"),
        rendered(Some("Etc/UTC"), "en"),
        "no zone on file reads as UTC"
    );
    assert_eq!(
        rendered(Some("america/toronto"), "en"),
        rendered(None, "en"),
        "a name that is not a zone reads as UTC"
    );
}

fn started(id: &str, at: &str) -> Activity {
    ActivityBuilder::new(id, "Run", SportType::Run, instant(at), 1_800, "strava").build()
}

/// A goal counts the activities started strictly after it was set. The
/// instant it was set is compared as an instant, whatever offset it is kept
/// in, down to the nanosecond.
#[test]
fn a_goal_counts_what_started_after_the_instant_it_was_set() {
    let history = [
        started("an-hour-before", "2026-11-17T14:00:05"),
        started("a-nanosecond-before", "2026-11-17T15:00:04.999999999"),
        started("the-same-instant", "2026-11-17T15:00:05"),
        started("a-nanosecond-after", "2026-11-17T15:00:05.000000001"),
        started("an-hour-after", "2026-11-17T16:00:05"),
    ];
    let counted = |set_at: &str| -> Vec<String> {
        activities_toward_goal(&history, Some(engine::offset_instant(set_at)), None)
            .iter()
            .map(|activity| activity.id().to_owned())
            .collect()
    };

    // The same instant written in three offsets counts the same activities.
    let after = ["a-nanosecond-after", "an-hour-after"];
    assert_eq!(counted("2026-11-17T15:00:05Z"), after, "set at Z");
    assert_eq!(counted("2026-11-17T10:00:05-05:00"), after, "set at -05:00");
    assert_eq!(counted("2026-11-17T20:30:05+05:30"), after, "set at +05:30");
    // A fraction on the goal moves the line with it.
    assert_eq!(
        counted("2026-11-17T10:00:04.999999999-05:00"),
        ["the-same-instant", "a-nanosecond-after", "an-hour-after"],
        "set a nanosecond before the same instant"
    );
    assert_eq!(
        counted("2026-11-17T10:00:05.000000001-05:00"),
        ["an-hour-after"],
        "set a nanosecond after the same instant"
    );
    // The wall clock alone is not what is compared: 15:00:05 at +05:30 is
    // five and a half hours earlier.
    assert_eq!(
        counted("2026-11-17T15:00:05+05:30").len(),
        5,
        "a +05:30 wall clock of 15:00:05 is compared as 09:30:05 UTC"
    );
}
