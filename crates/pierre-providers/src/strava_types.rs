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
use chrono::{DateTime, Utc};
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
    /// Strava's machine-readable error code (e.g. `invalid`), not a credential.
    #[serde(rename = "code")]
    pub(crate) error_code: String,
}

/// Strava API response for athlete data
#[derive(Debug, Deserialize)]
pub(crate) struct StravaAthleteResponse {
    pub(crate) id: u64,
    pub(crate) username: Option<String>,
    pub(crate) firstname: Option<String>,
    pub(crate) lastname: Option<String>,
    pub(crate) profile_medium: Option<String>,
    /// The athlete's sex as Strava's athlete profile states it: `M`, `F`, or
    /// absent when they gave none.
    #[serde(default)]
    pub(crate) sex: Option<String>,
    /// When the account was created: the floor of the athlete's history,
    /// since nothing is recorded on Strava before it.
    #[serde(default)]
    pub(crate) created_at: Option<DateTime<Utc>>,
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
    /// Seconds east of UTC on the clock the activity was recorded on. Strava
    /// sends it on the list and detail payloads alike, as a JSON number.
    pub(crate) utc_offset: Option<f64>,
    pub(crate) distance: Option<f32>,
    pub(crate) elapsed_time: Option<u32>,
    /// Seconds spent moving: `elapsed_time` minus the stops Strava
    /// subtracted. A long gap between the two is the athlete stopped or
    /// doing something other than the sport, which elapsed time alone hides.
    pub(crate) moving_time: Option<u32>,
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
    /// Strava's workout tag: for runs 0 default, 1 race, 2 long run,
    /// 3 workout; for rides 10 default, 11 race, 12 workout.
    pub(crate) workout_type: Option<u32>,
    /// Recorded on an indoor trainer or treadmill.
    pub(crate) trainer: Option<bool>,
    /// The athlete flagged the activity as a commute.
    pub(crate) commute: Option<bool>,

    // Location and GPS data from summary endpoint
    pub(crate) start_latlng: Option<Vec<f64>>,
    pub(crate) location_city: Option<String>,
    pub(crate) location_state: Option<String>,
    pub(crate) location_country: Option<String>,

    // Additional performance metrics from summary endpoint
    pub(crate) calories: Option<f32>,

    /// Route overview. Strava sends it on the list and detail payloads alike,
    /// with an empty `summary_polyline` for an activity recorded without GPS.
    pub(crate) map: Option<StravaMap>,
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

/// Strava segment effort: one entry of a detailed activity's
/// `segment_efforts`, and one row of `GET /segment_efforts`.
#[derive(Debug, Clone, Deserialize)]
pub struct StravaSegmentEffort {
    /// Unique identifier for this segment effort
    pub id: Option<u64>,
    /// Name of the segment
    pub name: Option<String>,
    /// When the effort started (UTC, RFC 3339)
    #[serde(default)]
    pub start_date: Option<String>,
    /// The athlete's rank of this effort among their own on the segment
    /// (1 = best), when it is one of their top three
    #[serde(default)]
    pub pr_rank: Option<u32>,
    /// The effort's overall rank on the segment's leaderboard, when it is in
    /// the top ten
    #[serde(default)]
    pub kom_rank: Option<u32>,
    /// The segment the effort was ridden or run on
    #[serde(default)]
    pub segment: Option<StravaSummarySegment>,
    /// The activity the effort belongs to; `GET /segment_efforts` names it,
    /// a detailed activity's own efforts name the activity itself
    #[serde(default)]
    pub activity: Option<StravaMetaActivity>,
    /// The activity's id where Strava names it flat (`activity_id`), as its
    /// documented `DetailedSegmentEffort` also does; read when `activity`
    /// names none
    #[serde(default)]
    pub activity_id: Option<u64>,
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

/// Strava's summary segment: the segment an effort was on, shared by every
/// effort over it.
#[derive(Debug, Clone, Deserialize)]
pub struct StravaSummarySegment {
    /// The segment's id, which groups one athlete's efforts on it
    pub id: Option<u64>,
    /// The segment's name, written by whoever created it. Untrusted text.
    pub name: Option<String>,
    /// The segment's length (meters)
    pub distance: Option<f64>,
    /// The segment's average grade (percent)
    pub average_grade: Option<f32>,
    /// Strava's climb category, 0 (uncategorized) to 5 (hors catégorie)
    pub climb_category: Option<i32>,
    /// The sport the segment is for (`Ride`, `Run`)
    pub activity_type: Option<String>,
}

/// Strava's meta activity: an activity named by its id alone.
#[derive(Debug, Clone, Deserialize)]
pub struct StravaMetaActivity {
    /// The activity's id
    pub id: Option<u64>,
}

/// The gear a detailed activity names: Strava's summary gear shape.
#[derive(Debug, Clone, Deserialize)]
pub struct StravaGear {
    /// Strava's gear id (`b12345` for a bike, `g12345` for shoes)
    pub id: Option<String>,
    /// The gear's display name as the athlete set it
    pub name: Option<String>,
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
    /// The athlete's free-text notes on the activity. Untrusted text.
    pub description: Option<String>,
    /// The athlete's private note, which only the athlete sees on Strava.
    /// Untrusted, and the athlete's alone.
    #[serde(default)]
    pub private_note: Option<String>,
    /// The gear the activity was recorded with (`{ id, name, … }`).
    #[serde(default)]
    pub gear: Option<StravaGear>,
    /// The athlete's Perceived Exertion, the 1–10 slider value, `null` when
    /// unset. Undocumented by Strava and absent from the list endpoint, so
    /// only a detail read carries it (see `strava_self_report`).
    pub perceived_exertion: Option<f32>,

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

/// One stream from `GET /activities/{id}/streams?key_by_type=true`.
///
/// Numeric samples arrive as JSON numbers with occasional `null`s (a sensor
/// dropout, a paused power meter); `latlng` arrives as `[lat, lng]` pairs,
/// also nullable during GPS dropouts.
#[derive(Debug, Deserialize)]
pub struct StravaStream<T> {
    /// The per-sample values, index-aligned with the `time` stream.
    pub data: Vec<Option<T>>,
}

/// The keyed stream set for one activity (`key_by_type=true`).
///
/// Every stream is optional: Strava omits the key entirely when the activity
/// recorded no such channel.
#[derive(Debug, Default, Deserialize)]
pub struct StravaStreamSet {
    /// Seconds from activity start, per sample.
    pub time: Option<StravaStream<u32>>,
    /// Heart rate, bpm.
    pub heartrate: Option<StravaStream<u32>>,
    /// Power, watts.
    pub watts: Option<StravaStream<u32>>,
    /// Cadence, rpm.
    pub cadence: Option<StravaStream<u32>>,
    /// Smoothed speed, m/s.
    pub velocity_smooth: Option<StravaStream<f32>>,
    /// Altitude, metres.
    pub altitude: Option<StravaStream<f32>>,
    /// Temperature, Celsius.
    pub temp: Option<StravaStream<f32>>,
    /// Distance covered since the activity start, metres (cumulative).
    pub distance: Option<StravaStream<f64>>,
    /// GPS positions as `[lat, lng]`.
    pub latlng: Option<StravaStream<[f64; 2]>>,
}

#[cfg(test)]
mod athlete_sex_tests {
    use super::StravaAthleteResponse;
    use pierre_core::models::PhysiologicalSex;

    fn sex_of(body: &str) -> Option<PhysiologicalSex> {
        serde_json::from_str::<StravaAthleteResponse>(body)
            .ok()
            .and_then(|athlete| athlete.sex)
            .as_deref()
            .and_then(PhysiologicalSex::from_provider_code)
    }

    #[test]
    fn strava_reports_sex_as_m_or_f_and_may_omit_it() {
        assert_eq!(
            sex_of(r#"{"id":1,"sex":"F"}"#),
            Some(PhysiologicalSex::Female)
        );
        assert_eq!(
            sex_of(r#"{"id":1,"sex":"M"}"#),
            Some(PhysiologicalSex::Male)
        );
        assert_eq!(sex_of(r#"{"id":1,"sex":null}"#), None);
        assert_eq!(sex_of(r#"{"id":1}"#), None);
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{StravaAthleteResponse, StravaErrorResponse};

    #[test]
    fn the_strava_athlete_reads_when_its_account_was_created() {
        let athlete: StravaAthleteResponse = serde_json::from_str(
            r#"{"id":1,"username":"runner","created_at":"2014-06-02T18:12:00Z"}"#,
        )
        .unwrap();
        assert_eq!(
            athlete.created_at,
            Some(Utc.with_ymd_and_hms(2014, 6, 2, 18, 12, 0).unwrap()),
            "bug: the athlete's created_at decodes from Strava's /athlete"
        );
        let without: StravaAthleteResponse = serde_json::from_str(r#"{"id":1}"#).unwrap();
        assert_eq!(
            without.created_at, None,
            "bug: an athlete with no created_at decodes as unknown"
        );
    }

    #[test]
    fn a_strava_error_reads_its_code_from_the_wire_code_field() {
        let response: StravaErrorResponse = serde_json::from_str(
            r#"{"message":"Bad Request","errors":[{"resource":"Activity","field":"id","code":"invalid"}]}"#,
        )
        .unwrap();
        let error = response.errors.unwrap().into_iter().next().unwrap();
        assert_eq!(error.error_code, "invalid", "Strava's `code` field decodes");
    }
}
