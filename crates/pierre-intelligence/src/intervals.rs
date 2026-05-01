// ABOUTME: Endurance Phase 3 intervals.json shape — converts cageux laps to per-interval IF/EF/decoupling/normalized power
// ABOUTME: Pure conversion; runs the same per-stream calculators as latest_snapshot but bucketed per lap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Endurance intervals
//!
//! [`build_intervals`] turns a single [`dravr_cageux::models::activity::Activity`]
//! into the Endurance `intervals.json` payload — one row per lap with the
//! per-interval Endurance metrics (avg HR, normalized power, IF, decoupling).
//!
//! When the activity has no laps, the output is a single synthetic interval
//! covering the whole activity so coaches can still compare apples to
//! apples against multi-lap sessions.

use chrono::{DateTime, Utc};
use dravr_cageux::models::activity::{Activity, Lap, TimeSeriesData};
use dravr_cageux::visitor::{DecouplingDetector, NormalizedPowerCalculator, TimeSeriesExt};
use serde::{Deserialize, Serialize};

/// One row in `intervals.json` representing a single lap or interval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalRow {
    /// 1-based interval index inside the activity.
    pub index: u32,
    /// Inclusive start time of the interval (UTC).
    pub start_time: DateTime<Utc>,
    /// Distance in metres. Zero for stationary intervals.
    pub distance_meters: f64,
    /// Elapsed seconds (includes stopped time).
    pub elapsed_seconds: u64,
    /// Moving seconds (excludes stopped time). `None` when provider omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moving_seconds: Option<u64>,
    /// Total elevation gain (m) over the interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_gain_meters: Option<f64>,
    /// Average heart rate (bpm) for this interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_hr: Option<u32>,
    /// Average power (watts) for this interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_power: Option<u32>,
    /// Normalized power (Coggan, 30s rolling EMA^4) over this interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_power: Option<f64>,
    /// Intensity factor (`NP / FTP`). `None` when FTP missing or no power.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intensity_factor: Option<f64>,
    /// Aerobic decoupling percentage over this interval. `None` when fewer
    /// than 20 paired HR + speed samples are available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoupling_pct: Option<f64>,
}

/// Endurance `intervals.json` payload for a single activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalsExport {
    /// Provider activity id.
    pub activity_id: String,
    /// Activity sport label (lowercase).
    pub sport: String,
    /// Number of intervals returned.
    pub interval_count: usize,
    /// Per-interval rows in chronological order.
    pub intervals: Vec<IntervalRow>,
}

/// Build the intervals payload for one activity.
///
/// `ftp_watts` is the user's FTP. When `None`, the per-interval intensity
/// factor is `None`.
#[must_use]
pub fn build_intervals(activity: &Activity, ftp_watts: Option<u32>) -> IntervalsExport {
    let sport = format!("{:?}", activity.sport_type()).to_ascii_lowercase();
    let activity_id = activity.id().to_owned();
    let base_start = activity.start_date();
    let stream = activity.time_series_data();

    let intervals: Vec<IntervalRow> = match activity.laps() {
        Some(laps) if !laps.is_empty() => laps
            .iter()
            .map(|lap| build_row_from_lap(base_start, lap, stream, ftp_watts))
            .collect(),
        _ => vec![build_row_from_whole_activity(activity, stream, ftp_watts)],
    };

    IntervalsExport {
        activity_id,
        sport,
        interval_count: intervals.len(),
        intervals,
    }
}

fn build_row_from_lap(
    base_start: DateTime<Utc>,
    lap: &Lap,
    stream: Option<&TimeSeriesData>,
    ftp_watts: Option<u32>,
) -> IntervalRow {
    let lap_start = base_start; // Cageux laps don't carry an absolute start; use activity start.
    let normalized_power = stream.and_then(np_for_window);
    let decoupling_pct = stream.and_then(decoupling_for_window);
    let intensity_factor = match (normalized_power, ftp_watts) {
        (Some(np), Some(ftp)) if ftp > 0 => Some(np / f64::from(ftp)),
        _ => None,
    };
    IntervalRow {
        index: lap.index,
        start_time: lap_start,
        distance_meters: lap.distance_meters,
        elapsed_seconds: lap.elapsed_time_seconds,
        moving_seconds: lap.moving_time_seconds,
        elevation_gain_meters: lap.elevation_gain_meters,
        avg_hr: lap.average_heart_rate,
        avg_power: lap.average_power,
        normalized_power,
        intensity_factor,
        decoupling_pct,
    }
}

fn build_row_from_whole_activity(
    activity: &Activity,
    stream: Option<&TimeSeriesData>,
    ftp_watts: Option<u32>,
) -> IntervalRow {
    let normalized_power = activity
        .normalized_power()
        .map(f64::from)
        .or_else(|| stream.and_then(np_for_window));
    let decoupling_pct = stream.and_then(decoupling_for_window);
    let intensity_factor = match (normalized_power, ftp_watts) {
        (Some(np), Some(ftp)) if ftp > 0 => Some(np / f64::from(ftp)),
        _ => None,
    };
    IntervalRow {
        index: 1,
        start_time: activity.start_date(),
        distance_meters: activity.distance_meters().unwrap_or(0.0),
        elapsed_seconds: activity.duration_seconds(),
        moving_seconds: None,
        elevation_gain_meters: activity.elevation_gain(),
        avg_hr: activity.average_heart_rate(),
        avg_power: activity.average_power(),
        normalized_power,
        intensity_factor,
        decoupling_pct,
    }
}

fn np_for_window(stream: &TimeSeriesData) -> Option<f64> {
    let mut calc = NormalizedPowerCalculator::default();
    stream.accept(&mut calc);
    calc.normalized_power()
}

fn decoupling_for_window(stream: &TimeSeriesData) -> Option<f64> {
    let mut det = DecouplingDetector::default();
    stream.accept(&mut det);
    det.decoupling_percentage()
}
