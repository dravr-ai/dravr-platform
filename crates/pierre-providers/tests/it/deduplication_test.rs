// ABOUTME: Integration tests for session merging across recordings and providers
// ABOUTME: Auto-split, dual-device, cross-provider, cross-sport, field enrichment, no-merge cases
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
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::Feel;
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_providers::deduplication::{merge_duplicates, DedupConfig, FilledField, FragmentReport};

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

/// Merge `activities` and return only the report.
fn detect(activities: &[Activity], config: &DedupConfig) -> FragmentReport {
    merge_duplicates(activities.to_vec(), config).1
}

fn ids(activities: &[Activity]) -> Vec<&str> {
    activities.iter().map(Activity::id).collect()
}

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 22, 14, 0, 0).unwrap()
}

#[test]
fn empty_input_returns_zero_counts() {
    let report = detect(&[], &DedupConfig::default());
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
    let report = detect(&activities, &DedupConfig::default());
    assert_eq!(report.raw_count, 1);
    assert_eq!(report.session_count, 1);
    assert!(report.groups.is_empty());
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
    let report = detect(&activities, &DedupConfig::default());

    assert_eq!(report.raw_count, 3);
    assert_eq!(report.session_count, 1);
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(group.fragment_ids.len(), 3);
    // Canonical: longest duration is tied (all 900s), distance tiebreak picks
    // the largest (2500.0 → frag-a).
    assert_eq!(group.canonical_id, "frag-a");

    // The session is the three pieces together (carnet#1248).
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(ids(&merged), vec!["frag-a"]);
    assert_eq!(merged[0].distance_meters(), Some(7_300.0));
    // Elapsed runs from the first start to the last end, the two 5 s gaps
    // included; start + duration is the session's real end.
    assert_eq!(merged[0].duration_seconds(), 2_710);
    assert_eq!(merged[0].start_date(), t);
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
    let report = detect(&activities, &DedupConfig::default());
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
    let report = detect(&activities, &DedupConfig::default());
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
    let report = detect(&activities, &DedupConfig::default());
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

    let report = detect(&activities, &DedupConfig::default());
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
fn merged_list_keeps_one_row_per_session_in_input_order() {
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
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(ids(&merged), vec!["b", "c"]);
    assert_eq!(report.session_count, 2);
    let group = &report.groups[0];
    assert_eq!(group.canonical_id, "b");
    assert_eq!(group.fragment_ids, vec!["b", "a"]);
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
    let report = detect(&activities, &tight);
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
    let report = detect(&activities, &DedupConfig::default());
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
    let report = detect(&activities, &DedupConfig::default());
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

fn provider_activity(
    id: &str,
    provider: &str,
    sport: SportType,
    start: DateTime<Utc>,
    duration_secs: u64,
) -> ActivityBuilder {
    ActivityBuilder::new(id, "Session", sport, start, duration_secs, provider)
}

#[test]
fn same_ride_from_two_providers_counts_once() {
    // The "Phil ride": 3.2 h / 84.8 km synced from both Strava and a Garmin
    // mirror. Same sport, distance within tolerance; Strava's copy of the
    // recording starts with it (one starting more than 5 s off would read
    // as cropped on Strava, carnet#1274).
    let t = base_time();
    let activities = vec![
        provider_activity(
            "strava-1",
            "strava",
            SportType::Ride,
            t + Duration::minutes(2),
            11_520,
        )
        .distance_meters(84_800.0)
        .build(),
        provider_activity(
            "garmin-1",
            "garmin",
            SportType::Ride,
            t + Duration::minutes(2),
            11_520,
        )
        .distance_meters(84_750.0)
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());
    // The one precedence rule puts the recording device's service ahead of
    // Strava's own copy (Athlete Data Consolidation §3.3).
    assert_eq!(ids(&merged), vec!["garmin-1"]);
    assert_eq!(report.groups[0].providers, vec!["garmin", "strava"]);
}

#[test]
fn distinct_rides_same_day_from_two_providers_are_both_kept() {
    let t = base_time();
    let activities = vec![
        provider_activity("strava-morning", "strava", SportType::Ride, t, 3600)
            .distance_meters(30_000.0)
            .build(),
        provider_activity(
            "garmin-evening",
            "garmin",
            SportType::Ride,
            t + Duration::hours(8),
            5400,
        )
        .distance_meters(60_000.0)
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(merged.len(), 2);
    assert!(!report.has_fragments());
}

#[test]
fn same_provider_overlapping_rides_are_one_session() {
    // A watch and a bike computer both upload one ride to Strava: two rows,
    // one provider, overlapping wall-clock time. Nobody rides two 20 km rides
    // at once, so the analytics and the listing both see one session.
    let t = base_time();
    let activities = vec![
        provider_activity("strava-a", "strava", SportType::Ride, t, 3600)
            .distance_meters(20_000.0)
            .build(),
        provider_activity(
            "strava-b",
            "strava",
            SportType::Ride,
            t + Duration::minutes(1),
            3600,
        )
        .distance_meters(20_000.0)
        .average_power(210)
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(ids(&merged), vec!["strava-a"]);
    // The lower id wins the tie and takes the bike computer's power.
    assert_eq!(merged[0].average_power(), Some(210));
    assert_eq!(report.groups[0].providers, vec!["strava"]);
}

#[test]
fn wrist_tracker_misclassified_ride_folds_into_the_gps_ride_and_donates_its_fields() {
    // 2026-08-22: WHOOP recorded a 200 km ride as a distance-less "run";
    // Strava had the real ride. Different sports, two providers, ~99% overlap.
    let t = base_time();
    let activities = vec![
        provider_activity(
            "whoop-run",
            "whoop",
            SportType::Run,
            t + Duration::minutes(2),
            23_700,
        )
        .average_heart_rate(141)
        .calories(4_100)
        .training_stress_score(17.9)
        .build(),
        provider_activity("strava-ride", "strava", SportType::Ride, t, 24_000)
            .distance_meters(200_000.0)
            .average_heart_rate(133)
            .average_power(185)
            .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["strava-ride"]);
    let ride = &merged[0];
    assert_eq!(ride.sport_type(), &SportType::Ride);
    assert_eq!(ride.distance_meters(), Some(200_000.0));
    // Strava's own heart rate stands; WHOOP only fills what Strava lacked.
    assert_eq!(ride.average_heart_rate(), Some(133));
    assert_eq!(ride.calories(), Some(4_100));
    assert_eq!(ride.training_stress_score(), Some(17.9));

    let group = &report.groups[0];
    assert_eq!(group.providers, vec!["strava", "whoop"]);
    assert_eq!(
        group.filled_fields,
        vec![
            FilledField {
                field: "calories",
                provider: "whoop".to_owned()
            },
            FilledField {
                field: "training_stress_score",
                provider: "whoop".to_owned()
            },
        ]
    );
}

#[test]
fn gps_recording_is_canonical_over_a_longer_farther_wrist_copy_with_a_distance() {
    // 2026-10-07: WHOOP reports a distance of its own, so carrying a distance
    // does not single out the GPS recording; a WHOOP copy a minute longer and
    // a little farther must not name the session.
    let t = base_time();
    let activities = vec![
        provider_activity("whoop-ride", "whoop", SportType::Ride, t, 6_272)
            .distance_meters(42_600.0)
            .build(),
        provider_activity("garmin-ride", "garmin", SportType::Ride, t, 6_212)
            .distance_meters(42_510.0)
            .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
            .build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(ids(&merged), vec!["garmin-ride"]);
    assert_eq!(merged[0].distance_meters(), Some(42_510.0));
}

#[test]
fn start_position_alone_marks_the_gps_recording_canonical() {
    // A copy whose payload carries no route overview still holds a GPS trace
    // when it carries a start position; a blank overview is no trace at all.
    let t = base_time();
    let activities = vec![
        provider_activity("whoop-ride", "whoop", SportType::Ride, t, 6_272)
            .distance_meters(42_600.0)
            .summary_polyline("   ".to_owned())
            .build(),
        provider_activity("strava-ride", "strava", SportType::Ride, t, 6_212)
            .distance_meters(42_510.0)
            .start_latitude(45.5017)
            .start_longitude(-73.5673)
            .build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(ids(&merged), vec!["strava-ride"]);
    assert_eq!(merged[0].start_latitude(), Some(45.5017));
}

#[test]
fn self_report_from_a_second_provider_lands_on_the_gps_session() {
    // Intervals.icu carries the athlete's RPE, feel and notes for a session
    // Strava also recorded; the merged session keeps both halves.
    let t = base_time();
    let activities = vec![
        provider_activity("strava-run", "strava", SportType::Run, t, 3_600)
            .distance_meters(12_000.0)
            .build(),
        provider_activity(
            "icu-run",
            "intervals_icu",
            SportType::Run,
            t + Duration::seconds(20),
            3_590,
        )
        .distance_meters(11_950.0)
        .perceived_exertion(7.0)
        .feel(Feel::Weak)
        .description("tempo felt hard".to_owned())
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["strava-run"]);
    assert_eq!(merged[0].perceived_exertion(), Some(7.0));
    assert_eq!(merged[0].feel(), Some(Feel::Weak));
    assert_eq!(merged[0].description(), Some("tempo felt hard"));
    let filled: Vec<&str> = report.groups[0]
        .filled_fields
        .iter()
        .map(|f| f.field)
        .collect();
    assert_eq!(filled, vec!["perceived_exertion", "feel", "description"]);
}

#[test]
fn an_auto_split_piece_adds_its_volume_but_none_of_its_averages() {
    // A Garmin ride split into 60 km and 28 km is one 88 km session
    // (carnet#1248). The tail's heart rate describes its own hour, not the
    // session, so it must not become the session's.
    let t = base_time();
    let activities = vec![
        provider_activity("ride-main", "garmin", SportType::Ride, t, 7_200)
            .distance_meters(60_000.0)
            .moving_time_seconds(7_000)
            .elevation_gain(500.0)
            .build(),
        provider_activity(
            "ride-tail",
            "garmin",
            SportType::Ride,
            t + Duration::seconds(7_203),
            3_600,
        )
        .distance_meters(28_000.0)
        .moving_time_seconds(3_500)
        .elevation_gain(150.0)
        .average_heart_rate(150)
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["ride-main"]);
    let session = &merged[0];
    assert_eq!(session.distance_meters(), Some(88_000.0));
    assert_eq!(session.duration_seconds(), 10_803);
    assert_eq!(session.moving_time_seconds(), Some(10_500));
    assert_eq!(session.elevation_gain(), Some(650.0));
    assert_eq!(session.start_date(), t);
    assert_eq!(session.average_heart_rate(), None);
    assert_eq!(report.groups[0].filled_fields, []);
    assert_eq!(
        report.groups[0].fragment_ids,
        vec!["ride-main", "ride-tail"]
    );
}

#[test]
fn a_chain_of_three_recordings_is_one_group_whatever_the_input_order() {
    // Watch ≈ bike computer (same provider, overlap) and bike computer ≈
    // Strava upload of it (two providers). One workout, three rows.
    let t = base_time();
    let watch = provider_activity("watch", "garmin", SportType::Ride, t, 7_200)
        .distance_meters(60_000.0)
        .build();
    let computer = provider_activity(
        "computer",
        "garmin",
        SportType::Ride,
        t + Duration::seconds(40),
        7_260,
    )
    .distance_meters(60_400.0)
    .build();
    // Strava's copy of the bike computer's recording: its window, so no
    // edit on Strava (carnet#1274).
    let upload = provider_activity(
        "upload",
        "strava",
        SportType::Ride,
        t + Duration::seconds(40),
        7_258,
    )
    .distance_meters(60_380.0)
    .suffer_score(88)
    .build();

    for order in [
        vec![watch.clone(), computer.clone(), upload.clone()],
        vec![upload, watch, computer],
    ] {
        let (merged, report) = merge_duplicates(order, &DedupConfig::default());
        assert_eq!(ids(&merged), vec!["computer"]);
        assert_eq!(merged[0].suffer_score(), Some(88));
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].fragment_ids.len(), 3);
    }
}

#[test]
fn a_date_only_scrape_pairs_with_the_timed_record_of_its_day() {
    // A scrape mirror resolves only the day (T00:00:00); the API copy of the
    // same run has the real start. Same sport, same day, matching distance.
    let day = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 0).unwrap();
    let activities = vec![
        provider_activity("scrape", "sciotte", SportType::Run, day, 2_400)
            .distance_meters(8_000.0)
            .build(),
        provider_activity(
            "api",
            "strava",
            SportType::Run,
            day + Duration::hours(17),
            2_410,
        )
        .distance_meters(8_010.0)
        .build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(merged.len(), 1);
}

#[test]
fn two_same_day_runs_captured_date_only_stay_two_beside_their_timed_rows() {
    // Garmin through its API (timed) and Strava through the usr/pwd capture
    // (date-only, midnight UTC), two 8 km runs at 07:00 and 18:00. Each
    // capture matches both timed runs by day and distance; the merge must
    // still pair each capture with one run, never chain all four into one.
    let day = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 0).unwrap();
    let activities = vec![
        provider_activity(
            "g-am",
            "garmin",
            SportType::Run,
            day + Duration::hours(7),
            2_400,
        )
        .distance_meters(8_000.0)
        .build(),
        provider_activity(
            "g-pm",
            "garmin",
            SportType::Run,
            day + Duration::hours(18),
            2_400,
        )
        .distance_meters(8_000.0)
        .build(),
        provider_activity("s-1", "sciotte", SportType::Run, day, 2_400)
            .source("strava")
            .distance_meters(8_000.0)
            .build(),
        provider_activity("s-2", "sciotte", SportType::Run, day, 2_400)
            .source("strava")
            .distance_meters(8_000.0)
            .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(merged.len(), 2, "two runs, each held twice: {merged:?}");
    assert_eq!(report.groups.len(), 2, "one group per run: {report:?}");
    for group in &report.groups {
        assert_eq!(
            group.fragment_ids.len(),
            2,
            "each run held twice: {group:?}"
        );
    }
}

#[test]
fn a_date_only_capture_pairs_with_the_run_of_its_distance() {
    // Same day, two timed runs of different lengths; one capture of each.
    // Each capture lands on the run whose distance it carries.
    let day = Utc.with_ymd_and_hms(2026, 5, 28, 0, 0, 0).unwrap();
    let activities = vec![
        provider_activity(
            "g-short",
            "garmin",
            SportType::Run,
            day + Duration::hours(7),
            1_800,
        )
        .distance_meters(5_000.0)
        .build(),
        provider_activity(
            "g-long",
            "garmin",
            SportType::Run,
            day + Duration::hours(18),
            3_600,
        )
        .distance_meters(5_300.0)
        .build(),
        provider_activity("s-long", "sciotte", SportType::Run, day, 3_600)
            .source("strava")
            .distance_meters(5_300.0)
            .build(),
        provider_activity("s-short", "sciotte", SportType::Run, day, 1_800)
            .source("strava")
            .distance_meters(5_000.0)
            .build(),
    ];
    let (_, report) = merge_duplicates(activities, &DedupConfig::default());
    let pairs: BTreeSet<BTreeSet<&str>> = report
        .groups
        .iter()
        .map(|g| g.fragment_ids.iter().map(String::as_str).collect())
        .collect();
    let expected: BTreeSet<BTreeSet<&str>> = [
        ["g-short", "s-short"].into_iter().collect(),
        ["g-long", "s-long"].into_iter().collect(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        pairs, expected,
        "each capture joins the run of its distance"
    );
}

#[test]
fn a_report_scoped_to_a_page_keeps_only_that_page_s_groups_and_counts() {
    let t = base_time();
    let activities = vec![
        provider_activity("s-1", "strava", SportType::Ride, t, 3_600)
            .distance_meters(30_000.0)
            .build(),
        provider_activity("g-1", "garmin", SportType::Ride, t, 3_600)
            .distance_meters(30_050.0)
            .build(),
        provider_activity(
            "s-2",
            "strava",
            SportType::Run,
            t + Duration::days(1),
            1_800,
        )
        .distance_meters(6_000.0)
        .build(),
        provider_activity(
            "g-2",
            "garmin",
            SportType::Run,
            t + Duration::days(1),
            1_800,
        )
        .distance_meters(6_010.0)
        .build(),
    ];
    let (sessions, report) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(report.groups.len(), 2);

    let page: Vec<Activity> = sessions
        .into_iter()
        .filter(|a| a.sport_type() == &SportType::Run)
        .collect();
    let scoped = report.scoped_to(&page);

    assert_eq!(scoped.session_count, 1);
    assert_eq!(scoped.raw_count, 2);
    assert_eq!(scoped.groups.len(), 1);
    assert_eq!(scoped.groups[0].fragment_ids, vec!["g-2", "s-2"]);
}

#[test]
fn a_scraped_recording_is_named_by_the_service_it_was_read_from() {
    // A Garmin connection signed in with the athlete's username and password
    // from Dravr stamps its rows with the mirror backend; the merge line and
    // `fragment_dedup` name the service, never the backend.
    let t = base_time();
    let activities = vec![
        provider_activity(
            "strava-1",
            "strava",
            SportType::Ride,
            t + Duration::minutes(2),
            11_520,
        )
        .distance_meters(84_800.0)
        .calories(2_100)
        .build(),
        provider_activity(
            "garmin-1",
            "sciotte",
            SportType::Ride,
            t + Duration::minutes(2),
            11_520,
        )
        .source("garmin")
        .distance_meters(84_750.0)
        .average_heart_rate(141)
        .build(),
    ];
    let (_, report) = merge_duplicates(activities, &DedupConfig::default());
    let group = &report.groups[0];
    // The Garmin recording stands; Strava's copy fills what it lacks.
    assert_eq!(group.providers, vec!["garmin", "strava"]);
    assert!(
        !group.filled_fields.is_empty(),
        "the Strava row donates its calories"
    );
    assert!(group.filled_fields.iter().all(|f| f.provider == "strava"));
    let said = serde_json::to_string(&report).unwrap();
    assert!(!said.to_lowercase().contains("sciotte"), "{said}");
}

// ============================================================================
// Garmin activity id across the API and the usr/pwd connection (carnet#1155)
// ============================================================================

/// A row the Garmin API pushed: provider `garmin`, keyed by Garmin's id.
fn garmin_api_row(id: &str, start: DateTime<Utc>, duration_secs: u64) -> Activity {
    ActivityBuilder::new(
        id,
        "Pushed Ride",
        SportType::Ride,
        start,
        duration_secs,
        "garmin",
    )
    .source("garmin")
    .distance_meters(40_000.0)
    .build()
}

/// A row the connection using usr/pwd from Dravr captured: provider
/// `sciotte`, source `garmin`, keyed by the same Garmin id.
fn garmin_credential_row(id: &str, start: DateTime<Utc>, duration_secs: u64) -> Activity {
    ActivityBuilder::new(
        id,
        "Captured Ride",
        SportType::Ride,
        start,
        duration_secs,
        "sciotte",
    )
    .source("garmin")
    .distance_meters(40_000.0)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .average_heart_rate(142)
    .build()
}

#[test]
fn one_garmin_activity_id_is_one_workout_and_the_api_row_wins() {
    let t = base_time();
    // The usr/pwd row carries a route the API summary lacks, so the ranking
    // alone would pick it; the API row must stand for its own activity.
    let activities = vec![
        garmin_credential_row("19876543210", t, 3600),
        garmin_api_row("19876543210", t, 3600),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(merged[0].provider(), "garmin");
    assert_eq!(merged[0].name(), "Pushed Ride");
    // The API row takes what it lacks from its usr/pwd twin.
    assert!(merged[0].summary_polyline().is_some());
    assert_eq!(merged[0].average_heart_rate(), Some(142));
    assert_eq!(report.groups.len(), 1);
    assert_eq!(report.groups[0].canonical_id, "19876543210");
    // Both rows name the same service: the report names it, never the
    // backend that captured one of them.
    assert_eq!(report.groups[0].providers, vec!["garmin"]);
}

#[test]
fn a_date_only_capture_still_folds_into_the_api_row_of_its_id() {
    // A usr/pwd row scraped without its time sits at midnight UTC; the API
    // row starts the evening before in UTC. Only the id ties them.
    let api_start = Utc.with_ymd_and_hms(2026, 5, 21, 23, 30, 0).unwrap();
    let midnight = Utc.with_ymd_and_hms(2026, 5, 22, 0, 0, 0).unwrap();
    let activities = vec![
        garmin_api_row("555", api_start, 1800),
        make_activity("other", SportType::Run, base_time(), 1800, 5000.0),
        garmin_credential_row("555", midnight, 0),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 2, "{merged:?}");
    assert!(merged
        .iter()
        .any(|a| a.id() == "555" && a.provider() == "garmin"));
    assert!(!merged.iter().any(|a| a.provider() == "sciotte"));
}

#[test]
fn equal_ids_from_other_services_are_not_one_workout() {
    // Ids are only comparable within Garmin: a Strava capture with the same
    // id string a day apart is another activity.
    let t = base_time();
    let strava_capture = ActivityBuilder::new(
        "777",
        "Strava Ride",
        SportType::Ride,
        t + Duration::days(1),
        3600,
        "sciotte",
    )
    .source("strava")
    .build();
    let activities = vec![garmin_api_row("777", t, 3600), strava_capture];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 2, "{merged:?}");
    assert!(report.groups.is_empty());
}

#[test]
fn distinct_garmin_ids_stay_distinct_activities() {
    let t = base_time();
    let activities = vec![
        garmin_api_row("1001", t, 1800),
        garmin_credential_row("1002", t + Duration::days(2), 1800),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());
    assert_eq!(ids(&merged).len(), 2);
}

// ============================================================================
// Strava activity id across the API, the usr/pwd capture and the imported
// archive (carnet#1242)
// ============================================================================

/// A row the Strava API served: provider `strava`, keyed by Strava's id.
fn strava_api_row(id: &str, start: DateTime<Utc>, duration_secs: u64) -> Activity {
    ActivityBuilder::new(
        id,
        "API Ride",
        SportType::Ride,
        start,
        duration_secs,
        "strava",
    )
    .distance_meters(40_000.0)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .build()
}

/// A row the athlete's imported Strava export holds: provider
/// `strava_archive`, keyed by the export's `Activity ID`, which is Strava's.
fn strava_archive_row(id: &str, start: DateTime<Utc>, duration_secs: u64) -> Activity {
    ActivityBuilder::new(
        id,
        "Archived Ride",
        SportType::Ride,
        start,
        duration_secs,
        "strava_archive",
    )
    .distance_meters(40_000.0)
    .start_latitude(45.5)
    .start_longitude(-73.6)
    .average_heart_rate(138)
    .build()
}

/// A row the connection using usr/pwd from Dravr captured of a Strava
/// activity: provider `sciotte`, source `strava`, scraped date-only (the list
/// page's day at midnight UTC), the sport the athlete relabelled it to.
fn strava_capture_row(id: &str, day: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id,
        "Captured Ride",
        SportType::MountainBike,
        day,
        3600,
        "sciotte",
    )
    .source("strava")
    .distance_meters(40_100.0)
    .build()
}

#[test]
fn a_date_only_strava_capture_folds_into_the_archive_row_of_its_id() {
    // An evening ride in Montreal: the export's start is past midnight UTC,
    // the capture sits at midnight of the athlete's own day, and the athlete
    // relabelled the sport on Strava. Only the id ties them.
    let archived = Utc.with_ymd_and_hms(2026, 5, 22, 1, 30, 0).unwrap();
    let local_day = Utc.with_ymd_and_hms(2026, 5, 21, 0, 0, 0).unwrap();
    let activities = vec![
        strava_capture_row("16012345678", local_day),
        make_activity("other", SportType::Run, base_time(), 1800, 5000.0),
        strava_archive_row("16012345678", archived, 3600),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 2, "{merged:?}");
    let session = merged.iter().find(|a| a.id() == "16012345678").unwrap();
    assert_eq!(
        session.provider(),
        "strava_archive",
        "the export's recording stands for the workout over a date-only capture"
    );
    assert_eq!(session.start_date(), archived, "its real start is kept");
    assert_eq!(
        report.groups.len(),
        1,
        "one group for the one ride: {report:?}"
    );
    assert_eq!(
        report.groups[0].providers,
        vec!["strava_archive", "strava"],
        "the capture is named by the service it was read from, never the backend"
    );
}

#[test]
fn the_strava_api_row_stands_for_its_archive_copy() {
    let t = base_time();
    // The archive copy arrives first and ties the API row on every ranked
    // field but heart rate; the API row must still stand for its activity.
    let activities = vec![
        strava_archive_row("16012345679", t, 3600),
        strava_api_row("16012345679", t, 3600),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(
        merged[0].provider(),
        "strava",
        "the API row stands for its activity whatever order the copies arrive in"
    );
    assert_eq!(
        merged[0].name(),
        "API Ride",
        "the API row's own fields are kept"
    );
    assert_eq!(
        merged[0].average_heart_rate(),
        Some(138),
        "the API row takes what it lacks from the export"
    );
    assert_eq!(
        report.groups[0].providers,
        vec!["strava", "strava_archive"],
        "the API row is named first, as the canonical recording"
    );
}

#[test]
fn an_archive_recording_outranks_a_capture_without_a_route() {
    // A treadmill run: neither copy has a route, and the capture claims the
    // longer duration, so the ranking alone would pick it.
    let t = base_time();
    let treadmill = ActivityBuilder::new(
        "16012345680",
        "Treadmill",
        SportType::Run,
        t,
        1800,
        "strava_archive",
    )
    .distance_meters(5_000.0)
    .average_heart_rate(150)
    .build();
    let capture = ActivityBuilder::new(
        "16012345680",
        "Treadmill",
        SportType::Run,
        t,
        1900,
        "sciotte",
    )
    .source("strava")
    .distance_meters(5_000.0)
    .build();
    let (merged, _) = merge_duplicates(vec![capture, treadmill], &DedupConfig::default());

    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(
        merged[0].provider(),
        "strava_archive",
        "the export's recording outranks the capture of the same activity"
    );
}

#[test]
fn an_archive_id_matches_no_other_service() {
    // A Garmin id that happens to spell a Strava id is another activity.
    let t = base_time();
    let activities = vec![
        garmin_api_row("888", t, 3600),
        strava_archive_row("888", t + Duration::days(1), 3600),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 2, "{merged:?}");
    assert!(
        report.groups.is_empty(),
        "ids are compared within one service only: {report:?}"
    );
}

#[test]
fn every_service_read_both_ways_keys_its_twins_by_its_own_id() {
    // The API's copy and the usr/pwd capture of one TrainingPeaks or COROS
    // activity, captured date-only and a day apart in UTC: one workout, the
    // API's copy standing for it. A capture cached under the dedicated mirror
    // backend name (`sciotte_garmin`) is Garmin's like a `sciotte` row with
    // source `garmin`.
    let start = Utc.with_ymd_and_hms(2026, 5, 21, 23, 30, 0).unwrap();
    let midnight = Utc.with_ymd_and_hms(2026, 5, 22, 0, 0, 0).unwrap();
    for (api, capture) in [
        (
            ActivityBuilder::new("901", "Api", SportType::Ride, start, 3600, "trainingpeaks")
                .build(),
            ActivityBuilder::new("901", "Captured", SportType::Run, midnight, 0, "sciotte")
                .source("trainingpeaks")
                .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
                .build(),
        ),
        (
            ActivityBuilder::new("902", "Api", SportType::Ride, start, 3600, "coros").build(),
            ActivityBuilder::new("902", "Captured", SportType::Run, midnight, 0, "sciotte")
                .source("coros")
                .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
                .build(),
        ),
        (
            garmin_api_row("903", start, 3600),
            ActivityBuilder::new(
                "903",
                "Captured",
                SportType::Run,
                midnight,
                0,
                "sciotte_garmin",
            )
            .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
            .build(),
        ),
    ] {
        let service = api.provider().to_owned();
        let (merged, report) = merge_duplicates(vec![capture, api], &DedupConfig::default());
        assert_eq!(merged.len(), 1, "{service}: {merged:?}");
        assert_eq!(merged[0].provider(), service, "the API's copy stands");
        assert_eq!(
            report.groups[0].providers,
            vec![service.clone()],
            "{service}: both copies are named by the service"
        );
    }
}

// ============================================================================
// Pieces of one session sum; recordings of the same time never do (carnet#1248)
// ============================================================================

/// A Garmin auto-split piece carrying its own route.
fn garmin_piece(id: &str, start: DateTime<Utc>, duration_secs: u64, distance_m: f64) -> Activity {
    provider_activity(id, "garmin", SportType::Ride, start, duration_secs)
        .distance_meters(distance_m)
        .moving_time_seconds(duration_secs - 60)
        .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
        .build()
}

#[test]
fn two_pieces_sum_but_two_recordings_of_the_same_time_do_not() {
    let t = base_time();
    // Two pieces: a 70 km ride the recorder split, the second piece starting
    // two seconds after the first ended.
    let pieces = vec![
        garmin_piece("g-1", t, 7_200, 70_000.0),
        garmin_piece("g-2", t + Duration::seconds(7_202), 3_600, 30_000.0),
    ];
    let (merged, _) = merge_duplicates(pieces, &DedupConfig::default());
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].distance_meters(), Some(100_000.0));
    assert_eq!(merged[0].duration_seconds(), 10_802);

    // Two recordings: a watch and a bike computer on the same 70 km ride,
    // starting 30 s apart.
    let recordings = vec![
        garmin_piece("watch", t, 7_200, 70_000.0),
        provider_activity(
            "bikecomp",
            "wahoo",
            SportType::Ride,
            t + Duration::seconds(30),
            7_190,
        )
        .distance_meters(70_400.0)
        .build(),
    ];
    let (merged, _) = merge_duplicates(recordings, &DedupConfig::default());
    assert_eq!(ids(&merged), vec!["watch"]);
    assert_eq!(merged[0].distance_meters(), Some(70_000.0));
    assert_eq!(merged[0].duration_seconds(), 7_200);
}

#[test]
fn pieces_from_two_providers_sum_into_one_session() {
    // The first leg on the watch (Garmin), the second on the phone (Strava)
    // after the watch died: one session, both legs.
    let t = base_time();
    let activities = vec![
        garmin_piece("garmin-leg", t, 3_600, 32_000.0),
        provider_activity(
            "strava-leg",
            "strava",
            SportType::Ride,
            t + Duration::seconds(3_660),
            1_800,
        )
        .distance_meters(15_000.0)
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["garmin-leg"]);
    assert_eq!(merged[0].distance_meters(), Some(47_000.0));
    assert_eq!(merged[0].duration_seconds(), 5_460);
    assert_eq!(report.groups[0].providers, vec!["garmin", "strava"]);
}

/// Two Garmin pieces of a 100 km ride and Strava's recording of the whole of
/// it. `strava_gps` decides whether the Strava row carries the route.
fn split_ride_and_its_whole_recording(strava_gps: bool) -> Vec<Activity> {
    let t = base_time();
    let mut strava = provider_activity(
        "strava-whole",
        "strava",
        SportType::Ride,
        t + Duration::seconds(20),
        10_790,
    )
    .distance_meters(100_300.0)
    .moving_time_seconds(10_500)
    .average_heart_rate(141);
    if strava_gps {
        strava = strava.summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned());
    }
    let mut first = provider_activity("garmin-1", "garmin", SportType::Ride, t, 7_200)
        .distance_meters(70_000.0)
        .moving_time_seconds(7_000);
    let mut second = provider_activity(
        "garmin-2",
        "garmin",
        SportType::Ride,
        t + Duration::seconds(7_202),
        3_600,
    )
    .distance_meters(30_000.0)
    .moving_time_seconds(3_500);
    if !strava_gps {
        first = first.summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned());
        second = second.summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned());
    }
    vec![first.build(), strava.build(), second.build()]
}

#[test]
fn the_recording_services_pieces_stand_over_stravas_whole_recording() {
    // Strava's copy carries the route, but the Garmin watch recorded the
    // ride: its pieces stand for the session under the one precedence rule
    // and sum to it, and Strava's recording of the whole lends the route it
    // lacks, adding no volume.
    let (merged, report) = merge_duplicates(
        split_ride_and_its_whole_recording(true),
        &DedupConfig::default(),
    );

    assert_eq!(ids(&merged), vec!["garmin-1"]);
    assert_eq!(
        merged[0].distance_meters(),
        Some(100_000.0),
        "bug: the session's distance meters is not the standing copy's"
    );
    assert_eq!(
        merged[0].duration_seconds(),
        10_802,
        "bug: the session's duration seconds is not the standing copy's"
    );
    assert!(
        merged[0].summary_polyline().is_some(),
        "the route fills from Strava"
    );
    assert_eq!(report.groups[0].fragment_ids.len(), 3);
}

#[test]
fn split_pieces_sum_once_beside_a_whole_session_recording_of_them() {
    // The Garmin pieces carry the route, so one stands for the session: the
    // other piece adds its volume, and Strava's recording of the whole ride,
    // overlapping both, adds none of it — the ride is 100 km, never 200 km.
    let (merged, report) = merge_duplicates(
        split_ride_and_its_whole_recording(false),
        &DedupConfig::default(),
    );

    assert_eq!(ids(&merged), vec!["garmin-1"]);
    assert_eq!(merged[0].distance_meters(), Some(100_000.0));
    assert_eq!(merged[0].duration_seconds(), 10_802);
    assert_eq!(merged[0].moving_time_seconds(), Some(10_500));
    // Strava's row covers the whole session, so it still fills what the
    // pieces lack.
    assert_eq!(merged[0].average_heart_rate(), Some(141));
    assert_eq!(report.groups[0].fragment_ids.len(), 3);
}

#[test]
fn two_recordings_of_the_same_tail_add_it_once() {
    // The first hour on the watch; the second hour recorded by both the
    // watch and a bike computer. The tail counts once.
    let t = base_time();
    let tail_start = t + Duration::seconds(3_602);
    let activities = vec![
        garmin_piece("head", t, 3_600, 30_000.0),
        garmin_piece("tail-watch", tail_start, 3_600, 31_000.0),
        provider_activity(
            "tail-bikecomp",
            "wahoo",
            SportType::Ride,
            tail_start + Duration::seconds(15),
            3_580,
        )
        .distance_meters(30_900.0)
        .build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].distance_meters(), Some(61_000.0));
    assert_eq!(merged[0].duration_seconds(), 7_202);
}

#[test]
fn a_date_only_row_never_adds_volume() {
    // A capture scraped date-only sits at midnight: its window is not when it
    // was recorded, so it can never be a piece.
    let t = base_time();
    let day = t.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let activities = vec![
        provider_activity("timed", "strava", SportType::Run, t, 3_600)
            .distance_meters(10_000.0)
            .build(),
        provider_activity("scraped", "sciotte", SportType::Run, day, 3_600)
            .distance_meters(10_050.0)
            .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    // Strava's API copy stands over its usr/pwd capture; the capture adds
    // nothing.
    assert_eq!(report.session_count, 1);
    assert_eq!(ids(&merged), vec!["timed"]);
    assert_eq!(
        merged[0].distance_meters(),
        Some(10_000.0),
        "bug: the session's distance meters is not the standing copy's"
    );
    assert_eq!(merged[0].duration_seconds(), 3_600);
}

#[test]
fn twins_of_one_strava_activity_never_add_volume_whatever_their_clocks() {
    // The archive copy of a Strava activity can carry a start the API row
    // does not (the original recording's, before an edit): two windows that
    // do not overlap, but one activity id, so one recording.
    let t = base_time();
    let activities = vec![
        strava_api_row("15000000001", t, 3_600),
        strava_archive_row("15000000001", t + Duration::seconds(3_700), 3_600),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["15000000001"]);
    assert_eq!(merged[0].provider(), "strava");
    assert_eq!(merged[0].distance_meters(), Some(40_000.0));
    assert_eq!(merged[0].duration_seconds(), 3_600);
}

#[test]
fn a_twin_of_a_recording_never_adds_volume_beside_a_third_service_s_row() {
    // One 40 km ride: Garmin's row (one second longer, so it stands for the
    // session), Strava's API row of it, and the archive copy of the same
    // Strava id on the original recording's clock, an hour later. The archive
    // row overlaps no counted row, but it is Strava's API row again: the ride
    // stays 40 km, never 80 km.
    let t = base_time();
    let activities = vec![
        garmin_piece("21000000000", t, 3_601, 40_000.0),
        strava_api_row("15000000001", t, 3_600),
        strava_archive_row("15000000001", t + Duration::seconds(3_700), 3_600),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["21000000000"]);
    assert_eq!(merged[0].provider(), "garmin");
    assert_eq!(merged[0].distance_meters(), Some(40_000.0));
    assert_eq!(merged[0].duration_seconds(), 3_601);
    assert_eq!(report.groups[0].fragment_ids.len(), 3);
}

#[test]
fn a_row_of_another_sport_never_adds_its_volume_to_the_session() {
    // A brick: a Garmin ride, then a Garmin run five minutes later, and one
    // WHOOP "cycling" row over both. WHOOP ties the run to the ride (cross
    // sport, most of the run's time), but the run's kilometres are not the
    // ride's: the ride stays 30 km on its own clock.
    let t = base_time();
    let activities = vec![
        garmin_piece("garmin-ride", t, 3_600, 30_000.0),
        provider_activity(
            "garmin-run",
            "garmin",
            SportType::Run,
            t + Duration::seconds(3_900),
            1_800,
        )
        .distance_meters(6_000.0)
        .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
        .build(),
        provider_activity("whoop-cycling", "whoop", SportType::Ride, t, 5_700).build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());

    let ride = merged
        .iter()
        .find(|a| a.id() == "garmin-ride")
        .expect("the ride stands for its session");
    assert_eq!(ride.sport_type(), &SportType::Ride);
    assert_eq!(ride.distance_meters(), Some(30_000.0));
    assert_eq!(ride.duration_seconds(), 3_600);
}

#[test]
fn a_session_with_a_stop_between_its_pieces_ends_when_its_last_piece_ends() {
    // Two Garmin pieces, 14:00-15:00 and 15:20-16:00 after a café stop, and
    // Strava's recording of the whole ride, which ties them into one group.
    // The session runs to 16:00: its elapsed time covers the stop, its moving
    // time does not.
    let t = base_time();
    let activities = vec![
        garmin_piece("first", t, 3_600, 30_000.0),
        garmin_piece("second", t + Duration::minutes(80), 2_400, 20_000.0),
        provider_activity("strava-whole", "strava", SportType::Ride, t, 7_200)
            .distance_meters(50_100.0)
            .build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["first"]);
    let session = &merged[0];
    assert_eq!(session.distance_meters(), Some(50_000.0));
    assert_eq!(session.start_date(), t);
    assert_eq!(session.duration_seconds(), 7_200);
    assert_eq!(session.moving_time_seconds(), Some(5_880));
}

#[test]
fn a_split_session_spends_the_training_stress_and_calories_of_both_pieces() {
    // A Wahoo ride split into 70 km / 2 h (TSS 120, 1 800 kcal) and 30 km /
    // 1 h (TSS 60, 800 kcal): training load and fuelling read the session's
    // 180 TSS and 2 600 kcal, not the first piece's.
    let t = base_time();
    let activities = vec![
        provider_activity("wahoo-1", "wahoo", SportType::Ride, t, 7_200)
            .distance_meters(70_000.0)
            .training_stress_score(120.0)
            .calories(1_800)
            .build(),
        provider_activity(
            "wahoo-2",
            "wahoo",
            SportType::Ride,
            t + Duration::seconds(7_202),
            3_600,
        )
        .distance_meters(30_000.0)
        .training_stress_score(60.0)
        .calories(800)
        .build(),
    ];
    let (merged, _) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["wahoo-1"]);
    assert_eq!(merged[0].distance_meters(), Some(100_000.0));
    assert_eq!(merged[0].training_stress_score(), Some(180.0));
    assert_eq!(merged[0].calories(), Some(2_600));
}

// ============================================================================
// One precedence over a workout's copies: the recording's own service, unless
// edited on Strava (carnet#1274, Athlete Data Consolidation S3)
// ============================================================================

/// Strava's copy of a fenix ride, as its detail names the device: Strava
/// re-derives distance and speed from the samples.
fn strava_detail_of_fenix_ride(start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        "16000000001",
        "Sunday loop",
        SportType::Ride,
        start,
        7_202,
        "strava",
    )
    .device_name("Garmin fenix 7")
    .description("Easy spin with Bob".to_owned())
    .moving_time_seconds(6_900)
    .distance_meters(61_800.0)
    .average_speed(8.96)
    .average_heart_rate(140)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .build()
}

/// Garmin's copy of the same fenix ride, as the Garmin API pushed it.
fn garmin_push_of_fenix_ride(start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        "21000000001",
        "Montreal Cycling",
        SportType::Ride,
        start,
        7_200,
        "garmin",
    )
    .source("garmin")
    .device_name("Garmin fenix 7")
    .distance_meters(62_400.0)
    .average_speed(8.67)
    .average_heart_rate(141)
    .calories(1_450)
    .start_latitude(45.5)
    .start_longitude(-73.6)
    .build()
}

/// T5: a Garmin-recorded ride seen through the Garmin push and the Strava
/// API stands as Garmin's copy, with Garmin's totals; Strava's copy fills
/// what Garmin's lacks — the athlete's description, the moving time, the
/// route.
#[test]
fn a_garmin_recorded_ride_in_garmin_and_strava_stands_as_garmins_copy() {
    let t = base_time();
    let (merged, report) = merge_duplicates(
        vec![strava_detail_of_fenix_ride(t), garmin_push_of_fenix_ride(t)],
        &DedupConfig::default(),
    );

    assert_eq!(merged.len(), 1, "{merged:?}");
    let session = &merged[0];
    assert_eq!(session.provider(), "garmin");
    assert_eq!(report.groups[0].canonical_id, "21000000001");
    assert_eq!(
        session.distance_meters(),
        Some(62_400.0),
        "the fenix's distance"
    );
    assert_eq!(
        session.average_speed(),
        Some(8.67),
        "bug: the session's average speed is not the standing copy's"
    );
    assert_eq!(
        session.average_heart_rate(),
        Some(141),
        "bug: the session's average heart rate is not the standing copy's"
    );
    assert_eq!(
        session.calories(),
        Some(1_450),
        "bug: the session's calories is not the standing copy's"
    );
    assert_eq!(
        session.duration_seconds(),
        7_200,
        "bug: the session's duration seconds is not the standing copy's"
    );
    assert_eq!(
        session.moving_time_seconds(),
        Some(6_900),
        "Garmin lacks one"
    );
    assert_eq!(session.description(), Some("Easy spin with Bob"));
    assert!(
        session.summary_polyline().is_some(),
        "the route fills from Strava"
    );
}

/// A Strava list row names no device, yet Garmin's own copy of the ride still
/// stands over Strava's: a recording device ranks over Strava for a workout.
#[test]
fn a_strava_list_row_naming_no_device_still_yields_to_garmins_copy() {
    let t = base_time();
    let list_row = ActivityBuilder::new(
        "16000000001",
        "Sunday loop",
        SportType::Ride,
        t,
        7_202,
        "strava",
    )
    .distance_meters(61_800.0)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .build();
    let (merged, _) = merge_duplicates(
        vec![list_row, garmin_push_of_fenix_ride(t)],
        &DedupConfig::default(),
    );
    assert_eq!(merged[0].provider(), "garmin");
    assert_eq!(
        merged[0].distance_meters(),
        Some(62_400.0),
        "bug: the session's distance meters is not the standing copy's"
    );
}

/// T6: the athlete cropped the ride on Strava (its window 5 % shorter and
/// starting later): Strava's copy stands, its distance and elapsed time with
/// it.
#[test]
fn a_ride_cropped_on_strava_stands_as_stravas_copy() {
    let t = base_time();
    let cropped = ActivityBuilder::new(
        "16000000002",
        "Tempo",
        SportType::Ride,
        t + Duration::seconds(360),
        6_840,
        "strava",
    )
    .device_name("Garmin fenix 7")
    .distance_meters(59_280.0)
    .average_heart_rate(144)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .build();
    let (merged, report) = merge_duplicates(
        vec![garmin_push_of_fenix_ride(t), cropped],
        &DedupConfig::default(),
    );

    assert_eq!(merged.len(), 1, "{merged:?}");
    let session = &merged[0];
    assert_eq!(session.provider(), "strava", "the cropped copy stands");
    assert_eq!(report.groups[0].canonical_id, "16000000002");
    assert_eq!(
        session.distance_meters(),
        Some(59_280.0),
        "bug: the session's distance meters is not the standing copy's"
    );
    assert_eq!(
        session.duration_seconds(),
        6_840,
        "bug: the session's duration seconds is not the standing copy's"
    );
    assert_eq!(
        session.average_heart_rate(),
        Some(144),
        "bug: the session's average heart rate is not the standing copy's"
    );
    // What the edited copy lacks still fills from the recording.
    assert_eq!(
        session.calories(),
        Some(1_450),
        "bug: the session's calories is not the standing copy's"
    );
}

/// A distance the athlete corrected on Strava (more than a fifth off the
/// device's) is an edit too: Strava's copy stands.
#[test]
fn a_distance_corrected_on_strava_stands_as_stravas_copy() {
    let t = base_time();
    let corrected = ActivityBuilder::new(
        "16000000001",
        "Sunday loop",
        SportType::Ride,
        t,
        7_202,
        "strava",
    )
    .device_name("Garmin fenix 7")
    .distance_meters(80_000.0)
    .build();
    let (merged, _) = merge_duplicates(
        vec![garmin_push_of_fenix_ride(t), corrected],
        &DedupConfig::default(),
    );
    assert_eq!(merged[0].provider(), "strava");
    assert_eq!(
        merged[0].distance_meters(),
        Some(80_000.0),
        "bug: the session's distance meters is not the standing copy's"
    );
}

/// Strava's processing (the same window, a distance a few percent off) is no
/// edit: Garmin's copy stands.
#[test]
fn stravas_own_processing_is_no_edit() {
    let t = base_time();
    let (merged, _) = merge_duplicates(
        vec![
            garmin_push_of_fenix_ride(t),
            strava_detail_of_fenix_ride(t + Duration::seconds(3)),
        ],
        &DedupConfig::default(),
    );
    assert_eq!(merged[0].provider(), "garmin");
}

/// A run Strava's own app recorded stands as Strava's, over a wrist
/// tracker's auto-detected copy.
#[test]
fn a_strava_app_run_stands_over_a_wrist_trackers_copy() {
    let t = base_time();
    let strava = ActivityBuilder::new(
        "16000000003",
        "Lunch run",
        SportType::Run,
        t,
        2_400,
        "strava",
    )
    .device_name("Strava iPhone App")
    .distance_meters(6_000.0)
    .build();
    let whoop = ActivityBuilder::new(
        "w-1",
        "Running",
        SportType::Run,
        t + Duration::seconds(30),
        2_350,
        "whoop",
    )
    .average_heart_rate(152)
    .build();
    let (merged, _) = merge_duplicates(vec![whoop, strava], &DedupConfig::default());
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(merged[0].provider(), "strava");
    assert_eq!(
        merged[0].average_heart_rate(),
        Some(152),
        "WHOOP fills the heart rate"
    );
}

/// The order never depends on arrival: every permutation of one workout's
/// copies merges to the same session.
#[test]
fn the_standing_copy_does_not_depend_on_arrival_order() {
    let t = base_time();
    let copies = [
        garmin_push_of_fenix_ride(t),
        strava_detail_of_fenix_ride(t),
        ActivityBuilder::new(
            "21000000001",
            "Montreal Cycling",
            SportType::Ride,
            t,
            7_200,
            "sciotte",
        )
        .source("garmin")
        .distance_meters(62_350.0)
        .build(),
    ];
    let orders: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in orders {
        let input: Vec<Activity> = order.iter().map(|&i| copies[i].clone()).collect();
        let (merged, report) = merge_duplicates(input, &DedupConfig::default());
        assert_eq!(merged.len(), 1, "{order:?}");
        assert_eq!(report.groups[0].canonical_id, "21000000001", "{order:?}");
        assert_eq!(
            merged[0].provider(),
            "garmin",
            "{order:?}: the API row over its capture"
        );
    }
}

/// A Strava list row names no device (only the detail does), yet it is
/// Strava's copy of the recording it merges with: cropped on Strava, it
/// stands for the session as the detail would (review of carnet#1303).
#[test]
fn a_strava_list_row_cropped_on_strava_stands_as_stravas_copy() {
    let t = base_time();
    let cropped_list_row = ActivityBuilder::new(
        "16000000002",
        "Tempo",
        SportType::Ride,
        t + Duration::seconds(360),
        6_840,
        "strava",
    )
    .distance_meters(59_280.0)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .build();
    let (merged, report) = merge_duplicates(
        vec![garmin_push_of_fenix_ride(t), cropped_list_row],
        &DedupConfig::default(),
    );

    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(merged[0].provider(), "strava", "the cropped copy stands");
    assert_eq!(report.groups[0].canonical_id, "16000000002");
    assert_eq!(
        merged[0].duration_seconds(),
        6_840,
        "bug: the session's duration seconds is not the standing copy's"
    );
    assert_eq!(
        merged[0].distance_meters(),
        Some(59_280.0),
        "bug: the session's distance meters is not the standing copy's"
    );
}

/// A Strava copy naming another recorder (Strava's own app) is a second
/// recording, never Strava's copy of the Garmin one: nothing is lifted.
#[test]
fn a_strava_app_recording_beside_a_garmin_one_is_no_edit_of_it() {
    let t = base_time();
    let phone = ActivityBuilder::new(
        "16000000003",
        "Phone ride",
        SportType::Ride,
        t + Duration::seconds(40),
        7_100,
        "strava",
    )
    .device_name("Strava iPhone App")
    .distance_meters(61_000.0)
    .build();
    let (merged, _) = merge_duplicates(
        vec![phone, garmin_push_of_fenix_ride(t)],
        &DedupConfig::default(),
    );
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(merged[0].provider(), "garmin");
}

/// Garmin's capture reports a paused ride's timer, never its elapsed time:
/// Strava's elapsed time over it is the pauses, not a crop, so Garmin's
/// copy stands (review of carnet#1303; dev held six such rides).
#[test]
fn a_paused_garmin_recording_and_stravas_unedited_copy_stand_as_garmins() {
    let t = base_time();
    let paused_capture = ActivityBuilder::new(
        "21000000004",
        "Montreal Cycling",
        SportType::Ride,
        t,
        6_900,
        "sciotte_garmin",
    )
    .source("garmin")
    .distance_meters(62_400.0)
    .average_heart_rate(141)
    .build();
    let strava_copy = ActivityBuilder::new(
        "16000000004",
        "Sunday loop",
        SportType::Ride,
        t,
        7_200,
        "strava",
    )
    .distance_meters(61_800.0)
    .summary_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@".to_owned())
    .build();
    let (merged, _) = merge_duplicates(vec![strava_copy, paused_capture], &DedupConfig::default());
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(merged[0].provider(), "sciotte_garmin");
    assert_eq!(
        merged[0].distance_meters(),
        Some(62_400.0),
        "bug: the session's distance meters is not the standing copy's"
    );
}

/// Twins of one Strava activity rank by the one rule alone: the export's
/// file names the Garmin watch that recorded it, the API's list row names
/// nothing, and both are Garmin's recording relayed by Strava, so the API
/// row stands for its own activity (API over export), not the export.
#[test]
fn a_strava_api_list_row_stands_over_its_export_naming_the_device() {
    let t = base_time();
    let archive = ActivityBuilder::new(
        "16000000005",
        "Archived Ride",
        SportType::Ride,
        t,
        3_600,
        "strava_archive",
    )
    .device_name("Garmin Edge 840")
    .distance_meters(40_000.0)
    .average_heart_rate(138)
    .build();
    for copies in [
        vec![archive.clone(), strava_api_row("16000000005", t, 3_600)],
        vec![strava_api_row("16000000005", t, 3_600), archive],
    ] {
        let (merged, _) = merge_duplicates(copies, &DedupConfig::default());
        assert_eq!(merged.len(), 1, "{merged:?}");
        assert_eq!(merged[0].provider(), "strava");
        assert_eq!(
            merged[0].average_heart_rate(),
            Some(138),
            "bug: the session's average heart rate is not the standing copy's"
        );
    }
}

/// The athlete's Garmin export keys each activity by Garmin's own id, so its
/// row and the Garmin push of the same activity are one workout whatever
/// their clocks say (an export's local-time stamp), and the push stands.
#[test]
fn a_garmin_export_row_and_the_push_of_its_id_are_one_workout() {
    let t = base_time();
    let exported = ActivityBuilder::new(
        "19876543299",
        "Exported Ride",
        SportType::Ride,
        t + Duration::hours(4),
        3_600,
        "garmin_archive",
    )
    .distance_meters(40_000.0)
    .average_heart_rate(139)
    .build();
    let (merged, report) = merge_duplicates(
        vec![exported, garmin_api_row("19876543299", t, 3_600)],
        &DedupConfig::default(),
    );
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(merged[0].provider(), "garmin");
    assert_eq!(report.groups[0].canonical_id, "19876543299");
}

// ── The athlete's Strava title and sport (carnet#1315) ──────────────────────

/// The usr/pwd capture of a Garmin trail run, titled by Garmin.
fn garmin_capture_of_trail_run(start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        "21000000012",
        "Charlevoix-Est Trail",
        SportType::TrailRunning,
        start,
        21_600,
        "sciotte_garmin",
    )
    .source("garmin")
    .distance_meters(42_300.0)
    .elevation_gain(1_850.0)
    .average_heart_rate(148)
    .build()
}

/// Strava's list row of a Garmin recording: no device, the athlete's title.
fn strava_list_row(id: &str, name: &str, sport: SportType, start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(id, name, sport, start, 21_610, "strava")
        .distance_meters(41_900.0)
        .elevation_gain(1_790.0)
        .build()
}

/// A Garmin trail run the athlete renamed on Strava ("UTHC 42km" on dev,
/// 2026-10-31) stands as Garmin's copy with Garmin's figures, and with the
/// title the athlete gave it on Strava.
#[test]
fn a_garmin_run_renamed_on_strava_takes_stravas_title_and_keeps_garmins_figures() {
    let t = base_time();
    let (merged, report) = merge_duplicates(
        vec![
            strava_list_row("16000000012", "UTHC 42km", SportType::TrailRunning, t),
            garmin_capture_of_trail_run(t),
        ],
        &DedupConfig::default(),
    );

    assert_eq!(merged.len(), 1, "{merged:?}");
    let session = &merged[0];
    assert_eq!(
        session.provider(),
        "sciotte_garmin",
        "bug: Garmin's copy no longer stands"
    );
    assert_eq!(
        session.name(),
        "UTHC 42km",
        "bug: the athlete's Strava title was dropped"
    );
    assert_eq!(
        *session.sport_type(),
        SportType::TrailRunning,
        "bug: an unchanged sport moved"
    );
    assert_eq!(
        session.distance_meters(),
        Some(42_300.0),
        "bug: not Garmin's distance"
    );
    assert_eq!(
        session.elevation_gain(),
        Some(1_850.0),
        "bug: not Garmin's climb"
    );
    assert!(
        report.groups[0].filled_fields.contains(&FilledField {
            field: "name",
            provider: "strava".to_owned(),
        }),
        "bug: the merge line does not say the title came from Strava: {:?}",
        report.groups[0].filled_fields
    );
}

/// Strava's own title ("Course à pied de nuit") is no rename: the recorder's
/// title stands, as on 20 of the dev account's 25 Garmin/Strava pairs.
#[test]
fn stravas_own_title_leaves_the_recorders() {
    let t = base_time();
    for strava_title in [
        "Course à pied de nuit",
        "Morning Run",
        "Entrainement aux poids  le midi",
    ] {
        let (merged, report) = merge_duplicates(
            vec![
                strava_list_row("16000000013", strava_title, SportType::TrailRunning, t),
                garmin_capture_of_trail_run(t),
            ],
            &DedupConfig::default(),
        );
        assert_eq!(
            merged[0].name(),
            "Charlevoix-Est Trail",
            "bug: Strava's own title {strava_title:?} replaced the recorder's"
        );
        assert!(
            report.groups[0]
                .filled_fields
                .iter()
                .all(|f| f.field != "name"),
            "bug: {strava_title:?} reported as the athlete's title"
        );
    }
}

/// An indoor-rower session the athlete retyped as weight training on Strava
/// (4 sessions on dev): Strava's sport, its detail and its title stand, even
/// a title Strava wrote itself (the recorder's names the sport it is not),
/// and Garmin's figures stay.
#[test]
fn a_garmin_session_retyped_on_strava_takes_stravas_sport_and_title() {
    let t = base_time();
    let rower = ActivityBuilder::new(
        "21000000018",
        "Rameur d'intérieur",
        SportType::Rowing,
        t,
        2_700,
        "sciotte_garmin",
    )
    .source("garmin")
    .sport_type_detail("indoor_rowing".to_owned())
    .calories(310)
    .average_heart_rate(118)
    .build();
    let strava = ActivityBuilder::new(
        "16000000018",
        "Entrainement aux poids le midi",
        SportType::StrengthTraining,
        t,
        2_702,
        "strava",
    )
    .sport_type_detail("WeightTraining".to_owned())
    .calories(280)
    .build();

    let (merged, report) = merge_duplicates(vec![strava, rower], &DedupConfig::default());

    assert_eq!(merged.len(), 1, "{merged:?}");
    let session = &merged[0];
    assert_eq!(
        session.provider(),
        "sciotte_garmin",
        "bug: Garmin's copy no longer stands"
    );
    assert_eq!(
        *session.sport_type(),
        SportType::StrengthTraining,
        "bug: the athlete's Strava sport was dropped"
    );
    assert_eq!(
        session.sport_type_detail(),
        Some("WeightTraining"),
        "bug: a retyped session kept the rower's detail"
    );
    assert_eq!(
        session.name(),
        "Entrainement aux poids le midi",
        "bug: a retyped session kept the title naming the old sport"
    );
    assert_eq!(session.calories(), Some(310), "bug: not Garmin's calories");
    assert_eq!(
        report.groups[0].sport_type,
        SportType::StrengthTraining,
        "bug: the merge line names the old sport"
    );
}

/// Strava's coarser type (`Run` for a trail run) refines nothing: no retype.
#[test]
fn a_coarser_strava_type_is_no_retype() {
    let t = base_time();
    let (merged, _) = merge_duplicates(
        vec![
            strava_list_row("16000000014", "Morning Run", SportType::Run, t),
            garmin_capture_of_trail_run(t),
        ],
        &DedupConfig::default(),
    );
    assert_eq!(
        *merged[0].sport_type(),
        SportType::TrailRunning,
        "bug: Strava's Run hid the trail run"
    );
    assert_eq!(
        merged[0].name(),
        "Charlevoix-Est Trail",
        "bug: the title moved"
    );
}

/// Strava's detail (naming the Garmin device) and the athlete's Strava export
/// are Strava's copy too: the title the athlete gave stands from either.
#[test]
fn a_strava_detail_or_export_renamed_by_the_athlete_titles_the_session() {
    let t = base_time();
    let (merged, _) = merge_duplicates(
        vec![strava_detail_of_fenix_ride(t), garmin_push_of_fenix_ride(t)],
        &DedupConfig::default(),
    );
    assert_eq!(
        merged[0].provider(),
        "garmin",
        "bug: Garmin's copy no longer stands"
    );
    assert_eq!(
        merged[0].name(),
        "Sunday loop",
        "bug: the detail's title was dropped"
    );
    assert_eq!(
        merged[0].distance_meters(),
        Some(62_400.0),
        "bug: not Garmin's distance"
    );

    let export = ActivityBuilder::new(
        "16000000015",
        "UTHC 42km",
        SportType::TrailRunning,
        t,
        21_610,
        "strava_archive",
    )
    .device_name("Garmin fenix 7")
    .distance_meters(41_900.0)
    .build();
    let (merged, _) = merge_duplicates(
        vec![export, garmin_capture_of_trail_run(t)],
        &DedupConfig::default(),
    );
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert_eq!(
        merged[0].name(),
        "UTHC 42km",
        "bug: the export's title was dropped"
    );
    assert_eq!(
        merged[0].distance_meters(),
        Some(42_300.0),
        "bug: not Garmin's distance"
    );
}

/// A Strava-app recording beside the Garmin one is a second recording, not
/// Strava's copy of it: its title never names the Garmin session.
#[test]
fn a_strava_app_recordings_title_never_names_the_garmin_session() {
    let t = base_time();
    let phone = ActivityBuilder::new(
        "16000000016",
        "Phone ride",
        SportType::Ride,
        t + Duration::seconds(40),
        7_100,
        "strava",
    )
    .device_name("Strava iPhone App")
    .distance_meters(61_000.0)
    .build();
    let (merged, _) = merge_duplicates(
        vec![phone, garmin_push_of_fenix_ride(t)],
        &DedupConfig::default(),
    );
    assert_eq!(
        merged[0].name(),
        "Montreal Cycling",
        "bug: a second recording's title stood"
    );
}

/// Garmin Connect relays a Cardio recording to Strava as its catch-all
/// `Workout`, and files pilates as yoga where Strava keeps `Pilates`: the
/// athlete retyped nothing, so the session keeps the recorder's sport and
/// title (carnet#1315 review).
#[test]
fn a_vocabulary_difference_between_garmin_and_strava_is_no_retype() {
    let t = base_time();
    for (garmin_sport, garmin_title, strava_sport, strava_title) in [
        (
            SportType::StrengthTraining,
            "Cardio",
            SportType::Workout,
            "Entraînement le soir",
        ),
        (
            SportType::Yoga,
            "Pilates",
            SportType::Pilates,
            "Pilates le soir",
        ),
    ] {
        let garmin = ActivityBuilder::new(
            "21000000020",
            garmin_title,
            garmin_sport.clone(),
            t,
            3_600,
            "sciotte_garmin",
        )
        .source("garmin")
        .average_heart_rate(120)
        .build();
        let strava = ActivityBuilder::new(
            "16000000020",
            strava_title,
            strava_sport,
            t,
            3_602,
            "strava",
        )
        .build();
        let (merged, _) = merge_duplicates(vec![strava, garmin], &DedupConfig::default());
        assert_eq!(merged.len(), 1, "{merged:?}");
        assert_eq!(
            *merged[0].sport_type(),
            garmin_sport,
            "bug: Strava's name for the same sport read as a retype"
        );
        assert_eq!(
            merged[0].name(),
            garmin_title,
            "bug: a vocabulary difference replaced the recorder's title"
        );
    }
}

/// A Strava list row whose detail names Strava's own app is that app's
/// recording, not Strava's copy of the Garmin one, even when the two start
/// together: neither its sport nor its title reaches the Garmin session.
#[test]
fn a_strava_list_row_whose_detail_names_its_app_never_retypes_the_garmin_session() {
    let t = base_time();
    let list = strava_list_row("16000000021", "Morning Walk", SportType::Walk, t);
    let detail = ActivityBuilder::new(
        "16000000021",
        "Morning Walk",
        SportType::Walk,
        t,
        21_610,
        "strava",
    )
    .device_name("Strava iPhone App")
    .distance_meters(41_900.0)
    .build();
    let (merged, _) = merge_duplicates(
        vec![list, detail, garmin_capture_of_trail_run(t)],
        &DedupConfig::default(),
    );
    let session = merged
        .iter()
        .find(|a| a.provider() == "sciotte_garmin")
        .expect("Garmin's copy stands");
    assert_eq!(
        *session.sport_type(),
        SportType::TrailRunning,
        "bug: the phone app's sport retyped the Garmin session"
    );
    assert_eq!(
        session.name(),
        "Charlevoix-Est Trail",
        "bug: the phone app's title named the Garmin session"
    );
}

// ============================================================================
// The start window is for one recording's second clock (carnet#1381): it never
// joins what one provider's rows or one device's clock say are two workouts
// ============================================================================

const WATCH: &str = "Garmin fenix 5";

/// A row of one watch's recording, the way a FIT-built row carries it.
fn watch_row(
    id: &str,
    provider: &str,
    start: DateTime<Utc>,
    seconds: u64,
    metres: f64,
) -> Activity {
    provider_activity(id, provider, SportType::Ride, start, seconds)
        .distance_meters(metres)
        .device_name(WATCH)
        .build()
}

/// Two short rides saved apart, six minutes between them, each in the Garmin
/// export and the Strava export: the 15-minute start window meant for a second
/// provider's clock joins the two rides.
#[test]
fn two_short_rides_in_two_exports_stay_two() {
    let t = base_time();
    let second = t + Duration::minutes(14);
    let (garmin, strava) = (
        oauth_providers::GARMIN_ARCHIVE,
        oauth_providers::STRAVA_ARCHIVE,
    );
    let (sessions, _) = merge_duplicates(
        vec![
            watch_row("g1", garmin, t, 480, 2_500.0),
            watch_row("s1", strava, t, 480, 2_500.0),
            watch_row("g2", garmin, second, 480, 2_600.0),
            watch_row("s2", strava, second, 480, 2_600.0),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(
        sessions.len(),
        2,
        "bug: two rides summed into {:?} m",
        sessions[0].distance_meters()
    );
}

/// The same two rides with one copy each: no second row of one provider says
/// they are two, only that one clock made both.
#[test]
fn one_devices_two_rides_in_two_exports_stay_two() {
    let t = base_time();
    let (sessions, _) = merge_duplicates(
        vec![
            watch_row("g1", oauth_providers::GARMIN_ARCHIVE, t, 480, 2_500.0),
            watch_row(
                "s2",
                oauth_providers::STRAVA_ARCHIVE,
                t + Duration::minutes(14),
                480,
                2_600.0,
            ),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(
        sessions.len(),
        2,
        "bug: two rides summed into {:?} m",
        sessions[0].distance_meters()
    );
}

/// The same two rides as live rows naming no device: the two Garmin rows and
/// the two Strava rows are what say they are two.
#[test]
fn two_short_rides_on_two_live_connections_stay_two() {
    let t = base_time();
    let second = t + Duration::minutes(14);
    let row = |id, provider, start, metres| {
        provider_activity(id, provider, SportType::Ride, start, 480)
            .distance_meters(metres)
            .build()
    };
    let (sessions, _) = merge_duplicates(
        vec![
            row("g1", "garmin", t, 2_500.0),
            row("s1", "strava", t, 2_500.0),
            row("g2", "garmin", second, 2_600.0),
            row("s2", "strava", second, 2_600.0),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(
        sessions.len(),
        2,
        "bug: two rides on two connections folded"
    );
}

/// Stopped, saved, restarted four minutes on: touching windows are one session
/// on one device, and summed. A device says the same of an auto-split and of
/// two saves, so only a gap past the tolerance tells them apart.
#[test]
fn consecutive_saves_on_one_device_within_tolerance_are_summed() {
    let t = base_time();
    let (sessions, _) = merge_duplicates(
        vec![
            watch_row("g1", oauth_providers::GARMIN_ARCHIVE, t, 1_800, 15_000.0),
            watch_row(
                "s2",
                oauth_providers::STRAVA_ARCHIVE,
                t + Duration::minutes(34),
                1_800,
                14_000.0,
            ),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(sessions.len(), 1, "bug: touching saves stayed apart");
    assert_eq!(
        sessions[0].distance_meters(),
        Some(29_000.0),
        "bug: touching saves were not summed"
    );
}

/// A second provider's copy minutes off, the windows not touching: still one
/// workout when nothing says otherwise. The bike computer names another
/// device, the live row names none; neither is one clock.
#[test]
fn a_short_ride_minutes_off_on_a_second_provider_still_folds() {
    let t = base_time();
    let off = t + Duration::minutes(14);
    let watch = || watch_row("g1", "garmin", t, 480, 2_500.0);
    let another_device = provider_activity("w1", "wahoo", SportType::Ride, off, 480)
        .distance_meters(2_500.0)
        .device_name("Wahoo ELEMNT")
        .build();
    let no_device = provider_activity("s1", "strava", SportType::Ride, off, 480)
        .distance_meters(2_500.0)
        .build();
    for copy in [another_device, no_device] {
        let (sessions, _) =
            merge_duplicates(vec![watch(), copy], &DedupConfig::with_tolerance(300));
        assert_eq!(sessions.len(), 1, "bug: a copy minutes off stayed apart");
    }
}

/// A date-only capture names the watch it read and carries only the day: the
/// day and distance are its evidence, and one clock says nothing of a row with
/// no clock.
#[test]
fn a_date_only_row_of_one_device_still_pairs_with_the_timed_row() {
    let t = base_time();
    let day = t.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let (sessions, _) = merge_duplicates(
        vec![
            watch_row("g1", "garmin", t, 3_600, 30_000.0),
            watch_row("c1", "sciotte", day, 3_600, 30_000.0),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(sessions.len(), 1, "bug: a date-only copy stayed apart");
}

/// One recording on two providers, the second's clock 14 minutes off: the copy
/// adds no volume and no time. Summed, one 2.5 km ride reads as a 5 km session
/// stretched over 22 minutes.
#[test]
fn a_copy_minutes_off_on_a_second_provider_adds_no_volume() {
    let t = base_time();
    let off = t + Duration::minutes(14);
    let row = |id, provider, start| {
        provider_activity(id, provider, SportType::Ride, start, 480)
            .distance_meters(2_500.0)
            .build()
    };
    let (sessions, report) = merge_duplicates(
        vec![row("g1", "garmin", t), row("s1", "strava", off)],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(
        ids(&sessions),
        vec!["g1"],
        "bug: a clock off was read as a Strava edit"
    );
    assert_eq!(
        sessions[0].distance_meters(),
        Some(2_500.0),
        "bug: a copy added its distance"
    );
    assert_eq!(
        sessions[0].duration_seconds(),
        480,
        "bug: a copy stretched the session"
    );
    assert_eq!(
        report.groups[0].providers,
        vec!["garmin", "strava"],
        "bug: the report lost a provider of the session"
    );
}

/// A leg that touches another row is a leg, whichever provider's clock it sits
/// on: the Strava leg starts within the window of the first Garmin piece but
/// follows the second one, so the three are one ride in three parts.
#[test]
fn a_leg_touching_the_row_before_it_still_sums_inside_the_start_window() {
    let t = base_time();
    let leg = |id, provider, minute, metres| {
        provider_activity(
            id,
            provider,
            SportType::Ride,
            t + Duration::minutes(minute),
            300,
        )
        .distance_meters(metres)
        .build()
    };
    let (sessions, _) = merge_duplicates(
        vec![
            leg("g1", "garmin", 0, 1_000.0),
            leg("g2", "garmin", 7, 1_000.0),
            leg("s3", "strava", 14, 1_000.0),
        ],
        &DedupConfig::with_tolerance(120),
    );
    assert_eq!(sessions.len(), 1, "bug: touching legs stayed apart");
    assert_eq!(
        sessions[0].distance_meters(),
        Some(3_000.0),
        "bug: a touching leg was not summed"
    );
}

/// Pieces a stop apart that a whole-session recording ties together are still
/// the session's pieces, though their starts sit inside the window.
#[test]
fn pieces_a_whole_recording_ties_together_still_sum_inside_the_start_window() {
    let t = base_time();
    let row = |id, provider, minute, seconds, metres| {
        provider_activity(
            id,
            provider,
            SportType::Ride,
            t + Duration::minutes(minute),
            seconds,
        )
        .distance_meters(metres)
        .build()
    };
    let (sessions, _) = merge_duplicates(
        vec![
            row("g1", "garmin", 0, 480, 2_500.0),
            row("w2", "wahoo", 14, 480, 2_500.0),
            row("s", "strava", 0, 1_320, 5_000.0),
        ],
        &DedupConfig::with_tolerance(120),
    );
    assert_eq!(
        ids(&sessions),
        vec!["g1"],
        "bug: the first piece must stand"
    );
    assert_eq!(
        sessions[0].distance_meters(),
        Some(5_000.0),
        "bug: tied pieces were not summed"
    );
}

/// A copy minutes off of the first row, and a later leg touching that copy: the
/// copy adds nothing, the leg adds its own volume.
#[test]
fn a_copy_minutes_off_does_not_take_the_leg_it_touches_with_it() {
    let t = base_time();
    let row = |id, provider, minute, metres| {
        provider_activity(
            id,
            provider,
            SportType::Ride,
            t + Duration::minutes(minute),
            480,
        )
        .distance_meters(metres)
        .build()
    };
    let (sessions, _) = merge_duplicates(
        vec![
            row("g1", "garmin", 0, 2_500.0),
            row("s1", "strava", 14, 2_500.0),
            row("s2", "strava", 23, 5_000.0),
        ],
        &DedupConfig::with_tolerance(120),
    );
    assert_eq!(sessions.len(), 1, "bug: the copy and the leg stayed apart");
    assert_eq!(
        sessions[0].distance_meters(),
        Some(7_500.0),
        "bug: the leg must add its volume and the copy none"
    );
}

/// Two short rides, the first on Garmin and Strava, the second only in the
/// imported Strava export, naming no device. A source and its export are one
/// provider: Strava's two activity ids are two workouts.
#[test]
fn a_source_and_its_export_holding_two_ids_are_two_workouts() {
    let t = base_time();
    let row = |id, provider, start, metres| {
        provider_activity(id, provider, SportType::Ride, start, 480)
            .distance_meters(metres)
            .build()
    };
    let second = t + Duration::minutes(14);
    let (sessions, _) = merge_duplicates(
        vec![
            row("g1", "garmin", t, 2_500.0),
            row("s1", "strava", t, 2_500.0),
            row("a2", "strava_archive", second, 2_600.0),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(sessions.len(), 2, "bug: two ids folded: {sessions:?}");

    // The same pair alone, with nothing but the family to refuse it; likewise
    // for Garmin and its export.
    for (live, archive) in [("strava", "strava_archive"), ("garmin", "garmin_archive")] {
        let (sessions, _) = merge_duplicates(
            vec![
                row("l1", live, t, 2_500.0),
                row("a2", archive, second, 2_600.0),
            ],
            &DedupConfig::with_tolerance(300),
        );
        assert_eq!(sessions.len(), 2, "bug: {live} and {archive} folded");
    }
}

/// The refusal never splits one activity: an export holding Strava's id is a
/// twin of the live row whatever clock it carries, and a third provider's copy
/// minutes off joins them; an export row under another id whose window touches
/// the live row's is another copy of that time.
#[test]
fn a_source_and_its_export_holding_one_activity_stay_one_session() {
    let t = base_time();
    let row = |id, provider, start| {
        provider_activity(id, provider, SportType::Ride, start, 480)
            .distance_meters(2_500.0)
            .build()
    };
    let off = t + Duration::minutes(14);
    for rows in [
        // One id, the clocks past the window apart.
        vec![
            row("x", "strava", t),
            row("x", "strava_archive", t + Duration::hours(5)),
            row("g1", "garmin", off),
        ],
        // One id, one clock, a third provider minutes off.
        vec![
            row("x", "strava", t),
            row("x", "strava_archive", t),
            row("g1", "garmin", off),
        ],
        // Two ids, one stretch of time.
        vec![row("x", "strava", t), row("y", "strava_archive", t)],
        // A multi-session export file keys each session by its own id, the
        // live row covering them both.
        vec![
            provider_activity("x", "strava", SportType::Ride, t, 1_320)
                .distance_meters(2_500.0)
                .build(),
            row("x.s0", "strava_archive", t),
            row("x.s1", "strava_archive", off),
        ],
    ] {
        let (sessions, _) = merge_duplicates(rows, &DedupConfig::with_tolerance(300));
        assert_eq!(sessions.len(), 1, "bug: one activity split: {sessions:?}");
        assert_eq!(
            sessions[0].distance_meters(),
            Some(2_500.0),
            "bug: copies of one activity were summed"
        );
    }
}

/// A ride of 480 s and the given distance on one provider's clock.
fn timed_ride(id: &str, provider: &str, start: DateTime<Utc>, metres: f64) -> Activity {
    provider_activity(id, provider, SportType::Ride, start, 480)
        .distance_meters(metres)
        .build()
}

/// Every order the rows can arrive in.
fn arrival_orders(rows: Vec<Activity>) -> Vec<Vec<Activity>> {
    if rows.len() < 2 {
        return vec![rows];
    }
    let mut orders = Vec::new();
    for first in 0..rows.len() {
        let mut rest = rows.clone();
        let head = rest.remove(first);
        for mut order in arrival_orders(rest) {
            order.insert(0, head.clone());
            orders.push(order);
        }
    }
    orders
}

/// Three copies of one recording, each clock 14 minutes off the last: the first
/// and the third share no start window, only the middle row ties them. Whichever
/// provider ranks canonical, the ride is counted once.
#[test]
fn a_chain_of_start_windows_adds_no_volume_whichever_copy_stands() {
    let t = base_time();
    for (first, second, third) in [
        ("garmin", "strava", "wahoo"),
        ("wahoo", "garmin", "strava"),
        ("strava", "wahoo", "garmin"),
    ] {
        let (sessions, _) = merge_duplicates(
            vec![
                timed_ride("a", first, t, 2_500.0),
                timed_ride("b", second, t + Duration::minutes(14), 2_500.0),
                timed_ride("c", third, t + Duration::minutes(28), 2_500.0),
            ],
            &DedupConfig::with_tolerance(300),
        );
        assert_eq!(
            sessions.len(),
            1,
            "bug: {first}, {second}, {third} did not fold"
        );
        assert_eq!(
            sessions[0].distance_meters(),
            Some(2_500.0),
            "bug: copies summed for {first}, {second}, {third}"
        );
        assert_eq!(
            sessions[0].duration_seconds(),
            480,
            "bug: copies stretched the session for {first}, {second}, {third}"
        );
    }
}

/// One Strava id on two clocks hours apart, and a third row beside the far
/// clock: the API row is that third row's copy through its twin, though it
/// ranks ahead of the twin and shares no time with the row.
#[test]
fn a_twin_on_a_counted_rows_clock_adds_no_volume() {
    let t = base_time();
    let far = t + Duration::hours(5);
    for third in [far, far + Duration::minutes(14)] {
        let (sessions, _) = merge_duplicates(
            vec![
                timed_ride("x", "strava", t, 2_500.0),
                timed_ride("x", "strava_archive", far, 2_500.0),
                timed_ride("g1", "garmin_archive", third, 2_500.0),
            ],
            &DedupConfig::with_tolerance(300),
        );
        assert_eq!(sessions.len(), 1, "bug: one activity left {sessions:?}");
        assert_eq!(
            sessions[0].distance_meters(),
            Some(2_500.0),
            "bug: a twin of a recording was summed"
        );
        assert_eq!(
            sessions[0].duration_seconds(),
            480,
            "bug: a twin of a recording stretched the session"
        );
    }
}

/// The same from the other side: the row beside the far twin ranks between the
/// canonical row and that twin, so it is tried while the twin is uncounted.
#[test]
fn a_row_on_a_counted_rows_twin_clock_adds_no_volume() {
    let t = base_time();
    let far = t + Duration::hours(5);
    let (sessions, _) = merge_duplicates(
        vec![
            timed_ride("x", "garmin", t, 2_500.0),
            timed_ride("x", "garmin_archive", far, 2_500.0),
            timed_ride("w1", "wahoo", far, 2_500.0),
        ],
        &DedupConfig::with_tolerance(300),
    );
    assert_eq!(sessions.len(), 1, "bug: one activity left {sessions:?}");
    assert_eq!(
        sessions[0].distance_meters(),
        Some(2_500.0),
        "bug: a recording of a twin's time was summed"
    );
}

/// A piece keeps its volume though a twin of the canonical row was captured
/// date-only: a row with no clock says nothing about whose time a piece shares.
#[test]
fn a_date_only_twin_leaves_the_pieces_of_a_session() {
    let t = base_time();
    let day = t.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let (sessions, _) = merge_duplicates(
        vec![
            timed_ride("g1", "garmin", t, 2_500.0),
            ActivityBuilder::new(
                "g1",
                "Captured",
                SportType::Ride,
                day,
                480,
                "sciotte_garmin",
            )
            .source("garmin")
            .distance_meters(2_500.0)
            .build(),
            timed_ride("g2", "garmin", t + Duration::minutes(9), 2_500.0),
        ],
        &DedupConfig::with_tolerance(120),
    );
    assert_eq!(sessions.len(), 1, "bug: one session left {sessions:?}");
    assert_eq!(
        sessions[0].distance_meters(),
        Some(5_000.0),
        "bug: a date-only twin hid a piece"
    );
}

/// Two ids of one service are two workouts whichever path each came in by: a
/// timed capture keys its row by the service's own id, as the API and the export
/// do. A date-only capture has no clock to tell a ride from a copy, and still
/// pairs (`a_date_only_scrape_pairs_with_the_timed_record_of_its_day`).
#[test]
fn a_timed_capture_under_another_id_is_another_workout() {
    let t = base_time();
    let second = t + Duration::minutes(14);
    let capture = |service| {
        let backend = if service == "garmin" {
            "sciotte_garmin"
        } else {
            "sciotte"
        };
        ActivityBuilder::new("c2", "Captured", SportType::Ride, second, 480, backend)
            .source(service)
            .distance_meters(2_600.0)
            .build()
    };
    for (first, service) in [
        ("strava", "strava"),
        ("strava_archive", "strava"),
        ("garmin", "garmin"),
        ("garmin_archive", "garmin"),
    ] {
        let (sessions, _) = merge_duplicates(
            vec![timed_ride("a1", first, t, 2_500.0), capture(service)],
            &DedupConfig::with_tolerance(300),
        );
        assert_eq!(
            sessions.len(),
            2,
            "bug: {first} and a capture of {service} under two ids folded"
        );
    }
}

/// What the merge yields depends on the rows, never on the order they came in.
#[test]
fn the_start_window_rules_do_not_depend_on_arrival_order() {
    let t = base_time();
    let minute = |at| t + Duration::minutes(at);
    let shapes = vec![
        (
            vec![
                timed_ride("g", "garmin", t, 2_500.0),
                timed_ride("s", "strava", minute(14), 2_500.0),
                timed_ride("w", "wahoo", minute(28), 2_500.0),
            ],
            2_500.0,
        ),
        (
            vec![
                timed_ride("g1", "garmin", t, 2_500.0),
                timed_ride("s1", "strava", minute(14), 2_500.0),
                timed_ride("s2", "strava", minute(23), 5_000.0),
            ],
            7_500.0,
        ),
        (
            vec![
                timed_ride("g1", "garmin", t, 2_500.0),
                timed_ride("s1", "strava", t, 2_500.0),
                timed_ride("a2", "strava_archive", minute(14), 2_600.0),
            ],
            5_100.0,
        ),
    ];
    for (rows, metres) in shapes {
        for order in arrival_orders(rows) {
            let (sessions, _) = merge_duplicates(order, &DedupConfig::with_tolerance(120));
            let total: f64 = sessions.iter().filter_map(Activity::distance_meters).sum();
            assert!(
                (total - metres).abs() < f64::EPSILON,
                "bug: {total} m, not {metres} m, from {sessions:?}"
            );
        }
    }
}
