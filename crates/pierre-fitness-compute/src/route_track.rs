// ABOUTME: One activity's drawable route — privacy-trimmed coordinates with index-aligned elevation, distance and climbs
// ABOUTME: Built from recorded streams or a provider's route overview; the one derivation the chat map and Home share

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Drawable route tracks
//!
//! A recorded track runs to thousands of coordinates and starts and ends at
//! the door it was recorded from. [`RouteTrack`] is what the platform draws
//! instead: the points both halves of a stream record, minus the endpoint
//! neighbourhoods [`trim_route_endpoints`] removes, with the corners that
//! frame a map and the climbs the terrain analysis finds.
//!
//! Every series carried alongside the coordinates is either absent or exactly
//! as long as them. They are built from the same filtered pass and sliced by
//! the same privacy trim, so a client can index one by the other without
//! checking. A padded elevation array would put a climb marker on the wrong
//! kilometre, which is worse than drawing no marker at all.
//!
//! Two inputs build one: an activity's per-second streams
//! ([`RouteTrack::from_streams`]) and a provider's route overview, such as a
//! decoded Strava `summary_polyline` ([`RouteTrack::from_overview`]). Both go
//! through the same gate and the same trim, which is why the chat map, the
//! Home page and anything after them cannot disagree about where a ride ends.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::error::Error;
use std::fmt;

use pierre_core::models::TimeSeriesData;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::polyline::{decode_polyline, encode_polyline};
use crate::route_privacy::{trim_route_endpoints, DEFAULT_PRIVACY_RADIUS_METERS};
use crate::routes::{build_route_summary_from_streams, haversine_meters_between, ClimbCategory};

/// Mean Earth radius in metres, for the planar distances simplification ranks
/// points by. The same sphere the haversine uses.
const EARTH_RADIUS_METERS: f64 = 6_371_000.0;

/// How far off its chord a point must sit for simplification to keep it: a
/// centimetre changes nothing a map can draw, and floating-point noise on a
/// straight stretch must not read as a corner.
const MIN_DEVIATION_METERS: f64 = 0.01;

/// Why an activity has no drawable track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTrackError {
    /// The activity recorded no GPS channel: indoor, trainer, manual entry.
    NoGps,
    /// A track exists, but fewer than two of its points survive the validity
    /// gate and the privacy trim: a single point is a pin, and a ride that
    /// never leaves its own doorstep cannot be drawn without publishing it.
    TooShort,
}

impl RouteTrackError {
    /// The stable slug a client and a stored row carry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoGps => "no_gps",
            Self::TooShort => "too_short",
        }
    }

    /// The reason a slug names, or `None` for a slug no reason carries.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "no_gps" => Some(Self::NoGps),
            "too_short" => Some(Self::TooShort),
            _ => None,
        }
    }
}

impl fmt::Display for RouteTrackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoGps => "the activity has no recorded GPS track, so no map can be drawn",
            Self::TooShort => {
                "the activity's track is too short, or too close to where it started, to draw \
                 without publishing that address"
            }
        })
    }
}

impl Error for RouteTrackError {}

/// The corners of a drawn track, so a client frames the map without walking
/// every point first.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RouteBounds {
    /// Southernmost latitude on the track, in degrees.
    pub min_latitude: f64,
    /// Northernmost latitude on the track, in degrees.
    pub max_latitude: f64,
    /// Westernmost longitude on the track, in degrees.
    pub min_longitude: f64,
    /// Easternmost longitude on the track, in degrees.
    pub max_longitude: f64,
}

/// One sustained ascent along a track.
///
/// The indices address [`RouteTrack::coordinates`], so a client marks the
/// climb by slicing the line it already drew.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteClimb {
    /// First coordinate of the climb.
    pub start_index: usize,
    /// Last coordinate of the climb, inclusive.
    pub end_index: usize,
    /// Average gradient in percent — 5.4 means 5.4 %.
    ///
    /// The compute crate reports a fraction; the conversion happens here
    /// because every consumer of this struct renders a percentage, and a
    /// fraction reaching a `%` template prints a 5.4 % climb as 0.1 %.
    pub avg_gradient: f64,
    /// Strava-style grade as the athlete reads it: `HC`, or `1` through `4`.
    ///
    /// `None` for an ascent below the category threshold, so a client omits
    /// the label rather than captioning it "Cat none". The compute crate's
    /// enum is the authority on which grade a climb earns; this is only how
    /// that grade is spelled for display.
    pub category: Option<String>,
}

/// One activity's recorded track, ready to be drawn.
///
/// Every series here is either absent or exactly as long as `coordinates`:
/// they are built from the same filtered pass and sliced by the same privacy
/// trim, so a client can index one by the other without checking. A padded
/// series would put a climb marker on the wrong kilometre, which is worse than
/// no marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteTrack {
    /// `(latitude, longitude)` in degrees, in recorded order.
    pub coordinates: Vec<(f64, f64)>,
    /// Corners of `coordinates`.
    pub bounds: RouteBounds,
    /// Altitude per coordinate. Absent when the provider recorded none, which
    /// is also when `climbs` is empty — a climb needs the vertical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_meters: Option<Vec<f64>>,
    /// Distance into the *recorded* ride at each coordinate, in metres. The
    /// first entry is where the drawn line picks the ride up, which is the
    /// privacy trim's radius rather than zero. Absent for a track built from a
    /// route overview, which carries no distance stream to measure the ride
    /// along.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distances_meters: Option<Vec<f64>>,
    /// Sustained ascents along the track, in order.
    pub climbs: Vec<RouteClimb>,
}

impl RouteTrack {
    /// Build the drawable track from an activity's recorded streams.
    ///
    /// The line this returns is the ride minus its endpoint neighbourhoods:
    /// where a trace starts and stops is an address, and it is removed before
    /// the geometry exists as a track rather than at the surface that draws it.
    ///
    /// # Errors
    ///
    /// [`RouteTrackError::NoGps`] when the activity has no GPS channel, or an
    /// empty one; [`RouteTrackError::TooShort`] when fewer than two of its
    /// points survive the validity gate — a single point is a pin, not a
    /// route — or when the whole ride sits inside the privacy radius, where
    /// the only honest map is none.
    pub fn from_streams(streams: &TimeSeriesData) -> Result<Self, RouteTrackError> {
        let Some(recorded) = streams
            .gps_coordinates
            .as_deref()
            .filter(|recorded| !recorded.is_empty())
        else {
            return Err(RouteTrackError::NoGps);
        };
        let (recorded_points, altitudes) = paired_points(recorded, streams.altitude.as_deref());
        // The vertical rides on the pairing above: with an altitude channel
        // present a point is kept only when both halves are usable, so equal
        // lengths hold by construction and are what the index alignment means.
        let has_vertical = altitudes.len() == recorded_points.len();
        let recorded_elevations: Option<Vec<f64>> =
            has_vertical.then(|| altitudes.iter().copied().map(f64::from).collect());
        // Measured on the whole ride, then sliced with it: a surviving sample
        // keeps the distance it sat at in the real ride, so an elevation
        // profile still reads as "kilometres in" rather than restarting at the
        // trim.
        let recorded_distances = cumulative_distances(&recorded_points);
        Self::trimmed(
            &recorded_points,
            recorded_elevations.as_deref(),
            Some(&recorded_distances),
        )
    }

    /// Build the drawable track from a provider's route overview: the
    /// simplified line a list payload carries, such as a decoded Strava
    /// `summary_polyline`.
    ///
    /// An overview carries neither altitude nor distance, so the track has no
    /// vertical, no distance series and no climbs. The same validity gate and
    /// privacy trim as [`Self::from_streams`] apply.
    ///
    /// # Errors
    ///
    /// [`RouteTrackError::NoGps`] for an empty overview;
    /// [`RouteTrackError::TooShort`] when fewer than two points survive the
    /// gate and the trim.
    pub fn from_overview(points: &[(f64, f64)]) -> Result<Self, RouteTrackError> {
        if points.is_empty() {
            return Err(RouteTrackError::NoGps);
        }
        let (valid_points, _) = paired_points(points, None);
        Self::trimmed(&valid_points, None, None)
    }

    /// Trim the endpoint neighbourhoods off gated points and frame what is
    /// left, finding climbs on the trimmed line when it carries a vertical.
    fn trimmed(
        points: &[(f64, f64)],
        elevations: Option<&[f64]>,
        distances: Option<&[f64]>,
    ) -> Result<Self, RouteTrackError> {
        // The endpoints are the athlete's door. Trimmed here rather than in a
        // client because the geometry travels: the messaging path hands a
        // render URL to third-party servers, which fetch the whole payload.
        let trimmed =
            trim_route_endpoints(points, elevations, distances, DEFAULT_PRIVACY_RADIUS_METERS)
                .ok_or(RouteTrackError::TooShort)?;
        // Two survivors are guaranteed by the trim, so the corners come off the
        // same slice the map is drawn from.
        let bounds = bounds_of(&trimmed.coordinates).ok_or(RouteTrackError::TooShort)?;
        // Climbs are found on the trimmed line, not the recorded one, because
        // their indices address the coordinates this track carries.
        let climbs = trimmed
            .elevations
            .as_deref()
            .map_or_else(Vec::new, |elevations| {
                detect_climbs(&trimmed.coordinates, elevations)
            });
        Ok(Self {
            coordinates: trimmed.coordinates,
            bounds,
            elevation_meters: trimmed.elevations,
            distances_meters: trimmed.distances,
            climbs,
        })
    }

    /// The same track drawn with at most `max_points` coordinates.
    ///
    /// Points are kept in order of how far the line would move without them
    /// (Douglas–Peucker, refined greedily until the budget is spent), so the
    /// corners and switchbacks of a ride survive and the straight stretches
    /// thin out. The first and last points, and the two ends of every climb,
    /// are always kept, so each climb still starts and ends where it did;
    /// the parallel series are sliced to the kept points and the corners
    /// re-read from them. A track already within the budget is returned as
    /// it is.
    #[must_use]
    pub fn simplified(self, max_points: usize) -> Self {
        let len = self.coordinates.len();
        if len <= max_points.max(2) {
            return self;
        }
        let mut anchors = Vec::with_capacity(2 + self.climbs.len() * 2);
        anchors.extend([0, len - 1]);
        for climb in &self.climbs {
            anchors.extend([climb.start_index, climb.end_index]);
        }
        let kept = simplify_indices(&self.coordinates, &anchors, max_points);
        let pick = |series: &[f64]| -> Vec<f64> { kept.iter().map(|&i| series[i]).collect() };
        let coordinates: Vec<(f64, f64)> = kept.iter().map(|&i| self.coordinates[i]).collect();
        let climbs = self
            .climbs
            .iter()
            .filter_map(|climb| {
                Some(RouteClimb {
                    start_index: kept.binary_search(&climb.start_index).ok()?,
                    end_index: kept.binary_search(&climb.end_index).ok()?,
                    ..climb.clone()
                })
            })
            .collect();
        Self {
            bounds: bounds_of(&coordinates).unwrap_or(self.bounds),
            elevation_meters: self.elevation_meters.as_deref().map(pick),
            distances_meters: self.distances_meters.as_deref().map(pick),
            coordinates,
            climbs,
        }
    }
}

/// A route overview polyline with its endpoint neighbourhoods removed,
/// re-encoded at the same precision.
///
/// What a list of activities may carry to a client for a route sketch: the
/// line a provider sent, minus the door it starts and ends at. `None` when the
/// polyline is empty or malformed, or when too little of it survives the trim
/// to draw — the sketch is then absent rather than a stub.
#[must_use]
pub fn trimmed_overview_polyline(encoded: &str) -> Option<String> {
    let points = decode_polyline(encoded.trim())?;
    RouteTrack::from_overview(&points)
        .ok()
        .map(|track| encode_polyline(&track.coordinates))
}

/// Pair the GPS and altitude channels and keep the points both halves record.
///
/// Mirrors the gate the terrain analysis applies to the same two streams — the
/// shorter channel ends the track, and a point with a non-finite or
/// out-of-Earth value is dropped rather than carried into a haversine. The
/// returned altitudes are empty when the provider recorded none.
fn paired_points(
    recorded: &[(f64, f64)],
    altitudes: Option<&[f32]>,
) -> (Vec<(f64, f64)>, Vec<f32>) {
    let mut coordinates = Vec::with_capacity(recorded.len());
    let mut kept_altitudes = Vec::new();
    for (index, &(latitude, longitude)) in recorded.iter().enumerate() {
        let elevation = match altitudes.map(|series| series.get(index)) {
            // The altitude channel ran out first, so the track ends here.
            Some(None) => break,
            Some(Some(&sample)) => Some(sample),
            None => None,
        };
        if !latitude.is_finite() || !longitude.is_finite() {
            continue;
        }
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            continue;
        }
        if let Some(sample) = elevation {
            if !sample.is_finite() {
                continue;
            }
            kept_altitudes.push(sample);
        }
        coordinates.push((latitude, longitude));
    }
    (coordinates, kept_altitudes)
}

/// The corners of a track, or `None` for a track with no points.
fn bounds_of(coordinates: &[(f64, f64)]) -> Option<RouteBounds> {
    let (&(first_latitude, first_longitude), rest) = coordinates.split_first()?;
    let mut bounds = RouteBounds {
        min_latitude: first_latitude,
        max_latitude: first_latitude,
        min_longitude: first_longitude,
        max_longitude: first_longitude,
    };
    for &(latitude, longitude) in rest {
        bounds.min_latitude = bounds.min_latitude.min(latitude);
        bounds.max_latitude = bounds.max_latitude.max(latitude);
        bounds.min_longitude = bounds.min_longitude.min(longitude);
        bounds.max_longitude = bounds.max_longitude.max(longitude);
    }
    Some(bounds)
}

/// Distance from the start at each coordinate, measured along the track.
///
/// Through the compute crate's haversine rather than a local one, so "18 km
/// in" means the same number here as in the terrain analysis the same track
/// feeds.
fn cumulative_distances(coordinates: &[(f64, f64)]) -> Vec<f64> {
    let mut distances = Vec::with_capacity(coordinates.len());
    let mut travelled = 0.0;
    distances.push(travelled);
    for (&(from_latitude, from_longitude), &(to_latitude, to_longitude)) in
        coordinates.iter().zip(coordinates.iter().skip(1))
    {
        travelled +=
            haversine_meters_between(from_latitude, from_longitude, to_latitude, to_longitude);
        distances.push(travelled);
    }
    distances
}

/// The sustained ascents the terrain analysis finds in this track.
///
/// The analysis re-runs its own validity filter, so its indices only address
/// our coordinates when it kept every point we did. It does — both sides apply
/// the same gate — and the count check is what proves it on the day someone
/// changes one of them: a mismatch drops the marks and keeps the map, rather
/// than drawing a climb across the wrong kilometres.
fn detect_climbs(coordinates: &[(f64, f64)], elevations: &[f64]) -> Vec<RouteClimb> {
    // Narrowed back to the width the provider recorded them at: every value
    // here came from an `f32` altitude sample and was widened on the way in,
    // so this is the exact inverse and loses nothing.
    let altitudes: Vec<f32> = elevations.iter().map(|&metres| metres as f32).collect();
    let Some(summary) = build_route_summary_from_streams(coordinates, &altitudes) else {
        return Vec::new();
    };
    if summary.point_count != coordinates.len() {
        warn!(
            analysed = summary.point_count,
            carried = coordinates.len(),
            "route track: terrain analysis kept a different point count; drawing the route \
             without climb marks"
        );
        return Vec::new();
    }
    summary
        .climbs
        .into_iter()
        .map(|climb| RouteClimb {
            start_index: climb.start_index,
            end_index: climb.end_index,
            avg_gradient: climb.avg_gradient * 100.0,
            category: match climb.category {
                ClimbCategory::Hc => Some("HC".to_owned()),
                ClimbCategory::Cat1 => Some("1".to_owned()),
                ClimbCategory::Cat2 => Some("2".to_owned()),
                ClimbCategory::Cat3 => Some("3".to_owned()),
                ClimbCategory::Cat4 => Some("4".to_owned()),
                ClimbCategory::None => None,
            },
        })
        .collect()
}

/// One stretch of the line between two kept points, ranked by how far its
/// farthest interior point sits from the chord that would replace it.
#[derive(Debug, Clone, Copy)]
struct Stretch {
    /// Metres from the chord to the farthest interior point.
    deviation: f64,
    /// Index of that point.
    farthest: usize,
    /// First kept index of the stretch.
    start: usize,
    /// Last kept index of the stretch.
    end: usize,
}

impl PartialEq for Stretch {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Stretch {}

impl PartialOrd for Stretch {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Stretch {
    fn cmp(&self, other: &Self) -> Ordering {
        self.deviation
            .total_cmp(&other.deviation)
            // Equal deviations split the earlier stretch first, so the result
            // does not depend on the heap's internal order.
            .then_with(|| other.start.cmp(&self.start))
    }
}

/// The indices a simplified line keeps, ascending: every anchor, then the
/// point whose removal would move the line furthest, repeatedly, until
/// `max_points` are kept or every remaining point sits on its chord (within
/// [`MIN_DEVIATION_METERS`]).
fn simplify_indices(
    coordinates: &[(f64, f64)],
    anchors: &[usize],
    max_points: usize,
) -> Vec<usize> {
    let planar = project(coordinates);
    let mut kept: Vec<usize> = anchors
        .iter()
        .copied()
        .filter(|&i| i < coordinates.len())
        .collect();
    kept.sort_unstable();
    kept.dedup();
    let mut stretches: BinaryHeap<Stretch> = kept
        .windows(2)
        .filter_map(|pair| farthest_point(&planar, pair[0], pair[1]))
        .collect();
    while kept.len() < max_points {
        let Some(stretch) = stretches.pop() else {
            break;
        };
        if stretch.deviation < MIN_DEVIATION_METERS {
            break;
        }
        if let Err(position) = kept.binary_search(&stretch.farthest) {
            kept.insert(position, stretch.farthest);
        }
        stretches.extend(farthest_point(&planar, stretch.start, stretch.farthest));
        stretches.extend(farthest_point(&planar, stretch.farthest, stretch.end));
    }
    kept
}

/// Coordinates as planar metres around the track's mean latitude — an
/// equirectangular projection, which is exact enough at the scale of one ride
/// to rank which points matter.
fn project(coordinates: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let count = coordinates.len().max(1) as f64;
    let mean_latitude = coordinates.iter().map(|&(lat, _)| lat).sum::<f64>() / count;
    let metres_per_degree = EARTH_RADIUS_METERS.to_radians();
    let longitude_scale = metres_per_degree * mean_latitude.to_radians().cos();
    coordinates
        .iter()
        .map(|&(latitude, longitude)| (longitude * longitude_scale, latitude * metres_per_degree))
        .collect()
}

/// The interior point of `start..=end` farthest from the chord joining its
/// ends, or `None` when the stretch has no interior point.
fn farthest_point(planar: &[(f64, f64)], start: usize, end: usize) -> Option<Stretch> {
    let (a, b) = (planar[start], planar[end]);
    (start + 1..end)
        .map(|index| (index, distance_to_chord(planar[index], a, b)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(farthest, deviation)| Stretch {
            deviation,
            farthest,
            start,
            end,
        })
}

/// Metres from `point` to the segment `a`–`b` (to `a` itself when the two
/// ends coincide, as they do on a loop).
fn distance_to_chord(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_squared = dx.mul_add(dx, dy * dy);
    let along = if length_squared > 0.0 {
        ((point.0 - a.0).mul_add(dx, (point.1 - a.1) * dy) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let nearest = (along.mul_add(dx, a.0), along.mul_add(dy, a.1));
    (point.0 - nearest.0).hypot(point.1 - nearest.1)
}
