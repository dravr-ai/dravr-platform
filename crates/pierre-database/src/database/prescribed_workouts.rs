// ABOUTME: SQLite implementation of PrescribedWorkoutRepository (Endurance Phase 5)
// ABOUTME: Audit-trail upsert + recent-list read for the prescribe-to-provider path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{PrescribedWorkout, SportType, TenantId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::PrescribedWorkoutRepository;

const ISO_DATE_FMT: &str = "%Y-%m-%d";
const MAX_LIST_LIMIT: u32 = 200;

#[async_trait]
impl PrescribedWorkoutRepository for Database {
    async fn upsert_prescribed_workout(&self, prescribed: &PrescribedWorkout) -> AppResult<()> {
        let sport = serde_json::to_string(&prescribed.sport)
            .map_err(|e| AppError::database(format!("serialize sport: {e}")))?;
        sqlx::query(
            r"
            INSERT INTO prescribed_workouts (
                id, tenant_id, user_id, coach_id, template_slug, sport,
                prescribed_for_date, provider, provider_event_id,
                payload_json, status, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                provider_event_id = excluded.provider_event_id,
                status = excluded.status
            ",
        )
        .bind(prescribed.id.to_string())
        .bind(prescribed.tenant_id.to_string())
        .bind(prescribed.user_id.to_string())
        .bind(prescribed.coach_id.as_deref())
        .bind(&prescribed.template_slug)
        .bind(&sport)
        .bind(
            prescribed
                .prescribed_for_date
                .format(ISO_DATE_FMT)
                .to_string(),
        )
        .bind(&prescribed.provider)
        .bind(prescribed.provider_event_id.as_deref())
        .bind(&prescribed.payload_json)
        .bind(&prescribed.status)
        .bind(prescribed.created_at.to_rfc3339())
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
            WHERE tenant_id = ? AND user_id = ?
            ORDER BY created_at DESC
            LIMIT ?
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(i64::from(bounded))
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list_prescribed_workouts: {e}")))?;

        rows.iter().map(row_to_prescribed).collect()
    }
}

fn row_to_prescribed(row: &SqliteRow) -> AppResult<PrescribedWorkout> {
    let id_str: String = row
        .try_get("id")
        .map_err(|e| AppError::database(format!("read id: {e}")))?;
    let id = Uuid::parse_str(&id_str)
        .map_err(|e| AppError::database(format!("parse id {id_str}: {e}")))?;
    let tenant_id_str: String = row
        .try_get("tenant_id")
        .map_err(|e| AppError::database(format!("read tenant_id: {e}")))?;
    let tenant_id = Uuid::parse_str(&tenant_id_str)
        .map_err(|e| AppError::database(format!("parse tenant_id: {e}")))?;
    let user_id_str: String = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("read user_id: {e}")))?;
    let user_id = Uuid::parse_str(&user_id_str)
        .map_err(|e| AppError::database(format!("parse user_id: {e}")))?;
    let sport_str: String = row
        .try_get("sport")
        .map_err(|e| AppError::database(format!("read sport: {e}")))?;
    let sport: SportType = serde_json::from_str(&sport_str)
        .map_err(|e| AppError::database(format!("parse sport: {e}")))?;
    let date_str: String = row
        .try_get("prescribed_for_date")
        .map_err(|e| AppError::database(format!("read prescribed_for_date: {e}")))?;
    let prescribed_for_date = NaiveDate::parse_from_str(&date_str, ISO_DATE_FMT)
        .map_err(|e| AppError::database(format!("parse prescribed_for_date: {e}")))?;
    let created_at_str: String = row
        .try_get("created_at")
        .map_err(|e| AppError::database(format!("read created_at: {e}")))?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("parse created_at: {e}")))?;

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
        payload_json: row
            .try_get("payload_json")
            .map_err(|e| AppError::database(format!("read payload_json: {e}")))?,
        status: row
            .try_get("status")
            .map_err(|e| AppError::database(format!("read status: {e}")))?,
        created_at,
    })
}
