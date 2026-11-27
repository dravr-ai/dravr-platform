// ABOUTME: Endurance Phase 3 unit tests for build_intervals
// ABOUTME: Pure tests; locks in interval shape conformance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_cageux::models::activity::{Activity, ActivityBuilder, Lap, TimeSeriesData};
use dravr_cageux::models::sport::SportType;
use dravr_tronc::dates::Timestamp;
use pierre_fitness_compute::intervals::build_intervals;

fn synthetic_activity_with_laps(laps: Vec<Lap>) -> Activity {
    ActivityBuilder::new(
        "act-1".to_owned(),
        "Test Run".to_owned(),
        SportType::Run,
        Timestamp::parse_from_rfc3339("2026-04-01T12:00:00Z").unwrap(),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(160)
    .average_power(220)
    .laps(laps)
    .build()
}

fn lap(index: u32, distance: f64, seconds: u64, hr: u32, power: u32) -> Lap {
    Lap {
        id: None,
        index,
        distance_meters: distance,
        elapsed_time_seconds: seconds,
        moving_time_seconds: Some(seconds),
        elevation_gain_meters: Some(20.0),
        average_speed_mps: Some(distance / seconds as f64),
        max_speed_mps: None,
        average_heart_rate: Some(hr),
        max_heart_rate: None,
        average_cadence: None,
        average_power: Some(power),
    }
}

#[test]
fn intervals_returns_one_row_per_lap() {
    let laps = vec![
        lap(1, 1000.0, 360, 150, 200),
        lap(2, 1000.0, 350, 165, 240),
        lap(3, 1000.0, 340, 175, 260),
    ];
    let activity = synthetic_activity_with_laps(laps);
    let export = build_intervals(&activity, Some(280));
    assert_eq!(export.interval_count, 3);
    assert_eq!(export.intervals.len(), 3);
    assert_eq!(export.intervals[0].avg_hr, Some(150));
    assert_eq!(export.intervals[2].avg_power, Some(260));
}

#[test]
fn intervals_synthesises_one_row_when_no_laps() {
    let activity = ActivityBuilder::new(
        "lone".to_owned(),
        "No Laps Run".to_owned(),
        SportType::Run,
        Timestamp::parse_from_rfc3339("2026-04-01T12:00:00Z").unwrap(),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(155)
    .average_power(210)
    .normalized_power(220)
    .build();
    let export = build_intervals(&activity, Some(250));
    assert_eq!(export.interval_count, 1);
    let row = &export.intervals[0];
    assert_eq!(row.index, 1);
    assert_eq!(row.avg_hr, Some(155));
    assert_eq!(row.normalized_power, Some(220.0));
    assert_eq!(row.intensity_factor, Some(220.0 / 250.0));
}

/// Three 60-second laps, each held at a constant power so a lap's own
/// normalized power is exactly that power. Only lap 2 drifts its heart rate
/// upward at constant speed, so only lap 2 decouples.
fn three_lap_stream() -> TimeSeriesData {
    let mut timestamps = Vec::with_capacity(180);
    let mut power = Vec::with_capacity(180);
    let mut heart_rate = Vec::with_capacity(180);
    let mut speed = Vec::with_capacity(180);
    for second in 0..180_u32 {
        timestamps.push(second);
        match second {
            0..=59 => {
                power.push(100);
                heart_rate.push(140);
                speed.push(3.0);
            }
            60..=119 => {
                power.push(200);
                heart_rate.push(if second < 90 { 150 } else { 165 });
                speed.push(3.0);
            }
            _ => {
                power.push(300);
                heart_rate.push(170);
                speed.push(4.0);
            }
        }
    }
    TimeSeriesData {
        timestamps,
        heart_rate: Some(heart_rate.into_iter().map(Some).collect()),
        power: Some(power.into_iter().map(Some).collect()),
        cadence: None,
        speed: Some(speed.into_iter().map(Some).collect()),
        altitude: None,
        temperature: None,
        gps_coordinates: None,
        distance: None,
        grade_smooth: None,
        moving: None,
    }
}

fn activity_with_stream(laps: Vec<Lap>, stream: TimeSeriesData) -> Activity {
    ActivityBuilder::new(
        "act-laps".to_owned(),
        "Interval Session".to_owned(),
        SportType::Run,
        Timestamp::parse_from_rfc3339("2026-04-01T12:00:00Z").unwrap(),
        180,
        "synthetic".to_owned(),
    )
    .distance_meters(900.0)
    .laps(laps)
    .time_series_data(stream)
    .build()
}

#[test]
fn intervals_compute_stream_metrics_over_each_lap_window() {
    let laps = vec![
        lap(1, 300.0, 60, 140, 100),
        lap(2, 300.0, 60, 155, 200),
        lap(3, 300.0, 60, 170, 300),
    ];
    let activity = activity_with_stream(laps, three_lap_stream());
    let export = build_intervals(&activity, Some(250));
    assert_eq!(export.intervals.len(), 3);

    let np = |index: usize| export.intervals[index].normalized_power.unwrap();
    let intensity = |index: usize| export.intervals[index].intensity_factor.unwrap();
    let drift = |index: usize| export.intervals[index].decoupling_pct.unwrap();
    let start = |index: usize| export.intervals[index].start_time;

    // Each lap holds a constant power, so its own NP is that power. Reading
    // the whole activity gives all three rows one blended number instead.
    assert!((np(0) - 100.0).abs() < 1e-6, "lap 1 NP was {}", np(0));
    assert!((np(1) - 200.0).abs() < 1e-6, "lap 2 NP was {}", np(1));
    assert!((np(2) - 300.0).abs() < 1e-6, "lap 3 NP was {}", np(2));

    assert!(
        (intensity(0) - 0.4).abs() < 1e-6,
        "lap 1 IF {}",
        intensity(0)
    );
    assert!(
        (intensity(1) - 0.8).abs() < 1e-6,
        "lap 2 IF {}",
        intensity(1)
    );
    assert!(
        (intensity(2) - 1.2).abs() < 1e-6,
        "lap 3 IF {}",
        intensity(2)
    );

    // Constant HR at constant speed does not decouple; lap 2 drifts 150 → 165
    // bpm across its halves at a fixed 3.0 m/s, which is exactly 10 %.
    assert!(drift(0).abs() < 1e-6, "lap 1 drifted {}", drift(0));
    assert!((drift(1) - 10.0).abs() < 1e-6, "lap 2 drift {}", drift(1));
    assert!(drift(2).abs() < 1e-6, "lap 3 drifted {}", drift(2));

    // Each lap starts where the previous one ended.
    assert_eq!(
        start(0),
        Timestamp::parse_from_rfc3339("2026-04-01T12:00:00Z").unwrap()
    );
    assert_eq!(
        start(1),
        Timestamp::parse_from_rfc3339("2026-04-01T12:01:00Z").unwrap()
    );
    assert_eq!(
        start(2),
        Timestamp::parse_from_rfc3339("2026-04-01T12:02:00Z").unwrap()
    );
}

#[test]
fn intervals_leave_stream_metrics_absent_for_a_lap_with_no_samples() {
    let stream = TimeSeriesData {
        timestamps: (0..60_u32).collect(),
        heart_rate: Some(vec![Some(140); 60]),
        power: Some(vec![Some(100); 60]),
        cadence: None,
        speed: Some(vec![Some(3.0); 60]),
        altitude: None,
        temperature: None,
        gps_coordinates: None,
        distance: None,
        grade_smooth: None,
        moving: None,
    };
    let laps = vec![lap(1, 300.0, 60, 140, 100), lap(2, 300.0, 60, 150, 180)];
    let activity = activity_with_stream(laps, stream);
    let export = build_intervals(&activity, Some(250));

    let rows = &export.intervals;
    assert!((rows[0].normalized_power.unwrap() - 100.0).abs() < 1e-6);
    assert!(
        rows[1].normalized_power.is_none(),
        "the second lap has no samples, so it must not borrow the first lap's power"
    );
    assert!(rows[1].intensity_factor.is_none());
    assert!(rows[1].decoupling_pct.is_none());
    assert_eq!(
        rows[1].start_time,
        Timestamp::parse_from_rfc3339("2026-04-01T12:01:00Z").unwrap()
    );
}

#[test]
fn intervals_intensity_factor_none_when_ftp_missing() {
    let activity = synthetic_activity_with_laps(vec![lap(1, 1000.0, 360, 150, 200)]);
    let export = build_intervals(&activity, None);
    assert!(export.intervals[0].intensity_factor.is_none());
}
