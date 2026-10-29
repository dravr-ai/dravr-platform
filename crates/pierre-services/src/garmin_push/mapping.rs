// ABOUTME: Pure conversions from Garmin push records to Dravr's models — activities, laps, samples, sleep, day metrics
// ABOUTME: Nothing here reads or writes a store; a missing field stays missing, never a zero

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin push records as Dravr's models.
//!
//! The activity a summary becomes is keyed by Garmin Connect's `activityId`
//! (the key the usr/pwd connection's copy of the same activity also carries),
//! filed under the provider `garmin` with `source` `garmin`, and named after
//! its device with the Garmin brand (`garmin_service_attribution`): Garmin
//! names its devices without it. Laps come from the lap starts an
//! `activityDetails` record carries, measured over its samples; the samples
//! become the activity's [`TimeSeriesData`].
//!
//! A summary carries no power: Garmin reports it only as the samples'
//! `powerInWatts`. The activity's average, maximum and normalized power are
//! therefore derived from its samples ([`with_power`]), normalized power by
//! cageux's [`NormalizedPowerCalculator`], so a ride's intensity, efficiency
//! and variability factors can be read off it like any provider's.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use dravr_cageux::visitor::NormalizedPowerCalculator;
use dravr_riviere::DataPoint;
use dravr_sciotte::models::SportType as GarminSportKey;
use pierre_core::constants::oauth_providers::{garmin_service_attribution, GARMIN};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ActivityBuilder, Lap, SportType, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, StoredSleepStage, StoredSleepStageType, TimeSeriesData,
};
use pierre_providers::sciotte_provider::convert_sport_type;
use serde_json::Value;

use super::wire::{
    ActivityLap, ActivitySample, ActivitySummary, LevelRange, OffsetSeries, SleepRecord,
};

/// Activities longer than this are left out of `activityDetails` by Garmin,
/// so their samples come from the activity's file instead.
pub(crate) const DETAILS_MAX_SECONDS: u64 = 24 * 60 * 60;

/// Why a pushed activity summary is not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skipped {
    /// Entered by hand, not recorded on a device.
    Manual,
    /// Uploaded through the Garmin Connect web app.
    WebUpload,
    /// The parent of a multisport activity: its legs arrive as activities of
    /// their own, and keeping the parent too would count the session twice.
    MultisportParent,
    /// No `activityId`, `startTimeInSeconds` or `durationInSeconds`.
    Incomplete,
}

impl Skipped {
    /// The reason as a log field.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::WebUpload => "web_upload",
            Self::MultisportParent => "multisport_parent",
            Self::Incomplete => "incomplete",
        }
    }
}

/// The sport a Garmin activity type names.
///
/// Read with the same key table and the same conversion the usr/pwd
/// connection reads Garmin's type keys with ([`convert_sport_type`]), so one
/// activity carries one sport whichever connection captured it.
#[must_use]
pub(crate) fn sport_type(activity_type: Option<&str>) -> SportType {
    activity_type.map_or_else(
        || SportType::Other("unknown".to_owned()),
        |key| convert_sport_type(&GarminSportKey::from_garmin(&key.to_ascii_lowercase())),
    )
}

/// A float reading as a whole non-negative number.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole(value: Option<f64>) -> Option<u32> {
    value
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= f64::from(u32::MAX))
        .map(|v| v.round() as u32)
}

/// Unix seconds as an instant.
fn instant(seconds: i64) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(seconds, 0)
}

/// The activity a summary describes, keyed by `activity_id`, or why it is
/// not kept.
///
/// # Errors
///
/// Returns the [`Skipped`] reason for a manual entry, a web upload, a
/// multisport parent or a summary missing its id, start or length.
pub(crate) fn activity_from_summary(
    activity_id: Option<&str>,
    summary: &ActivitySummary,
) -> Result<Activity, Skipped> {
    if summary.manual == Some(true) {
        return Err(Skipped::Manual);
    }
    if summary.is_web_upload == Some(true) {
        return Err(Skipped::WebUpload);
    }
    if summary.is_parent == Some(true) {
        return Err(Skipped::MultisportParent);
    }
    let id = activity_id
        .or(summary.activity_id.as_deref())
        .ok_or(Skipped::Incomplete)?;
    let start = summary
        .start_time_in_seconds
        .and_then(instant)
        .ok_or(Skipped::Incomplete)?;
    let duration = summary
        .duration_in_seconds
        .and_then(|d| u64::try_from(d).ok())
        .ok_or(Skipped::Incomplete)?;
    let sport = sport_type(summary.activity_type.as_deref());
    let name = summary
        .activity_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| sport.display_name().to_owned());
    let cadence = summary
        .average_run_cadence_in_steps_per_minute
        .or(summary.average_bike_cadence_in_rounds_per_minute);
    let max_cadence = summary
        .max_run_cadence_in_steps_per_minute
        .or(summary.max_bike_cadence_in_rounds_per_minute);
    Ok(
        ActivityBuilder::new(id, name, sport, start, duration, GARMIN)
            .source(GARMIN)
            .device_name(garmin_service_attribution(summary.device_name.as_deref()))
            .distance_meters_opt(summary.distance_in_meters)
            .elevation_gain_opt(summary.total_elevation_gain_in_meters)
            .average_speed_opt(summary.average_speed_in_meters_per_second)
            .max_speed_opt(summary.max_speed_in_meters_per_second)
            .average_heart_rate_opt(whole(summary.average_heart_rate_in_beats_per_minute))
            .max_heart_rate_opt(whole(summary.max_heart_rate_in_beats_per_minute))
            .average_cadence_opt(whole(cadence))
            .max_cadence_opt(whole(max_cadence))
            .calories_opt(whole(summary.active_kilocalories))
            .steps_opt(whole(summary.steps))
            .start_latitude_opt(summary.starting_latitude_in_degree)
            .start_longitude_opt(summary.starting_longitude_in_degree)
            .build(),
    )
}

/// The power an activity's samples carry: their mean, their maximum and
/// their normalized power, each `None` when no sample carries power (or, for
/// normalized power, fewer than one 30-second window does).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PowerFigures {
    /// Mean of the samples' power, watts.
    pub(crate) average: Option<u32>,
    /// Highest sample, watts.
    pub(crate) max: Option<u32>,
    /// Normalized power, watts.
    pub(crate) normalized: Option<u32>,
}

/// The power figures of `streams`.
#[must_use]
pub(crate) fn power_figures(streams: &TimeSeriesData) -> PowerFigures {
    use dravr_cageux::visitor::TimeSeriesExt;

    let watts: Vec<u32> = streams
        .power
        .iter()
        .flatten()
        .filter_map(|sample| *sample)
        .collect();
    if watts.is_empty() {
        return PowerFigures::default();
    }
    let sum: u64 = watts.iter().map(|w| u64::from(*w)).sum();
    #[allow(clippy::cast_precision_loss)]
    let mean = sum as f64 / watts.len() as f64;
    let mut calculator = NormalizedPowerCalculator::default();
    streams.accept(&mut calculator);
    PowerFigures {
        average: whole(Some(mean)),
        max: watts.iter().copied().max(),
        normalized: whole(calculator.normalized_power()),
    }
}

/// `activity` with the power its samples carry.
///
/// A built [`Activity`] takes no new figures, so they are set through its
/// serialized form, under the fields the model serializes them as, as
/// `cached_activity_reads::with_streams` attaches samples. A figure the
/// samples do not carry leaves the activity's own standing.
///
/// # Errors
///
/// Returns an internal error when the activity does not round-trip through
/// its serialized form.
pub(crate) fn with_power(activity: Activity, streams: &TimeSeriesData) -> AppResult<Activity> {
    let figures = power_figures(streams);
    if figures == PowerFigures::default() {
        return Ok(activity);
    }
    let mut value = serde_json::to_value(&activity)
        .map_err(|e| AppError::internal(format!("serialize Garmin activity: {e}")))?;
    let Value::Object(fields) = &mut value else {
        return Err(AppError::internal(
            "a Garmin activity does not serialize as an object",
        ));
    };
    for (field, figure) in [
        ("average_power", figures.average),
        ("max_power", figures.max),
        ("normalized_power", figures.normalized),
    ] {
        if let Some(watts) = figure {
            fields.insert(field.to_owned(), Value::from(watts));
        }
    }
    serde_json::from_value(value)
        .map_err(|e| AppError::internal(format!("set Garmin activity power: {e}")))
}

/// One channel of the samples, `None` when no sample carries it, else one
/// entry per sample with `None` where that sample lacks it.
fn channel<T>(
    samples: &[&ActivitySample],
    read: impl Fn(&ActivitySample) -> Option<T>,
) -> Option<Vec<Option<T>>> {
    let values: Vec<Option<T>> = samples.iter().map(|s| read(s)).collect();
    values.iter().any(Option::is_some).then_some(values)
}

/// A reading as `f32`.
#[allow(clippy::cast_possible_truncation)]
fn narrow(value: Option<f64>) -> Option<f32> {
    value.filter(|v| v.is_finite()).map(|v| v as f32)
}

/// The samples of an activity that started at `start` (unix seconds), as the
/// series the activity model carries.
///
/// They are timed from the start. Samples before
/// the start or without a time are left out; `None` when none is left.
#[must_use]
pub(crate) fn streams_from_samples(
    start: i64,
    samples: &[ActivitySample],
) -> Option<TimeSeriesData> {
    let timed: Vec<(u32, &ActivitySample)> = samples
        .iter()
        .filter_map(|sample| {
            let at = sample.start_time_in_seconds?;
            u32::try_from(at.checked_sub(start)?)
                .ok()
                .map(|offset| (offset, sample))
        })
        .collect();
    if timed.is_empty() {
        return None;
    }
    let samples: Vec<&ActivitySample> = timed.iter().map(|(_, sample)| *sample).collect();
    Some(TimeSeriesData {
        timestamps: timed.iter().map(|(offset, _)| *offset).collect(),
        heart_rate: channel(&samples, |s| whole(s.heart_rate)),
        power: channel(&samples, |s| whole(s.power_in_watts)),
        cadence: channel(&samples, |s| {
            whole(
                s.bike_cadence_in_rpm
                    .or(s.steps_per_minute)
                    .or(s.swim_cadence_in_strokes_per_minute),
            )
        }),
        speed: channel(&samples, |s| narrow(s.speed_meters_per_second)),
        altitude: channel(&samples, |s| narrow(s.elevation_in_meters)),
        temperature: channel(&samples, |s| narrow(s.air_temperature_celsius)),
        gps_coordinates: channel(&samples, |s| {
            Some((s.latitude_in_degree?, s.longitude_in_degree?))
        }),
        distance: channel(&samples, |s| s.total_distance_in_meters),
    })
}

/// The laps of an activity that started at `start` and lasted `duration`
/// seconds.
///
/// Each runs from its start to the next lap's start, the last to
/// the activity's end, measured over the samples timed inside it. `None` when
/// the record names no lap.
#[must_use]
pub(crate) fn laps_from_details(
    start: i64,
    duration: i64,
    laps: &[ActivityLap],
    samples: &[ActivitySample],
) -> Option<Vec<Lap>> {
    let mut starts: Vec<i64> = laps
        .iter()
        .filter_map(|lap| lap.start_time_in_seconds)
        .collect();
    starts.sort_unstable();
    starts.dedup();
    if starts.is_empty() {
        return None;
    }
    let end = start.saturating_add(duration);
    let built = starts
        .iter()
        .enumerate()
        .map(|(index, &lap_start)| {
            let lap_end = starts.get(index + 1).copied().unwrap_or(end).max(lap_start);
            lap_over(index, lap_start, lap_end, samples)
        })
        .collect();
    Some(built)
}

/// One lap from `from` to `to` (unix seconds), over the samples inside it.
#[allow(clippy::cast_precision_loss)]
fn lap_over(index: usize, from: i64, to: i64, samples: &[ActivitySample]) -> Lap {
    let inside: Vec<&ActivitySample> = samples
        .iter()
        .filter(|s| {
            s.start_time_in_seconds
                .is_some_and(|at| from <= at && at < to)
        })
        .collect();
    let distances: Vec<f64> = inside
        .iter()
        .filter_map(|s| s.total_distance_in_meters)
        .collect();
    let distance = match (distances.first(), distances.last()) {
        (Some(first), Some(last)) => (last - first).max(0.0),
        _ => 0.0,
    };
    let elapsed = u64::try_from(to - from).unwrap_or(0);
    let mean = |read: fn(&ActivitySample) -> Option<f64>| {
        let values: Vec<f64> = inside.iter().filter_map(|s| read(s)).collect();
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    };
    let max = |read: fn(&ActivitySample) -> Option<f64>| {
        inside
            .iter()
            .filter_map(|s| read(s))
            .fold(None, |best: Option<f64>, v| {
                Some(best.map_or(v, |b| b.max(v)))
            })
    };
    Lap {
        id: None,
        index: u32::try_from(index + 1).unwrap_or(u32::MAX),
        distance_meters: distance,
        elapsed_time_seconds: elapsed,
        moving_time_seconds: None,
        elevation_gain_meters: None,
        average_speed_mps: (elapsed > 0 && distance > 0.0).then(|| distance / elapsed as f64),
        max_speed_mps: max(|s| s.speed_meters_per_second),
        average_heart_rate: whole(mean(|s| s.heart_rate)),
        max_heart_rate: whole(max(|s| s.heart_rate)),
        average_cadence: whole(mean(|s| s.bike_cadence_in_rpm.or(s.steps_per_minute))),
        average_power: whole(mean(|s| s.power_in_watts)),
    }
}

/// A sleep level's name as a stored stage type.
fn stage_type(level: &str) -> StoredSleepStageType {
    match level.to_ascii_lowercase().as_str() {
        "deep" => StoredSleepStageType::Deep,
        "light" => StoredSleepStageType::Light,
        "rem" => StoredSleepStageType::Rem,
        "awake" => StoredSleepStageType::Awake,
        _ => StoredSleepStageType::Unknown,
    }
}

/// The stages a sleep's level map lays out, in time order.
fn stages(levels: &BTreeMap<String, Vec<LevelRange>>) -> Vec<StoredSleepStage> {
    let mut stages: Vec<StoredSleepStage> = levels
        .iter()
        .flat_map(|(level, ranges)| {
            ranges.iter().filter_map(move |range| {
                let from = range.start_time_in_seconds?;
                let to = range.end_time_in_seconds?;
                let seconds = u32::try_from(to.checked_sub(from)?).ok()?;
                Some(StoredSleepStage {
                    stage_type: stage_type(level),
                    start: instant(from)?,
                    end: instant(to)?,
                    duration_seconds: seconds,
                })
            })
        })
        .collect();
    stages.sort_by_key(|stage| stage.start);
    stages
}

/// Non-negative seconds as `u32`.
fn seconds(value: Option<i64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

/// The sleep session a `sleeps` record describes, for `user_id` under the
/// data source `data_source_id`, or `None` when it names no start.
///
/// The session keeps the record's start, the key Garmin keeps across the
/// updates of one night (`AUTO_TENTATIVE` then a final validation), so a
/// later record replaces the earlier one. Its end is the last stage's end,
/// else the start plus the time asleep and awake.
#[must_use]
pub(crate) fn sleep_session(
    record: &SleepRecord,
    user_id: &str,
    data_source_id: &str,
) -> Option<StoredSleepSession> {
    let start_seconds = record.start_time_in_seconds?;
    let start = instant(start_seconds)?;
    let stages = stages(&record.sleep_levels_map);
    let asleep = seconds(record.duration_in_seconds);
    let awake = seconds(record.awake_duration_in_seconds);
    let window = i64::from(asleep.unwrap_or(0)) + i64::from(awake.unwrap_or(0));
    let end = stages
        .iter()
        .map(|stage| stage.end)
        .max()
        .unwrap_or_else(|| start + Duration::seconds(window));
    Some(StoredSleepSession {
        id: String::new(),
        user_id: user_id.to_owned(),
        data_source_id: data_source_id.to_owned(),
        is_nap: false,
        start_datetime: start,
        end_datetime: end.max(start),
        total_sleep_seconds: asleep.filter(|v| *v > 0),
        deep_sleep_seconds: seconds(record.deep_sleep_duration_in_seconds),
        light_sleep_seconds: seconds(record.light_sleep_duration_in_seconds),
        rem_sleep_seconds: seconds(record.rem_sleep_in_seconds),
        awake_seconds: awake,
        sleep_efficiency: None,
        avg_heart_rate: None,
        min_heart_rate: None,
        avg_hrv: None,
        sleep_score: record
            .overall_sleep_score
            .as_ref()
            .and_then(|score| whole(score.value)),
        stages,
        source_name: GARMIN.to_owned(),
    })
}

/// The readings of an offset map, each at `start` plus its offset, ordered;
/// readings `keep` refuses (Garmin's negative "no reading" codes) and keys
/// that are not offsets are left out.
#[must_use]
pub(crate) fn offset_points(
    start: i64,
    series: &OffsetSeries,
    keep: fn(f64) -> bool,
) -> Vec<DataPoint> {
    let mut points: Vec<DataPoint> = series
        .iter()
        .filter(|(_, value)| value.is_finite() && keep(**value))
        .filter_map(|(offset, value)| {
            let offset: i64 = offset.trim().parse().ok()?;
            Some(DataPoint::new(instant(start.checked_add(offset)?)?, *value))
        })
        .collect();
    points.sort_by_key(|point| point.timestamp);
    points
}

/// A reading Garmin marks as real: its "no reading" codes are negative.
#[must_use]
pub(crate) fn measured(value: f64) -> bool {
    value >= 0.0
}

/// A `yyyy-mm-dd` calendar date.
#[must_use]
pub(crate) fn calendar_date(text: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text?.trim(), "%Y-%m-%d").ok()
}

/// The readings one push brings to a day's recovery row, each `None` when
/// the push does not carry it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RecoveryReadings {
    /// Resting heart rate, bpm (`dailies`).
    pub(crate) resting_heart_rate: Option<u32>,
    /// Average stress, 0–100 (`dailies`).
    pub(crate) stress: Option<u32>,
    /// The day's highest Body Battery (`stressDetails`).
    pub(crate) body_battery: Option<u32>,
    /// The night's average rMSSD, ms (`hrv`).
    pub(crate) hrv_rmssd: Option<f64>,
    /// The night's skin-temperature deviation, Celsius (`skinTemp`).
    pub(crate) skin_temp_deviation: Option<f64>,
}

impl RecoveryReadings {
    /// Whether the push carries no reading at all.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The day's recovery row once `readings` land on `existing`.
///
/// `existing` is the row already stored for that day under `garmin`, if any:
/// every reading the push carries replaces the stored one, every other stored
/// reading stays.
///
/// Body Battery fills the recovery and readiness scores as the usr/pwd
/// connection's rows do, so a tool reads both connections' rows alike.
#[must_use]
pub(crate) fn merged_recovery(
    existing: Option<StoredRecoveryMetrics>,
    readings: RecoveryReadings,
    user_id: &str,
    data_source_id: &str,
    date: NaiveDate,
) -> StoredRecoveryMetrics {
    let mut row = existing.unwrap_or_else(|| StoredRecoveryMetrics {
        id: String::new(),
        user_id: user_id.to_owned(),
        data_source_id: data_source_id.to_owned(),
        date,
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: GARMIN.to_owned(),
        recorded_at: date.and_time(NaiveTime::MIN).and_utc(),
    });
    if readings.resting_heart_rate.is_some() {
        row.resting_heart_rate = readings.resting_heart_rate;
    }
    if readings.stress.is_some() {
        row.stress_score = readings.stress;
    }
    if readings.body_battery.is_some() {
        row.body_battery = readings.body_battery;
        row.recovery_score = readings.body_battery;
        row.readiness_score = readings.body_battery;
    }
    if readings.hrv_rmssd.is_some() {
        row.hrv_ms = readings.hrv_rmssd;
        row.hrv_rmssd = readings.hrv_rmssd;
    }
    if readings.skin_temp_deviation.is_some() {
        row.skin_temp_deviation = readings.skin_temp_deviation;
    }
    row
}

/// The measurements one push brings to a day's body row.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct BodyReadings {
    /// Weight, kg (`bodyComps`).
    pub(crate) weight_kg: Option<f64>,
    /// Body fat, percent (`bodyComps`).
    pub(crate) body_fat_pct: Option<f64>,
    /// Body mass index (`bodyComps`).
    pub(crate) bmi: Option<f64>,
    /// Muscle mass, kg (`bodyComps`).
    pub(crate) muscle_mass_kg: Option<f64>,
    /// Bone mass, kg (`bodyComps`).
    pub(crate) bone_mass_kg: Option<f64>,
    /// Body water, percent (`bodyComps`).
    pub(crate) water_pct: Option<f64>,
    /// `VO2max`, ml/kg/min (`userMetrics`).
    pub(crate) vo2_max: Option<f64>,
}

impl BodyReadings {
    /// Whether the push carries no measurement at all.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The day's body row once `readings` land on `existing`, as
/// [`merged_recovery`] merges a recovery row.
#[must_use]
pub(crate) fn merged_body(
    existing: Option<StoredHealthMetrics>,
    readings: BodyReadings,
    user_id: &str,
    data_source_id: &str,
    date: NaiveDate,
) -> StoredHealthMetrics {
    let mut row = existing.unwrap_or_else(|| StoredHealthMetrics {
        id: String::new(),
        user_id: user_id.to_owned(),
        data_source_id: data_source_id.to_owned(),
        date,
        weight_kg: None,
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: None,
        vo2_max: None,
        source_name: GARMIN.to_owned(),
        recorded_at: date.and_time(NaiveTime::MIN).and_utc(),
    });
    let fill = |slot: &mut Option<f64>, value: Option<f64>| {
        if value.is_some() {
            *slot = value;
        }
    };
    fill(&mut row.weight_kg, readings.weight_kg);
    fill(&mut row.body_fat_pct, readings.body_fat_pct);
    fill(&mut row.bmi, readings.bmi);
    fill(&mut row.muscle_mass_kg, readings.muscle_mass_kg);
    fill(&mut row.bone_mass_kg, readings.bone_mass_kg);
    fill(&mut row.water_pct, readings.water_pct);
    fill(&mut row.vo2_max, readings.vo2_max);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::garmin_push::wire::{LevelRange, SleepScore};

    const START: i64 = 1_760_000_000;

    fn summary(overrides: impl FnOnce(&mut ActivitySummary)) -> ActivitySummary {
        let mut summary = ActivitySummary {
            activity_id: Some("900001".to_owned()),
            activity_name: Some("Lunch tempo".to_owned()),
            activity_type: Some("ROAD_BIKING".to_owned()),
            start_time_in_seconds: Some(START),
            duration_in_seconds: Some(3_600),
            distance_in_meters: Some(30_000.0),
            average_heart_rate_in_beats_per_minute: Some(141.0),
            device_name: Some("Edge 1050".to_owned()),
            ..ActivitySummary::default()
        };
        overrides(&mut summary);
        summary
    }

    fn sample(offset: i64, hr: f64, distance: Option<f64>) -> ActivitySample {
        ActivitySample {
            start_time_in_seconds: Some(START + offset),
            heart_rate: Some(hr),
            total_distance_in_meters: distance,
            latitude_in_degree: Some(45.5),
            longitude_in_degree: Some(-73.6),
            ..ActivitySample::default()
        }
    }

    #[test]
    fn a_summary_becomes_a_branded_garmin_activity_keyed_by_its_activity_id() {
        let activity = activity_from_summary(None, &summary(|_| {})).unwrap();
        assert_eq!(
            activity.id(),
            "900001",
            "the cached row is keyed by Garmin's activityId"
        );
        assert_eq!(
            activity.provider(),
            GARMIN,
            "a pushed activity is read under garmin"
        );
        assert_eq!(
            activity.source(),
            Some(GARMIN),
            "a pushed activity names garmin as its source"
        );
        assert_eq!(
            activity.device_name(),
            Some("Garmin Edge 1050"),
            "the device name is branded"
        );
        assert_eq!(
            *activity.sport_type(),
            SportType::Ride,
            "ROAD_BIKING reads as a ride"
        );
        assert_eq!(
            activity.duration_seconds(),
            3_600,
            "the duration is the summary's"
        );
        assert_eq!(
            activity.average_heart_rate(),
            Some(141),
            "the average heart rate is the summary's"
        );
    }

    #[test]
    fn manual_web_uploaded_and_multisport_parent_summaries_are_skipped() {
        let manual = summary(|s| s.manual = Some(true));
        let web = summary(|s| s.is_web_upload = Some(true));
        let parent = summary(|s| s.is_parent = Some(true));
        let no_start = summary(|s| s.start_time_in_seconds = None);
        assert_eq!(
            activity_from_summary(None, &manual).unwrap_err(),
            Skipped::Manual,
            "a manual entry is skipped"
        );
        assert_eq!(
            activity_from_summary(None, &web).unwrap_err(),
            Skipped::WebUpload,
            "a web upload is skipped"
        );
        assert_eq!(
            activity_from_summary(None, &parent).unwrap_err(),
            Skipped::MultisportParent,
            "a multisport parent is skipped"
        );
        assert_eq!(
            activity_from_summary(None, &no_start).unwrap_err(),
            Skipped::Incomplete,
            "a summary with no start is incomplete"
        );
    }

    #[test]
    fn a_multisport_leg_is_kept_as_its_own_activity() {
        let leg: ActivitySummary = serde_json::from_value(serde_json::json!({
            "activityId": 900_002,
            "activityType": "RUNNING",
            "startTimeInSeconds": START,
            "durationInSeconds": 1_800,
            "parentSummaryId": 900_000,
        }))
        .expect("a multisport leg's summary reads");
        assert!(
            activity_from_summary(None, &leg).is_ok(),
            "a multisport leg is kept"
        );
    }

    #[test]
    fn samples_are_timed_from_the_start_and_gaps_stay_gaps() {
        let mut gap = sample(2, 120.0, Some(10.0));
        gap.heart_rate = None;
        let samples = vec![
            sample(0, 118.0, Some(0.0)),
            gap,
            sample(4, 125.0, Some(20.0)),
        ];
        let streams = streams_from_samples(START, &samples).unwrap();
        assert_eq!(
            streams.timestamps,
            vec![0, 2, 4],
            "timestamps are seconds from the start"
        );
        assert_eq!(
            streams.heart_rate,
            Some(vec![Some(118), None, Some(125)]),
            "a missing reading stays a gap"
        );
        assert_eq!(
            streams.distance,
            Some(vec![Some(0.0), Some(10.0), Some(20.0)]),
            "distance follows the samples"
        );
        assert_eq!(
            streams.gps_coordinates.as_ref().map(Vec::len),
            Some(3),
            "one fix per sample"
        );
        assert_eq!(streams.power, None, "no sample carries power");
    }

    #[test]
    fn a_sample_without_a_fix_keeps_its_slot_without_one() {
        let at = |offset: i64, lat: Option<f64>| ActivitySample {
            latitude_in_degree: lat,
            ..sample(offset, 120.0, Some(0.0))
        };
        let samples = [
            at(0, None),
            at(1, Some(45.0)),
            at(2, None),
            at(3, Some(46.0)),
        ];
        let streams = streams_from_samples(START, &samples).unwrap();
        let lats: Vec<Option<f64>> = streams
            .gps_coordinates
            .expect("the route is kept")
            .iter()
            .map(|p| p.map(|fix| fix.0))
            .collect();
        assert_eq!(
            lats,
            vec![None, Some(45.0), None, Some(46.0)],
            "index-aligned, a missing fix is a gap, never a copied one"
        );

        let none = [at(0, None), at(1, None)];
        assert_eq!(
            streams_from_samples(START, &none).unwrap().gps_coordinates,
            None,
            "no fix at all: no route"
        );
    }

    #[test]
    fn laps_run_from_each_start_to_the_next_and_the_last_to_the_end() {
        let samples: Vec<ActivitySample> = (0..10)
            .map(|i| {
                sample(
                    i * 10,
                    100.0 + f64::from(i32::try_from(i).unwrap()),
                    Some(i as f64 * 50.0),
                )
            })
            .collect();
        let laps = vec![
            ActivityLap {
                start_time_in_seconds: Some(START),
            },
            ActivityLap {
                start_time_in_seconds: Some(START + 50),
            },
        ];
        let built = laps_from_details(START, 100, &laps, &samples).unwrap();
        assert_eq!(built.len(), 2, "one lap per lap start");
        assert_eq!(built[0].index, 1, "laps are numbered from one");
        assert_eq!(
            built[0].elapsed_time_seconds, 50,
            "a lap runs to the next lap start"
        );
        assert_eq!(
            built[1].elapsed_time_seconds, 50,
            "the last lap runs to the activity end"
        );
        assert!(
            (built[0].distance_meters - 200.0).abs() < f64::EPSILON,
            "a lap's distance is the distance covered inside it"
        );
        assert!(
            laps_from_details(START, 100, &[], &samples).is_none(),
            "no lap starts, no laps"
        );
    }

    #[test]
    fn a_sleep_keeps_its_start_stages_and_score() {
        let mut levels = BTreeMap::new();
        levels.insert(
            "deep".to_owned(),
            vec![LevelRange {
                start_time_in_seconds: Some(START),
                end_time_in_seconds: Some(START + 3_600),
            }],
        );
        levels.insert(
            "rem".to_owned(),
            vec![LevelRange {
                start_time_in_seconds: Some(START + 3_600),
                end_time_in_seconds: Some(START + 5_400),
            }],
        );
        let record = SleepRecord {
            start_time_in_seconds: Some(START),
            duration_in_seconds: Some(5_400),
            deep_sleep_duration_in_seconds: Some(3_600),
            rem_sleep_in_seconds: Some(1_800),
            sleep_levels_map: levels,
            overall_sleep_score: Some(SleepScore { value: Some(81.0) }),
            ..SleepRecord::default()
        };
        let session = sleep_session(&record, "user", "source").unwrap();
        assert_eq!(
            session.start_datetime.timestamp(),
            START,
            "the night starts at the record's start"
        );
        assert_eq!(
            session.end_datetime.timestamp(),
            START + 5_400,
            "the night ends start plus duration"
        );
        assert_eq!(session.stages.len(), 2, "one stage per level range");
        assert_eq!(
            session.stages[0].stage_type,
            StoredSleepStageType::Deep,
            "stages are in time order"
        );
        assert_eq!(
            session.sleep_score,
            Some(81),
            "the overall sleep score is kept"
        );
        assert_eq!(
            session.total_sleep_seconds,
            Some(5_400),
            "total sleep is the record's duration"
        );
    }

    #[test]
    fn offset_points_drop_garmin_no_reading_codes() {
        let mut series = OffsetSeries::new();
        series.insert("0".to_owned(), 30.0);
        series.insert("180".to_owned(), -2.0);
        series.insert("360".to_owned(), 45.0);
        let points = offset_points(START, &series, measured);
        assert_eq!(points.len(), 2, "a negative no-reading code is dropped");
        assert_eq!(
            points[1].timestamp.timestamp(),
            START + 360,
            "a point is timed by its offset from the start"
        );
    }

    #[test]
    fn a_recovery_push_keeps_what_an_earlier_push_stored() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let after_dailies = merged_recovery(
            None,
            RecoveryReadings {
                resting_heart_rate: Some(48),
                stress: Some(22),
                ..RecoveryReadings::default()
            },
            "user",
            "source",
            date,
        );
        let after_hrv = merged_recovery(
            Some(after_dailies),
            RecoveryReadings {
                hrv_rmssd: Some(61.0),
                ..RecoveryReadings::default()
            },
            "user",
            "source",
            date,
        );
        assert_eq!(
            after_hrv.resting_heart_rate,
            Some(48),
            "the dailies reading stays after an hrv push"
        );
        assert_eq!(
            after_hrv.stress_score,
            Some(22),
            "the dailies stress stays after an hrv push"
        );
        assert_eq!(
            after_hrv.hrv_rmssd,
            Some(61.0),
            "the hrv push lands its rMSSD"
        );
        assert_eq!(after_hrv.hrv_ms, Some(61.0), "hrv_ms mirrors rMSSD");
    }

    #[test]
    fn a_body_push_keeps_what_an_earlier_push_stored() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let weighed = merged_body(
            None,
            BodyReadings {
                weight_kg: Some(71.4),
                ..BodyReadings::default()
            },
            "user",
            "source",
            date,
        );
        let both = merged_body(
            Some(weighed),
            BodyReadings {
                vo2_max: Some(55.0),
                ..BodyReadings::default()
            },
            "user",
            "source",
            date,
        );
        assert_eq!(both.weight_kg, Some(71.4), "the earlier weight stays");
        assert_eq!(both.vo2_max, Some(55.0), "the later VO2max lands");
    }

    #[test]
    fn garmin_type_keys_read_as_the_usr_pwd_connection_reads_them() {
        assert_eq!(
            sport_type(Some("RUNNING")),
            SportType::Run,
            "RUNNING reads as a run"
        );
        assert_eq!(
            sport_type(Some("TRAIL_RUNNING")),
            SportType::TrailRunning,
            "TRAIL_RUNNING reads as a trail run"
        );
        assert_eq!(
            sport_type(Some("LAP_SWIMMING")),
            SportType::Swim,
            "LAP_SWIMMING reads as a swim"
        );
        assert_eq!(
            sport_type(Some("INDOOR_CYCLING")),
            SportType::VirtualRide,
            "INDOOR_CYCLING reads as a virtual ride"
        );
    }

    /// Samples a second apart for `seconds`, 150 W and 250 W in 30 s blocks.
    fn powered(seconds: i64) -> Vec<ActivitySample> {
        (0..seconds)
            .map(|offset| ActivitySample {
                power_in_watts: Some(if (offset / 30) % 2 == 0 { 150.0 } else { 250.0 }),
                ..sample(offset, 140.0, None)
            })
            .collect()
    }

    #[test]
    fn a_rides_power_is_derived_from_its_samples_with_cageuxs_normalized_power() {
        let streams = streams_from_samples(START, &powered(120)).unwrap();
        let figures = power_figures(&streams);
        assert_eq!(
            figures.average,
            Some(200),
            "bug: the mean of 150 W and 250 W blocks"
        );
        assert_eq!(figures.max, Some(250), "bug: the highest sample");
        let expected =
            NormalizedPowerCalculator::from_power_stream(streams.power.as_deref().unwrap())
                .unwrap();
        assert_eq!(
            figures.normalized,
            whole(Some(expected)),
            "bug: normalized power is cageux's, not one re-derived here"
        );
        assert!(
            figures.normalized > figures.average,
            "bug: a ride alternating 150 W and 250 W has an NP above its average"
        );
    }

    #[test]
    fn the_samples_power_is_set_on_the_activity_and_samples_without_it_leave_it_alone() {
        let activity = activity_from_summary(None, &summary(|_| {})).unwrap();
        assert_eq!(
            activity.normalized_power(),
            None,
            "a summary carries no power"
        );

        let streams = streams_from_samples(START, &powered(120)).unwrap();
        let figures = power_figures(&streams);
        let ride = with_power(activity.clone(), &streams).unwrap();
        assert_eq!(
            ride.average_power(),
            figures.average,
            "bug: the samples' mean power is set on the activity"
        );
        assert_eq!(
            ride.max_power(),
            figures.max,
            "bug: the samples' highest power is set on the activity"
        );
        assert_eq!(
            ride.normalized_power(),
            figures.normalized,
            "bug: the samples' normalized power is set on the activity"
        );
        assert_eq!(
            ride.id(),
            activity.id(),
            "bug: the rest of the activity stands"
        );
        assert_eq!(
            ride.average_heart_rate(),
            activity.average_heart_rate(),
            "bug: setting power leaves the heart rate standing"
        );

        let unpowered =
            streams_from_samples(START, &[sample(0, 140.0, None), sample(1, 141.0, None)]).unwrap();
        assert_eq!(
            power_figures(&unpowered),
            PowerFigures::default(),
            "bug: samples without power give no power figures"
        );
        let unchanged = with_power(activity, &unpowered).unwrap();
        assert_eq!(
            unchanged.average_power(),
            None,
            "bug: samples without power set none"
        );
    }

    #[test]
    fn a_ride_shorter_than_one_window_has_an_average_but_no_normalized_power() {
        let streams = streams_from_samples(START, &powered(20)).unwrap();
        let figures = power_figures(&streams);
        assert_eq!(
            figures.average,
            Some(150),
            "bug: 20 s at 150 W average 150 W"
        );
        assert_eq!(
            figures.normalized, None,
            "bug: NP needs one 30-second window"
        );
    }
}
