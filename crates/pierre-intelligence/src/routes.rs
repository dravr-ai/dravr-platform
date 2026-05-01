// ABOUTME: Endurance Phase 3 GPX terrain classification + climb detection — pure compute over GPX track points
// ABOUTME: Outputs RouteSummary (terrain mix + climbs); no IO; powers GET /api/v1/endurance/routes/{activity_id}
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Endurance routes (GPX)
//!
//! Lightweight GPX parser + terrain classifier. The Endurance Phase 3
//! payload has two top-level slots:
//!
//! - **`terrain`** — share of the route in each terrain bucket
//!   (`flat` < 1 % gradient, `rolling` 1-4 %, `climb` 4-8 %, `steep` ≥ 8 %)
//!   measured by horizontal distance. Descents are bucketed by absolute
//!   gradient on the way back down.
//! - **`climbs`** — distinct climb segments, each with start/end indices,
//!   gradient, length, vertical gain, and a Strava-style climb category
//!   (`hc` / `1` / `2` / `3` / `4` / `none`) derived from
//!   `(elevation_gain_m × avg_gradient × 100)`.
//!
//! GPX parsing is intentionally minimal — we look for `<trkpt lat="..."
//! lon="..."><ele>X</ele></trkpt>` blocks, ignoring everything else.
//! Real-world GPX files from Strava / Garmin / Coros consistently emit
//! that shape; coros exports also include `<time>` which we skip.

use std::str;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Earth's mean radius in metres, used by the haversine distance.
const EARTH_RADIUS_METERS: f64 = 6_371_000.0;

/// Default minimum climb length in metres.
const MIN_CLIMB_LENGTH_METERS: f64 = 250.0;
/// Default minimum sustained gradient for a climb segment (1.5 %).
const MIN_CLIMB_GRADIENT_PCT: f64 = 1.5;
/// Smoothing window for the per-point gradient (samples on each side).
const GRADIENT_SMOOTHING_WINDOW: usize = 2;

/// Lower bound on horizontal delta between consecutive points (metres).
/// Pairs closer than this are treated as duplicate timestamps and contribute
/// `gradient = 0` instead of being amplified through `f64::EPSILON`.
const MIN_VALID_DELTA_METERS: f64 = 0.5;

/// Track point parsed from a GPX file.
#[derive(Debug, Clone, Copy)]
struct TrackPoint {
    lat: f64,
    lon: f64,
    elevation_meters: f64,
}

/// Per-point segment with cumulative distance + smoothed gradient.
#[derive(Debug, Clone, Copy)]
struct GradientSample {
    /// Horizontal distance from the previous point (m).
    delta_meters: f64,
    /// Cumulative distance from the start (m).
    cumulative_meters: f64,
    /// Smoothed gradient as a fraction (e.g. 0.045 for 4.5 %).
    gradient: f64,
}

/// Endurance terrain mix bucketed by horizontal distance.
///
/// The four buckets sum (within rounding) to `total_distance_meters`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TerrainSummary {
    /// Total horizontal distance analysed (m).
    pub total_distance_meters: f64,
    /// Distance with absolute gradient < 1 % (m).
    pub flat_meters: f64,
    /// Distance with absolute gradient in [1 %, 4 %) (m).
    pub rolling_meters: f64,
    /// Distance with absolute gradient in [4 %, 8 %) (m).
    pub climb_meters: f64,
    /// Distance with absolute gradient ≥ 8 % (m).
    pub steep_meters: f64,
    /// Total elevation gain (positive deltas only) in metres.
    pub elevation_gain_meters: f64,
    /// Total elevation loss (negative deltas summed as positives) in metres.
    pub elevation_loss_meters: f64,
}

/// Strava climb category derived from the Fiets-style index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClimbCategory {
    /// Hors catégorie — `score >= 80`.
    Hc,
    /// Category 1 — `score in [50, 80)`.
    Cat1,
    /// Category 2 — `score in [20, 50)`.
    Cat2,
    /// Category 3 — `score in [10, 20)`.
    Cat3,
    /// Category 4 — `score in [5, 10)`.
    Cat4,
    /// Below the climb-category threshold — surfaced for transparency.
    None,
}

/// One climb segment found inside the route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Climb {
    /// 0-based index of the first track point in the climb.
    pub start_index: usize,
    /// 0-based index of the last track point in the climb (inclusive).
    pub end_index: usize,
    /// Horizontal length in metres.
    pub length_meters: f64,
    /// Total elevation gain over the climb (m).
    pub elevation_gain_meters: f64,
    /// Average gradient of the climb (fraction; 0.054 = 5.4 %).
    pub avg_gradient: f64,
    /// Strava-style climb category (Fiets-style `gain * gradient * 100`).
    pub category: ClimbCategory,
}

/// Endurance `routes.json` payload for a single activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSummary {
    /// SHA-256 hex of the input GPX bytes — used as cache key.
    pub gpx_hash: String,
    /// Number of track points consumed.
    pub point_count: usize,
    /// Terrain distribution.
    pub terrain: TerrainSummary,
    /// Distinct climb segments in chronological order.
    pub climbs: Vec<Climb>,
}

impl RouteSummary {
    /// Returns `Some(self)` when every metric is finite, `None` otherwise.
    ///
    /// The Endurance cache table stores the JSON of this struct keyed by
    /// `gpx_hash`; once a non-finite value sneaks past parse it would
    /// poison the row and every subsequent read on the same activity. We
    /// drop the whole summary instead of writing a partial-but-bad cache
    /// entry — the handler will return an HTTP 4xx rather than a 200 with
    /// `null` / `NaN` in the JSON.
    #[must_use]
    fn validate(self) -> Option<Self> {
        let t = &self.terrain;
        let totals_finite = [
            t.total_distance_meters,
            t.flat_meters,
            t.rolling_meters,
            t.climb_meters,
            t.steep_meters,
            t.elevation_gain_meters,
            t.elevation_loss_meters,
        ]
        .iter()
        .all(|v| v.is_finite());
        let climbs_finite = self.climbs.iter().all(|c| {
            [c.length_meters, c.elevation_gain_meters, c.avg_gradient]
                .iter()
                .all(|v| v.is_finite())
        });
        if totals_finite && climbs_finite {
            Some(self)
        } else {
            None
        }
    }
}

/// Parse a GPX byte slice into the Endurance route summary.
///
/// Returns `None` when the input contains fewer than two valid track
/// points (`<trkpt lat="" lon=""><ele>X</ele>` triples), when every point
/// is filtered out by the validity checks (NaN / infinity / lat-lon
/// out of Earth range), or when the produced summary contains a
/// non-finite metric. The "everything-or-nothing" stance keeps
/// downstream consumers (cache, JSON encoder) from ever seeing a
/// poisoned row.
#[must_use]
pub fn build_route_summary(gpx_bytes: &[u8]) -> Option<RouteSummary> {
    let points = parse_trackpoints(gpx_bytes);
    if points.len() < 2 {
        return None;
    }
    let gradients = compute_gradients(&points);
    let terrain = build_terrain(&points, &gradients);
    let climbs = detect_climbs(&points, &gradients);
    let mut hasher = Sha256::new();
    hasher.update(gpx_bytes);
    let gpx_hash = hex::encode(hasher.finalize());
    let summary = RouteSummary {
        gpx_hash,
        point_count: points.len(),
        terrain,
        climbs,
    };
    summary.validate()
}

/// Build a Endurance route summary directly from a GPS+altitude time series.
///
/// Convenience for activities whose provider streams lat / lon / altitude
/// per second instead of exposing a downloadable GPX file.
///
/// `gps_coordinates` and `altitudes` must be aligned (same length); when
/// they are not, the shorter slice's length wins. Returns `None` when:
/// - fewer than two paired points are available,
/// - every paired point fails the validity check (NaN / infinity, lat
///   outside `[-90, 90]`, lon outside `[-180, 180]`), or
/// - the produced summary contains a non-finite metric — protects the
///   `route_summaries` cache from being poisoned with malformed data.
#[must_use]
pub fn build_route_summary_from_streams(
    gps_coordinates: &[(f64, f64)],
    altitudes: &[f32],
) -> Option<RouteSummary> {
    let n = gps_coordinates.len().min(altitudes.len());
    if n < 2 {
        return None;
    }
    let points: Vec<TrackPoint> = (0..n)
        .filter_map(|idx| {
            let (lat, lon) = gps_coordinates[idx];
            let elevation_meters = f64::from(altitudes[idx]);
            valid_point(lat, lon, elevation_meters)
        })
        .collect();
    if points.len() < 2 {
        return None;
    }
    let gradients = compute_gradients(&points);
    let terrain = build_terrain(&points, &gradients);
    let climbs = detect_climbs(&points, &gradients);
    // Hash the synthesised "GPX" — pack the floats into stable bytes so
    // (`tenant_id`, `user_id`, `activity_id`, `gpx_hash`) cache hits remain
    // deterministic across repeated computes.
    let mut hasher = Sha256::new();
    for p in &points {
        hasher.update(p.lat.to_le_bytes());
        hasher.update(p.lon.to_le_bytes());
        hasher.update(p.elevation_meters.to_le_bytes());
    }
    let gpx_hash = hex::encode(hasher.finalize());
    let summary = RouteSummary {
        gpx_hash,
        point_count: points.len(),
        terrain,
        climbs,
    };
    summary.validate()
}

/// Returns a [`TrackPoint`] only when every coordinate is finite and within
/// Earth's lat / lon range. Out-of-range or non-finite values would
/// produce non-finite haversine distances downstream, so we drop the
/// point at parse time rather than amplifying it through the
/// gradient pipeline.
fn valid_point(lat: f64, lon: f64, elevation_meters: f64) -> Option<TrackPoint> {
    if !lat.is_finite() || !lon.is_finite() || !elevation_meters.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some(TrackPoint {
        lat,
        lon,
        elevation_meters,
    })
}

fn parse_trackpoints(bytes: &[u8]) -> Vec<TrackPoint> {
    let Ok(xml) = str::from_utf8(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(start) = xml[cursor..].find("<trkpt") {
        let absolute = cursor + start;
        let Some(end_tag) = xml[absolute..].find('>') else {
            break;
        };
        let header = &xml[absolute..absolute + end_tag];
        let lat = parse_attr(header, "lat");
        let lon = parse_attr(header, "lon");
        let close = xml[absolute..]
            .find("</trkpt>")
            .map_or(xml.len() - absolute, |c| c);
        let body = &xml[absolute..absolute + close];
        let elevation = parse_inner_tag(body, "ele");
        cursor = absolute + close + "</trkpt>".len();
        if let (Some(lat), Some(lon), Some(ele)) = (lat, lon, elevation) {
            if let Some(point) = valid_point(lat, lon, ele) {
                out.push(point);
            }
        }
    }
    out
}

fn parse_attr(header: &str, attr: &str) -> Option<f64> {
    let needle = format!("{attr}=\"");
    let start = header.find(&needle)? + needle.len();
    let end = header[start..].find('"')? + start;
    let parsed: f64 = header[start..end].parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

fn parse_inner_tag(body: &str, tag: &str) -> Option<f64> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)? + start;
    let parsed: f64 = body[start..end].trim().parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

fn haversine_meters(a: TrackPoint, b: TrackPoint) -> f64 {
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = (b.lon - a.lon).to_radians();
    let half_lat = (dlat / 2.0).sin();
    let half_lon = (dlon / 2.0).sin();
    let h = (lat1.cos() * lat2.cos()).mul_add(half_lon.powi(2), half_lat.powi(2));
    let c = 2.0 * h.sqrt().asin();
    EARTH_RADIUS_METERS * c
}

fn compute_gradients(points: &[TrackPoint]) -> Vec<GradientSample> {
    let mut raw: Vec<GradientSample> = Vec::with_capacity(points.len());
    let mut cumulative = 0.0;
    raw.push(GradientSample {
        delta_meters: 0.0,
        cumulative_meters: 0.0,
        gradient: 0.0,
    });
    for window in points.windows(2) {
        let raw_delta = haversine_meters(window[0], window[1]);
        let elevation_change = window[1].elevation_meters - window[0].elevation_meters;
        // Duplicate-timestamp guard: when consecutive points are at the same
        // (or near-same) coordinate the haversine returns ~0; dividing
        // elevation_change by `f64::EPSILON` would otherwise amplify a
        // benign duplicate into a 1e16 gradient and miscategorise it as
        // steep. Treat sub-half-metre deltas as flat (0 gradient) and
        // exclude them from cumulative distance entirely.
        let (delta, gradient) = if raw_delta < MIN_VALID_DELTA_METERS || !raw_delta.is_finite() {
            (0.0, 0.0)
        } else {
            (raw_delta, elevation_change / raw_delta)
        };
        cumulative += delta;
        raw.push(GradientSample {
            delta_meters: delta,
            cumulative_meters: cumulative,
            gradient,
        });
    }
    smooth_gradients(&raw)
}

#[allow(clippy::cast_precision_loss)]
fn smooth_gradients(raw: &[GradientSample]) -> Vec<GradientSample> {
    let n = raw.len();
    let mut out: Vec<GradientSample> = Vec::with_capacity(n);
    for (idx, sample) in raw.iter().enumerate() {
        let lo = idx.saturating_sub(GRADIENT_SMOOTHING_WINDOW);
        let hi = (idx + GRADIENT_SMOOTHING_WINDOW + 1).min(n);
        let count = hi - lo;
        let sum: f64 = raw[lo..hi].iter().map(|s| s.gradient).sum();
        let mean = if count == 0 { 0.0 } else { sum / count as f64 };
        out.push(GradientSample {
            delta_meters: sample.delta_meters,
            cumulative_meters: sample.cumulative_meters,
            gradient: mean,
        });
    }
    out
}

fn build_terrain(points: &[TrackPoint], gradients: &[GradientSample]) -> TerrainSummary {
    let mut terrain = TerrainSummary::default();
    for (idx, sample) in gradients.iter().enumerate().skip(1) {
        let abs_grad = sample.gradient.abs();
        terrain.total_distance_meters += sample.delta_meters;
        if abs_grad < 0.01 {
            terrain.flat_meters += sample.delta_meters;
        } else if abs_grad < 0.04 {
            terrain.rolling_meters += sample.delta_meters;
        } else if abs_grad < 0.08 {
            terrain.climb_meters += sample.delta_meters;
        } else {
            terrain.steep_meters += sample.delta_meters;
        }
        let elev_change = points[idx].elevation_meters - points[idx - 1].elevation_meters;
        if elev_change > 0.0 {
            terrain.elevation_gain_meters += elev_change;
        } else {
            terrain.elevation_loss_meters += -elev_change;
        }
    }
    terrain
}

fn detect_climbs(points: &[TrackPoint], gradients: &[GradientSample]) -> Vec<Climb> {
    let mut climbs = Vec::new();
    let mut current_start: Option<usize> = None;
    let threshold = MIN_CLIMB_GRADIENT_PCT / 100.0;
    for (idx, sample) in gradients.iter().enumerate() {
        if sample.gradient >= threshold {
            current_start.get_or_insert(idx);
        } else if let Some(start) = current_start.take() {
            push_climb(&mut climbs, points, gradients, start, idx.saturating_sub(1));
        }
    }
    if let Some(start) = current_start {
        push_climb(&mut climbs, points, gradients, start, gradients.len() - 1);
    }
    climbs
}

fn push_climb(
    climbs: &mut Vec<Climb>,
    points: &[TrackPoint],
    gradients: &[GradientSample],
    start: usize,
    end: usize,
) {
    if end <= start {
        return;
    }
    let length = gradients[start..=end]
        .iter()
        .map(|s| s.delta_meters)
        .sum::<f64>();
    if length < MIN_CLIMB_LENGTH_METERS {
        return;
    }
    let elevation_gain = points[end].elevation_meters - points[start].elevation_meters;
    if elevation_gain <= 0.0 {
        return;
    }
    let avg_gradient = elevation_gain / length;
    climbs.push(Climb {
        start_index: start,
        end_index: end,
        length_meters: length,
        elevation_gain_meters: elevation_gain,
        avg_gradient,
        category: classify_climb(elevation_gain, avg_gradient),
    });
}

fn classify_climb(elevation_gain_meters: f64, avg_gradient: f64) -> ClimbCategory {
    // Fiets-style score scaled to Strava category bands.
    let score = elevation_gain_meters * avg_gradient * 100.0;
    if score >= 80.0 {
        ClimbCategory::Hc
    } else if score >= 50.0 {
        ClimbCategory::Cat1
    } else if score >= 20.0 {
        ClimbCategory::Cat2
    } else if score >= 10.0 {
        ClimbCategory::Cat3
    } else if score >= 5.0 {
        ClimbCategory::Cat4
    } else {
        ClimbCategory::None
    }
}
