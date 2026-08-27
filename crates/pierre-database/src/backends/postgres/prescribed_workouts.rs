// ABOUTME: PostgreSQL implementation of PrescribedWorkoutRepository — the ledger of calendar entries Dravr wrote to a provider
// ABOUTME: Mirrors crates/pierre-database/src/database/prescribed_workouts.rs for the Postgres tier
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CalendarEventSource, PrescribedWorkout, SportType, TenantId};
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
                prescribed_for_date, provider, provider_event_id, external_id,
                source, plan_week_id, replaces_id, payload_json, payload_hash,
                status, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            ON CONFLICT (id) DO UPDATE SET
                provider_event_id = EXCLUDED.provider_event_id,
                status = EXCLUDED.status,
                payload_json = EXCLUDED.payload_json,
                payload_hash = EXCLUDED.payload_hash,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(prescribed.id)
        .bind(prescribed.tenant_id)
        .bind(prescribed.user_id)
        .bind(prescribed.coach_id.as_deref())
        .bind(prescribed.template_slug.as_deref())
        .bind(&sport)
        .bind(prescribed.prescribed_for_date)
        .bind(&prescribed.provider)
        .bind(prescribed.provider_event_id.as_deref())
        .bind(prescribed.external_id.as_deref())
        .bind(prescribed.source.as_str())
        .bind(prescribed.plan_week_id.as_deref())
        .bind(prescribed.replaces_id)
        .bind(payload)
        .bind(prescribed.payload_hash.as_deref())
        .bind(&prescribed.status)
        .bind(prescribed.created_at)
        .bind(prescribed.updated_at)
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
                   prescribed_for_date, provider, provider_event_id, external_id,
                   source, plan_week_id, replaces_id, payload_json, payload_hash,
                   status, created_at, updated_at
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

    async fn get_prescribed_workout(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        id: Uuid,
    ) -> AppResult<Option<PrescribedWorkout>> {
        let row = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_id, template_slug, sport,
                   prescribed_for_date, provider, provider_event_id, external_id,
                   source, plan_week_id, replaces_id, payload_json, payload_hash,
                   status, created_at, updated_at
            FROM prescribed_workouts
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("get_prescribed_workout: {e}")))?;

        row.as_ref().map(row_to_prescribed).transpose()
    }

    async fn list_live_calendar_events(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        provider: &str,
        from: Option<NaiveDate>,
    ) -> AppResult<Vec<PrescribedWorkout>> {
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_id, template_slug, sport,
                   prescribed_for_date, provider, provider_event_id, external_id,
                   source, plan_week_id, replaces_id, payload_json, payload_hash,
                   status, created_at, updated_at
            FROM prescribed_workouts
            WHERE tenant_id = $1 AND user_id = $2 AND provider = $3
              AND status = $4
              AND ($5::date IS NULL OR prescribed_for_date >= $5)
            ORDER BY prescribed_for_date ASC, created_at ASC
            ",
        )
        .bind(tenant_id.as_uuid())
        .bind(user_id)
        .bind(provider)
        .bind(PrescribedWorkout::STATUS_PUSHED)
        .bind(from)
        .fetch_all(self.pool())
        .await
        .map_err(|e| AppError::database(format!("list_live_calendar_events: {e}")))?;

        rows.iter().map(row_to_prescribed).collect()
    }

    async fn set_prescribed_workout_status(
        &self,
        tenant_id: TenantId,
        id: Uuid,
        status: &str,
    ) -> AppResult<()> {
        let result = sqlx::query(
            r"
            UPDATE prescribed_workouts
            SET status = $1, updated_at = $2
            WHERE tenant_id = $3 AND id = $4
            ",
        )
        .bind(status)
        .bind(Utc::now())
        .bind(tenant_id.as_uuid())
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("set_prescribed_workout_status: {e}")))?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!(
                "prescribed workout {id} not found in tenant {tenant_id}"
            )));
        }
        Ok(())
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
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("read updated_at: {e}")))?;
    let payload: Value = row
        .try_get("payload_json")
        .map_err(|e| AppError::database(format!("read payload_json: {e}")))?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| AppError::database(format!("serialize payload_json: {e}")))?;
    let source_str: String = row
        .try_get("source")
        .map_err(|e| AppError::database(format!("read source: {e}")))?;
    let source = CalendarEventSource::parse(&source_str)
        .ok_or_else(|| AppError::database(format!("unknown source '{source_str}'")))?;

    Ok(PrescribedWorkout {
        id,
        tenant_id,
        user_id,
        // Decoded as `Option<String>` rather than `try_get::<String>().ok()`:
        // the `.ok()` form swallows every decode error, so a genuinely absent
        // value and a column this mapper failed to read are indistinguishable.
        // The SQLite tier has the sharper version of the same bug (NULL TEXT
        // decodes to an empty string there), and both backends serve the same
        // trait, so they agree here.
        coach_id: row
            .try_get("coach_id")
            .map_err(|e| AppError::database(format!("read coach_id: {e}")))?,
        template_slug: row
            .try_get("template_slug")
            .map_err(|e| AppError::database(format!("read template_slug: {e}")))?,
        sport,
        prescribed_for_date,
        provider: row
            .try_get("provider")
            .map_err(|e| AppError::database(format!("read provider: {e}")))?,
        provider_event_id: row
            .try_get("provider_event_id")
            .map_err(|e| AppError::database(format!("read provider_event_id: {e}")))?,
        external_id: row
            .try_get("external_id")
            .map_err(|e| AppError::database(format!("read external_id: {e}")))?,
        source,
        plan_week_id: row
            .try_get("plan_week_id")
            .map_err(|e| AppError::database(format!("read plan_week_id: {e}")))?,
        replaces_id: row
            .try_get("replaces_id")
            .map_err(|e| AppError::database(format!("read replaces_id: {e}")))?,
        payload_hash: row
            .try_get("payload_hash")
            .map_err(|e| AppError::database(format!("read payload_hash: {e}")))?,
        payload_json,
        status: row
            .try_get("status")
            .map_err(|e| AppError::database(format!("read status: {e}")))?,
        created_at,
        updated_at,
    })
}
