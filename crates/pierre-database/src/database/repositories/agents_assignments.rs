// ABOUTME: SQLite agent assignment and per-user visibility writes, split out of agents_impl
// ABOUTME: Free functions over the pool so the AgentsRepository impl stays inside its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent assignment and visibility.
//!
//! Split out of the `AgentsRepository` impl purely for file size, mirroring
//! `agents_versions`: a single trait impl cannot span modules, so the bodies
//! move here and the trait methods delegate.

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{Agent, AgentAssignment};
use pierre_core::models::TenantId;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::agents_impl::is_agent_hideable;
use crate::database::agents::row_to_agent;

pub(super) async fn assign_agent(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: Uuid,
    assigned_by: Uuid,
) -> AppResult<bool> {
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        r"INSERT OR IGNORE INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
        VALUES ($1, $2, $3, $4, $5, 0, 0, NULL)",
    ).bind(id.to_string()).bind(agent_id).bind(user_id.to_string())
    .bind(assigned_by.to_string()).bind(&now).execute(pool).await
    .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn unassign_agent(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    let result = sqlx::query("DELETE FROM agent_assignments WHERE agent_id = $1 AND user_id = $2")
        .bind(agent_id)
        .bind(user_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_assignments(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<Vec<AgentAssignment>> {
    let rows = sqlx::query(
        r"SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM agent_assignments ca LEFT JOIN users u ON ca.user_id = u.id
        WHERE ca.agent_id = $1 ORDER BY ca.created_at DESC",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
    rows.iter()
        .map(|row| {
            Ok(AgentAssignment {
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
    agent_id: &str,
    tenant_id: TenantId,
) -> AppResult<Vec<AgentAssignment>> {
    let rows = sqlx::query(
        r"SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM agent_assignments ca LEFT JOIN users u ON ca.user_id = u.id
        INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
        WHERE ca.agent_id = $1 ORDER BY ca.created_at DESC",
    )
    .bind(agent_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;
    rows.iter()
        .map(|row| {
            Ok(AgentAssignment {
                user_id: row.get("user_id"),
                user_email: row.get("email"),
                assigned_at: row.get("created_at"),
                assigned_by: row.get("assigned_by"),
            })
        })
        .collect()
}

pub(super) async fn hide_agent(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<bool> {
    if !is_agent_hideable(pool, agent_id, user_id, tenant_id).await? {
        return Err(AppError::invalid_input(
            "Only system or assigned coaches can be hidden",
        ));
    }
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r"INSERT INTO user_agent_preferences (id, user_id, agent_id, is_hidden, created_at)
        VALUES ($1, $2, $3, 1, $4) ON CONFLICT(user_id, agent_id) DO UPDATE SET is_hidden = 1",
    )
    .bind(id.to_string())
    .bind(user_id.to_string())
    .bind(agent_id)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;
    Ok(true)
}

pub(super) async fn show_agent(
    pool: &SqlitePool,
    agent_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    // No tenant predicate: user_agent_preferences has no tenant_id column —
    // the hidden-set is a per-user preference (see the trait doc). The
    // caller's handler enforces the tenant gate.
    let result = sqlx::query(
        "DELETE FROM user_agent_preferences WHERE agent_id = $1 AND user_id = $2 AND is_hidden = 1",
    )
    .bind(agent_id)
    .bind(user_id.to_string())
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;
    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_hidden_agents(
    pool: &SqlitePool,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Agent>> {
    let rows = sqlx::query(
        r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
               c.category, c.tags, c.sample_prompts, c.token_count,
               c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
               c.forked_from, c.slug, c.max_tool_iterations, c.temperature
        FROM agents c
        INNER JOIN user_agent_preferences ucp ON c.id = ucp.agent_id
        WHERE ucp.user_id = $1 AND ucp.is_hidden = 1 AND c.tenant_id = $2
        ORDER BY c.title",
    )
    .bind(user_id.to_string())
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list hidden coaches: {e}")))?;
    rows.iter().map(row_to_agent).collect()
}
