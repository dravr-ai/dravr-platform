// ABOUTME: SQLite implementation of RosterRepository — coach-athlete junction CRUD
// ABOUTME: Active rows are revoked_at IS NULL; revoked rows stay for audit history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAthleteAssignment, TenantId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use uuid::Uuid;

use super::Database;
use crate::repositories::RosterRepository;

fn parse_datetime(value: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::internal(format!("Invalid RFC3339 datetime {value}: {e}")))
}

fn parse_uuid_text(value: &str, column: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value)
        .map_err(|e| AppError::internal(format!("Invalid UUID in column {column}: {e}")))
}

fn parse_optional_uuid_text(value: Option<String>, column: &str) -> AppResult<Option<Uuid>> {
    value.map_or(Ok(None), |v| parse_uuid_text(&v, column).map(Some))
}

fn row_to_assignment(row: &SqliteRow) -> AppResult<CoachAthleteAssignment> {
    let id_str: String = row.get("id");
    let coach_str: String = row.get("coach_user_id");
    let athlete_str: String = row.get("athlete_user_id");
    let tenant_str: String = row.get("tenant_id");
    let assigned_by: Option<String> = row.get("assigned_by");
    let assigned_at_str: String = row.get("assigned_at");
    let revoked_at_str: Option<String> = row.get("revoked_at");
    let revoked_by: Option<String> = row.get("revoked_by");

    Ok(CoachAthleteAssignment {
        id: parse_uuid_text(&id_str, "id")?,
        coach_user_id: parse_uuid_text(&coach_str, "coach_user_id")?,
        athlete_user_id: parse_uuid_text(&athlete_str, "athlete_user_id")?,
        tenant_id: TenantId::from_uuid(parse_uuid_text(&tenant_str, "tenant_id")?),
        assigned_by: parse_optional_uuid_text(assigned_by, "assigned_by")?,
        assigned_at: parse_datetime(&assigned_at_str)?,
        revoked_at: revoked_at_str.as_deref().map(parse_datetime).transpose()?,
        revoked_by: parse_optional_uuid_text(revoked_by, "revoked_by")?,
    })
}

#[async_trait]
impl RosterRepository for Database {
    async fn list_athletes_for_coach(
        &self,
        coach_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAthleteAssignment>> {
        let rows = sqlx::query(
            r"
            SELECT id, coach_user_id, athlete_user_id, tenant_id,
                   assigned_by, assigned_at, revoked_at, revoked_by
            FROM coach_athlete_assignments
            WHERE coach_user_id = ?1
              AND tenant_id = ?2
              AND revoked_at IS NULL
            ORDER BY assigned_at DESC
            ",
        )
        .bind(coach_user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list athletes for coach: {e}")))?;

        rows.iter().map(row_to_assignment).collect()
    }

    async fn list_coaches_for_athlete(
        &self,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAthleteAssignment>> {
        let rows = sqlx::query(
            r"
            SELECT id, coach_user_id, athlete_user_id, tenant_id,
                   assigned_by, assigned_at, revoked_at, revoked_by
            FROM coach_athlete_assignments
            WHERE athlete_user_id = ?1
              AND tenant_id = ?2
              AND revoked_at IS NULL
            ORDER BY assigned_at DESC
            ",
        )
        .bind(athlete_user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list coaches for athlete: {e}")))?;

        rows.iter().map(row_to_assignment).collect()
    }

    async fn assign_athlete(
        &self,
        assignment: &CoachAthleteAssignment,
    ) -> AppResult<Option<CoachAthleteAssignment>> {
        let result = sqlx::query(
            r"
            INSERT INTO coach_athlete_assignments
                (id, coach_user_id, athlete_user_id, tenant_id,
                 assigned_by, assigned_at, revoked_at, revoked_by)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL)
            ON CONFLICT DO NOTHING
            ",
        )
        .bind(assignment.id.to_string())
        .bind(assignment.coach_user_id.to_string())
        .bind(assignment.athlete_user_id.to_string())
        .bind(assignment.tenant_id.as_uuid().to_string())
        .bind(assignment.assigned_by.map(|u| u.to_string()))
        .bind(assignment.assigned_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to assign athlete: {e}")))?;

        if result.rows_affected() == 0 {
            // Active assignment for the same (coach, athlete, tenant) already exists.
            return Ok(None);
        }
        Ok(Some(assignment.clone()))
    }

    async fn revoke_assignment(
        &self,
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
        revoked_by: Option<Uuid>,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            r"
            UPDATE coach_athlete_assignments
            SET revoked_at = ?1,
                revoked_by = ?2
            WHERE coach_user_id = ?3
              AND athlete_user_id = ?4
              AND tenant_id = ?5
              AND revoked_at IS NULL
            ",
        )
        .bind(&now)
        .bind(revoked_by.map(|u| u.to_string()))
        .bind(coach_user_id.to_string())
        .bind(athlete_user_id.to_string())
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to revoke assignment: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn is_athlete_managed_by(
        &self,
        coach_user_id: Uuid,
        athlete_user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM coach_athlete_assignments
            WHERE coach_user_id = ?1
              AND athlete_user_id = ?2
              AND tenant_id = ?3
              AND revoked_at IS NULL
            ",
        )
        .bind(coach_user_id.to_string())
        .bind(athlete_user_id.to_string())
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?;

        Ok(count > 0)
    }
}
