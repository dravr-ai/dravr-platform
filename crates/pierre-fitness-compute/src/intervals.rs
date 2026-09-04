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
//! Stream-derived metrics are computed over the lap's own window of the
//! activity stream. Cageux laps carry no absolute start, so a lap's window —
//! and its start time — is addressed by the cumulative elapsed seconds of the
//! laps before it.
//!
//! When the activity has no laps, the output is a single synthetic interval
//! covering the whole activity so coaches can still compare apples to
//! apples against multi-lap sessions.

use chrono::{DateTime, TimeDelta, Utc};
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
        Some(laps) if !laps.is_empty() => rows_from_laps(base_start, laps, stream, ftp_watts),
        _ => vec![build_row_from_whole_activity(activity, stream, ftp_watts)],
    };

    IntervalsExport {
        activity_id,
        sport,
        interval_count: intervals.len(),
        intervals,
    }
}

/// Walk the laps in order, handing each one its own slice of the stream.
///
/// Laps are contiguous, so a lap starts where the previous ones end: the
/// running offset is the sum of the preceding laps' elapsed seconds, which is
/// also what addresses this lap's samples in the stream.
fn rows_from_laps(
    base_start: DateTime<Utc>,
    laps: &[Lap],
    stream: Option<&TimeSeriesData>,
    ftp_watts: Option<u32>,
) -> Vec<IntervalRow> {
    let mut offset_seconds: u64 = 0;
    let mut rows = Vec::with_capacity(laps.len());
    for lap in laps {
        let window =
            stream.and_then(|full| lap_window(full, offset_seconds, lap.elapsed_time_seconds));
        rows.push(build_row_from_lap(
            base_start,
            offset_seconds,
            lap,
            window.as_ref(),
            ftp_watts,
        ));
        offset_seconds = offset_seconds.saturating_add(lap.elapsed_time_seconds);
    }
    rows
}

/// Build one row from a lap and the stream slice covering that lap.
///
/// `offset_seconds` is the lap's distance in seconds from the activity start:
/// cageux laps carry no absolute start, so the wall-clock start is the base
/// start advanced by the elapsed time of the laps that precede this one.
/// `window` holds only that lap's samples, so the stream-derived metrics
/// describe the interval rather than the whole session.
fn build_row_from_lap(
    base_start: DateTime<Utc>,
    offset_seconds: u64,
    lap: &Lap,
    window: Option<&TimeSeriesData>,
    ftp_watts: Option<u32>,
) -> IntervalRow {
    let lap_start = start_at_offset(base_start, offset_seconds);
    let normalized_power = window.and_then(np_for_window);
    let decoupling_pct = window.and_then(decoupling_for_window);
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

/// Wall-clock instant `offset_seconds` after the activity start.
///
/// Falls back to the activity start when the offset cannot be represented as a
/// signed duration, which keeps the row ordered inside the activity instead of
/// wrapping to an unrelated instant.
fn start_at_offset(base_start: DateTime<Utc>, offset_seconds: u64) -> DateTime<Utc> {
    i64::try_from(offset_seconds)
        .ok()
        .and_then(TimeDelta::try_seconds)
        .and_then(|delta| base_start.checked_add_signed(delta))
        .unwrap_or(base_start)
}

/// Slice `stream` down to the samples belonging to one lap.
///
/// Stream timestamps are offsets in seconds from the activity start, so a lap
/// beginning `start_offset_seconds` in and running for `elapsed_seconds` owns
/// the samples in `[start, start + elapsed)`. `None` when no sample falls in
/// that window, which leaves the window-derived metrics absent rather than
/// filling them with a neighbouring lap's numbers.
fn lap_window(
    stream: &TimeSeriesData,
    start_offset_seconds: u64,
    elapsed_seconds: u64,
) -> Option<TimeSeriesData> {
    let end_offset_seconds = start_offset_seconds.saturating_add(elapsed_seconds);
    let mut first: Option<usize> = None;
    let mut end = 0_usize;
    for (idx, &timestamp) in stream.timestamps.iter().enumerate() {
        let offset = u64::from(timestamp);
        if offset >= start_offset_seconds && offset < end_offset_seconds {
            if first.is_none() {
                first = Some(idx);
            }
            end = idx.saturating_add(1);
        }
    }
    let start = first?;
    Some(TimeSeriesData {
        timestamps: stream
            .timestamps
            .get(start..end)
            .unwrap_or_default()
            .to_vec(),
        heart_rate: slice_channel(stream.heart_rate.as_deref(), start, end),
        power: slice_channel(stream.power.as_deref(), start, end),
        cadence: slice_channel(stream.cadence.as_deref(), start, end),
        speed: slice_channel(stream.speed.as_deref(), start, end),
        altitude: slice_channel(stream.altitude.as_deref(), start, end),
        temperature: slice_channel(stream.temperature.as_deref(), start, end),
        gps_coordinates: slice_channel(stream.gps_coordinates.as_deref(), start, end),
    })
}

/// Copy `channel[start..end]`, clamped to the channel's own length.
///
/// Channels are allowed to be shorter than the timestamp axis — the visitor
/// pairs them by index and skips what is missing — so a lap late in the
/// activity can land entirely past the end of a short channel, which yields
/// `None`.
fn slice_channel<T: Clone>(channel: Option<&[T]>, start: usize, end: usize) -> Option<Vec<T>> {
    let values = channel?;
    let window = values.get(start.min(values.len())..end.min(values.len()))?;
    if window.is_empty() {
        return None;
    }
    Some(window.to_vec())
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
