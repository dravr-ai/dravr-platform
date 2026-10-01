// ABOUTME: The activity view's split and lap rows, and the view of one workout projected from its merged session
// ABOUTME: Every figure is the provider's own, read from the cache; a figure it does not hold is null, never estimated

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The shape `GET /api/me/activities/{provider}/{activity_id}` answers in:
//! the rows of the workout's splits and laps, and the projection of its
//! merged session into [`ActivityDetailResponse`].

use pierre_core::models::{Activity, Lap, Split};
use pierre_database::repositories::CachedActivityRow;
use serde::Serialize;

use super::{ActivityDetailResponse, HomeActivity};

/// One split of an activity: a uniform distance bucket the provider carved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActivitySplit {
    /// 1-based position along the activity.
    pub index: u32,
    /// Distance covered, in metres.
    pub distance_meters: f64,
    /// Elapsed time, stops included, in seconds.
    pub elapsed_time_seconds: u64,
    /// Moving time, stops left out, in seconds.
    pub moving_time_seconds: Option<u64>,
    /// Net elevation change over the split, in metres; negative downhill.
    pub elevation_difference_meters: Option<f64>,
    /// Average speed in metres per second.
    pub average_speed_mps: Option<f64>,
    /// Average heart rate in beats per minute.
    pub average_heart_rate: Option<u32>,
}

impl From<&Split> for ActivitySplit {
    fn from(split: &Split) -> Self {
        Self {
            index: split.index,
            distance_meters: split.distance_meters,
            elapsed_time_seconds: split.elapsed_time_seconds,
            moving_time_seconds: split.moving_time_seconds,
            elevation_difference_meters: split.elevation_difference_meters,
            average_speed_mps: split.average_speed_mps,
            average_heart_rate: split.average_heart_rate,
        }
    }
}

/// One lap of an activity, as the athlete's button or the workout marked it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActivityLap {
    /// 1-based position along the activity.
    pub index: u32,
    /// Distance covered, in metres.
    pub distance_meters: f64,
    /// Elapsed time, stops included, in seconds.
    pub elapsed_time_seconds: u64,
    /// Moving time, stops left out, in seconds.
    pub moving_time_seconds: Option<u64>,
    /// Elevation gained, in metres.
    pub elevation_gain_meters: Option<f64>,
    /// Average speed in metres per second.
    pub average_speed_mps: Option<f64>,
    /// Average heart rate in beats per minute.
    pub average_heart_rate: Option<u32>,
    /// Highest heart rate in beats per minute.
    pub max_heart_rate: Option<u32>,
    /// Average power in watts.
    pub average_power: Option<u32>,
}

impl From<&Lap> for ActivityLap {
    fn from(lap: &Lap) -> Self {
        Self {
            index: lap.index,
            distance_meters: lap.distance_meters,
            elapsed_time_seconds: lap.elapsed_time_seconds,
            moving_time_seconds: lap.moving_time_seconds,
            elevation_gain_meters: lap.elevation_gain_meters,
            average_speed_mps: lap.average_speed_mps,
            average_heart_rate: lap.average_heart_rate,
            max_heart_rate: lap.max_heart_rate,
            average_power: lap.average_power,
        }
    }
}

impl ActivityDetailResponse {
    /// The view of one workout: `session` is the merged session, `row` the
    /// copy whose route it draws. The thread its view opened is looked up
    /// apart, across every copy of the workout, and `conversation_id` is
    /// `None` until then.
    pub(super) fn from_session(row: &CachedActivityRow, session: &Activity) -> Self {
        Self {
            activity: HomeActivity::from_session(row, session),
            average_heart_rate: session.average_heart_rate(),
            max_heart_rate: session.max_heart_rate(),
            average_speed_mps: session.average_speed(),
            max_speed_mps: session.max_speed(),
            average_power: session.average_power(),
            calories: session.calories(),
            splits: session
                .splits()
                .map(|splits| splits.iter().map(ActivitySplit::from).collect())
                .unwrap_or_default(),
            laps: session
                .laps()
                .map(|laps| laps.iter().map(ActivityLap::from).collect())
                .unwrap_or_default(),
            conversation_id: None,
        }
    }
}
