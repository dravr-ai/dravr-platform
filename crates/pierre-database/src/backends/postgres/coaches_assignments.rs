// ABOUTME: PostgreSQL coach assignment and per-user visibility writes, split out of coaches.rs
// ABOUTME: Free functions over the pool so the CoachesRepository impl stays inside its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach assignment and visibility.
//!
//! Split out of the `CoachesRepository` impl purely for file size: a single
//! trait impl cannot span modules, so the bodies move here and the trait
//! methods delegate. Mirrors the `SQLite` side's `coaches_assignments`.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{Coach, CoachAssignment};
use pierre_core::models::TenantId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::coaches_rows::row_to_coach_pg;

pub(super) async fn assign_coach(
    pool: &PgPool,
    coach_id: &str,
    user_id: Uuid,
    assigned_by: Uuid,
) -> AppResult<bool> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    // Use INSERT ... ON CONFLICT DO NOTHING to handle duplicates gracefully
    let result = sqlx::query(
        r"
        INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
        VALUES ($1, $2, $3, $4, $5, FALSE, 0, NULL)
        ON CONFLICT (coach_id, user_id) DO NOTHING
        ",
    )
    .bind(id.to_string())
    .bind(coach_id)
    .bind(user_id)
    .bind(assigned_by)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;

    Ok(result.rows_affected() > 0)
}

pub(super) async fn unassign_coach(
    pool: &PgPool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    let result = sqlx::query(
        r"
        DELETE FROM coach_assignments
        WHERE coach_id = $1 AND user_id = $2
        ",
    )
    .bind(coach_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;

    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_assignments(
    pool: &PgPool,
    coach_id: &str,
) -> AppResult<Vec<CoachAssignment>> {
    let rows = sqlx::query(
        r"
        SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM coach_assignments ca
        LEFT JOIN users u ON ca.user_id = u.id
        WHERE ca.coach_id = $1
        ORDER BY ca.created_at DESC
        ",
    )
    .bind(coach_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;

    rows.iter()
        .map(|row| {
            let user_id: Uuid = row.get("user_id");
            let created_at: DateTime<Utc> = row.get("created_at");
            let assigned_by: Option<Uuid> = row.get("assigned_by");
            let user_email: Option<String> = row.get("email");

            Ok(CoachAssignment {
                user_id: user_id.to_string(),
                user_email,
                assigned_at: created_at.to_rfc3339(),
                assigned_by: assigned_by.map(|u| u.to_string()),
            })
        })
        .collect()
}

pub(super) async fn list_assignments_for_tenant(
    pool: &PgPool,
    coach_id: &str,
    tenant_id: TenantId,
) -> AppResult<Vec<CoachAssignment>> {
    let rows = sqlx::query(
        r"
        SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM coach_assignments ca
        LEFT JOIN users u ON ca.user_id = u.id
        INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
        WHERE ca.coach_id = $1
        ORDER BY ca.created_at DESC
        ",
    )
    .bind(coach_id)
    .bind(tenant_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;

    rows.iter()
        .map(|row| {
            let user_id: Uuid = row.get("user_id");
            let created_at: DateTime<Utc> = row.get("created_at");
            let assigned_by: Option<Uuid> = row.get("assigned_by");
            let user_email: Option<String> = row.get("email");

            Ok(CoachAssignment {
                user_id: user_id.to_string(),
                user_email,
                assigned_at: created_at.to_rfc3339(),
                assigned_by: assigned_by.map(|u| u.to_string()),
            })
        })
        .collect()
}

pub(super) async fn hide_coach(pool: &PgPool, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
    // Check if the coach is hideable (must be system or assigned, not personal)
    if !is_coach_hideable(pool, coach_id, user_id).await? {
        return Err(AppError::invalid_input(
            "Only system or assigned coaches can be hidden",
        ));
    }

    let id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        r"
        INSERT INTO user_coach_preferences (id, user_id, coach_id, is_hidden, created_at)
        VALUES ($1, $2, $3, TRUE, $4)
        ON CONFLICT(user_id, coach_id) DO UPDATE SET is_hidden = TRUE
        ",
    )
    .bind(id.to_string())
    .bind(user_id)
    .bind(coach_id)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;

    Ok(true)
}

pub(super) async fn show_coach(pool: &PgPool, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
    let result = sqlx::query(
        r"
        DELETE FROM user_coach_preferences
        WHERE coach_id = $1 AND user_id = $2 AND is_hidden = TRUE
        ",
    )
    .bind(coach_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;

    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_hidden_coaches(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Coach>> {
    let rows = sqlx::query(
        r"
        SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
               c.category, c.tags, c.sample_prompts, c.token_count,
               c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
               c.forked_from, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
               c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria
        FROM coaches c
        INNER JOIN user_coach_preferences ucp ON c.id = ucp.coach_id
        WHERE ucp.user_id = $1 AND ucp.is_hidden = TRUE AND c.tenant_id = $2
        ORDER BY c.title
        ",
    )
    .bind(user_id)
    .bind(tenant_id.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list hidden coaches: {e}")))?;

    rows.iter().map(row_to_coach_pg).collect()
}

/// Check if a coach can be hidden by a user
///
/// A coach is hideable if it's a system coach or assigned to the user,
/// but NOT if it's a personal coach created by the user.
async fn is_coach_hideable(pool: &PgPool, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
    // Check if it's a system coach (system coaches are visible across all tenants)
    let is_system = sqlx::query(
        r"
        SELECT 1 FROM coaches
        WHERE id = $1 AND is_system = TRUE
        ",
    )
    .bind(coach_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to check system coach: {e}")))?
    .is_some();

    if is_system {
        return Ok(true);
    }

    // Check if it's assigned to the user
    let is_assigned = sqlx::query(
        r"
        SELECT 1 FROM coach_assignments
        WHERE coach_id = $1 AND user_id = $2
        ",
    )
    .bind(coach_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?
    .is_some();

    Ok(is_assigned)
}
