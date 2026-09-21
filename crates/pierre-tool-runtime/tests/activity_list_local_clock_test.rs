// ABOUTME: Pins that the agent's activity list renders the athlete's civil clock
// ABOUTME: A night activity must not be attributed to the following morning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The list block is the agent's only view of the athlete's training on a
//! grounded turn, and the only place it quotes dates back to them.
//!
//! Production Telegram, 2026-08-28: an athlete in `America/Toronto` started a
//! hike at 22:59 on the 27th — Strava named it "Night Hike". Rendered in UTC it
//! became `2026-08-28`, while the prompt's own `{{CURRENT_DATE}}` anchor said
//! `2026-08-27 22:59 (America/Toronto)`. Two calendars in one prompt. The agent
//! read the row's date as local, invented a time of day to fit, and told him it
//! was "ce matin". Every activity after ~20:00 local moved to the next day, and
//! day-counting drifted with it.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::implementations::activity_list_render::format_activities_as_list;

/// 2026-08-28 02:59:03 UTC — which is 2026-08-27 22:59 in America/Toronto.
/// The exact instant from the incident.
fn night_hike() -> Vec<Activity> {
    let start = Utc.with_ymd_and_hms(2026, 8, 28, 2, 59, 3).unwrap();
    vec![ActivityBuilder::new(
        "hike-1",
        "Night Hike",
        SportType::Hike,
        start,
        2_760,
        "sciotte",
    )
    .distance_meters(4_200.0)
    .build()]
}

/// The wall clock the 2026-08-28 rows are rendered against: 2026-08-29 noon UTC,
/// a day after the fixture so no row reads as today or yesterday and the older
/// assertions stay about the zone and the weekday alone.
fn incident_clock() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap()
}

#[test]
fn an_evening_activity_keeps_the_athletes_date_not_the_utc_one() {
    let temps = HashMap::new();
    let rendered = format_activities_as_list(
        &night_hike(),
        &temps,
        None,
        "fr",
        Some("America/Toronto"),
        incident_clock(),
    );

    assert!(
        rendered.contains("2026-08-27 jeu 22:59"),
        "the athlete started this at 22:59 on the 27th in his own timezone; the \
         list must say so: {rendered}"
    );
    assert!(
        !rendered.contains("2026-08-28"),
        "rendering the UTC date moves a night activity to the next morning — the \
         2026-08-28 defect: {rendered}"
    );
}

/// The same instant, no timezone on file: UTC, exactly as before.
///
/// Guards the fallback rather than the fix — it passes either way, and exists so
/// a future change cannot silently start guessing a zone for an athlete who has
/// not set one. The prompt anchor falls back to UTC too, so the two still agree.
#[test]
fn without_a_timezone_the_list_stays_on_utc() {
    let temps = HashMap::new();
    let rendered =
        format_activities_as_list(&night_hike(), &temps, None, "fr", None, incident_clock());

    assert!(
        rendered.contains("2026-08-28 ven 02:59"),
        "with no zone on file the list stays on UTC: {rendered}"
    );
}

/// An unparseable zone is not an error — it falls back, it does not panic.
#[test]
fn an_unparseable_timezone_falls_back_rather_than_failing() {
    let temps = HashMap::new();
    let rendered = format_activities_as_list(
        &night_hike(),
        &temps,
        None,
        "fr",
        Some("Mars/Olympus_Mons"),
        incident_clock(),
    );

    assert!(
        rendered.contains("2026-08-28 ven 02:59"),
        "a zone that does not exist must degrade to UTC: {rendered}"
    );
}

/// The clock reaches the row, not just the date: a 06:00 run and a 22:59 hike
/// must be distinguishable, which is what lets an agent say "this morning" at all.
#[test]
fn the_row_carries_the_time_of_day() {
    let temps = HashMap::new();
    let morning = Utc.with_ymd_and_hms(2026, 8, 28, 10, 15, 0).unwrap();
    let activities = vec![ActivityBuilder::new(
        "run-1",
        "Sortie du matin",
        SportType::Run,
        morning,
        1_800,
        "sciotte",
    )
    .distance_meters(5_300.0)
    .build()];

    let rendered = format_activities_as_list(
        &activities,
        &temps,
        None,
        "fr",
        Some("America/Toronto"),
        incident_clock(),
    );

    assert!(
        rendered.contains("2026-08-28 ven 06:15"),
        "10:15 UTC is 06:15 in Toronto, and the coach cannot tell morning from \
         evening without it: {rendered}"
    );
}

/// The weekday is stated, and it is the athlete's weekday.
///
/// Production Telegram, 2026-09-02: no surface in the prompt named a weekday.
/// The rows carried a bare `%Y-%m-%d` and the model derived "dimanche" /
/// "mardi" / "jeudi" from them by calendar arithmetic — the same class of error
/// the epoch table in `prompt_assembly` exists to remove. It got them wrong,
/// reassigned the same five activities three times across the conversation
/// (*"road 2 aus etait hier, mardi. T'es melé big"*, *"date ride etait lundi.
/// Ca va pas les dates"*), and the athlete left over it.
///
/// The 22:59-local instant makes the two halves inseparable: a UTC weekday here
/// would read `ven`, a whole day off, so this fails both if the weekday goes
/// missing and if it is derived in the wrong zone.
#[test]
fn the_row_names_the_athletes_weekday_not_the_utc_one() {
    let temps = HashMap::new();
    let rendered = format_activities_as_list(
        &night_hike(),
        &temps,
        None,
        "fr",
        Some("America/Toronto"),
        incident_clock(),
    );

    assert!(
        rendered.contains("2026-08-27 jeu"),
        "2026-08-27 was a Thursday in Toronto and the row must say so rather \
         than leaving the model to work it out: {rendered}"
    );
    assert!(
        !rendered.contains("ven"),
        "UTC would make this Friday — the weekday must follow the athlete's \
         zone, not the server's: {rendered}"
    );
}

/// The weekday renders in the athlete's language, like every other label on the
/// row. An English `Thu` inside a French list is the kind of seam the model
/// paraphrases rather than copies.
#[test]
fn the_weekday_follows_the_chat_locale() {
    let temps = HashMap::new();
    for (locale, expected) in [
        ("fr", "jeu"),
        ("en", "Thu"),
        ("es", "jue"),
        ("de", "Do"),
        ("pt", "qui"),
    ] {
        let rendered = format_activities_as_list(
            &night_hike(),
            &temps,
            None,
            locale,
            Some("America/Toronto"),
            incident_clock(),
        );
        assert!(
            rendered.contains(&format!("2026-08-27 {expected}")),
            "locale {locale} must render the weekday as {expected}: {rendered}"
        );
    }
}

/// The 2026-09-21 incident, as the list now renders it.
///
/// Production Telegram, 08:05 in `America/Toronto`: the athlete asked whether a
/// 30-minute ride *that morning* would be too much. The newest row was a ride
/// he had done the previous afternoon — `2026-09-20 dim 16:59` — and the reply
/// called it "ce matin", then dated the rest of the week a day early to match.
/// The date anchor said 09-21; the model took the newest row as today anyway.
///
/// So the list says it: the header names today and the newest row, and the row
/// itself carries `(hier)`. Both are one subtraction the model no longer does.
#[test]
fn yesterdays_ride_is_tagged_hier_and_the_header_names_today() {
    let temps = HashMap::new();
    // 16:59 on Sunday the 20th in Toronto is 20:59 UTC.
    let ride = vec![ActivityBuilder::new(
        "ride-1",
        "Tester la nouvelle!",
        SportType::MountainBike,
        Utc.with_ymd_and_hms(2026, 9, 20, 20, 59, 15).unwrap(),
        4_671,
        "sciotte",
    )
    .distance_meters(14_140.0)
    .build()];
    // 08:05 on Monday the 21st in Toronto is 12:05 UTC.
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 5, 0).unwrap();

    let rendered =
        format_activities_as_list(&ride, &temps, None, "fr", Some("America/Toronto"), now);

    assert!(
        rendered.contains("2026-09-20 dim 16:59 (hier)"),
        "the row must say it was yesterday, next to the date it already \
         carries: {rendered}"
    );
    assert!(
        rendered.contains("[Today] 2026-09-21 lun — newest activity listed: 2026-09-20 dim (hier)"),
        "the header must name the athlete's today and place the newest row \
         relative to it: {rendered}"
    );
    assert!(
        !rendered.contains("aujourd'hui"),
        "nothing in this list happened today: {rendered}"
    );
}

/// Today and yesterday are the athlete's, not the server's.
///
/// 23:30 on the 20th in Toronto is 03:30 UTC on the 21st. A 06:00 run that
/// morning is `today` for the athlete; on the UTC calendar it is yesterday.
#[test]
fn the_relative_day_follows_the_athletes_midnight_not_utc() {
    let temps = HashMap::new();
    let run = vec![ActivityBuilder::new(
        "run-1",
        "Sortie du matin",
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 9, 20, 10, 0, 0).unwrap(),
        1_800,
        "sciotte",
    )
    .distance_meters(5_000.0)
    .build()];
    let late_evening = Utc.with_ymd_and_hms(2026, 9, 21, 3, 30, 0).unwrap();

    let rendered = format_activities_as_list(
        &run,
        &temps,
        None,
        "fr",
        Some("America/Toronto"),
        late_evening,
    );

    assert!(
        rendered.contains("2026-09-20 dim 06:00 (aujourd'hui)"),
        "at 23:30 local the morning run is still today: {rendered}"
    );
    assert!(
        rendered.contains("[Today] 2026-09-20 dim"),
        "the header's today is the athlete's day, not the UTC one: {rendered}"
    );
}

/// A row older than yesterday carries no tag — its date and weekday already
/// say everything, and "two days ago" is not how a session is referred to.
#[test]
fn older_rows_carry_no_relative_tag() {
    let temps = HashMap::new();
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 5, 0).unwrap();

    let rendered = format_activities_as_list(
        &night_hike(),
        &temps,
        None,
        "fr",
        Some("America/Toronto"),
        now,
    );

    assert!(
        rendered.contains("2026-08-27 jeu 22:59 - "),
        "an August row must render date, weekday, time and then the distance \
         with nothing in between: {rendered}"
    );
    assert!(
        !rendered.contains("(hier)") && !rendered.contains("(aujourd'hui)"),
        "no row here is today or yesterday: {rendered}"
    );
    assert!(
        rendered.contains("newest activity listed: 2026-08-27 jeu\n"),
        "the header names the newest row without a relative tag: {rendered}"
    );
}

/// The tag speaks the row's language, like the weekday beside it.
#[test]
fn the_relative_tag_follows_the_chat_locale() {
    let temps = HashMap::new();
    let ride = vec![ActivityBuilder::new(
        "ride-1",
        "Tester la nouvelle!",
        SportType::MountainBike,
        Utc.with_ymd_and_hms(2026, 9, 20, 20, 59, 15).unwrap(),
        4_671,
        "sciotte",
    )
    .build()];
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 5, 0).unwrap();

    for (locale, expected) in [
        ("fr", "(hier)"),
        ("en", "(yesterday)"),
        ("es", "(ayer)"),
        ("de", "(gestern)"),
        ("pt", "(ontem)"),
    ] {
        let rendered =
            format_activities_as_list(&ride, &temps, None, locale, Some("America/Toronto"), now);
        assert!(
            rendered.contains(expected),
            "locale {locale} must tag yesterday as {expected}: {rendered}"
        );
    }
}

/// With nothing to list, the header still states today — it is the one line
/// a stale-data branch in the prompt contract can anchor on.
#[test]
fn an_empty_list_still_states_today() {
    let temps = HashMap::new();
    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 5, 0).unwrap();

    let rendered = format_activities_as_list(&[], &temps, None, "fr", Some("America/Toronto"), now);

    assert!(
        rendered.contains("[Today] 2026-09-21 lun\n"),
        "an empty window still names the day: {rendered}"
    );
    assert!(
        !rendered.contains("newest activity listed"),
        "there is no newest row to name: {rendered}"
    );
}
