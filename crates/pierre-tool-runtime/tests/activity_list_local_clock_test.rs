// ABOUTME: Pins that the coach's activity list renders the athlete's civil clock
// ABOUTME: A night activity must not be attributed to the following morning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The list block is the coach's only view of the athlete's training on a
//! grounded turn, and the only place it quotes dates back to them.
//!
//! Production Telegram, 2026-08-28: an athlete in `America/Toronto` started a
//! hike at 22:59 on the 27th — Strava named it "Night Hike". Rendered in UTC it
//! became `2026-08-28`, while the prompt's own `{{CURRENT_DATE}}` anchor said
//! `2026-08-27 22:59 (America/Toronto)`. Two calendars in one prompt. The coach
//! read the row's date as local, invented a time of day to fit, and told him it
//! was "ce matin". Every activity after ~20:00 local moved to the next day, and
//! day-counting drifted with it.

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
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

#[test]
fn an_evening_activity_keeps_the_athletes_date_not_the_utc_one() {
    let temps = HashMap::new();
    let rendered =
        format_activities_as_list(&night_hike(), &temps, None, "fr", Some("America/Toronto"));

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
    let rendered = format_activities_as_list(&night_hike(), &temps, None, "fr", None);

    assert!(
        rendered.contains("2026-08-28 ven 02:59"),
        "with no zone on file the list stays on UTC: {rendered}"
    );
}

/// An unparseable zone is not an error — it falls back, it does not panic.
#[test]
fn an_unparseable_timezone_falls_back_rather_than_failing() {
    let temps = HashMap::new();
    let rendered =
        format_activities_as_list(&night_hike(), &temps, None, "fr", Some("Mars/Olympus_Mons"));

    assert!(
        rendered.contains("2026-08-28 ven 02:59"),
        "a zone that does not exist must degrade to UTC: {rendered}"
    );
}

/// The clock reaches the row, not just the date: a 06:00 run and a 22:59 hike
/// must be distinguishable, which is what lets a coach say "this morning" at all.
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

    let rendered =
        format_activities_as_list(&activities, &temps, None, "fr", Some("America/Toronto"));

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
    let rendered =
        format_activities_as_list(&night_hike(), &temps, None, "fr", Some("America/Toronto"));

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
        let rendered =
            format_activities_as_list(&night_hike(), &temps, None, locale, Some("America/Toronto"));
        assert!(
            rendered.contains(&format!("2026-08-27 {expected}")),
            "locale {locale} must render the weekday as {expected}: {rendered}"
        );
    }
}
