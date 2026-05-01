// ABOUTME: Endurance Phase 1 unit tests for build_latest_snapshot — IF / EF / VI / decoupling / zones
// ABOUTME: Pure-function tests against synthetic Activity inputs to lock in the deterministic-output contract
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
use dravr_cageux::models::activity::{Activity, ActivityBuilder, TimeSeriesData};
use dravr_cageux::models::sport::SportType;
use pierre_core::models::zones::HrZoneSet;
use pierre_intelligence::latest_snapshot::{build_latest_snapshot, MAX_WINDOW_DAYS};

struct SyntheticActivity<'a> {
    id: &'a str,
    days_ago: i64,
    duration_seconds: u64,
    distance_meters: f64,
    avg_hr: Option<u32>,
    avg_power: Option<u32>,
    normalized_power: Option<u32>,
    streams: Option<TimeSeriesData>,
}

fn synthetic_activity(spec: SyntheticActivity<'_>) -> Activity {
    let mut builder = ActivityBuilder::new(
        spec.id.to_owned(),
        format!("Test {}", spec.id),
        SportType::Run,
        Utc::now() - Duration::days(spec.days_ago),
        spec.duration_seconds,
        "synthetic".to_owned(),
    );
    if spec.distance_meters > 0.0 {
        builder = builder.distance_meters(spec.distance_meters);
    }
    if let Some(v) = spec.avg_hr {
        builder = builder.average_heart_rate(v);
    }
    if let Some(v) = spec.avg_power {
        builder = builder.average_power(v);
    }
    if let Some(v) = spec.normalized_power {
        builder = builder.normalized_power(v);
    }
    if let Some(stream) = spec.streams {
        builder = builder.time_series_data(stream);
    }
    builder.build()
}

fn typical_run(id: &str, days_ago: i64, np: Option<u32>) -> Activity {
    synthetic_activity(SyntheticActivity {
        id,
        days_ago,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: Some(150),
        avg_power: Some(180),
        normalized_power: np,
        streams: None,
    })
}

fn flat_power_stream(seconds: usize, watts: u32) -> TimeSeriesData {
    let timestamps: Vec<u32> = (0..seconds)
        .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    let power: Vec<u32> = vec![watts; seconds];
    TimeSeriesData {
        timestamps,
        heart_rate: None,
        power: Some(power),
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: None,
    }
}

fn flat_hr_stream(seconds: usize, bpm: u32) -> TimeSeriesData {
    let timestamps: Vec<u32> = (0..seconds)
        .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    let heart_rate: Vec<u32> = vec![bpm; seconds];
    TimeSeriesData {
        timestamps,
        heart_rate: Some(heart_rate),
        power: None,
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: None,
    }
}

#[test]
fn empty_input_yields_empty_snapshot_with_clamped_window() {
    let snap = build_latest_snapshot(&[], 9999, None, None);
    assert_eq!(snap.activity_count, 0);
    assert_eq!(snap.activities.len(), 0);
    assert!(snap.avg_intensity_factor.is_none());
    assert!(snap.aggregated_zone_distribution.is_none());
    assert!(snap.window_days <= MAX_WINDOW_DAYS);
    assert!(snap.window_days >= 1);
}

#[test]
fn intensity_factor_uses_normalized_power_over_ftp() {
    let activity = typical_run("a1", 1, Some(200));
    let snap = build_latest_snapshot(&[activity], 7, Some(250), None);
    assert_eq!(snap.activity_count, 1);
    let metrics = snap.activities[0].metrics;
    let intensity = metrics.intensity_factor.expect("intensity factor present");
    assert!(
        (intensity - 0.8).abs() < 1e-9,
        "expected IF = NP/FTP = 200/250 = 0.8, got {intensity}"
    );
}

#[test]
fn variability_index_is_normalized_over_average_power() {
    let activity = synthetic_activity(SyntheticActivity {
        id: "vi-1",
        days_ago: 1,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: None,
        avg_power: Some(180),
        normalized_power: Some(200),
        streams: None,
    });
    let snap = build_latest_snapshot(&[activity], 7, None, None);
    let vi = snap.activities[0]
        .metrics
        .variability_index
        .expect("variability index present");
    assert!(
        (vi - (200.0 / 180.0)).abs() < 1e-9,
        "expected VI = NP/avg_power, got {vi}"
    );
}

#[test]
fn efficiency_factor_uses_avg_hr() {
    let activity = typical_run("ef-1", 1, Some(200));
    let snap = build_latest_snapshot(&[activity], 7, None, None);
    let ef = snap.activities[0]
        .metrics
        .efficiency_factor
        .expect("efficiency factor present");
    assert!(
        (ef - (200.0 / 150.0)).abs() < 1e-9,
        "expected EF = NP/avg_hr, got {ef}"
    );
}

#[test]
fn missing_ftp_skips_intensity_factor_and_does_not_substitute() {
    let activity = typical_run("no-ftp", 1, Some(200));
    let snap = build_latest_snapshot(&[activity], 7, None, None);
    assert!(
        snap.activities[0].metrics.intensity_factor.is_none(),
        "no FTP must not produce a virtual IF"
    );
    assert!(snap.avg_intensity_factor.is_none());
}

#[test]
fn missing_power_stream_skips_normalized_power_metrics() {
    let activity = synthetic_activity(SyntheticActivity {
        id: "no-power",
        days_ago: 1,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: Some(150),
        avg_power: None,
        normalized_power: None,
        streams: None,
    });
    let snap = build_latest_snapshot(&[activity], 7, Some(250), None);
    let metrics = snap.activities[0].metrics;
    assert!(metrics.intensity_factor.is_none());
    assert!(metrics.efficiency_factor.is_none());
    assert!(metrics.variability_index.is_none());
}

#[test]
fn zone_distribution_buckets_against_user_zones() {
    let zones = HrZoneSet::new(120, 140, 160, 180, 200).unwrap();
    let activity = synthetic_activity(SyntheticActivity {
        id: "zone-test",
        days_ago: 0,
        duration_seconds: 300,
        distance_meters: 2_000.0,
        avg_hr: Some(135),
        avg_power: None,
        normalized_power: None,
        streams: Some(flat_hr_stream(300, 135)),
    });
    let snap = build_latest_snapshot(&[activity], 7, None, Some(zones));
    let zd = snap.activities[0]
        .zone_distribution
        .expect("zone distribution present");
    // 300 samples × 1s gap = 300 seconds, all of them in Z2 (121..=140).
    assert_eq!(zd.z2_seconds, 300);
    assert_eq!(zd.z1_seconds, 0);
    assert_eq!(zd.z3_seconds, 0);
    assert_eq!(zd.above_max_seconds, 0);
    assert_eq!(snap.aggregated_zone_distribution.unwrap().z2_seconds, 300);
}

#[test]
fn normalized_power_falls_back_to_stream_when_field_absent() {
    // 1800s of constant 200W → NormalizedPowerCalculator yields ~200W
    let stream = flat_power_stream(1800, 200);
    let activity = synthetic_activity(SyntheticActivity {
        id: "stream-np",
        days_ago: 0,
        duration_seconds: 1800,
        distance_meters: 5_000.0,
        avg_hr: Some(150),
        avg_power: Some(200),
        normalized_power: None,
        streams: Some(stream),
    });
    let snap = build_latest_snapshot(&[activity], 7, Some(250), None);
    let metrics = snap.activities[0].metrics;
    let intensity = metrics.intensity_factor.expect("intensity factor present");
    assert!(
        (intensity - 0.8).abs() < 0.05,
        "expected IF ~= 200/250 = 0.8, got {intensity}"
    );
}

#[test]
fn activities_outside_window_are_excluded() {
    let recent = typical_run("recent", 1, Some(200));
    let stale = typical_run("stale", 30, Some(200));
    let snap = build_latest_snapshot(&[recent, stale], 7, Some(250), None);
    assert_eq!(snap.activity_count, 1);
    assert_eq!(snap.activities[0].activity_id, "recent");
}

#[test]
fn aggregate_means_use_only_contributing_activities() {
    let with_ftp = synthetic_activity(SyntheticActivity {
        id: "with-power",
        days_ago: 1,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: Some(150),
        avg_power: Some(200),
        normalized_power: Some(220),
        streams: None,
    });
    let no_power = synthetic_activity(SyntheticActivity {
        id: "no-power",
        days_ago: 2,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: Some(150),
        avg_power: None,
        normalized_power: None,
        streams: None,
    });
    let snap = build_latest_snapshot(&[with_ftp, no_power], 7, Some(250), None);
    assert_eq!(snap.activity_count, 2);
    let avg_if = snap.avg_intensity_factor.expect("at least one contributor");
    assert!((avg_if - 0.88).abs() < 1e-9, "got {avg_if}");
}

#[test]
fn ftp_zero_does_not_produce_nan_intensity_factor() {
    // ftp=0 must be rejected, not divide normalized power by zero.
    let activity = typical_run("zero-ftp", 1, Some(200));
    let snap = build_latest_snapshot(&[activity], 7, Some(0), None);
    assert!(snap.activities[0].metrics.intensity_factor.is_none());
    assert!(snap.avg_intensity_factor.is_none());
}

#[test]
fn zero_avg_hr_does_not_produce_nan_efficiency_factor() {
    let activity = synthetic_activity(SyntheticActivity {
        id: "zero-hr",
        days_ago: 1,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: Some(0),
        avg_power: Some(180),
        normalized_power: Some(200),
        streams: None,
    });
    let snap = build_latest_snapshot(&[activity], 7, None, None);
    assert!(snap.activities[0].metrics.efficiency_factor.is_none());
}

#[test]
fn zero_avg_power_does_not_produce_nan_variability_index() {
    let activity = synthetic_activity(SyntheticActivity {
        id: "zero-avg-power",
        days_ago: 1,
        duration_seconds: 3600,
        distance_meters: 10_000.0,
        avg_hr: Some(150),
        avg_power: Some(0),
        normalized_power: Some(200),
        streams: None,
    });
    let snap = build_latest_snapshot(&[activity], 7, None, None);
    assert!(snap.activities[0].metrics.variability_index.is_none());
}

#[test]
fn nonfinite_distance_does_not_poison_window_total() {
    // ActivityBuilder.distance_meters accepts any f64 — including NaN —
    // because cageux is provider-agnostic. The snapshot must not
    // propagate that NaN into the aggregate or the per-row distance.
    let activity = ActivityBuilder::new(
        "nan-distance".to_owned(),
        "Test nan-distance".to_owned(),
        SportType::Run,
        Utc::now() - Duration::days(1),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(f64::NAN)
    .average_heart_rate(150)
    .average_power(180)
    .normalized_power(200)
    .build();
    let snap = build_latest_snapshot(&[activity], 7, Some(250), None);
    assert!(
        snap.total_distance_meters.is_finite(),
        "NaN distance must be discarded, not propagated into the window total — got {}",
        snap.total_distance_meters
    );
    assert!(
        snap.activities[0].distance_meters.is_none(),
        "row distance_meters must drop NaN, not surface it as Some(NaN)"
    );
}

#[test]
fn snapshot_window_total_stays_finite_for_degenerate_activity() {
    // Smallest possible degenerate input: 0-watt average power, 1-bpm HR,
    // 2-second duration, no power stream. None of the four metrics should
    // be present and none of the aggregates should be NaN/Inf.
    let activity = synthetic_activity(SyntheticActivity {
        id: "degenerate",
        days_ago: 0,
        duration_seconds: 2,
        distance_meters: 0.0,
        avg_hr: Some(1),
        avg_power: Some(0),
        normalized_power: None,
        streams: None,
    });
    let snap = build_latest_snapshot(&[activity], 7, Some(250), None);
    let row = &snap.activities[0];
    assert!(row.metrics.intensity_factor.is_none());
    assert!(row.metrics.efficiency_factor.is_none());
    assert!(row.metrics.variability_index.is_none());

    assert!(snap.total_distance_meters.is_finite());
    assert!(snap.avg_intensity_factor.is_none());
    assert!(snap.avg_efficiency_factor.is_none());
    assert!(snap.avg_variability_index.is_none());
    assert!(snap.avg_decoupling_pct.is_none());

    // Round-trip through JSON to confirm the snapshot encodes cleanly even
    // for a degenerate input — serde_json refuses to encode bare NaN.
    let json = serde_json::to_string(&snap).expect("snapshot serializes for degenerate input");
    assert!(
        !json.contains("NaN"),
        "encoded snapshot must not contain NaN literal — got {json}"
    );
    assert!(
        !json.contains("Infinity"),
        "encoded snapshot must not contain Infinity literal — got {json}"
    );
}
