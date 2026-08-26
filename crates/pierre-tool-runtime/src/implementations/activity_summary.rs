// ABOUTME: ActivitySummary — the scalar per-activity shape the coach model reads
// ABOUTME: Split out of fitness_support.rs so that file stays within its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach-facing activity DTO.
//
// `mode=summary` renders a list of these. The shape is the contract the model
// reasons over, so a sensor the provider reports but this struct omits is
// invisible to the coach no matter how faithfully it was fetched.

use pierre_core::models::{Activity, SportType, ZoneDistribution};
use serde::Serialize;

/// Activity summary with scalar sensor fields for efficient list queries.
///
/// Used when `mode=summary`. Carries the full set of scalar fields every
/// coach persona needs for basic reasoning (HR zones, elevation load,
/// calorie estimate, cadence, power) without the arrays (splits, laps,
/// segments, HR zones, power zones, time-series data) that only a deep
/// per-activity analysis coach needs. All sensor fields are `Option<T>`
/// and `#[serde(skip_serializing_if = "Option::is_none")]` so activities
/// recorded without an HRM or on indoor trainers render cleanly without
/// null noise.
#[derive(Debug, Clone, Serialize)]
pub struct ActivitySummary {
    /// Unique activity identifier
    pub id: String,
    /// Activity name/title
    pub name: String,
    /// Activity sport type (e.g., "run", "ride", "cross\_country\_skiing")
    pub sport_type: SportType,
    /// Start date/time in ISO 8601 format (UTC). Kept UTC so day-windowing,
    /// sorting, and fragment detection stay timezone-stable.
    pub start_date: String,
    /// Start time rendered in the user's local IANA timezone (RFC3339 with
    /// offset, e.g. `2026-05-29T08:36:07-04:00`), when the user has a timezone
    /// on file. This is the field to DISPLAY to the user — `start_date` is the
    /// raw UTC instant. `None` when the user has no timezone configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date_local: Option<String>,
    /// Distance in meters (0.0 if not available)
    pub distance_meters: f64,
    /// Duration in seconds
    pub duration_seconds: u64,
    /// Total elevation gained in meters, when the provider reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_gain_meters: Option<f64>,
    /// Average heart rate in BPM over the activity, when the user wore an HRM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_heart_rate: Option<u32>,
    /// Maximum heart rate in BPM over the activity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_heart_rate: Option<u32>,
    /// Provider-reported calorie estimate, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calories: Option<u32>,
    /// Average cadence (rpm for cycling, spm for running).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_cadence: Option<u32>,
    /// Average power output in watts (cycling / rowing / running power meters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_power: Option<u32>,
    /// Normalized Power in watts, as the provider computed it over the ride's
    /// own power samples — Strava calls this "weighted average power",
    /// intervals.icu reports it under the same name. `None` for activities
    /// recorded without a power meter, and for providers that omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_power: Option<u32>,
    /// Strava's "Suffer Score" (Relative Effort) when available. Surrogate for
    /// perceived exertion grounded in HR-in-zone time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffer_score: Option<u32>,
    /// Average ambient temperature in Celsius when the provider reports it.
    /// Outdoor activities from Strava and Garmin OAuth surface this when the
    /// recording device captured ambient temp; Coros does too if its watch
    /// reported it. Whoop / Fitbit / Terra don't expose ambient temperature
    /// on workouts (skin temp on Whoop Recovery is recorded separately, on
    /// the recovery record, not the activity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Endurance Intensity Factor — `normalized_power / ftp` (Coggan).
    /// Populated by the Endurance latest-snapshot pipeline; `None` for
    /// activities without a power stream or for users without an FTP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity_factor: Option<f64>,
    /// Endurance Efficiency Factor — `normalized_power / average_heart_rate`.
    /// `None` when either input is missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub efficiency_factor: Option<f64>,
    /// Endurance Variability Index — `normalized_power / average_power`.
    /// `None` when the activity has no power stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variability_index: Option<f64>,
    /// Endurance aerobic decoupling percentage. `None` when the activity
    /// has fewer than 20 paired HR+speed samples (Coggan threshold).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoupling_pct: Option<f64>,
    /// Endurance time-in-zone distribution computed against the user's
    /// configured `HrZoneSet`. `None` when no HR stream or no user zones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_distribution: Option<ZoneDistribution>,
}

impl From<&Activity> for ActivitySummary {
    fn from(activity: &Activity) -> Self {
        Self {
            id: activity.id().to_owned(),
            name: activity.name().to_owned(),
            sport_type: activity.sport_type().clone(),
            start_date: activity.start_date().to_rfc3339(),
            // Populated by prepare_activity_data when the user's timezone is
            // known; the From conversion has no timezone context.
            start_date_local: None,
            distance_meters: activity.distance_meters().unwrap_or(0.0),
            duration_seconds: activity.duration_seconds(),
            elevation_gain_meters: activity.elevation_gain(),
            average_heart_rate: activity.average_heart_rate(),
            max_heart_rate: activity.max_heart_rate(),
            calories: activity.calories(),
            average_cadence: activity.average_cadence(),
            average_power: activity.average_power(),
            normalized_power: activity.normalized_power(),
            suffer_score: activity.suffer_score(),
            temperature: activity.temperature(),
            // Endurance metrics are derived in the latest_snapshot pipeline,
            // not at the per-activity summary boundary. Keep them None here
            // so the JSON shape is stable for non-Section-11 callers.
            intensity_factor: None,
            efficiency_factor: None,
            variability_index: None,
            decoupling_pct: None,
            zone_distribution: None,
        }
    }
}
