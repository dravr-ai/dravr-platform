// ABOUTME: Garmin health pushes into the wellness store — sleep, the day's recovery and body rows, intraday series
// ABOUTME: A day's row takes only what each summary carries, since several summary types feed it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Health pushes.
//!
//! Rows are stored under the provider `garmin`, the name the usr/pwd
//! connection's health sync stores Garmin rows under too, so every tool reads
//! them as it reads that connection's; they are filed under the API's own
//! data source, so a purge of the API's wellness leaves that connection's.
//! Everything is written through the one wellness writer,
//! [`crate::wellness_store::WellnessStore`]. A night is written whole and
//! keeps the latest push's values even when an older push is retried after
//! it; a day's row is written as a
//! [`WellnessWriteMode::PartialSummary`], reading by reading, since dailies,
//! HRV, stress, skin temperature and body composition each arrive alone.
//! Series points go to `data_point_series` under the owner's Garmin data
//! source, keyed by the riviere series type, which is where the
//! continuous-series read finds intraday heart rate and rMSSD.

use chrono::{DateTime, NaiveDate, Utc};
use dravr_riviere::{DataPoint, SeriesType};
use pierre_core::errors::AppResult;
use serde_json::Value;

use super::mapping::{
    body_row, calendar_date, measured, offset_points, recovery_row, sleep_session, BodyReadings,
    RecoveryReadings,
};
use super::wire::{
    BodyCompRecord, DailyRecord, HrvRecord, SkinTempRecord, SleepRecord, StressDetailsRecord,
    UserMetricsRecord,
};
use super::{records_as, BatchOutcome, OwnerBatch};
use crate::wellness_store::{WellnessLanded, WellnessStore, WellnessWriteMode};

/// Write `readings` onto the owner's Garmin recovery row for `date`, filed
/// under the API's data source: what the push carries replaces the stored
/// reading, everything else stays.
async fn land_recovery(
    batch: &OwnerBatch<'_>,
    source_id: &str,
    date: NaiveDate,
    readings: RecoveryReadings,
) -> AppResult<()> {
    let row = recovery_row(readings, &batch.owner.user_id.to_string(), source_id, date);
    WellnessStore::new(batch.repos)
        .writes("garmin push recovery")
        .recovery(
            batch.owner.tenant_id,
            &row,
            WellnessWriteMode::PartialSummary,
            batch.received_at,
        )
        .await
        .map(|_| ())
}

/// Write `readings` onto the owner's Garmin body row for `date`, as
/// [`land_recovery`] writes a recovery row.
async fn land_body(
    batch: &OwnerBatch<'_>,
    source_id: &str,
    date: NaiveDate,
    readings: BodyReadings,
) -> AppResult<()> {
    let row = body_row(readings, &batch.owner.user_id.to_string(), source_id, date);
    WellnessStore::new(batch.repos)
        .writes("garmin push body")
        .body(
            batch.owner.tenant_id,
            &row,
            WellnessWriteMode::PartialSummary,
            batch.received_at,
        )
        .await
        .map(|_| ())
}

/// Store `points` as the `series` of the owner's Garmin data source.
async fn store_series(
    batch: &OwnerBatch<'_>,
    source_id: &str,
    series: SeriesType,
    points: Vec<DataPoint>,
) -> AppResult<()> {
    WellnessStore::new(batch.repos)
        .store_series(source_id, series.id(), points)
        .await
}

/// A reading as a whole non-negative number.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole(value: Option<f64>) -> Option<u32> {
    value
        .filter(|v| v.is_finite() && *v >= 0.0 && *v <= f64::from(u32::MAX))
        .map(|v| v.round() as u32)
}

/// `dailies`: resting heart rate and stress on the day's recovery row, the
/// day's heart-rate samples as a series.
pub(super) async fn write_dailies(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let dailies: Vec<DailyRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    for daily in &dailies {
        let Some(date) = calendar_date(daily.calendar_date.as_deref()) else {
            outcome.unreadable += 1;
            continue;
        };
        let readings = RecoveryReadings {
            resting_heart_rate: whole(daily.resting_heart_rate_in_beats_per_minute),
            stress: whole(daily.average_stress_level),
            ..RecoveryReadings::default()
        };
        if !readings.is_empty() {
            land_recovery(batch, &source_id, date, readings).await?;
        }
        if let Some(start) = daily.start_time_in_seconds {
            let points = offset_points(start, &daily.time_offset_heart_rate_samples, measured);
            store_series(batch, &source_id, SeriesType::HeartRate, points).await?;
        }
        outcome.stored += 1;
    }
    Ok(outcome)
}

/// `sleeps`: one session per sleep, replaced by each update of the same
/// night. Its intraday `SpO2` and breathing-rate readings are not kept: no
/// reader serves them.
pub(super) async fn write_sleeps(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let sleeps: Vec<SleepRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    let user = batch.owner.user_id.to_string();
    for sleep in &sleeps {
        let Some(session) = sleep_session(sleep, &user, &source_id) else {
            outcome.unreadable += 1;
            continue;
        };
        let landed = WellnessStore::new(batch.repos)
            .writes("garmin push sleep")
            .sleep(batch.owner.tenant_id, &session, batch.received_at)
            .await?;
        if landed != WellnessLanded::Written {
            // A later push of the same night already landed; its readings stand.
            outcome.skipped += 1;
            continue;
        }
        outcome.stored += 1;
    }
    Ok(outcome)
}

/// `hrv`: the night's average rMSSD on the day's recovery row, its readings
/// as the rMSSD series.
pub(super) async fn write_hrv(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let nights: Vec<HrvRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    for night in &nights {
        let Some(date) = calendar_date(night.calendar_date.as_deref()) else {
            outcome.unreadable += 1;
            continue;
        };
        let rmssd = night.last_night_avg.filter(|v| v.is_finite() && *v > 0.0);
        if rmssd.is_some() {
            let readings = RecoveryReadings {
                hrv_rmssd: rmssd,
                ..RecoveryReadings::default()
            };
            land_recovery(batch, &source_id, date, readings).await?;
        }
        if let Some(start) = night.start_time_in_seconds {
            let points = offset_points(start, &night.hrv_values, measured);
            store_series(batch, &source_id, SeriesType::HrvRmssd, points).await?;
        }
        outcome.stored += 1;
    }
    Ok(outcome)
}

/// `userMetrics`: the `VO2max` estimate on the day's body row, the running
/// estimate first and the cycling one when it is the only one.
pub(super) async fn write_user_metrics(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let metrics: Vec<UserMetricsRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    for metric in &metrics {
        let Some(date) = calendar_date(metric.calendar_date.as_deref()) else {
            outcome.unreadable += 1;
            continue;
        };
        let vo2_max = metric
            .vo2_max
            .or(metric.vo2_max_cycling)
            .filter(|v| v.is_finite() && *v > 0.0);
        if vo2_max.is_none() {
            outcome.skipped += 1;
            continue;
        }
        let readings = BodyReadings {
            vo2_max,
            ..BodyReadings::default()
        };
        land_body(batch, &source_id, date, readings).await?;
        outcome.stored += 1;
    }
    Ok(outcome)
}

/// `bodyComps`: the measurement on the body row of the day it was taken on,
/// in the device's local time; grams read as kilograms.
pub(super) async fn write_body_comps(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let measurements: Vec<BodyCompRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    let kilograms = |grams: Option<f64>| {
        grams
            .filter(|g| g.is_finite() && *g > 0.0)
            .map(|g| g / 1000.0)
    };
    for measurement in &measurements {
        let Some(date) = measurement.measurement_time_in_seconds.and_then(|at| {
            let local =
                at.checked_add(measurement.measurement_time_offset_in_seconds.unwrap_or(0))?;
            DateTime::<Utc>::from_timestamp(local, 0).map(|instant| instant.date_naive())
        }) else {
            outcome.unreadable += 1;
            continue;
        };
        let readings = BodyReadings {
            weight_kg: kilograms(measurement.weight_in_grams),
            body_fat_pct: measurement.body_fat_in_percent,
            bmi: measurement.body_mass_index,
            muscle_mass_kg: kilograms(measurement.muscle_mass_in_grams),
            bone_mass_kg: kilograms(measurement.bone_mass_in_grams),
            water_pct: measurement.body_water_in_percent,
            vo2_max: None,
        };
        if readings.is_empty() {
            outcome.skipped += 1;
            continue;
        }
        land_body(batch, &source_id, date, readings).await?;
        outcome.stored += 1;
    }
    Ok(outcome)
}

/// `stressDetails`: the day's highest Body Battery on its recovery row. The
/// intraday stress readings are not kept: no reader serves them.
pub(super) async fn write_stress(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let days: Vec<StressDetailsRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    for stress in &days {
        let Some(date) = calendar_date(stress.calendar_date.as_deref()) else {
            outcome.unreadable += 1;
            continue;
        };
        let battery = stress
            .time_offset_body_battery_values
            .values()
            .copied()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .fold(None, |best: Option<f64>, v| {
                Some(best.map_or(v, |b| b.max(v)))
            });
        if let Some(battery) = whole(battery) {
            let readings = RecoveryReadings {
                body_battery: Some(battery),
                ..RecoveryReadings::default()
            };
            land_recovery(batch, &source_id, date, readings).await?;
        }
        outcome.stored += 1;
    }
    Ok(outcome)
}

/// `skinTemp`: the night's deviation from the athlete's baseline on the
/// day's recovery row, where the skin-temperature deviation is read. It is
/// a deviation, not a temperature, so it is not stored as the riviere
/// skin-temperature series.
pub(super) async fn write_skin_temp(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let nights: Vec<SkinTempRecord> = records_as(records, &mut outcome);
    let source_id = batch.data_source().await?;
    for night in &nights {
        let (Some(date), Some(deviation)) = (
            calendar_date(night.calendar_date.as_deref()),
            night.avg_deviation_celsius.filter(|v| v.is_finite()),
        ) else {
            outcome.unreadable += 1;
            continue;
        };
        let readings = RecoveryReadings {
            skin_temp_deviation: Some(deviation),
            ..RecoveryReadings::default()
        };
        land_recovery(batch, &source_id, date, readings).await?;
        outcome.stored += 1;
    }
    Ok(outcome)
}
