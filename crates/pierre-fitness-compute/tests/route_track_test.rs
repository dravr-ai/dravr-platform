// ABOUTME: Unit tests for route_track — overview tracks, simplification to a point budget, and the trimmed overview polyline
// ABOUTME: Locks that every drawn line is trimmed at both ends and that simplification keeps series and climbs aligned
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::TimeSeriesData;
use pierre_fitness_compute::routes::haversine_meters_between;
use pierre_fitness_compute::{
    decode_polyline, encode_polyline, trimmed_overview_polyline, RouteTrack, RouteTrackError,
    DEFAULT_PRIVACY_RADIUS_METERS,
};

/// Parc La Fontaine, Montréal.
const HOME: (f64, f64) = (45.5259, -73.5697);

/// ~11 m of latitude between two samples.
const STEP_DEGREES: f64 = 0.0001;

/// A northbound track of `count` samples that weaves east and west, so no
/// three consecutive points are collinear and simplification has real
/// corners to keep.
fn weaving(count: usize) -> Vec<(f64, f64)> {
    (0..count)
        .map(|i| {
            let step = i as f64;
            (
                STEP_DEGREES.mul_add(step, HOME.0),
                0.0005f64.mul_add((step * 0.21).sin(), HOME.1),
            )
        })
        .collect()
}

/// Streams for `points`, with a steady 4 % climb over the middle third so the
/// terrain analysis finds one real climb.
fn climbing_streams(points: &[(f64, f64)]) -> TimeSeriesData {
    let third = points.len() / 3;
    let altitude = (0..points.len())
        .map(|i| {
            let climbed = i.clamp(third, 2 * third) - third;
            0.45f32.mul_add(climbed as f32, 100.0)
        })
        .collect();
    TimeSeriesData {
        timestamps: (0..points.len() as u32).collect(),
        heart_rate: None,
        power: None,
        cadence: None,
        speed: None,
        altitude: Some(altitude),
        temperature: None,
        gps_coordinates: Some(points.to_vec()),
    }
}

fn metres_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    haversine_meters_between(a.0, a.1, b.0, b.1)
}

#[test]
fn an_overview_track_is_trimmed_at_both_ends_and_carries_no_vertical() {
    let recorded = weaving(300);
    let track = RouteTrack::from_overview(&recorded).unwrap();

    let first = track.coordinates[0];
    let last = *track.coordinates.last().unwrap();
    assert!(metres_between(first, recorded[0]) >= DEFAULT_PRIVACY_RADIUS_METERS);
    assert!(metres_between(last, *recorded.last().unwrap()) >= DEFAULT_PRIVACY_RADIUS_METERS);
    assert!(track.coordinates.len() < recorded.len());
    assert!(track.elevation_meters.is_none());
    assert!(track.distances_meters.is_none());
    assert!(track.climbs.is_empty());
    assert!(track.bounds.min_latitude <= first.0 && first.0 <= track.bounds.max_latitude);
}

#[test]
fn an_empty_overview_is_no_gps_and_a_doorstep_one_is_too_short() {
    assert_eq!(
        RouteTrack::from_overview(&[]).unwrap_err(),
        RouteTrackError::NoGps
    );
    let doorstep = weaving(10);
    assert_eq!(
        RouteTrack::from_overview(&doorstep).unwrap_err(),
        RouteTrackError::TooShort
    );
}

#[test]
fn an_empty_gps_channel_is_no_gps() {
    let mut streams = climbing_streams(&weaving(50));
    streams.gps_coordinates = Some(Vec::new());
    assert_eq!(
        RouteTrack::from_streams(&streams).unwrap_err(),
        RouteTrackError::NoGps
    );
}

#[test]
fn simplification_keeps_the_budget_the_ends_and_the_series_alignment() {
    let recorded = weaving(1_200);
    let track = RouteTrack::from_streams(&climbing_streams(&recorded)).unwrap();
    assert!(track.coordinates.len() > 1_000);
    assert_eq!(track.climbs.len(), 1, "the fixture holds one real climb");
    let original = track.clone();

    let simplified = track.simplified(200);

    assert_eq!(simplified.coordinates.len(), 200);
    assert_eq!(simplified.coordinates[0], original.coordinates[0]);
    assert_eq!(
        simplified.coordinates.last(),
        original.coordinates.last(),
        "the drawn line still ends where the trimmed ride does"
    );
    let elevations = simplified.elevation_meters.as_ref().unwrap();
    let distances = simplified.distances_meters.as_ref().unwrap();
    assert_eq!(elevations.len(), 200);
    assert_eq!(distances.len(), 200);
    // Every kept sample carries the altitude and distance it had before.
    for (index, point) in simplified.coordinates.iter().enumerate() {
        let source = original
            .coordinates
            .iter()
            .position(|p| p == point)
            .unwrap();
        assert_eq!(
            elevations[index].to_bits(),
            original.elevation_meters.as_ref().unwrap()[source].to_bits()
        );
        assert_eq!(
            distances[index].to_bits(),
            original.distances_meters.as_ref().unwrap()[source].to_bits()
        );
    }
    // The climb still starts and ends on the samples it did.
    let before = &original.climbs[0];
    let after = &simplified.climbs[0];
    assert_eq!(
        simplified.coordinates[after.start_index],
        original.coordinates[before.start_index]
    );
    assert_eq!(
        simplified.coordinates[after.end_index],
        original.coordinates[before.end_index]
    );
    assert!((after.avg_gradient - before.avg_gradient).abs() < f64::EPSILON);
}

#[test]
fn a_straight_line_simplifies_to_its_two_ends() {
    let straight: Vec<(f64, f64)> = (0..500)
        .map(|i| (STEP_DEGREES.mul_add(f64::from(i), HOME.0), HOME.1))
        .collect();
    let track = RouteTrack::from_overview(&straight).unwrap();
    let simplified = track.clone().simplified(200);
    assert_eq!(
        simplified.coordinates,
        vec![track.coordinates[0], *track.coordinates.last().unwrap()],
        "points on the chord carry nothing a map would show"
    );
}

#[test]
fn a_track_within_budget_is_returned_as_it_is() {
    let track = RouteTrack::from_overview(&weaving(120)).unwrap();
    assert_eq!(track.clone().simplified(200), track);
}

#[test]
fn the_trimmed_overview_polyline_starts_outside_the_privacy_radius() {
    let recorded = weaving(300);
    let raw = encode_polyline(&recorded);
    let trimmed = trimmed_overview_polyline(&raw).unwrap();
    let points = decode_polyline(&trimmed).unwrap();

    assert!(points.len() < recorded.len());
    assert!(metres_between(points[0], recorded[0]) >= DEFAULT_PRIVACY_RADIUS_METERS - 1.0);
    assert!(
        metres_between(*points.last().unwrap(), *recorded.last().unwrap())
            >= DEFAULT_PRIVACY_RADIUS_METERS - 1.0
    );
}

#[test]
fn a_malformed_or_doorstep_polyline_has_no_trimmed_overview() {
    assert!(trimmed_overview_polyline("_p~iF").is_none());
    assert!(trimmed_overview_polyline("").is_none());
    assert!(trimmed_overview_polyline(&encode_polyline(&weaving(10))).is_none());
}
