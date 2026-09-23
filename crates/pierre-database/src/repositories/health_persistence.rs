// ABOUTME: Shared statements, row decoders and body for data sources, sleep sessions, recovery metrics and health snapshots
// ABOUTME: One SQL text per operation; each backend shell supplies only its type
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Health persistence, written once.
//!
//! Four traits from [`super::health`] share these tables: `data_sources`,
//! `sleep_sessions`, `recovery_metrics` and `health_snapshots`. Every id on
//! them is a `TEXT` column on both backends, so ids bind as plain strings.
//! The sync cursors and the riviere time-series store that complete the
//! health domain live in [`super::sync_cursors`] and [`super::time_series`],
//! which reuse this module's [`column`] reader.
//!
//! Timestamps bind as [`DateTime<Utc>`] on both: sqlx-sqlite encodes one as
//! `to_rfc3339_opts(AutoSi, false)`, which is byte-for-byte what `to_rfc3339()`
//! produces, so the `TEXT` columns keep the bytes they always held and the
//! `start_time >= $3` string comparisons order as before, while Postgres gets
//! its native `TIMESTAMPTZ`. Calendar dates bind and decode as a [`NaiveDate`],
//! which sqlx writes as `YYYY-MM-DD` text on `SQLite` and a native `DATE` on
//! Postgres. Decoding goes through sqlx on both drivers, which accepts the
//! RFC 3339 text this module writes as well as the `datetime('now')` shape the
//! `SQLite` column defaults produce.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them; nothing on these four tables is spelled differently per engine.

use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DataSource, DeviceType, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
    TenantId,
};

// ============================================================================
// DeviceType <-> text
// ============================================================================

/// The text `data_sources.device_type` stores for each [`DeviceType`].
pub(crate) const fn device_type_to_str(dt: DeviceType) -> &'static str {
    match dt {
        DeviceType::Watch => "watch",
        DeviceType::Band => "band",
        DeviceType::Phone => "phone",
        DeviceType::Ring => "ring",
        DeviceType::Scale => "scale",
        DeviceType::Unknown => "unknown",
    }
}

/// The inverse of [`device_type_to_str`]; anything unrecognised is `Unknown`.
pub(crate) fn str_to_device_type(s: &str) -> DeviceType {
    match s.to_lowercase().as_str() {
        "watch" => DeviceType::Watch,
        "band" => DeviceType::Band,
        "phone" => DeviceType::Phone,
        "ring" => DeviceType::Ring,
        "scale" => DeviceType::Scale,
        _ => DeviceType::Unknown,
    }
}

// ============================================================================
// data_sources
// ============================================================================

/// Insert or refresh a data source. The conflict target matches
/// `idx_data_sources_identity` (NULL-coalesced, so provider-level sources
/// without device metadata dedupe). `RETURNING id` yields the row's actual
/// id — on conflict that is the EXISTING id, not the freshly generated one,
/// so foreign keys stamped from this value always resolve.
pub(crate) const UPSERT_DATA_SOURCE_SQL: &str = r"
            INSERT INTO data_sources (id, user_id, tenant_id, provider, device_model, software_version, source, device_type, original_source_name, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT(user_id, tenant_id, provider, COALESCE(device_model, ''), COALESCE(source, '')) DO UPDATE SET
                software_version = EXCLUDED.software_version,
                device_type = EXCLUDED.device_type,
                original_source_name = EXCLUDED.original_source_name,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            ";

/// The columns every data-source read decodes, in the order
/// [`data_source_from_row`] reads them.
macro_rules! data_source_columns {
    () => {
        "id, user_id, provider, device_model, software_version, source, device_type, original_source_name"
    };
}

/// One data source by id.
pub(crate) const GET_DATA_SOURCE_SQL: &str = concat!(
    "
            SELECT ",
    data_source_columns!(),
    "
            FROM data_sources
            WHERE id = $1
            "
);

/// Every data source a user has under a tenant, newest first.
pub(crate) const LIST_DATA_SOURCES_SQL: &str = concat!(
    "
            SELECT ",
    data_source_columns!(),
    "
            FROM data_sources
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            "
);

/// Every data source a user has under a tenant for one provider, newest first.
pub(crate) const LIST_DATA_SOURCES_BY_PROVIDER_SQL: &str = concat!(
    "
            SELECT ",
    data_source_columns!(),
    "
            FROM data_sources
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ORDER BY created_at DESC
            "
);

/// Remove a data source by id.
pub(crate) const DELETE_DATA_SOURCE_SQL: &str = "DELETE FROM data_sources WHERE id = $1";

// ============================================================================
// sleep_sessions
// ============================================================================

/// Insert or refresh a sleep session; the arbiter is the session's start.
pub(crate) const UPSERT_SLEEP_SESSION_SQL: &str = r"
            INSERT INTO sleep_sessions (id, user_id, tenant_id, provider, data_source_id, synced_at, start_time, end_time, time_in_bed, total_sleep_time, sleep_efficiency, sleep_score, stages_json, hrv_during_sleep, is_nap, created_at, deep_sleep_seconds, light_sleep_seconds, rem_sleep_seconds, awake_seconds, avg_heart_rate, min_heart_rate)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)
            ON CONFLICT(user_id, tenant_id, provider, start_time) DO UPDATE SET
                end_time = EXCLUDED.end_time,
                time_in_bed = EXCLUDED.time_in_bed,
                total_sleep_time = EXCLUDED.total_sleep_time,
                sleep_efficiency = EXCLUDED.sleep_efficiency,
                sleep_score = EXCLUDED.sleep_score,
                stages_json = EXCLUDED.stages_json,
                hrv_during_sleep = EXCLUDED.hrv_during_sleep,
                is_nap = EXCLUDED.is_nap,
                deep_sleep_seconds = EXCLUDED.deep_sleep_seconds,
                light_sleep_seconds = EXCLUDED.light_sleep_seconds,
                rem_sleep_seconds = EXCLUDED.rem_sleep_seconds,
                awake_seconds = EXCLUDED.awake_seconds,
                avg_heart_rate = EXCLUDED.avg_heart_rate,
                min_heart_rate = EXCLUDED.min_heart_rate
            ";

/// The columns every sleep read decodes, in the order
/// [`sleep_session_from_row`] reads them.
macro_rules! sleep_session_columns {
    () => {
        "id, user_id, provider, data_source_id, start_time, end_time,
                   total_sleep_time, sleep_efficiency, sleep_score, stages_json,
                   hrv_during_sleep, is_nap, deep_sleep_seconds, light_sleep_seconds,
                   rem_sleep_seconds, awake_seconds, avg_heart_rate, min_heart_rate"
    };
}

/// Live sessions starting inside an inclusive window, newest first.
pub(crate) const GET_SLEEP_SESSIONS_SQL: &str = concat!(
    "
            SELECT ",
    sleep_session_columns!(),
    "
            FROM sleep_sessions
            WHERE user_id = $1 AND tenant_id = $2 AND start_time >= $3 AND start_time <= $4
              AND deleted_at IS NULL
            ORDER BY start_time DESC
            "
);

/// The most recent live session.
pub(crate) const LATEST_SLEEP_SESSION_SQL: &str = concat!(
    "
            SELECT ",
    sleep_session_columns!(),
    "
            FROM sleep_sessions
            WHERE user_id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            ORDER BY start_time DESC
            LIMIT 1
            "
);

/// Hard-delete every session a provider wrote for a user.
pub(crate) const DELETE_SLEEP_SESSIONS_SQL: &str =
    "DELETE FROM sleep_sessions WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// Soft-delete one session: stamps `deleted_at` once, never twice.
pub(crate) const SOFT_DELETE_SLEEP_SESSION_SQL: &str = "UPDATE sleep_sessions SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL";

/// Hard-delete one session under its tenant.
pub(crate) const DELETE_SLEEP_SESSION_SQL: &str =
    "DELETE FROM sleep_sessions WHERE id = $1 AND tenant_id = $2";

/// The tenant a session was stored under, for the enforme adapter that only
/// holds the provider id.
pub(crate) const FIND_SLEEP_SESSION_TENANT_SQL: &str =
    "SELECT tenant_id FROM sleep_sessions WHERE id = $1";

// ============================================================================
// recovery_metrics
// ============================================================================

/// Insert or refresh one day's recovery metrics.
pub(crate) const UPSERT_RECOVERY_METRICS_SQL: &str = r"
            INSERT INTO recovery_metrics (id, user_id, tenant_id, provider, data_source_id, synced_at, date, recovery_score, readiness_score, hrv_ms, stress_level, resting_heart_rate, body_temperature, resting_respiratory_rate, created_at, hrv_rmssd, body_battery, spo2, athlete_note, training_load)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
            ON CONFLICT(user_id, tenant_id, provider, date) DO UPDATE SET
                recovery_score = EXCLUDED.recovery_score,
                readiness_score = EXCLUDED.readiness_score,
                hrv_ms = EXCLUDED.hrv_ms,
                hrv_rmssd = EXCLUDED.hrv_rmssd,
                body_battery = EXCLUDED.body_battery,
                spo2 = EXCLUDED.spo2,
                athlete_note = EXCLUDED.athlete_note,
                training_load = EXCLUDED.training_load,
                stress_level = EXCLUDED.stress_level,
                resting_heart_rate = EXCLUDED.resting_heart_rate,
                body_temperature = EXCLUDED.body_temperature,
                resting_respiratory_rate = EXCLUDED.resting_respiratory_rate
            ";

/// The columns every recovery read decodes, in the order
/// [`recovery_metrics_from_row`] reads them.
macro_rules! recovery_metrics_columns {
    () => {
        "id, user_id, provider, data_source_id, date, recovery_score, readiness_score,
                   hrv_ms, hrv_rmssd, stress_level, resting_heart_rate, body_battery, spo2,
                   body_temperature, resting_respiratory_rate, training_load, athlete_note,
                   created_at"
    };
}

/// Live recovery rows inside an inclusive date window, newest first.
pub(crate) const GET_RECOVERY_METRICS_SQL: &str = concat!(
    "
            SELECT ",
    recovery_metrics_columns!(),
    "
            FROM recovery_metrics
            WHERE user_id = $1 AND tenant_id = $2 AND date >= $3 AND date <= $4
              AND deleted_at IS NULL
            ORDER BY date DESC
            "
);

/// The most recent live recovery row.
pub(crate) const LATEST_RECOVERY_SQL: &str = concat!(
    "
            SELECT ",
    recovery_metrics_columns!(),
    "
            FROM recovery_metrics
            WHERE user_id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            ORDER BY date DESC
            LIMIT 1
            "
);

/// Soft-delete one recovery row: stamps `deleted_at` once, never twice.
pub(crate) const SOFT_DELETE_RECOVERY_METRIC_SQL: &str =
    "UPDATE recovery_metrics SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL";

/// Hard-delete one recovery row under its tenant.
pub(crate) const DELETE_RECOVERY_METRIC_SQL: &str =
    "DELETE FROM recovery_metrics WHERE id = $1 AND tenant_id = $2";

/// The tenant a recovery row was stored under.
pub(crate) const FIND_RECOVERY_METRIC_TENANT_SQL: &str =
    "SELECT tenant_id FROM recovery_metrics WHERE id = $1";

// ============================================================================
// health_snapshots
// ============================================================================

/// Insert or refresh one day's body metrics. `RETURNING id` reports the id
/// actually stored, which on the update path is the pre-existing row's.
pub(crate) const UPSERT_HEALTH_SNAPSHOT_SQL: &str = r"
            INSERT INTO health_snapshots (id, user_id, tenant_id, provider, data_source_id, synced_at, date, weight, body_fat_percentage, muscle_mass, bone_mass, body_water_percentage, bp_systolic, bp_diastolic, blood_glucose, created_at, bmi)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            ON CONFLICT(user_id, tenant_id, provider, date) DO UPDATE SET
                weight = EXCLUDED.weight,
                bmi = EXCLUDED.bmi,
                body_fat_percentage = EXCLUDED.body_fat_percentage,
                muscle_mass = EXCLUDED.muscle_mass,
                bone_mass = EXCLUDED.bone_mass,
                body_water_percentage = EXCLUDED.body_water_percentage,
                bp_systolic = EXCLUDED.bp_systolic,
                bp_diastolic = EXCLUDED.bp_diastolic,
                blood_glucose = EXCLUDED.blood_glucose
            RETURNING id
            ";

/// The columns every snapshot read decodes, in the order
/// [`health_metrics_from_row`] reads them.
macro_rules! health_snapshot_columns {
    () => {
        "id, user_id, provider, data_source_id, date, weight, body_fat_percentage,
                   muscle_mass, bmi, bone_mass, body_water_percentage, bp_systolic, bp_diastolic,
                   blood_glucose, created_at"
    };
}

/// Live snapshots inside an inclusive date window, newest first.
pub(crate) const GET_HEALTH_SNAPSHOTS_SQL: &str = concat!(
    "
            SELECT ",
    health_snapshot_columns!(),
    "
            FROM health_snapshots
            WHERE user_id = $1 AND tenant_id = $2 AND date >= $3 AND date <= $4
              AND deleted_at IS NULL
            ORDER BY date DESC
            "
);

/// The most recent live snapshot.
pub(crate) const LATEST_HEALTH_SNAPSHOT_SQL: &str = concat!(
    "
            SELECT ",
    health_snapshot_columns!(),
    "
            FROM health_snapshots
            WHERE user_id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            ORDER BY date DESC
            LIMIT 1
            "
);

/// Soft-delete one snapshot: stamps `deleted_at` once, never twice.
pub(crate) const SOFT_DELETE_HEALTH_SNAPSHOT_SQL: &str =
    "UPDATE health_snapshots SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL";

/// Hard-delete one snapshot under its tenant.
pub(crate) const DELETE_HEALTH_SNAPSHOT_SQL: &str =
    "DELETE FROM health_snapshots WHERE id = $1 AND tenant_id = $2";

/// The tenant a snapshot was stored under.
pub(crate) const FIND_HEALTH_SNAPSHOT_TENANT_SQL: &str =
    "SELECT tenant_id FROM health_snapshots WHERE id = $1";

// ============================================================================
// Row decoders
// ============================================================================

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
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
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
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
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
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
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
        // `training_load` is the provider's day load score: WHOOP day strain.
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
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    NaiveDate: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
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
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let s: String = column(row, "tenant_id")?;
    TenantId::parse_str(&s)
        .map_err(|e| AppError::database(format!("Invalid tenant_id stored: {e}")))
}

// ============================================================================
// The shared body
// ============================================================================

/// Emit the [`DataSourceRepository`], [`SleepRepository`],
/// [`RecoveryRepository`] and [`HealthSnapshotRepository`] implementations
/// for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
///
/// [`DataSourceRepository`]: super::health::DataSourceRepository
/// [`SleepRepository`]: super::health::SleepRepository
/// [`RecoveryRepository`]: super::health::RecoveryRepository
/// [`HealthSnapshotRepository`]: super::health::HealthSnapshotRepository
macro_rules! impl_health_persistence_repositories {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl DataSourceRepository for $ty {
            async fn upsert_data_source(
                &self,
                tenant_id: &TenantId,
                source: &DataSource,
            ) -> AppResult<String> {
                let id = if source.id.is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    source.id.clone()
                };
                let now = Utc::now();

                let row = sqlx::query(UPSERT_DATA_SOURCE_SQL)
                    .bind(&id)
                    .bind(&source.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&source.provider)
                    .bind(&source.device_model)
                    .bind(&source.software_version)
                    .bind(&source.source)
                    .bind(device_type_to_str(source.device_type))
                    .bind(&source.original_source_name)
                    .bind(now)
                    .bind(now)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert data source: {e}"))
                    })?;

                row.try_get("id")
                    .map_err(|e| AppError::database(format!("health column id: {e}")))
            }

            async fn get_data_source(&self, id: &str) -> AppResult<Option<DataSource>> {
                let row = sqlx::query(GET_DATA_SOURCE_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get data source: {e}")))?;

                row.as_ref().map(data_source_from_row).transpose()
            }

            async fn list_data_sources(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
            ) -> AppResult<Vec<DataSource>> {
                let rows = sqlx::query(LIST_DATA_SOURCES_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list data sources: {e}")))?;

                rows.iter().map(data_source_from_row).collect()
            }

            async fn list_data_sources_by_provider(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<Vec<DataSource>> {
                let rows = sqlx::query(LIST_DATA_SOURCES_BY_PROVIDER_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list data sources by provider: {e}"))
                    })?;

                rows.iter().map(data_source_from_row).collect()
            }

            async fn delete_data_source(&self, id: &str) -> AppResult<()> {
                sqlx::query(DELETE_DATA_SOURCE_SQL)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete data source: {e}"))
                    })?;

                Ok(())
            }
        }

        #[async_trait::async_trait]
        impl SleepRepository for $ty {
            async fn upsert_sleep_session(
                &self,
                tenant_id: &TenantId,
                session: &StoredSleepSession,
            ) -> AppResult<String> {
                let id = if session.id.is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    session.id.clone()
                };
                let now = Utc::now();
                let stages_json = serde_json::to_string(&session.stages).map_err(|e| {
                    AppError::database(format!("Failed to serialize sleep stages: {e}"))
                })?;

                // total_sleep_seconds excludes awake time; the in-bed figure is sleep + awake.
                let total_sleep_time = session.total_sleep_seconds.map_or(0i64, i64::from);
                let time_in_bed = total_sleep_time + session.awake_seconds.map_or(0i64, i64::from);
                let sleep_efficiency = session.sleep_efficiency.unwrap_or(0.0);
                let sleep_score = session.sleep_score.map(f64::from);
                let is_nap: i32 = i32::from(session.is_nap);

                sqlx::query(UPSERT_SLEEP_SESSION_SQL)
                    .bind(&id)
                    .bind(&session.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&session.source_name)
                    .bind(&session.data_source_id)
                    .bind(now)
                    .bind(session.start_datetime)
                    .bind(session.end_datetime)
                    .bind(time_in_bed)
                    .bind(total_sleep_time)
                    .bind(sleep_efficiency)
                    .bind(sleep_score)
                    .bind(&stages_json)
                    .bind(session.avg_hrv)
                    .bind(is_nap)
                    .bind(now)
                    .bind(session.deep_sleep_seconds.map(i64::from))
                    .bind(session.light_sleep_seconds.map(i64::from))
                    .bind(session.rem_sleep_seconds.map(i64::from))
                    .bind(session.awake_seconds.map(i64::from))
                    .bind(session.avg_heart_rate)
                    .bind(session.min_heart_rate.map(i64::from))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert sleep session: {e}"))
                    })?;

                Ok(id)
            }

            async fn get_sleep_sessions(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
            ) -> AppResult<Vec<StoredSleepSession>> {
                let rows = sqlx::query(GET_SLEEP_SESSIONS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(start)
                    .bind(end)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get sleep sessions: {e}"))
                    })?;

                rows.iter().map(sleep_session_from_row).collect()
            }

            async fn get_latest_sleep_session(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
            ) -> AppResult<Option<StoredSleepSession>> {
                let row = sqlx::query(LATEST_SLEEP_SESSION_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get latest sleep session: {e}"))
                    })?;

                row.as_ref().map(sleep_session_from_row).transpose()
            }

            async fn delete_sleep_sessions(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
            ) -> AppResult<u64> {
                let result = sqlx::query(DELETE_SLEEP_SESSIONS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete sleep sessions: {e}"))
                    })?;

                Ok(result.rows_affected())
            }

            async fn delete_sleep_session_by_id(
                &self,
                tenant_id: &TenantId,
                id: &str,
                soft: bool,
            ) -> AppResult<bool> {
                let result = if soft {
                    sqlx::query(SOFT_DELETE_SLEEP_SESSION_SQL)
                        .bind(Utc::now())
                        .bind(id)
                        .bind(tenant_id.to_string())
                        .execute(self.pool())
                        .await
                } else {
                    sqlx::query(DELETE_SLEEP_SESSION_SQL)
                        .bind(id)
                        .bind(tenant_id.to_string())
                        .execute(self.pool())
                        .await
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to delete sleep session by id: {e}"))
                })?;

                Ok(result.rows_affected() > 0)
            }

            async fn find_sleep_session_tenant(&self, id: &str) -> AppResult<Option<TenantId>> {
                let row = sqlx::query(FIND_SLEEP_SESSION_TENANT_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find sleep session tenant: {e}"))
                    })?;

                row.as_ref().map(tenant_from_row).transpose()
            }
        }

        #[async_trait::async_trait]
        impl RecoveryRepository for $ty {
            async fn upsert_recovery_metrics(
                &self,
                tenant_id: &TenantId,
                metrics: &StoredRecoveryMetrics,
            ) -> AppResult<String> {
                let id = if metrics.id.is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    metrics.id.clone()
                };
                let now = Utc::now();
                let recovery_score = metrics.recovery_score.map(f64::from);
                let readiness_score = metrics.readiness_score.map(f64::from);
                let stress_level = metrics.stress_score.map(f64::from);
                let resting_heart_rate = metrics.resting_heart_rate.map(i64::from);

                sqlx::query(UPSERT_RECOVERY_METRICS_SQL)
                    .bind(&id)
                    .bind(&metrics.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&metrics.source_name)
                    .bind(&metrics.data_source_id)
                    .bind(now)
                    .bind(metrics.date)
                    .bind(recovery_score)
                    .bind(readiness_score)
                    .bind(metrics.hrv_ms)
                    .bind(stress_level)
                    .bind(resting_heart_rate)
                    .bind(metrics.skin_temp_deviation)
                    .bind(metrics.respiratory_rate)
                    .bind(now)
                    .bind(metrics.hrv_rmssd)
                    .bind(metrics.body_battery.map(i64::from))
                    .bind(metrics.spo2)
                    .bind(metrics.athlete_note.as_deref())
                    .bind(metrics.daily_strain)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert recovery metrics: {e}"))
                    })?;

                Ok(id)
            }

            async fn get_recovery_metrics(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
            ) -> AppResult<Vec<StoredRecoveryMetrics>> {
                let rows = sqlx::query(GET_RECOVERY_METRICS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(start.date_naive())
                    .bind(end.date_naive())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get recovery metrics: {e}"))
                    })?;

                rows.iter().map(recovery_metrics_from_row).collect()
            }

            async fn get_latest_recovery(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
            ) -> AppResult<Option<StoredRecoveryMetrics>> {
                let row = sqlx::query(LATEST_RECOVERY_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get latest recovery: {e}"))
                    })?;

                row.as_ref().map(recovery_metrics_from_row).transpose()
            }

            async fn delete_recovery_metric_by_id(
                &self,
                tenant_id: &TenantId,
                id: &str,
                soft: bool,
            ) -> AppResult<bool> {
                let result = if soft {
                    sqlx::query(SOFT_DELETE_RECOVERY_METRIC_SQL)
                        .bind(Utc::now())
                        .bind(id)
                        .bind(tenant_id.to_string())
                        .execute(self.pool())
                        .await
                } else {
                    sqlx::query(DELETE_RECOVERY_METRIC_SQL)
                        .bind(id)
                        .bind(tenant_id.to_string())
                        .execute(self.pool())
                        .await
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to delete recovery metric by id: {e}"))
                })?;

                Ok(result.rows_affected() > 0)
            }

            async fn find_recovery_metric_tenant(&self, id: &str) -> AppResult<Option<TenantId>> {
                let row = sqlx::query(FIND_RECOVERY_METRIC_TENANT_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find recovery metric tenant: {e}"))
                    })?;

                row.as_ref().map(tenant_from_row).transpose()
            }
        }

        #[async_trait::async_trait]
        impl HealthSnapshotRepository for $ty {
            async fn upsert_health_snapshot(
                &self,
                tenant_id: &TenantId,
                snapshot: &StoredHealthMetrics,
            ) -> AppResult<String> {
                // Surrogate id, deliberately ignoring `snapshot.id`. A row's identity is the
                // natural key (user, tenant, provider, date) carried by the ON CONFLICT
                // arbiter, and WHOOP stamps ONE date-invariant id for body metrics
                // (`whoop-body-{user}`), so reusing it repeats the primary key on a new
                // date and the INSERT dies before the arbiter is consulted. RETURNING
                // reports the id actually stored, which on the update path is the
                // pre-existing row's — so legacy rows keep their id and heal in place.
                //
                // Scoped to health snapshots on purpose. `upsert_sleep_session` and
                // `upsert_recovery_metrics` keep provider-id reuse: their provider ids
                // vary with their own arbiters (sleep ids are per-session, keyed to
                // start_time), and the dravr-enforme adapter resolves those rows BY the
                // provider id through find_*_tenant / delete_*_by_id — a contract
                // health snapshots do not have.
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let bp_systolic = snapshot.systolic_bp.map(i64::from);
                let bp_diastolic = snapshot.diastolic_bp.map(i64::from);

                let row = sqlx::query(UPSERT_HEALTH_SNAPSHOT_SQL)
                    .bind(&id)
                    .bind(&snapshot.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&snapshot.source_name)
                    .bind(&snapshot.data_source_id)
                    .bind(now)
                    .bind(snapshot.date)
                    .bind(snapshot.weight_kg)
                    .bind(snapshot.body_fat_pct)
                    .bind(snapshot.muscle_mass_kg)
                    .bind(snapshot.bone_mass_kg)
                    .bind(snapshot.water_pct)
                    .bind(bp_systolic)
                    .bind(bp_diastolic)
                    .bind(snapshot.blood_glucose)
                    .bind(now)
                    .bind(snapshot.bmi)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert health snapshot: {e}"))
                    })?;

                row.try_get("id")
                    .map_err(|e| AppError::database(format!("health column id: {e}")))
            }

            async fn get_health_snapshots(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
            ) -> AppResult<Vec<StoredHealthMetrics>> {
                let rows = sqlx::query(GET_HEALTH_SNAPSHOTS_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(start.date_naive())
                    .bind(end.date_naive())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get health snapshots: {e}"))
                    })?;

                rows.iter().map(health_metrics_from_row).collect()
            }

            async fn get_latest_health_snapshot(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
            ) -> AppResult<Option<StoredHealthMetrics>> {
                let row = sqlx::query(LATEST_HEALTH_SNAPSHOT_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get latest health snapshot: {e}"))
                    })?;

                row.as_ref().map(health_metrics_from_row).transpose()
            }

            async fn delete_health_snapshot_by_id(
                &self,
                tenant_id: &TenantId,
                id: &str,
                soft: bool,
            ) -> AppResult<bool> {
                let result = if soft {
                    sqlx::query(SOFT_DELETE_HEALTH_SNAPSHOT_SQL)
                        .bind(Utc::now())
                        .bind(id)
                        .bind(tenant_id.to_string())
                        .execute(self.pool())
                        .await
                } else {
                    sqlx::query(DELETE_HEALTH_SNAPSHOT_SQL)
                        .bind(id)
                        .bind(tenant_id.to_string())
                        .execute(self.pool())
                        .await
                }
                .map_err(|e| {
                    AppError::database(format!("Failed to delete health snapshot by id: {e}"))
                })?;

                Ok(result.rows_affected() > 0)
            }

            async fn find_health_snapshot_tenant(&self, id: &str) -> AppResult<Option<TenantId>> {
                let row = sqlx::query(FIND_HEALTH_SNAPSHOT_TENANT_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find health snapshot tenant: {e}"))
                    })?;

                row.as_ref().map(tenant_from_row).transpose()
            }
        }
    };
}
pub(crate) use impl_health_persistence_repositories;
