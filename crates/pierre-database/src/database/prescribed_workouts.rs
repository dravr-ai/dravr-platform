// ABOUTME: SQLite implementation of PrescribedWorkoutRepository — the ledger of calendar entries Dravr wrote to a provider
// ABOUTME: Upsert by id, tenant-scoped reads (by id, recent, live-per-provider), and terminal status transitions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CalendarEventSource, PrescribedWorkout, SportType, TenantId};
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
                prescribed_for_date, provider, provider_event_id, external_id,
                source, plan_week_id, replaces_id, payload_json, payload_hash,
                status, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                provider_event_id = excluded.provider_event_id,
                status = excluded.status,
                payload_json = excluded.payload_json,
                payload_hash = excluded.payload_hash,
                updated_at = excluded.updated_at
            ",
        )
        .bind(prescribed.id.to_string())
        .bind(prescribed.tenant_id.to_string())
        .bind(prescribed.user_id.to_string())
        .bind(prescribed.coach_id.as_deref())
        .bind(prescribed.template_slug.as_deref())
        .bind(&sport)
        .bind(
            prescribed
                .prescribed_for_date
                .format(ISO_DATE_FMT)
                .to_string(),
        )
        .bind(&prescribed.provider)
        .bind(prescribed.provider_event_id.as_deref())
        .bind(prescribed.external_id.as_deref())
        .bind(prescribed.source.as_str())
        .bind(prescribed.plan_week_id.as_deref())
        .bind(prescribed.replaces_id.map(|id| id.to_string()))
        .bind(&prescribed.payload_json)
        .bind(prescribed.payload_hash.as_deref())
        .bind(&prescribed.status)
        .bind(prescribed.created_at.to_rfc3339())
        .bind(prescribed.updated_at.to_rfc3339())
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
            WHERE tenant_id = ? AND user_id = ? AND id = ?
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(id.to_string())
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
        let from_str = from.map(|d| d.format(ISO_DATE_FMT).to_string());
        let rows = sqlx::query(
            r"
            SELECT id, tenant_id, user_id, coach_id, template_slug, sport,
                   prescribed_for_date, provider, provider_event_id, external_id,
                   source, plan_week_id, replaces_id, payload_json, payload_hash,
                   status, created_at, updated_at
            FROM prescribed_workouts
            WHERE tenant_id = ? AND user_id = ? AND provider = ?
              AND status = ?
              AND (? IS NULL OR prescribed_for_date >= ?)
            ORDER BY prescribed_for_date ASC, created_at ASC
            ",
        )
        .bind(tenant_id.to_string())
        .bind(user_id.to_string())
        .bind(provider)
        .bind(PrescribedWorkout::STATUS_PUSHED)
        .bind(from_str.as_deref())
        .bind(from_str.as_deref())
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
            SET status = ?, updated_at = ?
            WHERE tenant_id = ? AND id = ?
            ",
        )
        .bind(status)
        .bind(Utc::now().to_rfc3339())
        .bind(tenant_id.to_string())
        .bind(id.to_string())
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

fn parse_rfc3339(column: &str, raw: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::database(format!("parse {column}: {e}")))
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
    let created_at = parse_rfc3339("created_at", &created_at_str)?;
    let updated_at_str: String = row
        .try_get("updated_at")
        .map_err(|e| AppError::database(format!("read updated_at: {e}")))?;
    let updated_at = parse_rfc3339("updated_at", &updated_at_str)?;
    let source_str: String = row
        .try_get("source")
        .map_err(|e| AppError::database(format!("read source: {e}")))?;
    let source = CalendarEventSource::parse(&source_str)
        .ok_or_else(|| AppError::database(format!("unknown source '{source_str}'")))?;
    let replaces_id_str: Option<String> = row
        .try_get("replaces_id")
        .map_err(|e| AppError::database(format!("read replaces_id: {e}")))?;
    let replaces_id = replaces_id_str
        .map(|s| {
            Uuid::parse_str(&s).map_err(|e| AppError::database(format!("parse replaces_id: {e}")))
        })
        .transpose()?;

    Ok(PrescribedWorkout {
        id,
        tenant_id,
        user_id,
        // Decoded as `Option<String>`, never `try_get::<String>().ok()`:
        // SQLite hands a NULL TEXT column back as an empty string rather than
        // an error, so the `.ok()` form silently turns "no value" into
        // `Some("")`. For provider_event_id that is the difference between a
        // prescription the provider never created and one whose calendar event
        // id is the empty string.
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
        replaces_id,
        payload_hash: row
            .try_get("payload_hash")
            .map_err(|e| AppError::database(format!("read payload_hash: {e}")))?,
        payload_json: row
            .try_get("payload_json")
            .map_err(|e| AppError::database(format!("read payload_json: {e}")))?,
        status: row
            .try_get("status")
            .map_err(|e| AppError::database(format!("read status: {e}")))?,
        created_at,
        updated_at,
    })
}
