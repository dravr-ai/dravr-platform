// ABOUTME: A coach's provider prerequisite means "needs activity data", satisfied by any connected provider
// ABOUTME: The missing-prerequisite message names the listed providers as examples, joined with "or"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashSet;

use pierre_core::models::coaches::CoachPrerequisites;
use pierre_services::coaches::check_prerequisites;

fn prereqs(providers: &[&str]) -> CoachPrerequisites {
    CoachPrerequisites {
        providers: providers.iter().map(|p| (*p).to_owned()).collect(),
        ..CoachPrerequisites::default()
    }
}

fn connected(providers: &[&str]) -> HashSet<String> {
    providers.iter().map(|p| (*p).to_owned()).collect()
}

/// Fifteen catalogue coaches declare `providers: [strava]`. A Garmin athlete
/// has the activity data they need, and must not be told to connect Strava.
#[test]
fn a_garmin_athlete_meets_a_strava_only_prerequisite() {
    let result = check_prerequisites(&prereqs(&["strava"]), &connected(&["garmin"]));
    assert!(result.met, "any connected provider satisfies the list");
    assert!(result.missing.is_empty());
}

/// With nothing connected, the one missing prerequisite names the listed
/// providers as examples, joined the way a sentence reads.
#[test]
fn with_no_provider_connected_the_message_names_the_listed_ones() {
    let single = check_prerequisites(&prereqs(&["strava"]), &connected(&[]));
    assert!(!single.met);
    assert_eq!(single.missing.len(), 1, "one entry, not one per provider");
    assert_eq!(single.missing[0].prerequisite_type, "provider");
    assert_eq!(single.missing[0].requirement, "strava");
    assert_eq!(
        single.missing[0].message,
        "Connect Strava to unlock this coach"
    );

    let many = check_prerequisites(&prereqs(&["strava", "garmin", "fitbit"]), &connected(&[]));
    assert_eq!(many.missing.len(), 1);
    assert_eq!(many.missing[0].requirement, "strava, garmin, fitbit");
    assert_eq!(
        many.missing[0].message,
        "Connect Strava, Garmin or Fitbit to unlock this coach"
    );
}

/// A coach with no provider prerequisite needs nothing, connected or not.
#[test]
fn an_empty_list_needs_nothing() {
    assert!(check_prerequisites(&prereqs(&[]), &connected(&[])).met);
    assert!(check_prerequisites(&prereqs(&[]), &connected(&["whoop"])).met);
}
