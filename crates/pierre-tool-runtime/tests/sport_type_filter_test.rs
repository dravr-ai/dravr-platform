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
fn wildcard_spanish_german_portuguese_return_every_activity() {
    // The platform ships fr/en/es/de/pt end to end and the coach answers in the
    // athlete's language, so the wildcard arrives in that language too. An
    // es/de/pt wildcard that filtered literally would reproduce the 2026-07-20
    // incident for those athletes: every activity dropped, "no recent data".
    for wildcard in [
        "todos",
        "todas",
        "todo",
        "cualquiera",
        "alle",
        "alles",
        "jede",
        "tudo",
        "qualquer",
    ] {
        let filtered = filter_activities_by_sport_type(seeded_activities(), Some(wildcard));
        assert_eq!(
            filtered.len(),
            3,
            "sport_type '{wildcard}' must pass every activity through, got {filtered:?}"
        );
    }
}

#[test]
fn wildcards_are_case_insensitive() {
    // The filter is compared after normalisation, so the capitalised form a
    // model actually writes at the start of a sentence still reads as a
    // wildcard rather than an unknown sport.
    for wildcard in ["ALL", "Tous", "Todos", "Alle"] {
        let filtered = filter_activities_by_sport_type(seeded_activities(), Some(wildcard));
        assert_eq!(
            filtered.len(),
            3,
            "sport_type '{wildcard}' must pass every activity through, got {filtered:?}"
        );
    }
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

fn cycling_activities() -> Vec<Activity> {
    let start = Utc.with_ymd_and_hms(2026, 8, 27, 12, 0, 0).unwrap();
    vec![
        ActivityBuilder::new(
            "ride-1",
            "Grosse boucane sul moteur",
            SportType::Ride,
            start,
            5_953,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "mtb-2",
            "Réveil matin",
            SportType::MountainBike,
            start,
            5_496,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "gravel-1",
            "Chemin du Roy",
            SportType::GravelRide,
            start,
            4_100,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "ebike-1",
            "Commute assisté",
            SportType::EbikeRide,
            start,
            1_800,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "vride-1",
            "Zwift sweet spot",
            SportType::VirtualRide,
            start,
            3_600,
            "sciotte",
        )
        .build(),
        ActivityBuilder::new(
            "walk-2",
            "Post Canadian, gros show de boucane",
            SportType::Walk,
            start,
            2_295,
            "sciotte",
        )
        .build(),
    ]
}

/// A generic `ride` ask means every discipline the athlete rode.
///
/// Exact equality made a cycling coach blind to cycling: an athlete whose
/// window held 22 mountain-bike and 7 gravel rides matched none of them, so a
/// `sport_types: ["Ride"]` coach saw an empty cycling history (2026-08-27).
#[test]
fn ride_filter_covers_the_whole_cycling_family() {
    let filtered = filter_activities_by_sport_type(cycling_activities(), Some("ride"));
    let mut names: Vec<&str> = filtered.iter().map(Activity::name).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "Chemin du Roy",
            "Commute assisté",
            "Grosse boucane sul moteur",
            "Réveil matin",
            "Zwift sweet spot",
        ],
        "'ride' must cover Ride, MountainBike, GravelRide, EbikeRide and VirtualRide, and only those"
    );
}

#[test]
fn french_velo_filter_covers_the_whole_cycling_family() {
    let filtered = filter_activities_by_sport_type(cycling_activities(), Some("vélo"));
    assert_eq!(
        filtered.len(),
        5,
        "'vélo' resolves to Ride and must cover the same family, got {filtered:?}"
    );
}

/// The mirror of the run-family rule: a *specific* discipline stays exact.
#[test]
fn a_specific_cycling_filter_stays_exact() {
    let filtered = filter_activities_by_sport_type(cycling_activities(), Some("gravel"));
    assert_eq!(filtered.len(), 1, "'gravel' must keep only the gravel ride");
    assert_eq!(filtered[0].name(), "Chemin du Roy");

    let filtered = filter_activities_by_sport_type(cycling_activities(), Some("vtt"));
    assert_eq!(
        filtered.len(),
        1,
        "'vtt' must keep only the mountain-bike ride"
    );
    assert_eq!(filtered[0].name(), "Réveil matin");
}

/// The incident shape: a mountain-bike ride must survive a generic ride ask.
#[test]
fn a_mountain_bike_ride_survives_a_generic_ride_filter() {
    let filtered = filter_activities_by_sport_type(seeded_activities(), Some("ride"));
    assert_eq!(
        filtered.len(),
        1,
        "the MTB session must not vanish from a 'ride' window, got {filtered:?}"
    );
    assert_eq!(filtered[0].name(), "Fin de ma job de testeur");
}
