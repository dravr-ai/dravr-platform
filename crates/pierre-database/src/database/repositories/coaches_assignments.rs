// ABOUTME: SQLite coach assignment and per-user visibility writes, split out of coaches_impl
// ABOUTME: Free functions over the pool so the CoachesRepository impl stays inside its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach assignment and visibility.
//!
//! Split out of the `CoachesRepository` impl purely for file size, mirroring
//! `coaches_versions`: a single trait impl cannot span modules, so the bodies
//! move here and the trait methods delegate.

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{Coach, CoachAssignment};
use pierre_core::models::TenantId;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::coaches_impl::is_coach_hideable;
use crate::database::coaches::row_to_coach;

pub(super) async fn assign_coach(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
    assigned_by: Uuid,
) -> AppResult<bool> {
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        r"INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
        VALUES ($1, $2, $3, $4, $5, 0, 0, NULL)",
    ).bind(id.to_string()).bind(coach_id).bind(user_id.to_string())
    .bind(assigned_by.to_string()).bind(&now).execute(pool).await
    .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn unassign_coach(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    let result = sqlx::query("DELETE FROM coach_assignments WHERE coach_id = $1 AND user_id = $2")
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_assignments(
    pool: &SqlitePool,
    coach_id: &str,
) -> AppResult<Vec<CoachAssignment>> {
    let rows = sqlx::query(
        r"SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM coach_assignments ca LEFT JOIN users u ON ca.user_id = u.id
        WHERE ca.coach_id = $1 ORDER BY ca.created_at DESC",
    )
    .bind(coach_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
    rows.iter()
        .map(|row| {
            Ok(CoachAssignment {
                user_id: row.get("user_id"),
                user_email: row.get("email"),
                assigned_at: row.get("created_at"),
                assigned_by: row.get("assigned_by"),
            })
        })
        .collect()
}

pub(super) async fn list_assignments_for_tenant(
    pool: &SqlitePool,
    coach_id: &str,
    tenant_id: TenantId,
) -> AppResult<Vec<CoachAssignment>> {
    let rows = sqlx::query(
        r"SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM coach_assignments ca LEFT JOIN users u ON ca.user_id = u.id
        INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
        WHERE ca.coach_id = $1 ORDER BY ca.created_at DESC",
    )
    .bind(coach_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
    rows.iter()
        .map(|row| {
            Ok(CoachAssignment {
                user_id: row.get("user_id"),
                user_email: row.get("email"),
                assigned_at: row.get("created_at"),
                assigned_by: row.get("assigned_by"),
            })
        })
        .collect()
}

pub(super) async fn hide_coach(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    if !is_coach_hideable(pool, coach_id, user_id).await? {
        return Err(AppError::invalid_input(
            "Only system or assigned coaches can be hidden",
        ));
    }
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r"INSERT INTO user_coach_preferences (id, user_id, coach_id, is_hidden, created_at)
        VALUES ($1, $2, $3, 1, $4) ON CONFLICT(user_id, coach_id) DO UPDATE SET is_hidden = 1",
    )
    .bind(id.to_string())
    .bind(user_id.to_string())
    .bind(coach_id)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;
    Ok(true)
}

pub(super) async fn show_coach(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    let result = sqlx::query(
        "DELETE FROM user_coach_preferences WHERE coach_id = $1 AND user_id = $2 AND is_hidden = 1",
    )
    .bind(coach_id)
    .bind(user_id.to_string())
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_hidden_coaches(
    pool: &SqlitePool,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Coach>> {
    let rows = sqlx::query(
        r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
               c.category, c.tags, c.sample_prompts, c.token_count,
               c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
               c.forked_from, c.slug, c.max_tool_iterations, c.temperature
        FROM coaches c
        INNER JOIN user_coach_preferences ucp ON c.id = ucp.coach_id
        WHERE ucp.user_id = $1 AND ucp.is_hidden = 1 AND c.tenant_id = $2
        ORDER BY c.title",
    )
    .bind(user_id.to_string())
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list hidden coaches: {e}")))?;
    rows.iter().map(row_to_coach).collect()
}
