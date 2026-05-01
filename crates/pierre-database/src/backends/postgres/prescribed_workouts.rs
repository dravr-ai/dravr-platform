// ABOUTME: PostgreSQL implementation of PrescribedWorkoutRepository (Endurance Phase 5)
// ABOUTME: Mirrors crates/pierre-database/src/database/prescribed_workouts.rs for the Postgres tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{PrescribedWorkout, SportType, TenantId};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::PrescribedWorkoutRepository;

const MAX_LIST_LIMIT: u32 = 200;

#[async_trait]
impl PrescribedWorkoutRepository for PostgresDatabase {
    async fn upsert_prescribed_workout(&self, prescribed: &PrescribedWorkout) -> AppResult<()> {
        let sport = serde_json::to_string(&prescribed.sport)
            .map_err(|e| AppError::database(format!("serialize sport: {e}")))?;
        let payload: Value = serde_json::from_str(&prescribed.payload_json)
            .map_err(|e| AppError::database(format!("parse payload_json: {e}")))?;
        sqlx::query(
            r"
            INSERT INTO prescribed_workouts (
                id, tenant_id, user_id, coach_id, template_slug, sport,
                prescribed_for_date, provider, provider_event_id,
                payload_json, status, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (id) DO UPDATE SET
                provider_event_id = EXCLUDED.provider_event_id,
                status = EXCLUDED.status
            ",
        )
        .bind(prescribed.id)
        .bind(prescribed.tenant_id)
        .bind(prescribed.user_id)
        .bind(prescribed.coach_id.as_deref())
        .bind(&prescribed.template_slug)
        .bind(&sport)
        .bind(prescribed.prescribed_for_date)
        .bind(&prescribed.provider)
        .bind(prescribed.provider_event_id.as_deref())
        .bind(payload)
        .bind(&prescribed.status)
        .bind(prescribed.created_at)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("upsert_prescribed_workout: {e}")))?;
        Ok(())
    }

    async fn list_prescribed_workouts(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<PrescribedWorkout>> {
        let bounded = limit.clamp(1, MAX_LIST_LIMIT);
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_id, template_slug, sport,
                   prescribed_for_date, provider, provider_event_id,
                   payload_json, status, created_at
            FROM prescribed_workouts
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY created_at DESC
            LIMIT $3
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(i64::from(bounded))
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list_prescribed_workouts: {e}")))?;

        rows.iter().map(row_to_prescribed).collect()
    }
}

fn row_to_prescribed(row: &PgRow) -> AppResult<PrescribedWorkout> {
    let id: Uuid = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("read id: {e}")))?;
    let tenant_id: Uuid = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("read tenant_id: {e}")))?;
    let user_id: Uuid = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("read user_id: {e}")))?;
    let sport_str: String = row
        .try_get("sport")
        .map_err(|e| AppError::database(format!("read sport: {e}")))?;
    let sport: SportType = serde_json::from_str(&sport_str)
        .map_err(|e| AppError::database(format!("parse sport: {e}")))?;
    let prescribed_for_date: NaiveDate = row
        .try_get("prescribed_for_date")
        .map_err(|e| AppError::database(format!("read prescribed_for_date: {e}")))?;
    let created_at: DateTime<Utc> = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("read created_at: {e}")))?;
    let payload: Value = row
        .try_get("payload_json")
        .map_err(|e| AppError::database(format!("read payload_json: {e}")))?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| AppError::database(format!("serialize payload_json: {e}")))?;

    Ok(PrescribedWorkout {
        id,
        tenant_id,
        user_id,
        coach_id: row.try_get("coach_id").ok(),
        template_slug: row
            .try_get("template_slug")
            .map_err(|e| AppError::database(format!("read template_slug: {e}")))?,
        sport,
        prescribed_for_date,
        provider: row
            .try_get("provider")
            .map_err(|e| AppError::database(format!("read provider: {e}")))?,
        provider_event_id: row.try_get("provider_event_id").ok(),
        payload_json,
        status: row
            .try_get("status")
            .map_err(|e| AppError::database(format!("read status: {e}")))?,
        created_at,
    })
}
