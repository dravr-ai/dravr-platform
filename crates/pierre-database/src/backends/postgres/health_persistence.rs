// ABOUTME: PostgreSQL health persistence repository implementations (data sources, sleep, recovery, health snapshots)
// ABOUTME: Implements DataSourceRepository, SleepRepository, RecoveryRepository, and HealthSnapshotRepository for PostgreSQL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::PostgresDatabase;
use crate::repositories::{
    ConnectedUserRow, DataSourceRepository, HealthSnapshotRepository, RecoveryRepository,
    SleepRepository, SyncCursorRepository, SyncCursorRow, TimeSeriesPointRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    DataSource, DeviceType, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
    TenantId,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

// ============================================================================
// Helper: DeviceType <-> String conversion
// ============================================================================

fn device_type_to_str(dt: DeviceType) -> &'static str {
    match dt {
        DeviceType::Watch => "watch",
        DeviceType::Band => "band",
        DeviceType::Phone => "phone",
        DeviceType::Ring => "ring",
        DeviceType::Scale => "scale",
        DeviceType::Unknown => "unknown",
    }
}

fn str_to_device_type(s: &str) -> DeviceType {
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
// DataSourceRepository
// ============================================================================

#[async_trait]
impl DataSourceRepository for PostgresDatabase {
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
        let device_type_str = device_type_to_str(source.device_type);

        sqlx::query(
            r"
            INSERT INTO data_sources (id, user_id, tenant_id, provider, device_model, software_version, source, device_type, original_source_name, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT(user_id, tenant_id, provider, device_model, source) DO UPDATE SET
                software_version = EXCLUDED.software_version,
                device_type = EXCLUDED.device_type,
                original_source_name = EXCLUDED.original_source_name,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(&id)
        .bind(&source.user_id)
        .bind(tenant_id.to_string())
        .bind(&source.provider)
        .bind(&source.device_model)
        .bind(&source.software_version)
        .bind(&source.source)
        .bind(device_type_str)
        .bind(&source.original_source_name)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert data source: {e}")))?;

        Ok(id)
    }

    async fn get_data_source(&self, id: &str) -> AppResult<Option<DataSource>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, provider, device_model, software_version, source, device_type, original_source_name
            FROM data_sources
            WHERE id = $1
            ",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get data source: {e}")))?;

        Ok(row.map(|r| {
            let device_type_str: String = r.get("device_type");
            DataSource {
                id: r.get("id"),
                user_id: r.get("user_id"),
                provider: r.get("provider"),
                device_model: r.get("device_model"),
                software_version: r.get("software_version"),
                source: r.get("source"),
                device_type: str_to_device_type(&device_type_str),
                original_source_name: r.get("original_source_name"),
            }
        }))
    }

    async fn list_data_sources(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Vec<DataSource>> {
        let user_id_str = user_id.to_string();

        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, device_model, software_version, source, device_type, original_source_name
            FROM data_sources
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list data sources: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let device_type_str: String = r.get("device_type");
                DataSource {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    provider: r.get("provider"),
                    device_model: r.get("device_model"),
                    software_version: r.get("software_version"),
                    source: r.get("source"),
                    device_type: str_to_device_type(&device_type_str),
                    original_source_name: r.get("original_source_name"),
                }
            })
            .collect())
    }

    async fn list_data_sources_by_provider(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<Vec<DataSource>> {
        let user_id_str = user_id.to_string();

        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, device_model, software_version, source, device_type, original_source_name
            FROM data_sources
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3
            ORDER BY created_at DESC
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(provider)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list data sources by provider: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let device_type_str: String = r.get("device_type");
                DataSource {
                    id: r.get("id"),
                    user_id: r.get("user_id"),
                    provider: r.get("provider"),
                    device_model: r.get("device_model"),
                    software_version: r.get("software_version"),
                    source: r.get("source"),
                    device_type: str_to_device_type(&device_type_str),
                    original_source_name: r.get("original_source_name"),
                }
            })
            .collect())
    }

    async fn delete_data_source(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM data_sources WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to delete data source: {e}")))?;

        Ok(())
    }
}

// ============================================================================
// SleepRepository
// ============================================================================

#[async_trait]
impl SleepRepository for PostgresDatabase {
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
        let stages_json = serde_json::to_string(&session.stages)
            .map_err(|e| AppError::database(format!("Failed to serialize sleep stages: {e}")))?;

        let time_in_bed = session.total_sleep_seconds.map_or(0i64, i64::from);
        let total_sleep_time = session.total_sleep_seconds.map_or(0i64, i64::from);
        let sleep_efficiency = session.sleep_efficiency.unwrap_or(0.0);
        let sleep_score = session.sleep_score.map(f64::from);
        let hrv_during_sleep = session.avg_hrv;
        let is_nap: i32 = i32::from(session.is_nap);

        sqlx::query(
            r"
            INSERT INTO sleep_sessions (id, user_id, tenant_id, provider, data_source_id, synced_at, start_time, end_time, time_in_bed, total_sleep_time, sleep_efficiency, sleep_score, stages_json, hrv_during_sleep, is_nap, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT(user_id, tenant_id, provider, start_time) DO UPDATE SET
                end_time = EXCLUDED.end_time,
                time_in_bed = EXCLUDED.time_in_bed,
                total_sleep_time = EXCLUDED.total_sleep_time,
                sleep_efficiency = EXCLUDED.sleep_efficiency,
                sleep_score = EXCLUDED.sleep_score,
                stages_json = EXCLUDED.stages_json,
                hrv_during_sleep = EXCLUDED.hrv_during_sleep,
                is_nap = EXCLUDED.is_nap
            ",
        )
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
        .bind(hrv_during_sleep)
        .bind(is_nap)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert sleep session: {e}")))?;

        Ok(id)
    }

    async fn get_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredSleepSession>> {
        let user_id_str = user_id.to_string();

        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, data_source_id, start_time, end_time,
                   total_sleep_time, sleep_efficiency, sleep_score, stages_json,
                   hrv_during_sleep, is_nap
            FROM sleep_sessions
            WHERE user_id = $1 AND tenant_id = $2 AND start_time >= $3 AND start_time <= $4
              AND deleted_at IS NULL
            ORDER BY start_time DESC
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get sleep sessions: {e}")))?;

        rows.into_iter()
            .map(|r| pg_row_to_stored_sleep_session(&r))
            .collect()
    }

    async fn get_latest_sleep_session(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredSleepSession>> {
        let user_id_str = user_id.to_string();

        let row = sqlx::query(
            r"
            SELECT id, user_id, provider, data_source_id, start_time, end_time,
                   total_sleep_time, sleep_efficiency, sleep_score, stages_json,
                   hrv_during_sleep, is_nap
            FROM sleep_sessions
            WHERE user_id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            ORDER BY start_time DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get latest sleep session: {e}")))?;

        row.map(|r| pg_row_to_stored_sleep_session(&r)).transpose()
    }

    async fn delete_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64> {
        let user_id_str = user_id.to_string();

        let result = sqlx::query(
            "DELETE FROM sleep_sessions WHERE user_id = $1 AND tenant_id = $2 AND provider = $3",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(provider)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete sleep sessions: {e}")))?;

        Ok(result.rows_affected())
    }

    async fn delete_sleep_session_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool> {
        let result = if soft {
            sqlx::query(
                "UPDATE sleep_sessions SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL",
            )
            .bind(Utc::now())
            .bind(id)
            .bind(tenant_id.to_string())
            .execute(&self.pool)
            .await
        } else {
            sqlx::query("DELETE FROM sleep_sessions WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id.to_string())
                .execute(&self.pool)
                .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete sleep session by id: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn find_sleep_session_tenant(&self, id: &str) -> AppResult<Option<TenantId>> {
        let row = sqlx::query("SELECT tenant_id FROM sleep_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to find sleep session tenant: {e}")))?;

        row.map(|r| {
            let s: String = r.get("tenant_id");
            s.parse::<TenantId>()
                .map_err(|e| AppError::database(format!("Invalid tenant_id stored: {e}")))
        })
        .transpose()
    }
}

/// Map a `PostgreSQL` row to a `StoredSleepSession`.
fn pg_row_to_stored_sleep_session(row: &PgRow) -> AppResult<StoredSleepSession> {
    let start_datetime: DateTime<Utc> = row.get("start_time");
    let end_datetime: DateTime<Utc> = row.get("end_time");
    let stages_json_str: String = row.get("stages_json");
    let is_nap_int: i32 = row.get("is_nap");
    let total_sleep_time: Option<i64> = row.get("total_sleep_time");
    let sleep_efficiency: Option<f64> = row.get("sleep_efficiency");
    let sleep_score: Option<f64> = row.get("sleep_score");
    let hrv_during_sleep: Option<f64> = row.get("hrv_during_sleep");

    let stages = serde_json::from_str(&stages_json_str)
        .map_err(|e| AppError::database(format!("Invalid stages_json: {e}")))?;

    Ok(StoredSleepSession {
        id: row.get("id"),
        user_id: row.get("user_id"),
        data_source_id: row
            .get::<Option<String>, _>("data_source_id")
            .unwrap_or_default(),
        is_nap: is_nap_int != 0,
        start_datetime,
        end_datetime,
        total_sleep_seconds: total_sleep_time.map(|v| v as u32),
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: None,
        sleep_efficiency,
        avg_heart_rate: None,
        min_heart_rate: None,
        avg_hrv: hrv_during_sleep,
        sleep_score: sleep_score.map(|v| v as u32),
        stages,
        source_name: row.get("provider"),
    })
}

// ============================================================================
// RecoveryRepository
// ============================================================================

#[async_trait]
impl RecoveryRepository for PostgresDatabase {
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
        let body_temperature = metrics.skin_temp_deviation;
        let resting_respiratory_rate = metrics.respiratory_rate;

        sqlx::query(
            r"
            INSERT INTO recovery_metrics (id, user_id, tenant_id, provider, data_source_id, synced_at, date, recovery_score, readiness_score, hrv_status, stress_level, resting_heart_rate, body_temperature, resting_respiratory_rate, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT(user_id, tenant_id, provider, date) DO UPDATE SET
                recovery_score = EXCLUDED.recovery_score,
                readiness_score = EXCLUDED.readiness_score,
                hrv_status = EXCLUDED.hrv_status,
                stress_level = EXCLUDED.stress_level,
                resting_heart_rate = EXCLUDED.resting_heart_rate,
                body_temperature = EXCLUDED.body_temperature,
                resting_respiratory_rate = EXCLUDED.resting_respiratory_rate
            ",
        )
        .bind(&id)
        .bind(&metrics.user_id)
        .bind(tenant_id.to_string())
        .bind(&metrics.source_name)
        .bind(&metrics.data_source_id)
        .bind(now)
        .bind(metrics.date)
        .bind(recovery_score)
        .bind(readiness_score)
        .bind(metrics.hrv_ms.map(|v| format!("{v:.1}ms")))
        .bind(stress_level)
        .bind(resting_heart_rate)
        .bind(body_temperature)
        .bind(resting_respiratory_rate)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert recovery metrics: {e}")))?;

        Ok(id)
    }

    async fn get_recovery_metrics(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredRecoveryMetrics>> {
        let user_id_str = user_id.to_string();

        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, data_source_id, date, recovery_score, readiness_score,
                   hrv_status, stress_level, resting_heart_rate, body_temperature,
                   resting_respiratory_rate, created_at
            FROM recovery_metrics
            WHERE user_id = $1 AND tenant_id = $2 AND date >= $3 AND date <= $4
              AND deleted_at IS NULL
            ORDER BY date DESC
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(start.date_naive())
        .bind(end.date_naive())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get recovery metrics: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| pg_row_to_stored_recovery(&r))
            .collect())
    }

    async fn get_latest_recovery(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredRecoveryMetrics>> {
        let user_id_str = user_id.to_string();

        let row = sqlx::query(
            r"
            SELECT id, user_id, provider, data_source_id, date, recovery_score, readiness_score,
                   hrv_status, stress_level, resting_heart_rate, body_temperature,
                   resting_respiratory_rate, created_at
            FROM recovery_metrics
            WHERE user_id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            ORDER BY date DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get latest recovery: {e}")))?;

        Ok(row.map(|r| pg_row_to_stored_recovery(&r)))
    }

    async fn delete_recovery_metric_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool> {
        let result = if soft {
            sqlx::query(
                "UPDATE recovery_metrics SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL",
            )
            .bind(Utc::now())
            .bind(id)
            .bind(tenant_id.to_string())
            .execute(&self.pool)
            .await
        } else {
            sqlx::query("DELETE FROM recovery_metrics WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id.to_string())
                .execute(&self.pool)
                .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete recovery metric by id: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn find_recovery_metric_tenant(&self, id: &str) -> AppResult<Option<TenantId>> {
        let row = sqlx::query("SELECT tenant_id FROM recovery_metrics WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to find recovery metric tenant: {e}"))
            })?;

        row.map(|r| {
            let s: String = r.get("tenant_id");
            s.parse::<TenantId>()
                .map_err(|e| AppError::database(format!("Invalid tenant_id stored: {e}")))
        })
        .transpose()
    }
}

/// Map a `PostgreSQL` row to a `StoredRecoveryMetrics`.
fn pg_row_to_stored_recovery(row: &PgRow) -> StoredRecoveryMetrics {
    let date: NaiveDate = row.get("date");
    let recorded_at: DateTime<Utc> = row.get("created_at");
    let recovery_score: Option<f64> = row.get("recovery_score");
    let readiness_score: Option<f64> = row.get("readiness_score");
    let stress_level: Option<f64> = row.get("stress_level");
    let resting_heart_rate: Option<i64> = row.get("resting_heart_rate");
    let body_temperature: Option<f64> = row.get("body_temperature");
    let resting_respiratory_rate: Option<f64> = row.get("resting_respiratory_rate");

    StoredRecoveryMetrics {
        id: row.get("id"),
        user_id: row.get("user_id"),
        data_source_id: row
            .get::<Option<String>, _>("data_source_id")
            .unwrap_or_default(),
        date,
        recovery_score: recovery_score.map(|v| v as u32),
        readiness_score: readiness_score.map(|v| v as u32),
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: resting_heart_rate.map(|v| v as u32),
        stress_score: stress_level.map(|v| v as u32),
        body_battery: None,
        spo2: None,
        respiratory_rate: resting_respiratory_rate,
        skin_temp_deviation: body_temperature,
        source_name: row.get("provider"),
        recorded_at,
    }
}

// ============================================================================
// HealthSnapshotRepository
// ============================================================================

#[async_trait]
impl HealthSnapshotRepository for PostgresDatabase {
    async fn upsert_health_snapshot(
        &self,
        tenant_id: &TenantId,
        snapshot: &StoredHealthMetrics,
    ) -> AppResult<String> {
        let id = if snapshot.id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            snapshot.id.clone()
        };
        let now = Utc::now();
        let bp_systolic = snapshot.systolic_bp.map(i64::from);
        let bp_diastolic = snapshot.diastolic_bp.map(i64::from);

        sqlx::query(
            r"
            INSERT INTO health_snapshots (id, user_id, tenant_id, provider, data_source_id, synced_at, date, weight, body_fat_percentage, muscle_mass, bone_mass, body_water_percentage, bp_systolic, bp_diastolic, blood_glucose, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT(user_id, tenant_id, provider, date) DO UPDATE SET
                weight = EXCLUDED.weight,
                body_fat_percentage = EXCLUDED.body_fat_percentage,
                muscle_mass = EXCLUDED.muscle_mass,
                bone_mass = EXCLUDED.bone_mass,
                body_water_percentage = EXCLUDED.body_water_percentage,
                bp_systolic = EXCLUDED.bp_systolic,
                bp_diastolic = EXCLUDED.bp_diastolic,
                blood_glucose = EXCLUDED.blood_glucose
            ",
        )
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
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert health snapshot: {e}")))?;

        Ok(id)
    }

    async fn get_health_snapshots(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredHealthMetrics>> {
        let user_id_str = user_id.to_string();

        let rows = sqlx::query(
            r"
            SELECT id, user_id, provider, data_source_id, date, weight, body_fat_percentage,
                   muscle_mass, bone_mass, body_water_percentage, bp_systolic, bp_diastolic,
                   blood_glucose, created_at
            FROM health_snapshots
            WHERE user_id = $1 AND tenant_id = $2 AND date >= $3 AND date <= $4
              AND deleted_at IS NULL
            ORDER BY date DESC
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .bind(start.date_naive())
        .bind(end.date_naive())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get health snapshots: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| pg_row_to_stored_health_metrics(&r))
            .collect())
    }

    async fn get_latest_health_snapshot(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredHealthMetrics>> {
        let user_id_str = user_id.to_string();

        let row = sqlx::query(
            r"
            SELECT id, user_id, provider, data_source_id, date, weight, body_fat_percentage,
                   muscle_mass, bone_mass, body_water_percentage, bp_systolic, bp_diastolic,
                   blood_glucose, created_at
            FROM health_snapshots
            WHERE user_id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            ORDER BY date DESC
            LIMIT 1
            ",
        )
        .bind(&user_id_str)
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get latest health snapshot: {e}")))?;

        Ok(row.map(|r| pg_row_to_stored_health_metrics(&r)))
    }

    async fn delete_health_snapshot_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool> {
        let result = if soft {
            sqlx::query(
                "UPDATE health_snapshots SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL",
            )
            .bind(Utc::now())
            .bind(id)
            .bind(tenant_id.to_string())
            .execute(&self.pool)
            .await
        } else {
            sqlx::query("DELETE FROM health_snapshots WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id.to_string())
                .execute(&self.pool)
                .await
        }
        .map_err(|e| AppError::database(format!("Failed to delete health snapshot by id: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn find_health_snapshot_tenant(&self, id: &str) -> AppResult<Option<TenantId>> {
        let row = sqlx::query("SELECT tenant_id FROM health_snapshots WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to find health snapshot tenant: {e}"))
            })?;

        row.map(|r| {
            let s: String = r.get("tenant_id");
            s.parse::<TenantId>()
                .map_err(|e| AppError::database(format!("Invalid tenant_id stored: {e}")))
        })
        .transpose()
    }
}

/// Map a `PostgreSQL` row to a `StoredHealthMetrics`.
fn pg_row_to_stored_health_metrics(row: &PgRow) -> StoredHealthMetrics {
    let date: NaiveDate = row.get("date");
    let recorded_at: DateTime<Utc> = row.get("created_at");
    let bp_systolic: Option<i64> = row.get("bp_systolic");
    let bp_diastolic: Option<i64> = row.get("bp_diastolic");

    StoredHealthMetrics {
        id: row.get("id"),
        user_id: row.get("user_id"),
        data_source_id: row
            .get::<Option<String>, _>("data_source_id")
            .unwrap_or_default(),
        date,
        weight_kg: row.get("weight"),
        body_fat_pct: row.get("body_fat_percentage"),
        muscle_mass_kg: row.get("muscle_mass"),
        bmi: None,
        bone_mass_kg: row.get("bone_mass"),
        water_pct: row.get("body_water_percentage"),
        systolic_bp: bp_systolic.map(|v| v as u32),
        diastolic_bp: bp_diastolic.map(|v| v as u32),
        blood_glucose: row.get("blood_glucose"),
        source_name: row.get("provider"),
        recorded_at,
    }
}

// ============================================================================
// SyncCursorRepository
// ============================================================================

#[async_trait]
impl SyncCursorRepository for PostgresDatabase {
    async fn get_sync_cursor(
        &self,
        user_id: &str,
        tenant_id: &TenantId,
        provider: &str,
        data_type: &str,
    ) -> AppResult<Option<SyncCursorRow>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, provider, data_type, cursor_value,
                   last_sync_at, last_sync_status, records_synced, error_message,
                   retry_count, next_retry_at
            FROM sync_state
            WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND data_type = $4
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .bind(provider)
        .bind(data_type)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get sync cursor: {e}")))?;

        Ok(row.map(|r| SyncCursorRow {
            id: r.get("id"),
            user_id: r.get("user_id"),
            tenant_id: r.get("tenant_id"),
            provider: r.get("provider"),
            data_type: r.get("data_type"),
            cursor_value: r.get("cursor_value"),
            last_sync_at: r.get("last_sync_at"),
            last_sync_status: r.get("last_sync_status"),
            records_synced: r.get("records_synced"),
            error_message: r.get("error_message"),
            retry_count: r.get("retry_count"),
            next_retry_at: r.get("next_retry_at"),
        }))
    }

    async fn upsert_sync_cursor(&self, cursor: &SyncCursorRow) -> AppResult<()> {
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO sync_state (id, user_id, tenant_id, provider, data_type, cursor_value,
                last_sync_at, last_sync_status, records_synced, error_message, retry_count,
                next_retry_at, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT(user_id, tenant_id, provider, data_type) DO UPDATE SET
                cursor_value = EXCLUDED.cursor_value,
                last_sync_at = EXCLUDED.last_sync_at,
                last_sync_status = EXCLUDED.last_sync_status,
                records_synced = EXCLUDED.records_synced,
                error_message = EXCLUDED.error_message,
                retry_count = EXCLUDED.retry_count,
                next_retry_at = EXCLUDED.next_retry_at,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(&cursor.id)
        .bind(&cursor.user_id)
        .bind(&cursor.tenant_id)
        .bind(&cursor.provider)
        .bind(&cursor.data_type)
        .bind(&cursor.cursor_value)
        .bind(&cursor.last_sync_at)
        .bind(&cursor.last_sync_status)
        .bind(cursor.records_synced)
        .bind(&cursor.error_message)
        .bind(cursor.retry_count)
        .bind(&cursor.next_retry_at)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert sync cursor: {e}")))?;

        Ok(())
    }

    async fn list_connected_provider_users(
        &self,
        provider: &str,
    ) -> AppResult<Vec<ConnectedUserRow>> {
        let rows = sqlx::query(
            r"
            SELECT DISTINCT user_id, tenant_id
            FROM user_oauth_tokens
            WHERE provider = $1
            ",
        )
        .bind(provider)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list connected provider users: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| ConnectedUserRow {
                user_id: r.get("user_id"),
                tenant_id: r.get("tenant_id"),
            })
            .collect())
    }
}

#[async_trait]
impl TimeSeriesPointRepository for PostgresDatabase {
    async fn insert_continuous_metrics_batch(
        &self,
        data_source_id: &str,
        series_type_id: u32,
        points: &[(DateTime<Utc>, f64)],
    ) -> AppResult<u64> {
        if points.is_empty() {
            return Ok(0);
        }

        // ON CONFLICT(data_source_id, series_type_id, recorded_at) DO UPDATE
        // gives last-writer-wins for the unique key, matching dravr-riviere
        // semantics ("second insert at the same timestamp replaces the first").
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::database(format!("begin tx for time-series insert: {e}")))?;

        let mut written: u64 = 0;
        for (recorded_at, value) in points {
            let id = Uuid::new_v4().to_string();
            let res = sqlx::query(
                r"
                INSERT INTO data_point_series
                    (id, data_source_id, series_type_id, recorded_at, zone_offset, value)
                VALUES ($1, $2, $3, $4, NULL, $5)
                ON CONFLICT(data_source_id, series_type_id, recorded_at) DO UPDATE SET
                    value = EXCLUDED.value
                ",
            )
            .bind(&id)
            .bind(data_source_id)
            .bind(i64::from(series_type_id))
            .bind(recorded_at)
            .bind(*value)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::database(format!("insert data_point_series row: {e}")))?;
            written += res.rows_affected();
        }

        tx.commit()
            .await
            .map_err(|e| AppError::database(format!("commit time-series insert: {e}")))?;

        Ok(written)
    }

    async fn get_continuous_metrics(
        &self,
        data_source_id: &str,
        series_type_id: u32,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<(DateTime<Utc>, f64)>> {
        let rows = sqlx::query(
            r"
            SELECT recorded_at, value
            FROM data_point_series
            WHERE data_source_id = $1
              AND series_type_id = $2
              AND recorded_at >= $3
              AND recorded_at <= $4
              AND deleted_at IS NULL
            ORDER BY recorded_at ASC
            ",
        )
        .bind(data_source_id)
        .bind(i64::from(series_type_id))
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("get_continuous_metrics: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let recorded_at: DateTime<Utc> = r.get("recorded_at");
            let value: f64 = r.get("value");
            out.push((recorded_at, value));
        }
        Ok(out)
    }
}
