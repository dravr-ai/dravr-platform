// ABOUTME: Endurance Phase 3 unit tests for build_intervals + build_route_summary
// ABOUTME: Pure tests; locks in interval shape conformance + GPX terrain bucketing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{TimeZone, Utc};
use dravr_cageux::models::activity::{Activity, ActivityBuilder, Lap, TimeSeriesData};
use dravr_cageux::models::sport::SportType;
use pierre_fitness_compute::intervals::build_intervals;
use pierre_fitness_compute::routes::{
    build_route_summary, build_route_summary_from_streams, route_summary_from_cache,
    stream_route_identity, ClimbCategory,
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
        heart_rate: Some(heart_rate),
        power: Some(power),
        cadence: None,
        speed: Some(speed),
        altitude: None,
        temperature: None,
        gps_coordinates: None,
    }
}

fn activity_with_stream(laps: Vec<Lap>, stream: TimeSeriesData) -> Activity {
    ActivityBuilder::new(
        "act-laps".to_owned(),
        "Interval Session".to_owned(),
        SportType::Run,
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 0, 0).unwrap(),
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
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 0, 0).unwrap()
    );
    assert_eq!(
        start(1),
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 1, 0).unwrap()
    );
    assert_eq!(
        start(2),
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 2, 0).unwrap()
    );
}

#[test]
fn intervals_leave_stream_metrics_absent_for_a_lap_with_no_samples() {
    let stream = TimeSeriesData {
        timestamps: (0..60_u32).collect(),
        heart_rate: Some(vec![140; 60]),
        power: Some(vec![100; 60]),
        cadence: None,
        speed: Some(vec![3.0; 60]),
        altitude: None,
        temperature: None,
        gps_coordinates: None,
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
        Utc.with_ymd_and_hms(2026, 4, 1, 12, 1, 0).unwrap()
    );
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

// ----------------------------------------------------------------------------
// route_summaries cache plumbing — stream_route_identity + route_summary_from_cache
// ----------------------------------------------------------------------------

#[test]
fn stream_route_identity_matches_full_build() {
    let coords: Vec<(f64, f64)> = (0..8)
        .map(|i| (f64::from(i).mul_add(0.001, 46.0), -73.0))
        .collect();
    let altitudes: Vec<f32> = vec![100.0, 110.0, 140.0, 180.0, 220.0, 250.0, 240.0, 200.0];
    let (gpx_hash, point_count) =
        stream_route_identity(&coords, &altitudes).expect("identity for a valid stream");
    let summary = build_route_summary_from_streams(&coords, &altitudes).expect("summary");
    assert_eq!(
        gpx_hash, summary.gpx_hash,
        "the identity hash must be the exact hash the full builder stamps — a divergence \
         makes every cache probe miss (or worse, hit a different track's row)"
    );
    assert_eq!(point_count, summary.point_count);
    assert_eq!(gpx_hash.len(), 64, "SHA-256 hex is 64 chars");
}

#[test]
fn stream_route_identity_applies_the_builder_validity_gate() {
    // Same gates as build_route_summary_from_streams: <2 paired points,
    // or every point filtered out by the finite/range checks.
    assert!(stream_route_identity(&[], &[]).is_none());
    assert!(stream_route_identity(&[(46.0, -73.0)], &[100.0]).is_none());
    let bad_lat = vec![(91.0, 0.0), (-91.0, 0.0)];
    let altitudes = vec![100.0_f32, 110.0];
    assert!(stream_route_identity(&bad_lat, &altitudes).is_none());
    let coords_ok = vec![(46.000, -73.0), (46.001, -73.0)];
    let alt_nan = vec![f32::NAN, 110.0];
    assert!(stream_route_identity(&coords_ok, &alt_nan).is_none());
}

#[test]
fn route_summary_from_cache_round_trips_the_stored_blobs() {
    let coords: Vec<(f64, f64)> = (0..8)
        .map(|i| (f64::from(i).mul_add(0.001, 46.0), -73.0))
        .collect();
    let altitudes: Vec<f32> = vec![100.0, 110.0, 140.0, 180.0, 220.0, 250.0, 240.0, 200.0];
    let summary = build_route_summary_from_streams(&coords, &altitudes).expect("summary");
    // The cache stores these two blobs; identity supplies hash + count.
    let terrain_json = serde_json::to_string(&summary.terrain).expect("terrain json");
    let climbs_json = serde_json::to_string(&summary.climbs).expect("climbs json");

    let rebuilt = route_summary_from_cache(
        &summary.gpx_hash,
        summary.point_count,
        &terrain_json,
        &climbs_json,
    )
    .expect("cached row must rebuild");
    assert_eq!(rebuilt.gpx_hash, summary.gpx_hash);
    assert_eq!(rebuilt.point_count, summary.point_count);
    assert!(
        (rebuilt.terrain.total_distance_meters - summary.terrain.total_distance_meters).abs()
            < f64::EPSILON
    );
    assert!(
        (rebuilt.terrain.elevation_gain_meters - summary.terrain.elevation_gain_meters).abs()
            < f64::EPSILON
    );
    assert_eq!(rebuilt.climbs.len(), summary.climbs.len());
    for (r, s) in rebuilt.climbs.iter().zip(summary.climbs.iter()) {
        assert_eq!(r.category, s.category);
        assert_eq!(r.start_index, s.start_index);
        assert_eq!(r.end_index, s.end_index);
    }
}

#[test]
fn route_summary_from_cache_rejects_a_row_of_another_shape() {
    // A row written before a TerrainSummary/Climb shape change must read as
    // a miss (None), never as a mangled summary.
    assert!(route_summary_from_cache("hash", 10, "not json", "[]").is_none());
    assert!(route_summary_from_cache("hash", 10, r#"{"unrelated": true}"#, "[]").is_none());
    let valid_terrain = r#"{"total_distance_meters":5000.0,"flat_meters":1000.0,"rolling_meters":2000.0,"climb_meters":1500.0,"steep_meters":500.0,"elevation_gain_meters":250.0,"elevation_loss_meters":200.0}"#;
    assert!(route_summary_from_cache("hash", 10, valid_terrain, r#"[{"bogus":1}]"#).is_none());
    assert!(
        route_summary_from_cache("hash", 10, valid_terrain, "[]").is_some(),
        "a well-shaped row must rebuild"
    );
}
