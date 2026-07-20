// ABOUTME: Regression tests for the get_activities sport_type filter
// ABOUTME: Pins that wildcard inputs ("all", "tous") pass every activity through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests for [`pierre_tool_runtime::implementations::fitness_support::filter_activities_by_sport_type`].
//!
//! An LLM commonly asks for every sport with `sport_type: "all"` (or the
//! French `"tous"`). 2026-07-20 dev incident: "all" was matched literally
//! against sport names, dropping all 10 scraped activities and making the
//! coach report "no recent data" while fresh activities existed. Wildcards
//! must pass the list through untouched; real sport filters must keep
//! filtering.

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::implementations::fitness_support::filter_activities_by_sport_type;

fn seeded_activities() -> Vec<Activity> {
    let start = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    vec![
        ActivityBuilder::new(
            "walk-1",
            "Afternoon Walk",
            SportType::Walk,
            start,
            6_081,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "mtb-1",
            "Fin de ma job de testeur",
            SportType::MountainBike,
            start,
            4_926,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "trail-1",
            "Relaxe avec les filles",
            SportType::TrailRunning,
            start,
            2_555,
            "sciotte",
        )
        .build(),
    ]
}

#[test]
fn wildcard_all_returns_every_activity() {
    let filtered = filter_activities_by_sport_type(seeded_activities(), Some("all"));
    assert_eq!(
        filtered.len(),
        3,
        "sport_type 'all' must pass every activity through, got {filtered:?}"
    );
}

#[test]
fn wildcard_french_tous_returns_every_activity() {
    let filtered = filter_activities_by_sport_type(seeded_activities(), Some("tous"));
    assert_eq!(
        filtered.len(),
        3,
        "sport_type 'tous' must pass every activity through, got {filtered:?}"
    );
}

#[test]
fn blank_filter_returns_every_activity() {
    let filtered = filter_activities_by_sport_type(seeded_activities(), Some("  "));
    assert_eq!(
        filtered.len(),
        3,
        "blank sport_type must pass every activity through, got {filtered:?}"
    );
}

#[test]
fn real_sport_filter_still_filters() {
    let filtered = filter_activities_by_sport_type(seeded_activities(), Some("walk"));
    assert_eq!(filtered.len(), 1, "'walk' must keep only the walk");
    assert_eq!(filtered[0].name(), "Afternoon Walk");
}

#[test]
fn unknown_sport_still_matches_nothing() {
    let filtered = filter_activities_by_sport_type(seeded_activities(), Some("underwaterhockey"));
    assert!(
        filtered.is_empty(),
        "an unknown concrete sport must not act as a wildcard, got {filtered:?}"
    );
}
