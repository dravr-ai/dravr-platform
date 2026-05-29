// ABOUTME: Integration tests for the fragment-detection module
// ABOUTME: Covers Garmin auto-split, dual-device overlap, brick adjacency, no-fragment cases
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cast_precision_loss)]
#![allow(missing_docs)]

//! Tests for [`pierre_providers::deduplication`].
//!
//! The fixtures here mimic the real-world fragment patterns the chat surface
//! exposed: 20 GPS recordings spread across a single day, mixing Garmin
//! auto-split, manual re-uploads, and dual-device captures. The detector must
//! group those into one canonical session per workout without coalescing
//! legitimate back-to-back sessions or brick workouts (ride immediately
//! followed by run).

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_providers::deduplication::{detect_fragments, DedupConfig, FragmentReport};

/// Helper: build a minimal `Activity` with id, sport, start, duration, distance.
fn make_activity(
    id: &str,
    sport: SportType,
    start: DateTime<Utc>,
    duration_secs: u64,
    distance_m: f64,
) -> Activity {
    ActivityBuilder::new(id, "Test Activity", sport, start, duration_secs, "test")
        .distance_meters(distance_m)
        .build()
}

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 22, 14, 0, 0).unwrap()
}

#[test]
fn empty_input_returns_zero_counts() {
    let report = detect_fragments(&[], &DedupConfig::default());
    assert_eq!(report.raw_count, 0);
    assert_eq!(report.session_count, 0);
    assert!(report.groups.is_empty());
    assert!(!report.has_fragments());
}

#[test]
fn single_activity_is_one_session_with_no_groups() {
    let activities = vec![make_activity(
        "1",
        SportType::Run,
        base_time(),
        1800,
        5000.0,
    )];
    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.raw_count, 1);
    assert_eq!(report.session_count, 1);
    assert!(report.groups.is_empty());
    assert!(report.is_canonical("1"));
}

#[test]
fn garmin_auto_split_three_fragments_collapse_to_one() {
    // Garmin sometimes splits a single ~45-min trail run into 3 chunks when
    // GPS drops briefly. The fragments are back-to-back with ~5 sec gaps.
    let t = base_time();
    let activities = vec![
        make_activity("frag-a", SportType::Run, t, 900, 2500.0), // 0–15 min
        make_activity(
            "frag-b",
            SportType::Run,
            t + Duration::seconds(905),
            900,
            2400.0,
        ), // 15:05–30:05
        make_activity(
            "frag-c",
            SportType::Run,
            t + Duration::seconds(1810),
            900,
            2400.0,
        ), // 30:10–45:10
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());

    assert_eq!(report.raw_count, 3);
    assert_eq!(report.session_count, 1);
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(group.fragment_ids.len(), 3);
    // Canonical: longest duration is tied (all 900s), distance tiebreak picks
    // the largest (2500.0 → frag-a).
    assert_eq!(group.canonical_id, "frag-a");
}

#[test]
fn dual_device_recording_groups_watch_and_bike_computer() {
    // Watch and bike computer both record the same 4-hour ride, starting 30
    // sec apart, ending 90 sec apart.
    let t = base_time();
    let activities = vec![
        make_activity("watch", SportType::Ride, t, 14_400, 95_000.0),
        make_activity(
            "bikecomp",
            SportType::Ride,
            t + Duration::seconds(30),
            14_490,
            96_500.0,
        ),
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.session_count, 1);
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    // Longest duration wins — bikecomp (14_490s > 14_400s).
    assert_eq!(group.canonical_id, "bikecomp");
    assert_eq!(group.fragment_ids.len(), 2);
}

#[test]
fn brick_workout_ride_then_run_stays_two_distinct_sessions() {
    // Cycling immediately followed by a run is a triathlon brick — distinct
    // sport_type means they must NEVER be grouped, even when temporally
    // adjacent.
    let t = base_time();
    let activities = vec![
        make_activity("ride", SportType::Ride, t, 3600, 40_000.0), // 0–60 min
        make_activity(
            "brick-run",
            SportType::Run,
            t + Duration::seconds(3605),
            1800,
            5000.0,
        ), // 60:05–90:05
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.session_count, 2);
    assert!(report.groups.is_empty());
}

#[test]
fn back_to_back_same_sport_with_long_gap_stays_distinct() {
    // Two trail runs 30 min apart on the same day are two distinct sessions,
    // not fragments. With the default 5-min tolerance, the gap exceeds the
    // threshold.
    let t = base_time();
    let activities = vec![
        make_activity("morning-run", SportType::Run, t, 1800, 5000.0), // 0–30
        make_activity(
            "afternoon-run",
            SportType::Run,
            t + Duration::seconds(3600), // 60 min after start = 30 min after end
            1800,
            5000.0,
        ),
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.session_count, 2);
    assert!(report.groups.is_empty());
}

#[test]
fn twenty_recordings_fold_to_three_sessions_across_three_workouts() {
    // The shape that triggered this whole investigation: ~20 GPS recordings
    // on a single day across three real workouts (one trail run + one ride +
    // one hike), each with multiple overlapping fragments from Garmin
    // auto-split + watch/bike-computer dual capture + a manual Strava
    // re-upload of the trail run.
    let day = base_time();
    let mut activities = Vec::with_capacity(20);

    // Workout 1: trail run, 7 GPS fragments around 14:00–15:30 UTC
    let run_start = day;
    for i in 0_i64..7 {
        let offset = Duration::seconds(i * 600); // each fragment ~10 min after prev
        let i_f = i as f64;
        activities.push(make_activity(
            &format!("run-{i}"),
            SportType::Run,
            run_start + offset,
            900, // 15 min each
            i_f.mul_add(100.0, 2500.0),
        ));
    }

    // Workout 2: ride, 10 dual-device fragments around 09:00–12:30 local
    // (synthesized as a separate non-overlapping window from the run)
    let ride_start = day + Duration::hours(4);
    for i in 0_i64..10 {
        let offset = Duration::seconds(i * 60); // staggered starts within 10 min
        let i_f = i as f64;
        activities.push(make_activity(
            &format!("ride-{i}"),
            SportType::Ride,
            ride_start + offset,
            12_600, // 3:30 hr each
            i_f.mul_add(200.0, 85_000.0),
        ));
    }

    // Workout 3: a real hike, 3 fragments spanning the trail
    let hike_start = day + Duration::hours(9);
    for i in 0_i64..3 {
        let offset = Duration::seconds(i * 30);
        let i_f = i as f64;
        activities.push(make_activity(
            &format!("hike-{i}"),
            SportType::Hike,
            hike_start + offset,
            5400, // 90 min each
            i_f.mul_add(50.0, 8000.0),
        ));
    }

    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.raw_count, 20);
    assert_eq!(report.session_count, 3, "{report:?}");
    assert_eq!(report.groups.len(), 3);
    assert_canonicals_present(&report, &["run-6", "ride-9", "hike-2"]);
}

/// Assert that the report's groups contain exactly the canonicals listed,
/// regardless of order. Distance tiebreaks pick the highest-distance member
/// when durations tie, hence "run-6" (largest distance among run-0..6).
fn assert_canonicals_present(report: &FragmentReport, expected: &[&str]) {
    let canonicals: BTreeSet<_> = report
        .groups
        .iter()
        .map(|g| g.canonical_id.as_str())
        .collect();
    let want: BTreeSet<_> = expected.iter().copied().collect();
    assert_eq!(canonicals, want);
}

#[test]
fn canonical_activities_filter_collapses_fragments_keeping_one_per_group() {
    let t = base_time();
    let activities = vec![
        make_activity("a", SportType::Run, t, 1000, 5000.0),
        make_activity(
            "b",
            SportType::Run,
            t + Duration::seconds(100),
            1200,
            6000.0,
        ),
        make_activity("c", SportType::Ride, t + Duration::hours(5), 3600, 40_000.0),
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());
    let canonical = report.canonical_activities(&activities);
    assert_eq!(canonical.len(), 2);
    let ids: Vec<_> = canonical.iter().map(|a| a.id().to_owned()).collect();
    assert!(ids.contains(&"b".to_owned()));
    assert!(ids.contains(&"c".to_owned()));
}

#[test]
fn group_lookup_finds_membership_and_canonicality() {
    let t = base_time();
    let activities = vec![
        make_activity("a", SportType::Run, t, 1000, 5000.0),
        make_activity(
            "b",
            SportType::Run,
            t + Duration::seconds(100),
            1200,
            6000.0,
        ),
        make_activity(
            "solo",
            SportType::Ride,
            t + Duration::hours(5),
            3600,
            40_000.0,
        ),
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());

    let group = report.group_of("a").expect("a is in a group");
    assert_eq!(group.canonical_id, "b");
    assert!(group.fragment_ids.contains(&"a".to_owned()));
    assert!(group.fragment_ids.contains(&"b".to_owned()));

    assert!(!report.is_canonical("a"));
    assert!(report.is_canonical("b"));
    // Solo activity is not part of any group → is_canonical returns true
    // because there is no group to demote it within.
    assert!(report.is_canonical("solo"));
    assert!(report.group_of("solo").is_none());
}

#[test]
fn custom_tolerance_tightens_grouping() {
    // With a 10-sec tolerance, fragments more than 10 sec apart don't merge.
    let t = base_time();
    let activities = vec![
        make_activity("a", SportType::Run, t, 60, 200.0),
        make_activity(
            "b",
            SportType::Run,
            t + Duration::seconds(80), // 20-sec gap > 10-sec tolerance
            60,
            200.0,
        ),
    ];
    let tight = DedupConfig::with_tolerance(10);
    let report = detect_fragments(&activities, &tight);
    assert_eq!(report.session_count, 2);
    assert!(report.groups.is_empty());
}

#[test]
fn determinism_same_input_yields_same_canonical() {
    // Construct a tie scenario: two activities with identical duration and
    // distance — canonical selection falls through to id order (lowest wins).
    let t = base_time();
    let activities = vec![
        make_activity("zzz", SportType::Run, t, 600, 2000.0),
        make_activity(
            "aaa",
            SportType::Run,
            t + Duration::seconds(60),
            600,
            2000.0,
        ),
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].canonical_id, "aaa");
}

#[test]
fn midnight_starts_are_not_merged_as_fragments() {
    // Sciotte's date-only list scraping yields T00:00:00 (UTC midnight) when the
    // start time is unknown. Three distinct same-sport sessions that all land on
    // midnight must NOT collapse into one bogus fragment group — every midnight
    // row trivially "overlaps" every other, which produced the "N sessions
    // today" hallucination. Unknown-time rows stay standalone.
    let midnight = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 0).unwrap();
    let activities = vec![
        make_activity("a", SportType::Run, midnight, 1800, 5000.0),
        make_activity("b", SportType::Run, midnight, 2400, 7000.0),
        make_activity("c", SportType::Run, midnight, 3000, 9000.0),
    ];
    let report = detect_fragments(&activities, &DedupConfig::default());
    assert_eq!(report.raw_count, 3);
    assert_eq!(
        report.session_count, 3,
        "midnight rows must stay distinct sessions"
    );
    assert!(
        report.groups.is_empty(),
        "no fragment groups should form from unknown-time (midnight) rows"
    );
}
