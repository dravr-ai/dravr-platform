// ABOUTME: Endurance latest.json snapshot — IF / EF / VI / decoupling / zone distribution per-window aggregate
// ABOUTME: Computes per-activity Endurance metrics from cageux primitives and aggregates over a sliding window
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Endurance latest snapshot
//!
//! Per-window summary backing `GET /api/v1/endurance/latest`.
//!
//! For each activity inside the window we derive the four Endurance
//! per-activity metrics — intensity factor, efficiency factor,
//! variability index, decoupling — using the cageux primitives
//! (`NormalizedPowerCalculator`, `DecouplingDetector`,
//! `ZoneTimeCalculator`). We then aggregate to a single snapshot record
//! that the endpoint serialises to JSON.
//!
//! All metric outputs are `Option`-wrapped: a metric that cannot be
//! computed because the underlying sensor stream was missing comes back
//! as `None`, in line with the Endurance deterministic-output rule
//! that forbids virtual math.

use chrono::{DateTime, Duration, Utc};
use dravr_cageux::models::activity::Activity;
use dravr_cageux::visitor::{DecouplingDetector, NormalizedPowerCalculator, TimeSeriesExt};
use pierre_core::models::zones::{HrZoneSet, ZoneDistribution};
use serde::{Deserialize, Serialize};

/// Default analysis window when the caller does not supply one.
pub const DEFAULT_WINDOW_DAYS: u32 = 7;
/// Maximum analysis window — bounded to keep query cost predictable.
pub const MAX_WINDOW_DAYS: u32 = 365;

/// Per-activity Endurance metrics block.
///
/// Each field is `Option<f64>` because the underlying calculation
/// requires sensor streams (power, HR) that may be missing on a given
/// activity. Per the Endurance deterministic-output rules, missing
/// sensor data emits `None` rather than a synthesised estimate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ActivitySection11Metrics {
    /// Intensity Factor — `normalized_power / ftp` (Coggan).
    /// `None` when the activity has no power stream or the user has no FTP.
    pub intensity_factor: Option<f64>,
    /// Efficiency Factor — `normalized_power / average_heart_rate`.
    /// `None` when either input is missing.
    pub efficiency_factor: Option<f64>,
    /// Variability Index — `normalized_power / average_power`.
    /// `None` when the activity has no power stream.
    pub variability_index: Option<f64>,
    /// Aerobic decoupling percentage — `(2nd-half HR/speed - 1st-half HR/speed) / 1st-half`.
    /// `None` when the activity has fewer than 20 paired HR+speed samples.
    pub decoupling_pct: Option<f64>,
}

/// One row in the per-activity table embedded in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestSnapshotActivityRow {
    /// Provider activity id.
    pub activity_id: String,
    /// Activity start time (UTC).
    pub start_date: DateTime<Utc>,
    /// Sport type label (`run`, `ride`, `swim`, …).
    pub sport: String,
    /// Total distance in metres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<f64>,
    /// Duration in seconds.
    pub duration_seconds: u64,
    /// Average heart rate (bpm).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_hr: Option<u32>,
    /// Average power (watts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_power: Option<u32>,
    /// Endurance derived metrics.
    pub metrics: ActivitySection11Metrics,
    /// Time-in-zone distribution computed from the HR stream against the
    /// user's [`HrZoneSet`]. `None` when no HR stream is present or no
    /// user zones are configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_distribution: Option<ZoneDistribution>,
}

/// Endurance latest-snapshot payload.
///
/// Window-aggregated over the activities passed to
/// [`build_latest_snapshot`]. Aggregates use simple arithmetic means
/// across activities that contributed a value (i.e. drop `None`s before
/// averaging — never substitute zeros).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestSnapshot {
    /// Window length in days that was analysed.
    pub window_days: u32,
    /// Inclusive start of the window (UTC).
    pub window_start: DateTime<Utc>,
    /// Inclusive end of the window (UTC).
    pub window_end: DateTime<Utc>,
    /// Number of activities considered.
    pub activity_count: usize,
    /// Sum of distances across the activities (metres).
    pub total_distance_meters: f64,
    /// Sum of durations across the activities (seconds).
    pub total_duration_seconds: u64,
    /// Mean intensity factor over activities that produced a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_intensity_factor: Option<f64>,
    /// Mean efficiency factor over activities that produced a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_efficiency_factor: Option<f64>,
    /// Mean variability index over activities that produced a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_variability_index: Option<f64>,
    /// Mean decoupling percentage over activities that produced a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_decoupling_pct: Option<f64>,
    /// Aggregated time-in-zone distribution across the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregated_zone_distribution: Option<ZoneDistribution>,
    /// Per-activity rows in chronological order (oldest first).
    pub activities: Vec<LatestSnapshotActivityRow>,
}

/// Build a Endurance latest snapshot for a window.
///
/// Activities outside the `window_days` window (relative to the most
/// recent activity, falling back to "now" when no activities are
/// supplied) are filtered out before aggregation. The optional
/// `hr_zones` lets the caller supply per-user zone boundaries that
/// override anything cageux derives — when `None` the snapshot omits
/// the per-activity zone distribution and the aggregate.
///
/// `ftp_watts` is the user's Functional Threshold Power; `None` skips
/// intensity-factor computation for every activity.
#[must_use]
pub fn build_latest_snapshot(
    activities: &[Activity],
    window_days: u32,
    ftp_watts: Option<u32>,
    hr_zones: Option<HrZoneSet>,
) -> LatestSnapshot {
    let window_days = window_days.clamp(1, MAX_WINDOW_DAYS);
    let now = Utc::now();
    let anchor = activities
        .iter()
        .map(Activity::start_date)
        .max()
        .unwrap_or(now);
    let window_start = anchor - Duration::days(i64::from(window_days));

    let mut filtered: Vec<&Activity> = activities
        .iter()
        .filter(|a| a.start_date() >= window_start && a.start_date() <= anchor)
        .collect();
    filtered.sort_by_key(|a| a.start_date());

    let mut rows: Vec<LatestSnapshotActivityRow> = Vec::with_capacity(filtered.len());
    let mut intensity_sum = 0.0_f64;
    let mut intensity_count: u32 = 0;
    let mut efficiency_sum = 0.0_f64;
    let mut efficiency_count: u32 = 0;
    let mut variability_sum = 0.0_f64;
    let mut variability_count: u32 = 0;
    let mut decoupling_sum = 0.0_f64;
    let mut decoupling_count: u32 = 0;

    let mut total_distance = 0.0_f64;
    let mut total_duration: u64 = 0;
    let mut aggregated_zones = ZoneDistribution::default();
    let mut any_zones = false;

    for activity in filtered {
        let metrics = compute_activity_metrics(activity, ftp_watts);
        let zone_distribution = hr_zones.and_then(|z| compute_zone_distribution(activity, z));

        if let Some(value) = metrics.intensity_factor {
            intensity_sum += value;
            intensity_count = intensity_count.saturating_add(1);
        }
        if let Some(value) = metrics.efficiency_factor {
            efficiency_sum += value;
            efficiency_count = efficiency_count.saturating_add(1);
        }
        if let Some(value) = metrics.variability_index {
            variability_sum += value;
            variability_count = variability_count.saturating_add(1);
        }
        if let Some(value) = metrics.decoupling_pct {
            decoupling_sum += value;
            decoupling_count = decoupling_count.saturating_add(1);
        }
        if let Some(zd) = zone_distribution {
            aggregated_zones = sum_zone_distributions(&aggregated_zones, &zd);
            any_zones = true;
        }

        // Guard against provider-side NaN / negative distance: a single bad
        // value would otherwise poison the window's `total_distance_meters`.
        if let Some(d) = activity.distance_meters().and_then(finite) {
            if d >= 0.0 {
                total_distance += d;
            }
        }
        total_duration = total_duration.saturating_add(activity.duration_seconds());

        rows.push(LatestSnapshotActivityRow {
            activity_id: activity.id().to_owned(),
            start_date: activity.start_date(),
            sport: format!("{:?}", activity.sport_type()).to_ascii_lowercase(),
            distance_meters: activity.distance_meters().and_then(finite),
            duration_seconds: activity.duration_seconds(),
            avg_hr: activity.average_heart_rate(),
            avg_power: activity.average_power(),
            metrics,
            zone_distribution,
        });
    }

    LatestSnapshot {
        window_days,
        window_start,
        window_end: anchor,
        activity_count: rows.len(),
        total_distance_meters: total_distance,
        total_duration_seconds: total_duration,
        avg_intensity_factor: mean(intensity_sum, intensity_count),
        avg_efficiency_factor: mean(efficiency_sum, efficiency_count),
        avg_variability_index: mean(variability_sum, variability_count),
        avg_decoupling_pct: mean(decoupling_sum, decoupling_count),
        aggregated_zone_distribution: any_zones.then_some(aggregated_zones),
        activities: rows,
    }
}

fn mean(sum: f64, count: u32) -> Option<f64> {
    if count == 0 {
        return None;
    }
    finite(sum / f64::from(count))
}

/// Pass-through gate that drops `NaN` / `±Infinity` so the snapshot can never
/// surface a non-finite value to the JSON encoder. Sensor outliers (avg HR = 1,
/// duration = 2 s, NP = 0) and cageux primitives that hit a degenerate window
/// can both produce non-finite values; per the Endurance deterministic-output
/// rule a missing-or-degenerate metric is `None`, never a synthesised number.
fn finite(value: f64) -> Option<f64> {
    if value.is_finite() {
        Some(value)
    } else {
        None
    }
}

fn sum_zone_distributions(a: &ZoneDistribution, b: &ZoneDistribution) -> ZoneDistribution {
    ZoneDistribution {
        z1_seconds: a.z1_seconds.saturating_add(b.z1_seconds),
        z2_seconds: a.z2_seconds.saturating_add(b.z2_seconds),
        z3_seconds: a.z3_seconds.saturating_add(b.z3_seconds),
        z4_seconds: a.z4_seconds.saturating_add(b.z4_seconds),
        z5_seconds: a.z5_seconds.saturating_add(b.z5_seconds),
        above_max_seconds: a.above_max_seconds.saturating_add(b.above_max_seconds),
    }
}

fn compute_activity_metrics(
    activity: &Activity,
    ftp_watts: Option<u32>,
) -> ActivitySection11Metrics {
    let avg_hr = activity.average_heart_rate();
    let avg_power = activity.average_power();

    let normalized_power = activity
        .normalized_power()
        .map(f64::from)
        .or_else(|| compute_normalized_power(activity))
        .and_then(finite)
        .filter(|np| *np > 0.0);

    let intensity_factor = match (normalized_power, ftp_watts) {
        (Some(np), Some(ftp)) if ftp > 0 => finite(np / f64::from(ftp)),
        _ => None,
    };

    let efficiency_factor = match (normalized_power, avg_hr) {
        (Some(np), Some(hr)) if hr > 0 => finite(np / f64::from(hr)),
        _ => None,
    };

    let variability_index = match (normalized_power, avg_power) {
        (Some(np), Some(avg)) if avg > 0 => finite(np / f64::from(avg)),
        _ => None,
    };

    let decoupling_pct = compute_decoupling(activity).and_then(finite);

    ActivitySection11Metrics {
        intensity_factor,
        efficiency_factor,
        variability_index,
        decoupling_pct,
    }
}

fn compute_normalized_power(activity: &Activity) -> Option<f64> {
    let stream = activity.time_series_data()?;
    let mut np_calc = NormalizedPowerCalculator::default();
    stream.accept(&mut np_calc);
    np_calc.normalized_power()
}

fn compute_decoupling(activity: &Activity) -> Option<f64> {
    let stream = activity.time_series_data()?;
    let mut detector = DecouplingDetector::default();
    stream.accept(&mut detector);
    detector.decoupling_percentage()
}

fn compute_zone_distribution(activity: &Activity, zones: HrZoneSet) -> Option<ZoneDistribution> {
    let stream = activity.time_series_data()?;
    let hr_samples = stream.heart_rate.as_ref()?;
    if hr_samples.is_empty() {
        return None;
    }
    let timestamps = &stream.timestamps;
    let mut zd = ZoneDistribution::default();
    for (idx, &bpm) in hr_samples.iter().enumerate() {
        let delta = sample_delta_seconds(timestamps, idx);
        let bpm_u16 = u16::try_from(bpm).unwrap_or(u16::MAX);
        match zones.classify(bpm_u16) {
            Some(1) => zd.z1_seconds = zd.z1_seconds.saturating_add(delta),
            Some(2) => zd.z2_seconds = zd.z2_seconds.saturating_add(delta),
            Some(3) => zd.z3_seconds = zd.z3_seconds.saturating_add(delta),
            Some(4) => zd.z4_seconds = zd.z4_seconds.saturating_add(delta),
            Some(5) => zd.z5_seconds = zd.z5_seconds.saturating_add(delta),
            _ => zd.above_max_seconds = zd.above_max_seconds.saturating_add(delta),
        }
    }
    if zd.total_seconds() == 0 {
        None
    } else {
        Some(zd)
    }
}

fn sample_delta_seconds(timestamps: &[u32], idx: usize) -> u32 {
    if idx == 0 {
        return 1;
    }
    let prev = timestamps.get(idx.saturating_sub(1)).copied().unwrap_or(0);
    let curr = timestamps.get(idx).copied().unwrap_or(prev);
    curr.saturating_sub(prev).max(1)
}
