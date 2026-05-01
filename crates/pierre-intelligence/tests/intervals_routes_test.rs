// ABOUTME: Endurance Phase 3 unit tests for build_intervals + build_route_summary
// ABOUTME: Pure tests; locks in interval shape conformance + GPX terrain bucketing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{TimeZone, Utc};
use dravr_cageux::models::activity::{Activity, ActivityBuilder, Lap};
use dravr_cageux::models::sport::SportType;
use pierre_intelligence::intervals::build_intervals;
use pierre_intelligence::routes::{
    build_route_summary, build_route_summary_from_streams, ClimbCategory,
};

fn synthetic_activity_with_laps(laps: Vec<Lap>) -> Activity {
    ActivityBuilder::new(
        "act-1".to_owned(),
        "Test Run".to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 0, 0).unwrap(),
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
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 0, 0).unwrap(),
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

#[test]
fn intervals_intensity_factor_none_when_ftp_missing() {
    let activity = synthetic_activity_with_laps(vec![lap(1, 1000.0, 360, 150, 200)]);
    let export = build_intervals(&activity, None);
    assert!(export.intervals[0].intensity_factor.is_none());
}

const SAMPLE_GPX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="test">
  <trk>
    <trkseg>
      <trkpt lat="46.0000" lon="-73.0000"><ele>100.0</ele></trkpt>
      <trkpt lat="46.0010" lon="-73.0000"><ele>110.0</ele></trkpt>
      <trkpt lat="46.0020" lon="-73.0000"><ele>140.0</ele></trkpt>
      <trkpt lat="46.0030" lon="-73.0000"><ele>180.0</ele></trkpt>
      <trkpt lat="46.0040" lon="-73.0000"><ele>220.0</ele></trkpt>
      <trkpt lat="46.0050" lon="-73.0000"><ele>250.0</ele></trkpt>
      <trkpt lat="46.0060" lon="-73.0000"><ele>240.0</ele></trkpt>
      <trkpt lat="46.0070" lon="-73.0000"><ele>200.0</ele></trkpt>
    </trkseg>
  </trk>
</gpx>"#;

#[test]
fn route_summary_parses_gpx_and_finds_terrain_buckets() {
    let summary = build_route_summary(SAMPLE_GPX.as_bytes()).expect("summary");
    assert!(summary.point_count >= 8);
    assert!(summary.terrain.total_distance_meters > 0.0);
    assert!(summary.terrain.elevation_gain_meters >= 150.0);
    assert!(summary.terrain.elevation_loss_meters >= 50.0);
    assert!(!summary.gpx_hash.is_empty());
}

#[test]
fn route_summary_detects_climb_segment() {
    let summary = build_route_summary(SAMPLE_GPX.as_bytes()).expect("summary");
    assert!(
        !summary.climbs.is_empty(),
        "the GPX fixture climbs ~150m over ~700m → must register at least one climb"
    );
    let climb = &summary.climbs[0];
    assert!(climb.length_meters >= 250.0);
    assert!(climb.avg_gradient > 0.015);
    // A 150m climb at ~20 % is HC-grade.
    assert_ne!(climb.category, ClimbCategory::None);
}

#[test]
fn route_summary_returns_none_for_empty_input() {
    assert!(build_route_summary(b"").is_none());
    assert!(build_route_summary(b"<gpx></gpx>").is_none());
}

#[test]
fn build_route_summary_from_streams_matches_gpx_terrain() {
    let coords: Vec<(f64, f64)> = (0..8)
        .map(|i| (f64::from(i).mul_add(0.001, 46.0), -73.0))
        .collect();
    let altitudes: Vec<f32> = vec![100.0, 110.0, 140.0, 180.0, 220.0, 250.0, 240.0, 200.0];
    let summary = build_route_summary_from_streams(&coords, &altitudes).expect("summary");
    assert!(summary.point_count >= 8);
    assert!(summary.terrain.elevation_gain_meters >= 150.0);
}

#[test]
fn build_route_summary_from_streams_returns_none_for_short_input() {
    assert!(build_route_summary_from_streams(&[], &[]).is_none());
    assert!(build_route_summary_from_streams(&[(46.0, -73.0)], &[100.0]).is_none());
}

// ----------------------------------------------------------------------------
// Phase 3 risk #3 — defensive GPX / stream parsing.
// ----------------------------------------------------------------------------

#[test]
fn build_route_summary_drops_nonfinite_lat_lon_alt() {
    // Two-point stream with NaN coordinates — both points must be filtered
    // before the haversine math runs.
    let coords = vec![(f64::NAN, -73.0), (46.001, f64::INFINITY)];
    let altitudes = vec![100.0_f32, 110.0];
    assert!(
        build_route_summary_from_streams(&coords, &altitudes).is_none(),
        "non-finite coordinates must be dropped, leaving fewer than 2 valid points"
    );

    let alt_nan = vec![f32::NAN, 110.0];
    let coords_ok = vec![(46.000, -73.0), (46.001, -73.0)];
    // One altitude NaN drops one point; the survivor is alone → None.
    assert!(build_route_summary_from_streams(&coords_ok, &alt_nan).is_none());
}

#[test]
fn build_route_summary_drops_out_of_range_lat_lon() {
    let bad_lat = vec![(91.0, 0.0), (-91.0, 0.0)];
    let altitudes = vec![100.0_f32, 110.0];
    assert!(
        build_route_summary_from_streams(&bad_lat, &altitudes).is_none(),
        "lat outside [-90, 90] must be filtered"
    );

    let bad_lon = vec![(0.0, 181.0), (0.0, -181.0)];
    assert!(
        build_route_summary_from_streams(&bad_lon, &altitudes).is_none(),
        "lon outside [-180, 180] must be filtered"
    );
}

#[test]
fn build_route_summary_handles_duplicate_consecutive_points() {
    // Sub-half-metre delta between consecutive points used to trip the
    // f64::EPSILON clamp and amplify elevation_change/EPSILON into a
    // ~1e16 gradient. Verify the duplicates contribute zero distance and
    // don't produce a non-finite terrain summary.
    let coords = vec![
        (46.000, -73.0),
        (46.000, -73.0), // exact duplicate (timestamp glitch)
        (46.000, -73.0),
        (46.001, -73.0),
        (46.002, -73.0),
    ];
    let altitudes = vec![100.0_f32, 100.5, 101.0, 110.0, 120.0];
    let summary =
        build_route_summary_from_streams(&coords, &altitudes).expect("summary should be produced");
    let t = &summary.terrain;
    assert!(
        t.total_distance_meters.is_finite() && t.total_distance_meters > 0.0,
        "duplicates must not collapse the total to 0 nor poison it with NaN"
    );
    assert!(t.flat_meters.is_finite());
    assert!(t.rolling_meters.is_finite());
    assert!(t.climb_meters.is_finite());
    assert!(t.steep_meters.is_finite());
    assert!(t.elevation_gain_meters.is_finite());
    assert!(t.elevation_loss_meters.is_finite());
}

#[test]
fn build_route_summary_drops_non_finite_parsed_attrs() {
    // Some GPX exporters emit "NaN" / "Infinity" as text — Rust's f64
    // parser accepts these, so we have to gate them at parse time.
    let gpx_with_nan_lat = r#"<?xml version="1.0"?>
<gpx>
  <trk><trkseg>
    <trkpt lat="NaN" lon="-73.0"><ele>100</ele></trkpt>
    <trkpt lat="46.001" lon="-73.0"><ele>110</ele></trkpt>
  </trkseg></trk>
</gpx>"#;
    // First trkpt is dropped (NaN lat); only one valid point remains → None.
    assert!(build_route_summary(gpx_with_nan_lat.as_bytes()).is_none());

    let gpx_with_inf_ele = r#"<?xml version="1.0"?>
<gpx>
  <trk><trkseg>
    <trkpt lat="46.000" lon="-73.0"><ele>Infinity</ele></trkpt>
    <trkpt lat="46.001" lon="-73.0"><ele>110</ele></trkpt>
  </trkseg></trk>
</gpx>"#;
    assert!(build_route_summary(gpx_with_inf_ele.as_bytes()).is_none());
}

#[test]
fn route_summary_serializes_without_nan_or_infinity_literals() {
    // Round-trip through serde_json — refuses to encode bare NaN/Inf.
    // If validate() lets a non-finite slip through, this fails.
    let coords: Vec<(f64, f64)> = (0..6)
        .map(|i| (f64::from(i).mul_add(0.001, 46.0), -73.0))
        .collect();
    let altitudes: Vec<f32> = vec![100.0, 110.0, 140.0, 180.0, 220.0, 250.0];
    let summary =
        build_route_summary_from_streams(&coords, &altitudes).expect("summary should produce");
    let json =
        serde_json::to_string(&summary).expect("RouteSummary must serialize cleanly to JSON");
    assert!(
        !json.contains("NaN"),
        "encoded summary contained NaN: {json}"
    );
    assert!(
        !json.contains("Infinity"),
        "encoded summary contained Infinity: {json}"
    );
}
