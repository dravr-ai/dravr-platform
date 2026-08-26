// ABOUTME: Raw Strava API response DTOs — the wire shapes the provider deserializes
// ABOUTME: Split out of strava_provider.rs so that file stays under the size ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava wire types.
//
// These mirror Strava's v3 JSON exactly — field names, optionality and all —
// so the provider module holds behaviour rather than shape. Each item keeps
// the visibility it had inside `strava_provider`; fields that were private to
// that module are `pub(crate)` so the provider can still read them.

use crate::models::PeriodTotals;
use serde::Deserialize;

/// Strava API error response format
#[derive(Debug, Deserialize)]
pub(crate) struct StravaErrorResponse {
    pub(crate) message: String,
    pub(crate) errors: Option<Vec<StravaError>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StravaError {
    pub(crate) resource: String,
    pub(crate) field: String,
    pub(crate) code: String,
}

/// Strava API response for athlete data
#[derive(Debug, Deserialize)]
pub(crate) struct StravaAthleteResponse {
    pub(crate) id: u64,
    pub(crate) username: Option<String>,
    pub(crate) firstname: Option<String>,
    pub(crate) lastname: Option<String>,
    pub(crate) profile_medium: Option<String>,
}

/// Strava map data in API responses
#[derive(Debug, Clone, Deserialize)]
pub struct StravaMap {
    /// Encoded polyline summary of the route
    pub summary_polyline: Option<String>,
}

/// Strava API response for activity data (summary endpoint)
#[derive(Debug, Clone, Deserialize)]
pub struct StravaActivityResponse {
    pub(crate) id: u64,
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) activity_type: String,
    /// Granular Strava sport type (e.g. `MountainBikeRide`, `GravelRide`).
    /// Introduced 2022 and preferred over the deprecated `type` field, which
    /// flattens every bike to `Ride`. Optional for backward compatibility with
    /// any response that omits it.
    pub(crate) sport_type: Option<String>,
    pub(crate) start_date: String,
    pub(crate) distance: Option<f32>,
    pub(crate) elapsed_time: Option<u32>,
    pub(crate) total_elevation_gain: Option<f32>,
    pub(crate) average_speed: Option<f32>,
    pub(crate) max_speed: Option<f32>,
    pub(crate) average_heartrate: Option<f32>,
    pub(crate) max_heartrate: Option<f32>,
    pub(crate) average_cadence: Option<f32>,
    pub(crate) average_watts: Option<f32>,
    pub(crate) max_watts: Option<f32>,
    /// Strava's "weighted average power" — its own name for Normalized Power,
    /// carried on both the summary and detailed activity payloads. Strava
    /// reports it only for rides recorded with a power meter; rides whose
    /// watts it estimated from speed and grade omit the field.
    pub(crate) weighted_average_watts: Option<f32>,
    pub(crate) suffer_score: Option<f32>,

    // Location and GPS data from summary endpoint
    pub(crate) start_latlng: Option<Vec<f64>>,
    pub(crate) location_city: Option<String>,
    pub(crate) location_state: Option<String>,
    pub(crate) location_country: Option<String>,

    // Additional performance metrics from summary endpoint
    pub(crate) calories: Option<f32>,
}

/// Strava split data from detailed activity endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct StravaSplit {
    /// Distance covered in this split (meters)
    pub distance: Option<f32>,
    /// Total elapsed time for the split (seconds)
    pub elapsed_time: Option<u32>,
    /// Elevation gain/loss in the split (meters)
    pub elevation_difference: Option<f32>,
    /// Time spent moving during the split (seconds)
    pub moving_time: Option<u32>,
    /// Split number (1-based index)
    pub split: Option<u32>,
    /// Average speed during the split (meters/second)
    pub average_speed: Option<f32>,
    /// Pace zone classification (0-5)
    pub pace_zone: Option<u32>,
}

/// Strava lap data from detailed activity endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct StravaLap {
    /// Unique identifier for this lap
    pub id: Option<u64>,
    /// Total elapsed time for the lap (seconds)
    pub elapsed_time: Option<u32>,
    /// Time spent moving during the lap (seconds)
    pub moving_time: Option<u32>,
    /// Distance covered in the lap (meters)
    pub distance: Option<f32>,
    /// Total elevation gain during the lap (meters)
    pub total_elevation_gain: Option<f32>,
    /// Average speed during the lap (meters/second)
    pub average_speed: Option<f32>,
    /// Maximum speed reached during the lap (meters/second)
    pub max_speed: Option<f32>,
    /// Average heart rate during the lap (bpm)
    pub average_heartrate: Option<f32>,
    /// Maximum heart rate during the lap (bpm)
    pub max_heartrate: Option<f32>,
    /// Average cadence during the lap (rpm/spm)
    pub average_cadence: Option<f32>,
    /// Average power output during the lap (watts)
    pub average_watts: Option<f32>,
}

/// Strava segment effort data from detailed activity endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct StravaSegmentEffort {
    /// Unique identifier for this segment effort
    pub id: Option<u64>,
    /// Name of the segment
    pub name: Option<String>,
    /// Total elapsed time for the segment (seconds)
    pub elapsed_time: Option<u32>,
    /// Time spent moving during the segment (seconds)
    pub moving_time: Option<u32>,
    /// Distance of the segment (meters)
    pub distance: Option<f32>,
    /// Average heart rate during the segment (bpm)
    pub average_heartrate: Option<f32>,
    /// Maximum heart rate during the segment (bpm)
    pub max_heartrate: Option<f32>,
    /// Average cadence during the segment (rpm/spm)
    pub average_cadence: Option<f32>,
    /// Average power output during the segment (watts)
    pub average_watts: Option<f32>,
}

/// Detailed activity response from GET /activities/{id} endpoint
/// Includes all summary fields plus additional detail-only fields like splits, laps, and segment efforts
#[derive(Debug, Clone, Deserialize)]
pub struct DetailedActivityResponse {
    /// All summary-level activity fields (flattened)
    #[serde(flatten)]
    pub summary: StravaActivityResponse,

    // Social and engagement data
    /// Number of kudos received
    pub kudos_count: Option<u32>,
    /// Number of comments
    pub comment_count: Option<u32>,
    /// Number of athletes who participated
    pub athlete_count: Option<u32>,
    /// Number of photos attached
    pub photo_count: Option<u32>,
    /// Number of achievements earned
    pub achievement_count: Option<u32>,

    // Additional elevation data
    /// Highest elevation point (meters)
    pub elev_high: Option<f32>,
    /// Lowest elevation point (meters)
    pub elev_low: Option<f32>,

    // Performance metrics
    /// Number of personal records achieved
    pub pr_count: Option<u32>,
    /// Name of the recording device
    pub device_name: Option<String>,

    // Complex nested data
    /// Metric splits (1km or 1mi intervals)
    pub splits_metric: Option<Vec<StravaSplit>>,
    /// Lap data from the activity
    pub laps: Option<Vec<StravaLap>>,
    /// Segment efforts completed during the activity
    pub segment_efforts: Option<Vec<StravaSegmentEffort>>,
}

/// Strava API response for stats
///
/// The `/athletes/{id}/stats` endpoint returns parallel `all_*` (lifetime) and
/// `ytd_*` (current calendar year) totals per sport. We deserialize both so the
/// stats tool can report annual figures distinctly from lifetime ones.
#[derive(Debug, Deserialize)]
pub(crate) struct StravaStatsResponse {
    #[serde(rename = "all_ride_totals")]
    pub(crate) all_ride: Option<StravaTotals>,
    #[serde(rename = "all_run_totals")]
    pub(crate) all_run: Option<StravaTotals>,
    #[serde(rename = "ytd_ride_totals")]
    pub(crate) ytd_ride: Option<StravaTotals>,
    #[serde(rename = "ytd_run_totals")]
    pub(crate) ytd_run: Option<StravaTotals>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StravaTotals {
    pub(crate) count: u32,
    pub(crate) distance: f32,
    pub(crate) moving_time: u32,
    pub(crate) elevation_gain: f32,
}

impl StravaStatsResponse {
    /// Combine a ride+run total pair into the canonical [`PeriodTotals`].
    ///
    /// Swim totals (`*_swim_totals`) are intentionally excluded to match the
    /// historical behaviour of the lifetime sum; including swims is tracked as
    /// a separate change.
    pub(crate) fn sum_pair(
        ride: Option<&StravaTotals>,
        run: Option<&StravaTotals>,
    ) -> PeriodTotals {
        PeriodTotals {
            total_activities: u64::from(ride.map_or(0, |t| t.count) + run.map_or(0, |t| t.count)),
            total_distance: f64::from(
                ride.map_or(0.0, |t| t.distance) + run.map_or(0.0, |t| t.distance),
            ),
            total_duration: u64::from(
                ride.map_or(0, |t| t.moving_time) + run.map_or(0, |t| t.moving_time),
            ),
            total_elevation_gain: f64::from(
                ride.map_or(0.0, |t| t.elevation_gain) + run.map_or(0.0, |t| t.elevation_gain),
            ),
        }
    }
}
