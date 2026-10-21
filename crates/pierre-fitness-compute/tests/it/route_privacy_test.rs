// ABOUTME: Unit tests for route_privacy::trim_route_endpoints — endpoint trimming over real Montreal coordinates
// ABOUTME: Locks the surviving point counts, the series alignment, and every "nothing safe to draw" refusal
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_fitness_compute::routes::haversine_meters_between;
use pierre_fitness_compute::{trim_route_endpoints, DEFAULT_PRIVACY_RADIUS_METERS};

/// Parc La Fontaine, Montréal — the start of every synthetic track below.
const HOME_LATITUDE: f64 = 45.5259;
const HOME_LONGITUDE: f64 = -73.5697;

/// One thousandth of a degree of latitude is 111.195 m on the WGS84 mean
/// radius the crate's haversine uses, so a due-north track spaced this way
/// puts point `i` exactly `i * 111.195` m from the start.
const STEP_DEGREES: f64 = 0.001;
const STEP_METERS: f64 = 111.194_926_644_558_73;

/// A due-north track of `count` points from the home coordinate.
fn northbound(count: usize) -> Vec<(f64, f64)> {
    (0..count)
        .map(|i| {
            (
                STEP_DEGREES.mul_add(i as f64, HOME_LATITUDE),
                HOME_LONGITUDE,
            )
        })
        .collect()
}

/// Elevation climbing 3 m per point, so a sliced series is identifiable.
fn elevations(count: usize) -> Vec<f64> {
    (0..count).map(|i| 3.0f64.mul_add(i as f64, 24.0)).collect()
}

/// Cumulative distance matching the northbound spacing.
fn distances(count: usize) -> Vec<f64> {
    (0..count).map(|i| STEP_METERS * i as f64).collect()
}

#[test]
fn step_spacing_matches_the_crate_haversine() {
    let track = northbound(2);
    let measured = haversine_meters_between(track[0].0, track[0].1, track[1].0, track[1].1);
    assert!(
        (measured - STEP_METERS).abs() < 1e-6,
        "northbound step measured {measured} m, expected {STEP_METERS} m"
    );
}

#[test]
fn trims_both_endpoint_runs_and_keeps_series_aligned() {
    let track = northbound(11);
    let elevation = elevations(11);
    let distance = distances(11);

    // At 250 m, points 0/1/2 (0, 111.2, 222.4 m from the start) and points
    // 10/9/8 (same distances from the finish) fall inside the radius.
    let trimmed = trim_route_endpoints(&track, Some(&elevation), Some(&distance), 250.0)
        .expect("a 11-point track keeps 5 points at a 250 m radius");

    assert_eq!(trimmed.coordinates.len(), 5);
    assert_eq!(trimmed.coordinates[0], track[3]);
    assert_eq!(trimmed.coordinates[4], track[7]);

    let kept_elevation = trimmed.elevations.expect("elevation series survives");
    assert_eq!(kept_elevation.len(), 5);
    assert_eq!(kept_elevation, vec![33.0, 36.0, 39.0, 42.0, 45.0]);

    let kept_distance = trimmed.distances.expect("distance series survives");
    assert_eq!(kept_distance.len(), 5);
    // Distances are sliced, not rebased: the first survivor keeps the three
    // steps of ride that were trimmed off in front of it.
    assert_eq!(kept_distance, distance[3..8].to_vec());
    assert!(
        kept_distance[0] > 333.0 && kept_distance[0] < 334.0,
        "first survivor is 3 steps into the ride, not at zero: {}",
        kept_distance[0]
    );
}

#[test]
fn absent_series_stay_absent() {
    let track = northbound(11);
    let trimmed = trim_route_endpoints(&track, None, None, 250.0)
        .expect("coordinates alone are enough to trim");

    assert_eq!(trimmed.coordinates.len(), 5);
    assert!(trimmed.elevations.is_none());
    assert!(trimmed.distances.is_none());
}

#[test]
fn zero_radius_keeps_every_point() {
    let track = northbound(11);
    let elevation = elevations(11);
    let trimmed = trim_route_endpoints(&track, Some(&elevation), None, 0.0)
        .expect("a zero radius trims nothing");

    assert_eq!(trimmed.coordinates.len(), 11);
    assert_eq!(trimmed.coordinates[0], track[0]);
    assert_eq!(trimmed.coordinates[10], track[10]);
    assert_eq!(
        trimmed.elevations.expect("elevation series survives").len(),
        11
    );
}

#[test]
fn negative_radius_keeps_every_point() {
    let track = northbound(6);
    let trimmed = trim_route_endpoints(&track, None, None, -50.0)
        .expect("no point is closer than a negative radius");

    assert_eq!(trimmed.coordinates.len(), 6);
    assert_eq!(trimmed.coordinates[0], track[0]);
}

#[test]
fn route_entirely_inside_the_radius_is_refused() {
    // 5 points spaced 11.12 m: the whole track spans 44 m, well inside 200 m.
    let track: Vec<(f64, f64)> = (0..5)
        .map(|i| {
            (
                0.000_1f64.mul_add(f64::from(i), HOME_LATITUDE),
                HOME_LONGITUDE,
            )
        })
        .collect();

    assert!(
        trim_route_endpoints(&track, None, None, DEFAULT_PRIVACY_RADIUS_METERS).is_none(),
        "a 44 m track around the start has nothing safe to draw"
    );
}

#[test]
fn single_surviving_point_is_refused() {
    // 7 points: the 250 m radius eats 0/1/2 from the front and 6/5/4 from the
    // back, leaving point 3 alone — one point draws no line.
    let track = northbound(7);
    assert!(trim_route_endpoints(&track, None, None, 250.0).is_none());
}

#[test]
fn two_surviving_points_are_kept() {
    // 8 points at the same radius leave 3 and 4 — the shortest drawable line.
    let track = northbound(8);
    let trimmed =
        trim_route_endpoints(&track, None, None, 250.0).expect("two survivors still draw a line");

    assert_eq!(trimmed.coordinates.len(), 2);
    assert_eq!(trimmed.coordinates[0], track[3]);
    assert_eq!(trimmed.coordinates[1], track[4]);
}

#[test]
fn a_mid_ride_pass_through_the_start_is_kept() {
    // Out to the north, back over the start, out again to the south: index 3
    // sits 11 m from home but is not part of either endpoint run.
    let track = vec![
        (HOME_LATITUDE, HOME_LONGITUDE),
        (HOME_LATITUDE + 0.003, HOME_LONGITUDE),
        (HOME_LATITUDE + 0.006, HOME_LONGITUDE),
        (HOME_LATITUDE + 0.000_1, HOME_LONGITUDE),
        (HOME_LATITUDE - 0.003, HOME_LONGITUDE),
        (HOME_LATITUDE - 0.006, HOME_LONGITUDE),
    ];
    let trimmed = trim_route_endpoints(&track, None, None, 250.0).expect("the middle survives");

    assert_eq!(trimmed.coordinates.len(), 4);
    assert_eq!(trimmed.coordinates[0], track[1]);
    assert_eq!(
        trimmed.coordinates[2], track[3],
        "the mid-ride pass over the start stays on the drawn line"
    );
}

#[test]
fn fewer_than_two_input_points_is_refused() {
    assert!(trim_route_endpoints(&[], None, None, 200.0).is_none());
    assert!(trim_route_endpoints(&[(HOME_LATITUDE, HOME_LONGITUDE)], None, None, 200.0).is_none());
}

#[test]
fn a_misaligned_companion_series_is_refused() {
    let track = northbound(11);
    let short_elevation = elevations(10);
    let long_distance = distances(12);

    assert!(
        trim_route_endpoints(&track, Some(&short_elevation), None, 250.0).is_none(),
        "a 10-sample elevation series cannot align with 11 points"
    );
    assert!(
        trim_route_endpoints(&track, None, Some(&long_distance), 250.0).is_none(),
        "a 12-sample distance series cannot align with 11 points"
    );
}

#[test]
fn a_non_finite_endpoint_is_refused() {
    let mut leading = northbound(11);
    leading[0] = (f64::NAN, HOME_LONGITUDE);
    assert!(
        trim_route_endpoints(&leading, None, None, 250.0).is_none(),
        "an unmeasurable start would silently publish the second point"
    );

    let mut trailing = northbound(11);
    trailing[10] = (HOME_LATITUDE, f64::INFINITY);
    assert!(trim_route_endpoints(&trailing, None, None, 250.0).is_none());
}

#[test]
fn a_non_finite_radius_is_refused() {
    let track = northbound(11);
    assert!(trim_route_endpoints(&track, None, None, f64::NAN).is_none());
    assert!(trim_route_endpoints(&track, None, None, f64::INFINITY).is_none());
}

#[test]
fn default_radius_trims_a_real_length_ride() {
    // 90 points, 111.2 m apart: a 10 km out-and-back leg. The 200 m default
    // eats points 0/1 at each end (0 and 111.2 m), leaving 86.
    let track = northbound(90);
    let elevation = elevations(90);
    let trimmed = trim_route_endpoints(
        &track,
        Some(&elevation),
        None,
        DEFAULT_PRIVACY_RADIUS_METERS,
    )
    .expect("a 10 km leg survives the default radius");

    assert_eq!(trimmed.coordinates.len(), 86);
    assert_eq!(trimmed.coordinates[0], track[2]);
    assert_eq!(trimmed.coordinates[85], track[87]);
    assert_eq!(
        trimmed.elevations.expect("elevation series survives").len(),
        86
    );
}
