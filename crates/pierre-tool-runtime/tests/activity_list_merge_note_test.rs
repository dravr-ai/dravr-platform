// ABOUTME: Pins the activity list's session-merge note: what each session combines
// ABOUTME: Providers of every recording and the fields a session took from another one
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An athlete with Strava and WHOOP connected records one ride twice. The list
//! the agent reads holds one session, and the note above it has to say so —
//! and say that the session carries WHOOP's calories — or the agent attributes
//! every number to Strava.

use std::collections::HashMap;

use chrono::{TimeZone, Utc};
use pierre_core::models::{ActivityBuilder, SportType};
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_tool_runtime::implementations::activity_list_render::format_activities_as_list;

#[test]
fn the_note_names_both_providers_and_the_fields_the_session_took() {
    let start = Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap();
    let recordings = vec![
        ActivityBuilder::new("s-1", "Long ride", SportType::Ride, start, 24_000, "strava")
            .distance_meters(200_000.0)
            .build(),
        ActivityBuilder::new("w-1", "Run", SportType::Run, start, 23_800, "whoop")
            .calories(4_100)
            .build(),
    ];
    let (sessions, report) = merge_duplicates(recordings, &DedupConfig::default());

    let rendered = format_activities_as_list(
        &sessions,
        &HashMap::new(),
        Some(&report),
        "en",
        None,
        Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap(),
    );

    assert!(
        rendered.contains("[Note] 2 recordings, representing 1 distinct training sessions"),
        "{rendered}"
    );
    assert!(
        rendered.contains("session s-1; recordings: [s-1, w-1] from strava + whoop"),
        "{rendered}"
    );
    assert!(
        rendered.contains("; took calories from whoop"),
        "{rendered}"
    );
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].calories(), Some(4_100));
}

#[test]
fn no_note_when_nothing_was_merged() {
    let start = Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap();
    let (sessions, report) = merge_duplicates(
        vec![
            ActivityBuilder::new("s-1", "Run", SportType::Run, start, 1_800, "strava")
                .distance_meters(5_000.0)
                .build(),
        ],
        &DedupConfig::default(),
    );
    let rendered = format_activities_as_list(
        &sessions,
        &HashMap::new(),
        Some(&report),
        "en",
        None,
        Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap(),
    );
    assert!(!rendered.contains("[Note]"), "{rendered}");
}

#[test]
fn a_long_page_of_merged_sessions_lists_ten_and_counts_the_rest() {
    let mut recordings = Vec::new();
    for day in 0..14_i64 {
        let start =
            Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap() + chrono::Duration::days(day);
        recordings.push(
            ActivityBuilder::new(
                format!("s-{day}"),
                "Ride",
                SportType::Ride,
                start,
                3_600,
                "strava",
            )
            .distance_meters(30_000.0)
            .build(),
        );
        recordings.push(
            ActivityBuilder::new(
                format!("g-{day}"),
                "Ride",
                SportType::Ride,
                start,
                3_600,
                "garmin",
            )
            .distance_meters(30_020.0)
            .build(),
        );
    }
    let (sessions, report) = merge_duplicates(recordings, &DedupConfig::default());
    assert_eq!(sessions.len(), 14);

    let rendered = format_activities_as_list(
        &sessions,
        &HashMap::new(),
        Some(&report),
        "en",
        None,
        Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0).unwrap(),
    );

    assert_eq!(
        rendered.matches("       - session ").count(),
        10,
        "{rendered}"
    );
    assert!(
        rendered.contains("and 4 more merged sessions"),
        "{rendered}"
    );
}
