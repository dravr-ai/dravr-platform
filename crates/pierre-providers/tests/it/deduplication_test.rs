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
    // mirror. Same sport, starts 2 min apart, distance within tolerance.
    let t = base_time();
    let activities = vec![
        provider_activity("strava-1", "strava", SportType::Ride, t, 11_520)
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
    assert_eq!(ids(&merged), vec!["strava-1"]);
    assert_eq!(report.groups[0].providers, vec!["strava", "garmin"]);
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
fn a_partial_fragment_collapses_but_donates_nothing() {
    // An auto-split piece covering a third of the ride describes that hour, not
    // the session: its heart rate must not become the session's.
    let t = base_time();
    let activities = vec![
        provider_activity("ride-main", "garmin", SportType::Ride, t, 7_200)
            .distance_meters(60_000.0)
            .build(),
        provider_activity(
            "ride-tail",
            "garmin",
            SportType::Ride,
            t + Duration::seconds(7_203),
            3_600,
        )
        .distance_meters(28_000.0)
        .average_heart_rate(150)
        .build(),
    ];
    let (merged, report) = merge_duplicates(activities, &DedupConfig::default());

    assert_eq!(ids(&merged), vec!["ride-main"]);
    assert_eq!(merged[0].average_heart_rate(), None);
    assert_eq!(report.groups[0].filled_fields, []);
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
    let upload = provider_activity(
        "upload",
        "strava",
        SportType::Ride,
        t + Duration::seconds(45),
        7_250,
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
        provider_activity("strava-1", "strava", SportType::Ride, t, 11_520)
            .distance_meters(84_800.0)
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
    assert_eq!(group.providers, vec!["strava", "garmin"]);
    assert!(
        !group.filled_fields.is_empty(),
        "the scraped row donates its heart rate"
    );
    assert!(group.filled_fields.iter().all(|f| f.provider == "garmin"));
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
