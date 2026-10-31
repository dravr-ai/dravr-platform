// ABOUTME: Row decoders for the health persistence tables: data sources, nights, recovery, body and activity days
// ABOUTME: Each reads columns by name through try_get, so a corrupt row is an error naming the column, never a panic

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Decoding the rows [`super`]'s statements read. sqlx decodes on both
//! drivers; every reader goes through [`column`].

use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DataSource, StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, TenantId,
};

use super::str_to_device_type;
use crate::repositories::WellnessRowOrigin;

/// Read one column by name via `try_get` — never `Row::get`, which is
/// `try_get().unwrap()` and panics the read path on a type or NULL surprise —
/// so a corrupt row surfaces as a recoverable error naming the column.
pub(crate) fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    T: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    row.try_get(name)
        .map_err(|e| AppError::database(format!("health column {name}: {e}")))
}

/// Decode a `data_sources` row.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn data_source_from_row<R>(row: &R) -> AppResult<DataSource>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let device_type_str: String = column(row, "device_type")?;
    Ok(DataSource {
        id: column(row, "id")?,
        user_id: column(row, "user_id")?,
        provider: column(row, "provider")?,
        device_model: column(row, "device_model")?,
        software_version: column(row, "software_version")?,
        source: column(row, "source")?,
        device_type: str_to_device_type(&device_type_str),
        original_source_name: column(row, "original_source_name")?,
    })
}

/// Decode a `sleep_sessions` row into the stored session shape. The
/// per-stage seconds and heart-rate fields are not columns on this table and
/// come back `None`; the stages themselves are in `stages_json`.
///
/// # Errors
/// Returns a database error when a column cannot be decoded or
/// `stages_json` is not the serialised stage list.
pub(crate) fn sleep_session_from_row<R>(row: &R) -> AppResult<StoredSleepSession>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let stages_json_str: String = column(row, "stages_json")?;
    let is_nap_int: i32 = column(row, "is_nap")?;
    let total_sleep_time: Option<i64> = column(row, "total_sleep_time")?;
    let sleep_efficiency: Option<f64> = column(row, "sleep_efficiency")?;
    let sleep_score: Option<f64> = column(row, "sleep_score")?;
    let seconds = |name: &str| -> AppResult<Option<u32>> {
        let value: Option<i32> = column(row, name)?;
        Ok(value.map(i32::cast_unsigned))
    };

    let stages = serde_json::from_str(&stages_json_str)
        .map_err(|e| AppError::database(format!("Invalid stages_json: {e}")))?;

    Ok(StoredSleepSession {
        id: column(row, "id")?,
        user_id: column(row, "user_id")?,
        data_source_id: column::<_, Option<String>>(row, "data_source_id")?.unwrap_or_default(),
        is_nap: is_nap_int != 0,
        start_datetime: column(row, "start_time")?,
        end_datetime: column(row, "end_time")?,
        // Both columns are NOT NULL and the write stores 0 for a metric the
        // record lacked; no real session sleeps 0 s or at 0 % efficiency, so a
        // 0 reads back as absent and a merge can fill it from another source.
        total_sleep_seconds: total_sleep_time.filter(|v| *v > 0).map(|v| v as u32),
        deep_sleep_seconds: seconds("deep_sleep_seconds")?,
        light_sleep_seconds: seconds("light_sleep_seconds")?,
        rem_sleep_seconds: seconds("rem_sleep_seconds")?,
        awake_seconds: seconds("awake_seconds")?,
        sleep_efficiency: sleep_efficiency.filter(|v| *v > 0.0),
        avg_heart_rate: column(row, "avg_heart_rate")?,
        min_heart_rate: seconds("min_heart_rate")?,
        avg_hrv: column(row, "hrv_during_sleep")?,
        sleep_score: sleep_score.map(|v| v as u32),
        avg_spo2: column(row, "avg_spo2")?,
        stages,
        source_name: column(row, "provider")?,
    })
}

/// Decode a `recovery_metrics` row. `resting_heart_rate` is an `INTEGER`
/// column on both backends and is read as `i32`: Postgres's strict decode
/// rejects an `i64` read of `INT4`, and `SQLite` widens either way.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn recovery_metrics_from_row<R>(row: &R) -> AppResult<StoredRecoveryMetrics>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let recovery_score: Option<f64> = column(row, "recovery_score")?;
    let readiness_score: Option<f64> = column(row, "readiness_score")?;
    let stress_level: Option<f64> = column(row, "stress_level")?;
    let resting_heart_rate: Option<i32> = column(row, "resting_heart_rate")?;
    let body_battery: Option<i32> = column(row, "body_battery")?;

    Ok(StoredRecoveryMetrics {
        id: column(row, "id")?,
        user_id: column(row, "user_id")?,
        data_source_id: column::<_, Option<String>>(row, "data_source_id")?.unwrap_or_default(),
        date: column(row, "date")?,
        recovery_score: recovery_score.map(|v| v as u32),
        readiness_score: readiness_score.map(|v| v as u32),
        hrv_ms: column(row, "hrv_ms")?,
        hrv_rmssd: column(row, "hrv_rmssd")?,
        resting_heart_rate: resting_heart_rate.map(i32::cast_unsigned),
        stress_score: stress_level.map(|v| v as u32),
        body_battery: body_battery.map(i32::cast_unsigned),
        spo2: column(row, "spo2")?,
        respiratory_rate: column(row, "resting_respiratory_rate")?,
        skin_temp_deviation: column(row, "body_temperature")?,
        // `training_load` is the provider's day load score. WHOOP's day strain,
        // the one such score a sync adapter maps, is withheld before storage.
        daily_strain: column(row, "training_load")?,
        athlete_note: column(row, "athlete_note")?,
        source_name: column(row, "provider")?,
        recorded_at: column(row, "created_at")?,
    })
}

/// Decode a `health_snapshots` row. The blood-pressure columns are
/// `INTEGER` on both backends and are read as `i32` for the same reason as
/// [`recovery_metrics_from_row`]'s heart rate.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn health_metrics_from_row<R>(row: &R) -> AppResult<StoredHealthMetrics>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let bp_systolic: Option<i32> = column(row, "bp_systolic")?;
    let bp_diastolic: Option<i32> = column(row, "bp_diastolic")?;

    Ok(StoredHealthMetrics {
        id: column(row, "id")?,
        user_id: column(row, "user_id")?,
        data_source_id: column::<_, Option<String>>(row, "data_source_id")?.unwrap_or_default(),
        date: column(row, "date")?,
        weight_kg: column(row, "weight")?,
        body_fat_pct: column(row, "body_fat_percentage")?,
        muscle_mass_kg: column(row, "muscle_mass")?,
        bmi: column(row, "bmi")?,
        bone_mass_kg: column(row, "bone_mass")?,
        water_pct: column(row, "body_water_percentage")?,
        systolic_bp: bp_systolic.map(i32::cast_unsigned),
        diastolic_bp: bp_diastolic.map(i32::cast_unsigned),
        blood_glucose: column(row, "blood_glucose")?,
        vo2_max: column(row, "vo2_max")?,
        source_name: column(row, "provider")?,
        recorded_at: column(row, "created_at")?,
    })
}

/// Decode a `daily_activity` row. The counts are `INTEGER` columns on both
/// backends and are read as `i32` for the same reason as
/// [`recovery_metrics_from_row`]'s heart rate.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn daily_activity_from_row<R>(row: &R) -> AppResult<StoredDailyActivity>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let steps: Option<i32> = column(row, "steps")?;
    let intensity_minutes: Option<i32> = column(row, "intensity_minutes")?;

    Ok(StoredDailyActivity {
        id: column(row, "id")?,
        user_id: column(row, "user_id")?,
        data_source_id: column::<_, Option<String>>(row, "data_source_id")?.unwrap_or_default(),
        date: column(row, "date")?,
        steps: steps.map(i32::cast_unsigned),
        active_kcal: column(row, "active_kcal")?,
        total_kcal: column(row, "total_kcal")?,
        intensity_minutes: intensity_minutes.map(i32::cast_unsigned),
        source_name: column(row, "provider")?,
        recorded_at: column(row, "created_at")?,
    })
}

/// Decode the `tenant_id` of a health row into the [`TenantId`] newtype.
///
/// # Errors
/// Returns a database error when the column is missing or not a uuid.
pub(crate) fn tenant_from_row<R>(row: &R) -> AppResult<TenantId>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let s: String = column(row, "tenant_id")?;
    TenantId::parse_str(&s)
        .map_err(|e| AppError::database(format!("Invalid tenant_id stored: {e}")))
}

/// Read a [`WellnessRowOrigin`] from a row of the `*_ORIGINS_SQL` reads.
///
/// # Errors
/// Returns a database error when a column is missing or not text.
pub(crate) fn origin_from_row<R>(row: &R) -> AppResult<WellnessRowOrigin>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(WellnessRowOrigin {
        provider: column(row, "provider")?,
        data_source_id: column(row, "data_source_id")?,
    })
}
