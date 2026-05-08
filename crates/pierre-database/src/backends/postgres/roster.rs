// ABOUTME: PostgreSQL implementation of RosterRepository — coach-athlete junction CRUD
// ABOUTME: Mirrors the SQLite path; uses native UUID and TIMESTAMPTZ types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAthleteAssignment, TenantId};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::RosterRepository;

fn row_to_assignment(row: &PgRow) -> CoachAthleteAssignment {
    let assigned_at: DateTime<Utc> = row.get("assigned_at");
    let revoked_at: Option<DateTime<Utc>> = row.get("revoked_at");
    let tenant_uuid: Uuid = row.get("tenant_id");

    CoachAthleteAssignment {
        id: row.get("id"),
        coach_user_id: row.get("coach_user_id"),
        athlete_user_id: row.get("athlete_user_id"),
        tenant_id: TenantId::from_uuid(tenant_uuid),
        assigned_by: row.get("assigned_by"),
        assigned_at,
        revoked_at,
        revoked_by: row.get("revoked_by"),
    }
}

#[async_trait]
impl RosterRepository for PostgresDatabase {
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
            WHERE coach_user_id = $1
              AND tenant_id = $2
              AND revoked_at IS NULL
            ORDER BY assigned_at DESC
            ",
        )
        .bind(coach_user_id)
        .bind(tenant_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list athletes for coach: {e}")))?;

        Ok(rows.iter().map(row_to_assignment).collect())
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
            WHERE athlete_user_id = $1
              AND tenant_id = $2
              AND revoked_at IS NULL
            ORDER BY assigned_at DESC
            ",
        )
        .bind(athlete_user_id)
        .bind(tenant_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list coaches for athlete: {e}")))?;

        Ok(rows.iter().map(row_to_assignment).collect())
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
            VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL)
            ON CONFLICT DO NOTHING
            ",
        )
        .bind(assignment.id)
        .bind(assignment.coach_user_id)
        .bind(assignment.athlete_user_id)
        .bind(assignment.tenant_id.as_uuid())
        .bind(assignment.assigned_by)
        .bind(assignment.assigned_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to assign athlete: {e}")))?;

        if result.rows_affected() == 0 {
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
        let result = sqlx::query(
            r"
            UPDATE coach_athlete_assignments
            SET revoked_at = NOW(),
                revoked_by = $1
            WHERE coach_user_id = $2
              AND athlete_user_id = $3
              AND tenant_id = $4
              AND revoked_at IS NULL
            ",
        )
        .bind(revoked_by)
        .bind(coach_user_id)
        .bind(athlete_user_id)
        .bind(tenant_id.as_uuid())
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
            WHERE coach_user_id = $1
              AND athlete_user_id = $2
              AND tenant_id = $3
              AND revoked_at IS NULL
            ",
        )
        .bind(coach_user_id)
        .bind(athlete_user_id)
        .bind(tenant_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?;

        Ok(count > 0)
    }
}
