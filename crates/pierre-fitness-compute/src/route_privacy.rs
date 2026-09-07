// ABOUTME: Endpoint privacy trimming for route geometry — drops the point runs that expose where a trace started and ended
// ABOUTME: Pure compute over the coordinate/elevation/distance triple; every surviving series stays index-aligned
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Route privacy trimming
//!
//! A GPS trace begins and ends at the door it was recorded from, so drawing
//! one verbatim publishes an address. The trim lives on the platform because
//! the geometry travels: the messaging path hands a signed render URL to
//! third-party servers (Telegram, Slack), and a client-side trim would leave
//! the untrimmed track sitting in the payload those servers fetch.
//!
//! [`trim_route_endpoints`] drops the leading run of points closer than
//! `radius_meters` to the first point and the trailing run closer than
//! `radius_meters` to the last point, slicing the elevation and distance
//! series to the same range so index alignment survives.
//!
//! Only the two endpoint runs are trimmed. A loop that passes back through
//! its own start mid-ride keeps that pass: excising it would leave a gap
//! whose two edges sit on a circle centred on the address it removed, which
//! locates the address more precisely than the pass itself does.
//!
//! Distances are sliced, never rebased — the surviving samples keep the
//! cumulative distance they carried in the full ride, so an elevation
//! profile drawn from them still reads as "kilometres into the ride".

use crate::routes::haversine_meters_between;

/// Radius trimmed from each end of a route when a caller has no reason to
/// pick another, in metres.
///
/// 200 m covers the block a trace started on — enough that the endpoint no
/// longer resolves to a building — while removing roughly a minute of an
/// easy run from each end, which keeps the drawn route recognisable.
pub const DEFAULT_PRIVACY_RADIUS_METERS: f64 = 200.0;

/// Route geometry with its endpoint neighbourhoods removed.
///
/// The three series are index-aligned: `elevations` and `distances` are
/// either absent or exactly `coordinates.len()` long.
#[derive(Debug, Clone)]
pub struct TrimmedRoute {
    /// Surviving `(latitude, longitude)` pairs in degrees, in track order.
    pub coordinates: Vec<(f64, f64)>,
    /// Elevation in metres for each surviving point, absent when the input
    /// carried no elevation series.
    pub elevations: Option<Vec<f64>>,
    /// Cumulative distance in metres for each surviving point, measured from
    /// the untrimmed start; absent when the input carried no distance series.
    pub distances: Option<Vec<f64>>,
}

/// Trim the endpoint neighbourhoods off a route, keeping every series aligned.
///
/// Drops the leading points strictly closer than `radius_meters` to
/// `coordinates[0]` and the trailing points strictly closer than
/// `radius_meters` to the last coordinate. The strict comparison makes
/// `radius_meters == 0.0` a no-op — no point is closer than zero metres to
/// itself — and a negative radius behaves the same way.
///
/// Returns `None` when there is nothing safe to draw:
/// - fewer than two input coordinates, or fewer than two survivors,
/// - every point falls inside one of the two radii,
/// - the first or last coordinate is not finite, so the distance to it
///   cannot be measured and no run can be trimmed,
/// - `radius_meters` is not finite,
/// - `elevations` or `distances` is present with a length other than
///   `coordinates.len()`, which no slice can bring back into alignment.
#[must_use]
pub fn trim_route_endpoints(
    coordinates: &[(f64, f64)],
    elevations: Option<&[f64]>,
    distances: Option<&[f64]>,
    radius_meters: f64,
) -> Option<TrimmedRoute> {
    let len = coordinates.len();
    if len < 2 || !radius_meters.is_finite() {
        return None;
    }
    if !series_aligned(elevations, len) || !series_aligned(distances, len) {
        return None;
    }
    let first = coordinates[0];
    let last = coordinates[len - 1];
    if !finite_coordinate(first) || !finite_coordinate(last) {
        return None;
    }
    let start = leading_bound(coordinates, first, radius_meters);
    let end = trailing_bound(coordinates, last, radius_meters, start);
    if end - start < 2 {
        return None;
    }
    Some(TrimmedRoute {
        coordinates: coordinates[start..end].to_vec(),
        elevations: elevations.map(|series| series[start..end].to_vec()),
        distances: distances.map(|series| series[start..end].to_vec()),
    })
}

/// A companion series is usable when absent, or exactly as long as the
/// coordinate series — a shorter one leaves points without a sample and a
/// longer one carries samples with no point to attach to.
fn series_aligned(series: Option<&[f64]>, len: usize) -> bool {
    series.is_none_or(|s| s.len() == len)
}

fn finite_coordinate((latitude, longitude): (f64, f64)) -> bool {
    latitude.is_finite() && longitude.is_finite()
}

/// Index of the first point outside `radius_meters` of `origin`, which is
/// where the kept range starts; `coordinates.len()` when none escapes.
fn leading_bound(coordinates: &[(f64, f64)], origin: (f64, f64), radius_meters: f64) -> usize {
    coordinates
        .iter()
        .position(|&point| !within(origin, point, radius_meters))
        .unwrap_or(coordinates.len())
}

/// Exclusive end of the kept range: one past the last point outside
/// `radius_meters` of `origin`. Never reaches below `floor`, so the leading
/// trim always wins a contest over the same points.
fn trailing_bound(
    coordinates: &[(f64, f64)],
    origin: (f64, f64),
    radius_meters: f64,
    floor: usize,
) -> usize {
    coordinates[floor..]
        .iter()
        .rposition(|&point| !within(origin, point, radius_meters))
        .map_or(floor, |offset| floor + offset + 1)
}

/// Strictly-closer-than test. A non-finite point yields a non-finite
/// distance, which compares false and stops the trim there rather than
/// swallowing the rest of the track.
fn within(origin: (f64, f64), point: (f64, f64), radius_meters: f64) -> bool {
    haversine_meters_between(origin.0, origin.1, point.0, point.1) < radius_meters
}
