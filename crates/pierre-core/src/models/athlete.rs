// ABOUTME: Athlete profile and statistics models from fitness providers
// ABOUTME: Athlete, Stats, PersonalRecord, and PrMetric definitions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Represents an athlete/user profile from any provider
///
/// Contains the essential profile information that's commonly available
/// across fitness platforms.
///
/// # Examples
///
/// ```rust
/// use pierre_core::models::Athlete;
///
/// let athlete = Athlete {
///     id: "12345".into(),
///     username: "runner123".into(),
///     firstname: Some("John".into()),
///     lastname: Some("Doe".into()),
///     profile_picture: Some("https://dgalywyr863hv.cloudfront.net/pictures/athletes/12345678/avatar/medium.jpg".into()),
///     provider: "strava".into(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Athlete {
    /// Unique identifier for the athlete (provider-specific)
    pub id: String,
    /// Public username/handle
    pub username: String,
    /// First name (may not be public on some providers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firstname: Option<String>,
    /// Last name (may not be public on some providers)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lastname: Option<String>,
    /// `URL` to profile picture/avatar
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_picture: Option<String>,
    /// Source provider of this athlete data
    pub provider: String,
}

/// Aggregated fitness statistics for an athlete
///
/// The top-level `total_*` fields are **all-time / lifetime** totals across the
/// athlete's entire history. When the provider also exposes calendar-year
/// figures, `year_to_date` carries the current-year totals so a consumer can
/// distinguish "annual" from "lifetime" without inferring it from the numbers.
///
/// # Examples
///
/// ```rust
/// use pierre_core::models::Stats;
///
/// let stats = Stats {
///     total_activities: 150,
///     total_distance: 1500000.0, // 1500 km in meters
///     total_duration: 540000, // 150 hours in seconds
///     total_elevation_gain: 25000.0, // 25km of elevation
///     year_to_date: None,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Stats {
    /// Total number of recorded activities over all time
    pub total_activities: u64,
    /// Total distance covered over all time (meters)
    pub total_distance: f64,
    /// Total time spent in activities over all time (seconds)
    pub total_duration: u64,
    /// Total elevation gained over all time (meters)
    pub total_elevation_gain: f64,
    /// Current calendar-year totals, when the provider supplies them.
    /// `None` means the provider does not expose a year-to-date breakdown
    /// (consumers must not treat the all-time totals as annual).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year_to_date: Option<PeriodTotals>,
}

/// Aggregated activity totals scoped to a single time period.
///
/// Used for the `year_to_date` breakdown on [`Stats`]; mirrors the shape of the
/// lifetime totals so a period can be reported with the same fields.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PeriodTotals {
    /// Number of recorded activities in the period
    pub total_activities: u64,
    /// Distance covered in the period (meters)
    pub total_distance: f64,
    /// Time spent in activities in the period (seconds)
    pub total_duration: u64,
    /// Elevation gained in the period (meters)
    pub total_elevation_gain: f64,
}

/// Types of personal record metrics tracked
///
/// Each metric represents a different aspect of athletic performance
/// that can be optimized and tracked over time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrMetric {
    /// Fastest pace achieved (seconds per meter)
    FastestPace,
    /// Longest distance covered in a single activity (meters)
    LongestDistance,
    /// Highest elevation gained in a single activity (meters)
    HighestElevation,
    /// Fastest completion time for a standard distance (seconds)
    FastestTime,
}

/// Represents a personal record achievement
///
/// Tracks the athlete's best performance in various metrics.
/// Links back to the specific activity where the record was achieved.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PersonalRecord {
    /// `ID` of the activity where this record was achieved
    pub activity_id: String,
    /// Type of performance metric
    pub metric: PrMetric,
    /// Value of the record (units depend on metric type)
    pub value: f64,
    /// When the record was achieved
    pub date: DateTime<Utc>,
}
