// ABOUTME: Pins which activities count toward a goal: those after it was set, of its sport's family
// ABOUTME: A run goal counts trail and treadmill runs and never a ride; a goal naming no sport counts all
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `set_goal` advertised a `sport` it never stored, so a cyclist's 300 km of
//! riding read as 300% of a 100 km running goal. The goal now carries its
//! sport and `track_progress` counts through `activities_toward_goal`.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_tool_runtime::implementations::goals_spec::activities_toward_goal;

fn activity(id: &str, sport: SportType, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("session {id}"),
        sport,
        Utc::now() - Duration::days(days_ago),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

fn ids(activities: &[&Activity]) -> Vec<String> {
    activities.iter().map(|a| a.id().to_owned()).collect()
}

fn set_days_ago(days: i64) -> DateTime<FixedOffset> {
    (Utc::now() - Duration::days(days)).fixed_offset()
}

fn history() -> Vec<Activity> {
    vec![
        activity("road-run", SportType::Run, 2),
        activity("trail-run", SportType::TrailRunning, 3),
        activity("treadmill", SportType::VirtualRun, 4),
        activity("ride", SportType::Ride, 5),
        activity("gravel", SportType::GravelRide, 6),
        activity("swim", SportType::Swim, 7),
        activity("old-run", SportType::Run, 30),
    ]
}

#[test]
fn a_run_goal_counts_every_run_since_it_was_set_and_no_ride() {
    let history = history();
    let counted = activities_toward_goal(&history, Some(set_days_ago(10)), Some(&SportType::Run));
    assert_eq!(ids(&counted), vec!["road-run", "trail-run", "treadmill"]);
}

#[test]
fn a_ride_goal_counts_the_ride_family_only() {
    let history = history();
    let counted = activities_toward_goal(&history, Some(set_days_ago(10)), Some(&SportType::Ride));
    assert_eq!(ids(&counted), vec!["ride", "gravel"]);
}

#[test]
fn a_goal_naming_no_sport_counts_every_activity_since_it_was_set() {
    let history = history();
    let counted = activities_toward_goal(&history, Some(set_days_ago(10)), None);
    assert_eq!(counted.len(), 6, "everything but the run before the goal");
    assert!(!ids(&counted).contains(&"old-run".to_owned()));
}

#[test]
fn a_goal_with_no_creation_time_counts_its_sport_across_the_whole_history() {
    let history = history();
    let counted = activities_toward_goal(&history, None, Some(&SportType::Run));
    assert_eq!(
        ids(&counted),
        vec!["road-run", "trail-run", "treadmill", "old-run"]
    );
}
