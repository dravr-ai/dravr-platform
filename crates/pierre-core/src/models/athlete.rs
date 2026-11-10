// ABOUTME: Athlete profile and statistics models from fitness providers
// ABOUTME: Athlete profile, Stats with the coverage its totals span, and period totals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Activity, PhysiologicalSex};

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
}

/// Aggregated fitness statistics for an athlete
///
/// The top-level `total_*` fields cover exactly what `coverage` says: the
/// athlete's whole history only when it is [`StatsCoverage::Lifetime`]. A
/// provider with no aggregate endpoint sums a recent page of workouts or a
/// window of days, and says so there, so a sample is never read as a lifetime
/// figure (carnet#919). When the provider also exposes calendar-year figures,
/// `year_to_date` carries the current-year totals so a consumer can
/// distinguish "annual" from "lifetime" without inferring it from the numbers.
///
/// # Examples
///
/// ```rust
/// use pierre_core::models::{Stats, StatsCoverage};
///
/// let stats = Stats {
///     total_activities: 150,
///     total_distance: 1500000.0, // 1500 km in meters
///     total_duration: 540000, // 150 hours in seconds
///     total_elevation_gain: 25000.0, // 25km of elevation
///     coverage: StatsCoverage::Lifetime,
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
    /// Which activities the `total_*` fields were summed over
    pub coverage: StatsCoverage,
    /// Current calendar-year totals, when the provider supplies them.
    /// `None` means the provider does not expose a year-to-date breakdown
    /// (consumers must not treat the `total_*` fields as annual).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year_to_date: Option<PeriodTotals>,
}

impl Stats {
    /// Totals summed over `activities`, which `coverage` describes, with no
    /// year-to-date breakdown.
    ///
    /// An activity missing a distance or an elevation adds nothing to that
    /// total. Each float total starts from `0.0`: `Iterator::sum` over `f64`
    /// starts from `-0.0`, which would reach an athlete as `-0`.
    #[must_use]
    pub fn summed_over(activities: &[Activity], coverage: StatsCoverage) -> Self {
        let total = |field: fn(&Activity) -> Option<f64>| {
            activities
                .iter()
                .filter_map(field)
                .fold(0.0, |sum, value| sum + value)
        };
        Self {
            total_activities: u64::try_from(activities.len()).unwrap_or(u64::MAX),
            total_distance: total(Activity::distance_meters),
            total_duration: activities.iter().map(Activity::duration_seconds).sum(),
            total_elevation_gain: total(Activity::elevation_gain),
            coverage,
            year_to_date: None,
        }
    }
}

/// Which activities a [`Stats`]' `total_*` fields were summed over.
///
/// Serialized with a `kind` tag (`lifetime`, `recent_activities`, `window`),
/// the field the `get_stats` tool description tells the model to read before
/// quoting a total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StatsCoverage {
    /// The athlete's whole history, as the provider's aggregate reports it
    Lifetime,
    /// Only the provider's `count` most recent activities, the oldest of
    /// which started at `oldest` (absent when there were none)
    RecentActivities {
        /// How many activities the totals were summed over
        count: u64,
        /// Start of the oldest activity counted
        #[serde(skip_serializing_if = "Option::is_none")]
        oldest: Option<DateTime<Utc>>,
    },
    /// Only the activities of the last `days` days
    Window {
        /// Length of the window, in days, ending now
        days: u32,
    },
}

impl StatsCoverage {
    /// The coverage of totals summed over `activities`, a provider's most
    /// recent page of them.
    #[must_use]
    pub fn recent(activities: &[Activity]) -> Self {
        Self::RecentActivities {
            count: u64::try_from(activities.len()).unwrap_or(u64::MAX),
            oldest: activities.iter().map(Activity::start_date).min(),
        }
    }
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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::models::{ActivityBuilder, SportType};

    fn run(id: &str, distance: Option<f64>, climb: Option<f64>) -> Activity {
        let mut builder = ActivityBuilder::new(
            id.to_owned(),
            "Run".to_owned(),
            SportType::Run,
            Utc.with_ymd_and_hms(2026, 11, 1, 12, 0, 0).unwrap(),
            1_800,
            "apple_health_archive".to_owned(),
        );
        if let Some(distance) = distance {
            builder = builder.distance_meters(distance);
        }
        if let Some(climb) = climb {
            builder = builder.elevation_gain(climb);
        }
        builder.build()
    }

    #[test]
    fn totals_with_nothing_to_sum_are_positive_zero() {
        let stats = Stats::summed_over(&[run("a", None, None)], StatsCoverage::Lifetime);
        assert!(
            stats.total_elevation_gain.is_sign_positive() && stats.total_elevation_gain == 0.0,
            "bug: no elevation must total 0.0, not -0.0"
        );
        assert!(
            stats.total_distance.is_sign_positive() && stats.total_distance == 0.0,
            "bug: no distance must total 0.0, not -0.0"
        );
        let empty = Stats::summed_over(&[], StatsCoverage::Lifetime);
        assert!(
            empty.total_elevation_gain.is_sign_positive(),
            "bug: no activities must total 0.0, not -0.0"
        );
    }

    #[test]
    fn totals_add_what_each_activity_records() {
        let activities = [
            run("a", Some(10_000.0), Some(120.0)),
            run("b", Some(5_000.0), None),
        ];
        let stats = Stats::summed_over(&activities, StatsCoverage::Lifetime);
        assert_eq!(stats.total_activities, 2, "bug: both activities count");
        assert!(
            (stats.total_distance - 15_000.0).abs() < f64::EPSILON,
            "bug: distances add"
        );
        assert!(
            (stats.total_elevation_gain - 120.0).abs() < f64::EPSILON,
            "bug: a missing elevation adds nothing"
        );
        assert_eq!(stats.total_duration, 3_600, "bug: durations add");
        assert!(
            stats.year_to_date.is_none(),
            "bug: no year-to-date breakdown"
        );
    }
}
