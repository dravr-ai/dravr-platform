// ABOUTME: Athlete profile and statistics models served to tools
// ABOUTME: Athlete profile, Stats with the coverage its totals span, and period totals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::coverage::Coverage;
use super::PhysiologicalSex;

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
///     sex: None,
///     created_at: None,
///     active_streak: None,
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
    /// Physiological sex, when the provider reports one (Strava's and
    /// intervals.icu's athlete `sex`). Never inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sex: Option<PhysiologicalSex>,
    /// When the athlete's account was created at the provider, when the
    /// provider reports it (Strava's athlete `created_at`): no activity is
    /// recorded before it, so a history walk stops there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// The athlete's current activity streak, as Strava itself reports it
    /// on the profile (the scraped Strava session only, the athlete's own
    /// account). It is Strava's own figure, never a
    /// Dravr computation: nothing derives or recomputes it from activities,
    /// and `None` means the provider did not report one, not that the
    /// athlete has no streak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_streak: Option<ActiveStreak>,
}

/// Aggregated fitness statistics for an athlete
///
/// The top-level `total_*` fields are summed over every source the athlete
/// holds, merged, so a workout two sources recorded counts once. They cover
/// exactly what `coverage` says: its window and whether the stored history
/// behind it is complete, never one provider's sample read as a lifetime
/// figure (carnet#919). `year_to_date` carries the current calendar year's
/// totals so a consumer can distinguish "annual" from "all-time" without
/// inferring it from the numbers.
///
/// # Examples
///
/// ```rust
/// use pierre_core::models::coverage::{Coverage, CoverageWindow};
/// use pierre_core::models::Stats;
///
/// let stats = Stats {
///     total_activities: 150,
///     total_distance: 1500000.0, // 1500 km in meters
///     total_duration: 540000, // 150 hours in seconds
///     total_elevation_gain: 25000.0, // 25km of elevation
///     coverage: Coverage::complete(CoverageWindow {
///         total: Some(150),
///         returned: 150,
///         ..CoverageWindow::default()
///     }),
///     year_to_date: None,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Stats {
    /// Number of recorded activities `coverage` spans
    pub total_activities: u64,
    /// Distance covered over the activities `coverage` spans (meters)
    pub total_distance: f64,
    /// Time spent in the activities `coverage` spans (seconds)
    pub total_duration: u64,
    /// Elevation gained over the activities `coverage` spans (meters)
    pub total_elevation_gain: f64,
    /// Which activities the `total_*` fields were summed over: the window
    /// asked, how many, from which day to which, and whether the stored
    /// history behind it is complete
    pub coverage: Coverage,
    /// Current calendar-year totals. `None` means no year-to-date breakdown
    /// was computed (consumers must not treat the `total_*` fields as
    /// annual).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year_to_date: Option<PeriodTotals>,
}

/// A provider's own report of the athlete's current streak of consecutive
/// active days. Carried as the provider states it: Dravr does not recompute
/// it from the athlete's activities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ActiveStreak {
    /// Days in the streak
    pub length: u32,
    /// Where the provider says the streak stands (Strava: `Ongoing`)
    pub status: String,
    /// Activities that counted toward streaks
    pub valid_activity_count: u32,
}

/// Aggregated activity totals scoped to a single time period.
///
/// Used for the `year_to_date` breakdown on [`Stats`]; mirrors the shape of the
/// all-time totals so a period can be reported with the same fields.
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
