// ABOUTME: Writes for data a provider pushes: newest-receipt-wins upserts, field-merging day rows, kept one-shot files
// ABOUTME: The SQL is written once here and both backends emit their impl from it; nothing here reads a provider

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pushed data.
//!
//! A provider that pushes its data (the Garmin Connect Developer Program,
//! carnet#1155) hands it over in separate requests that Dravr processes out
//! of order: a batch that failed for now is retried minutes or hours after
//! later ones landed, and several batches for one athlete and one day run at
//! once. The provider never sends a batch twice, so whatever lands last
//! stands for good. The writes here keep that from undoing newer data:
//!
//! - a row one push replaces whole (an activity, its samples, a night's
//!   sleep) is written only when the push was received no earlier than the
//!   one that wrote the stored row ([`PushedDataRepository::upsert_activity`],
//!   [`PushedDataRepository::upsert_sleep`]; an activity's samples go to the
//!   one sample store, [`crate::repositories::ActivityStreamRepository`],
//!   whose upsert keeps the same latest-receipt rule);
//! - a day's recovery or body row, which several summary types feed, is
//!   merged in one statement: each reading the push carries replaces the
//!   stored one, every reading it does not carry stays
//!   ([`PushedDataRepository::merge_recovery`],
//!   [`PushedDataRepository::merge_body`]). Two pushes for the same day can
//!   run at once without either erasing the other's readings.
//!
//! Every wellness write files the row under the pushing connection's data
//! source, taking over a row another connection under the same provider key
//! stored for that night or day, so a purge scoped to that data source
//! removes exactly the rows the pushing connection wrote.
//!
//! An activity file the provider offers once is kept from its download until
//! the samples it carries are stored ([`PushedDataRepository::keep_file`]),
//! so a write that fails after the download, or an instance that dies, does
//! not lose a file the provider will not serve again. That copy is a
//! transient retry buffer, dropped once the samples land and swept after 24
//! hours whatever became of it; it is never a store of original files.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres. Instants bind as `DateTime<Utc>`, which sqlx-sqlite stores as
//! RFC 3339 text in UTC (`AutoSi` precision), so the ordering compares here
//! read the same on both engines.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    Activity, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, TenantId,
};
use uuid::Uuid;

/// Writes for the data a provider pushes.
#[async_trait]
pub trait PushedDataRepository: Send + Sync {
    /// Write `activity` as the user's cached row under `provider`, received
    /// at `received_at`, unless the stored row was written from a later
    /// push. Returns whether the row was written.
    ///
    /// # Errors
    /// Returns a database error when the activity does not serialize or the
    /// write fails.
    async fn upsert_activity(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
        activity: &Activity,
        received_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Write a night's sleep, received at `received_at`, filed under the
    /// session's data source, unless the stored night came from a later
    /// push. Returns whether it was written.
    ///
    /// # Errors
    /// Returns a database error when the stages do not serialize or the
    /// write fails.
    async fn upsert_sleep(
        &self,
        tenant_id: &TenantId,
        session: &StoredSleepSession,
        received_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Merge the readings `row` carries into the stored recovery row of its
    /// day, in one statement: a reading `row` carries replaces the stored
    /// one, a reading it leaves `None` stays. The row is filed under
    /// `row.data_source_id`.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn merge_recovery(
        &self,
        tenant_id: &TenantId,
        row: &StoredRecoveryMetrics,
        received_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Merge the measurements `row` carries into the stored body row of its
    /// day, as [`Self::merge_recovery`] merges a recovery row.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn merge_body(
        &self,
        tenant_id: &TenantId,
        row: &StoredHealthMetrics,
        received_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Keep a downloaded activity file until its samples are stored,
    /// replacing any copy kept before.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn keep_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
        bytes: &[u8],
        fetched_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// The kept copy of an activity's file, or `None` when none is kept.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn kept_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<Option<Vec<u8>>>;

    /// Drop the kept copy of an activity's file: its samples are stored, or
    /// it is not a readable activity.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn drop_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<()>;

    /// Delete every kept file downloaded before `cutoff`, whoever it
    /// belongs to: one still kept that long belongs to a batch that was
    /// given up on. Returns how many were deleted.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn drop_files_fetched_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64>;
}

/// Write one cached activity unless the stored row came from a later push.
/// `$1` id … `$9` `data_json`, as `UPSERT_CACHED_ACTIVITY_SQL` binds them;
/// `$8` is the push's receipt.
pub(crate) const UPSERT_PUSHED_ACTIVITY_SQL: &str = r"
    INSERT INTO cached_activities (id, user_id, tenant_id, provider, activity_id, sport_type, start_date, synced_at, data_json)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
    ON CONFLICT(user_id, tenant_id, provider, activity_id) DO UPDATE SET
        sport_type = EXCLUDED.sport_type,
        start_date = EXCLUDED.start_date,
        synced_at = EXCLUDED.synced_at,
        data_json = EXCLUDED.data_json
    WHERE cached_activities.synced_at <= EXCLUDED.synced_at";

/// Write one night's sleep unless the stored night came from a later push,
/// filed under the pushing connection's data source.
pub(crate) const UPSERT_PUSHED_SLEEP_SQL: &str = r"
    INSERT INTO sleep_sessions (id, user_id, tenant_id, provider, data_source_id, synced_at, start_time, end_time, time_in_bed, total_sleep_time, sleep_efficiency, sleep_score, stages_json, hrv_during_sleep, is_nap, created_at, deep_sleep_seconds, light_sleep_seconds, rem_sleep_seconds, awake_seconds, avg_heart_rate, min_heart_rate)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)
    ON CONFLICT(user_id, tenant_id, provider, start_time) DO UPDATE SET
        data_source_id = EXCLUDED.data_source_id,
        synced_at = EXCLUDED.synced_at,
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
    WHERE sleep_sessions.synced_at IS NULL OR sleep_sessions.synced_at <= EXCLUDED.synced_at";

/// Merge one day's recovery readings: each reading bound replaces the stored
/// one, each NULL keeps it. The row is filed under the pushing connection's
/// data source, and its `synced_at` is the later of the two receipts.
pub(crate) const MERGE_PUSHED_RECOVERY_SQL: &str = r"
    INSERT INTO recovery_metrics (id, user_id, tenant_id, provider, data_source_id, synced_at, date, recovery_score, readiness_score, hrv_ms, stress_level, resting_heart_rate, body_temperature, resting_respiratory_rate, created_at, hrv_rmssd, body_battery, spo2, athlete_note, training_load)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
    ON CONFLICT(user_id, tenant_id, provider, date) DO UPDATE SET
        data_source_id = EXCLUDED.data_source_id,
        synced_at = CASE
            WHEN recovery_metrics.synced_at IS NULL OR recovery_metrics.synced_at < EXCLUDED.synced_at
            THEN EXCLUDED.synced_at ELSE recovery_metrics.synced_at END,
        recovery_score = COALESCE(EXCLUDED.recovery_score, recovery_metrics.recovery_score),
        readiness_score = COALESCE(EXCLUDED.readiness_score, recovery_metrics.readiness_score),
        hrv_ms = COALESCE(EXCLUDED.hrv_ms, recovery_metrics.hrv_ms),
        hrv_rmssd = COALESCE(EXCLUDED.hrv_rmssd, recovery_metrics.hrv_rmssd),
        body_battery = COALESCE(EXCLUDED.body_battery, recovery_metrics.body_battery),
        spo2 = COALESCE(EXCLUDED.spo2, recovery_metrics.spo2),
        athlete_note = COALESCE(EXCLUDED.athlete_note, recovery_metrics.athlete_note),
        training_load = COALESCE(EXCLUDED.training_load, recovery_metrics.training_load),
        stress_level = COALESCE(EXCLUDED.stress_level, recovery_metrics.stress_level),
        resting_heart_rate = COALESCE(EXCLUDED.resting_heart_rate, recovery_metrics.resting_heart_rate),
        body_temperature = COALESCE(EXCLUDED.body_temperature, recovery_metrics.body_temperature),
        resting_respiratory_rate = COALESCE(EXCLUDED.resting_respiratory_rate, recovery_metrics.resting_respiratory_rate)";

/// Merge one day's body measurements, as [`MERGE_PUSHED_RECOVERY_SQL`]
/// merges a recovery row.
pub(crate) const MERGE_PUSHED_BODY_SQL: &str = r"
    INSERT INTO health_snapshots (id, user_id, tenant_id, provider, data_source_id, synced_at, date, weight, body_fat_percentage, muscle_mass, bone_mass, body_water_percentage, bp_systolic, bp_diastolic, blood_glucose, created_at, bmi, vo2_max)
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
    ON CONFLICT(user_id, tenant_id, provider, date) DO UPDATE SET
        data_source_id = EXCLUDED.data_source_id,
        synced_at = CASE
            WHEN health_snapshots.synced_at IS NULL OR health_snapshots.synced_at < EXCLUDED.synced_at
            THEN EXCLUDED.synced_at ELSE health_snapshots.synced_at END,
        weight = COALESCE(EXCLUDED.weight, health_snapshots.weight),
        bmi = COALESCE(EXCLUDED.bmi, health_snapshots.bmi),
        body_fat_percentage = COALESCE(EXCLUDED.body_fat_percentage, health_snapshots.body_fat_percentage),
        muscle_mass = COALESCE(EXCLUDED.muscle_mass, health_snapshots.muscle_mass),
        bone_mass = COALESCE(EXCLUDED.bone_mass, health_snapshots.bone_mass),
        body_water_percentage = COALESCE(EXCLUDED.body_water_percentage, health_snapshots.body_water_percentage),
        bp_systolic = COALESCE(EXCLUDED.bp_systolic, health_snapshots.bp_systolic),
        bp_diastolic = COALESCE(EXCLUDED.bp_diastolic, health_snapshots.bp_diastolic),
        blood_glucose = COALESCE(EXCLUDED.blood_glucose, health_snapshots.blood_glucose),
        vo2_max = COALESCE(EXCLUDED.vo2_max, health_snapshots.vo2_max)";

/// Keep, or replace, one activity's downloaded file.
pub(crate) const KEEP_PUSHED_FILE_SQL: &str = r"
    INSERT INTO pushed_activity_files (tenant_id, user_id, provider, activity_id, file_bytes, fetched_at)
    VALUES ($1, $2, $3, $4, $5, $6)
    ON CONFLICT (tenant_id, user_id, provider, activity_id) DO UPDATE SET
        file_bytes = EXCLUDED.file_bytes,
        fetched_at = EXCLUDED.fetched_at";

/// One activity's kept file, scoped by tenant, user and provider.
pub(crate) const KEPT_PUSHED_FILE_SQL: &str = r"
    SELECT file_bytes FROM pushed_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND activity_id = $4";

/// Drop one activity's kept file.
pub(crate) const DROP_PUSHED_FILE_SQL: &str = r"
    DELETE FROM pushed_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND activity_id = $4";

/// Drop every kept file downloaded before `$1`.
///
/// ISOLATION EXEMPTION: this delete spans every tenant on purpose. It serves
/// the push worker's sweep, a system job that drops the copies of batches
/// given up on wherever they are held; it reads nothing back.
pub(crate) const DROP_STALE_PUSHED_FILES_SQL: &str =
    "DELETE FROM pushed_activity_files WHERE fetched_at < $1";

/// Emit the whole [`PushedDataRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts, helpers and types unqualified, so
/// the invoking shell must `use` every one of them.
macro_rules! impl_pushed_data_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl PushedDataRepository for $ty {
            async fn upsert_activity(
                &self,
                user_id: Uuid,
                tenant_id: &TenantId,
                provider: &str,
                activity: &Activity,
                received_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let data_json = serde_json::to_string(activity).map_err(|e| {
                    AppError::database(format!("Failed to serialize activity: {e}"))
                })?;
                let written = sqlx::query(UPSERT_PUSHED_ACTIVITY_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(provider)
                    .bind(activity.id())
                    .bind(sport_type_string(activity))
                    .bind(activity.start_date())
                    .bind(received_at)
                    .bind(&data_json)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert pushed activity: {e}")))?
                    .rows_affected();
                Ok(written > 0)
            }

            async fn upsert_sleep(
                &self,
                tenant_id: &TenantId,
                session: &StoredSleepSession,
                received_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let id = if session.id.is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    session.id.clone()
                };
                let stages_json = serde_json::to_string(&session.stages).map_err(|e| {
                    AppError::database(format!("Failed to serialize sleep stages: {e}"))
                })?;
                // As the health sync's upsert stores them: the in-bed figure
                // is sleep plus awake.
                let total_sleep_time = session.total_sleep_seconds.map_or(0i64, i64::from);
                let time_in_bed = total_sleep_time + session.awake_seconds.map_or(0i64, i64::from);
                let written = sqlx::query(UPSERT_PUSHED_SLEEP_SQL)
                    .bind(&id)
                    .bind(&session.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&session.source_name)
                    .bind(&session.data_source_id)
                    .bind(received_at)
                    .bind(session.start_datetime)
                    .bind(session.end_datetime)
                    .bind(time_in_bed)
                    .bind(total_sleep_time)
                    .bind(session.sleep_efficiency.unwrap_or(0.0))
                    .bind(session.sleep_score.map(f64::from))
                    .bind(&stages_json)
                    .bind(session.avg_hrv)
                    .bind(i32::from(session.is_nap))
                    .bind(Utc::now())
                    .bind(session.deep_sleep_seconds.map(i64::from))
                    .bind(session.light_sleep_seconds.map(i64::from))
                    .bind(session.rem_sleep_seconds.map(i64::from))
                    .bind(session.awake_seconds.map(i64::from))
                    .bind(session.avg_heart_rate)
                    .bind(session.min_heart_rate.map(i64::from))
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert pushed sleep: {e}")))?
                    .rows_affected();
                Ok(written > 0)
            }

            async fn merge_recovery(
                &self,
                tenant_id: &TenantId,
                row: &StoredRecoveryMetrics,
                received_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(MERGE_PUSHED_RECOVERY_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(&row.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&row.source_name)
                    .bind(&row.data_source_id)
                    .bind(received_at)
                    .bind(row.date)
                    .bind(row.recovery_score.map(f64::from))
                    .bind(row.readiness_score.map(f64::from))
                    .bind(row.hrv_ms)
                    .bind(row.stress_score.map(f64::from))
                    .bind(row.resting_heart_rate.map(i64::from))
                    .bind(row.skin_temp_deviation)
                    .bind(row.respiratory_rate)
                    .bind(Utc::now())
                    .bind(row.hrv_rmssd)
                    .bind(row.body_battery.map(i64::from))
                    .bind(row.spo2)
                    .bind(row.athlete_note.as_deref())
                    .bind(row.daily_strain)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("merge pushed recovery: {e}")))?;
                Ok(())
            }

            async fn merge_body(
                &self,
                tenant_id: &TenantId,
                row: &StoredHealthMetrics,
                received_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(MERGE_PUSHED_BODY_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(&row.user_id)
                    .bind(tenant_id.to_string())
                    .bind(&row.source_name)
                    .bind(&row.data_source_id)
                    .bind(received_at)
                    .bind(row.date)
                    .bind(row.weight_kg)
                    .bind(row.body_fat_pct)
                    .bind(row.muscle_mass_kg)
                    .bind(row.bone_mass_kg)
                    .bind(row.water_pct)
                    .bind(row.systolic_bp.map(i64::from))
                    .bind(row.diastolic_bp.map(i64::from))
                    .bind(row.blood_glucose)
                    .bind(Utc::now())
                    .bind(row.bmi)
                    .bind(row.vo2_max)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("merge pushed body: {e}")))?;
                Ok(())
            }

            async fn keep_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
                bytes: &[u8],
                fetched_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(KEEP_PUSHED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .bind(bytes)
                    .bind(fetched_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("keep pushed file: {e}")))?;
                Ok(())
            }

            async fn kept_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<Vec<u8>>> {
                sqlx::query_scalar(KEPT_PUSHED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("read kept pushed file: {e}")))
            }

            async fn drop_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<()> {
                sqlx::query(DROP_PUSHED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("drop pushed file: {e}")))?;
                Ok(())
            }

            async fn drop_files_fetched_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64> {
                sqlx::query(DROP_STALE_PUSHED_FILES_SQL)
                    .bind(cutoff)
                    .execute(self.pool())
                    .await
                    .map(|done| done.rows_affected())
                    .map_err(|e| AppError::database(format!("drop stale pushed files: {e}")))
            }
        }
    };
}
pub(crate) use impl_pushed_data_repository;
