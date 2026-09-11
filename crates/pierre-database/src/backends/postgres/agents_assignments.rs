// ABOUTME: PostgreSQL agent assignment and per-user visibility writes, split out of agents.rs
// ABOUTME: Free functions over the pool so the AgentsRepository impl stays inside its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent assignment and visibility.
//!
//! Split out of the `AgentsRepository` impl purely for file size: a single
//! trait impl cannot span modules, so the bodies move here and the trait
//! methods delegate. Mirrors the `SQLite` side's `agents_assignments`.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{Agent, AgentAssignment};
use pierre_core::models::TenantId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::agents_rows::row_to_agent_pg;

pub(super) async fn assign_agent(
    pool: &PgPool,
    agent_id: &str,
    user_id: Uuid,
    assigned_by: Uuid,
) -> AppResult<bool> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    // Use INSERT ... ON CONFLICT DO NOTHING to handle duplicates gracefully
    let result = sqlx::query(
        r"
        INSERT INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
        VALUES ($1, $2, $3, $4, $5, FALSE, 0, NULL)
        ON CONFLICT (agent_id, user_id) DO NOTHING
        ",
    )
    .bind(id.to_string())
    .bind(agent_id)
    .bind(user_id)
    .bind(assigned_by)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to assign coach: {e}")))?;

    Ok(result.rows_affected() > 0)
}

pub(super) async fn unassign_agent(
    pool: &PgPool,
    agent_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    let result = sqlx::query(
        r"
        DELETE FROM agent_assignments
        WHERE agent_id = $1 AND user_id = $2
        ",
    )
    .bind(agent_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to unassign coach: {e}")))?;

    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_assignments(
    pool: &PgPool,
    agent_id: &str,
) -> AppResult<Vec<AgentAssignment>> {
    let rows = sqlx::query(
        r"
        SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM agent_assignments ca
        LEFT JOIN users u ON ca.user_id = u.id
        WHERE ca.agent_id = $1
        ORDER BY ca.created_at DESC
        ",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list assignments: {e}")))?;

    rows.iter()
        .map(|row| {
            let user_id: Uuid = row.get("user_id");
            let created_at: DateTime<Utc> = row.get("created_at");
            let assigned_by: Option<Uuid> = row.get("assigned_by");
            let user_email: Option<String> = row.get("email");

            Ok(AgentAssignment {
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
    agent_id: &str,
    tenant_id: TenantId,
) -> AppResult<Vec<AgentAssignment>> {
    let rows = sqlx::query(
        r"
        SELECT ca.user_id, ca.created_at, ca.assigned_by, u.email
        FROM agent_assignments ca
        LEFT JOIN users u ON ca.user_id = u.id
        INNER JOIN tenant_users tu ON ca.user_id = tu.user_id AND tu.tenant_id = $2
        WHERE ca.agent_id = $1
        ORDER BY ca.created_at DESC
        ",
    )
    .bind(agent_id)
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

            Ok(AgentAssignment {
                user_id: user_id.to_string(),
                user_email,
                assigned_at: created_at.to_rfc3339(),
                assigned_by: assigned_by.map(|u| u.to_string()),
            })
        })
        .collect()
}

pub(super) async fn hide_agent(
    pool: &PgPool,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<bool> {
    // Check if the agent is hideable (must be system or assigned, not personal)
    if !is_agent_hideable(pool, agent_id, user_id, tenant_id).await? {
        return Err(AppError::invalid_input(
            "Only system or assigned coaches can be hidden",
        ));
    }

    let id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        r"
        INSERT INTO user_agent_preferences (id, user_id, agent_id, is_hidden, created_at)
        VALUES ($1, $2, $3, TRUE, $4)
        ON CONFLICT(user_id, agent_id) DO UPDATE SET is_hidden = TRUE
        ",
    )
    .bind(id.to_string())
    .bind(user_id)
    .bind(agent_id)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to hide coach: {e}")))?;

    Ok(true)
}

pub(super) async fn show_agent(pool: &PgPool, agent_id: &str, user_id: Uuid) -> AppResult<bool> {
    // No tenant predicate: user_agent_preferences has no tenant_id column —
    // the hidden-set is a per-user preference (see the trait doc). The
    // caller's handler enforces the tenant gate.
    let result = sqlx::query(
        r"
        DELETE FROM user_agent_preferences
        WHERE agent_id = $1 AND user_id = $2 AND is_hidden = TRUE
        ",
    )
    .bind(agent_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to show coach: {e}")))?;

    Ok(result.rows_affected() > 0)
}

pub(super) async fn list_hidden_agents(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Agent>> {
    let rows = sqlx::query(
        r"
        SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
               c.category, c.tags, c.sample_prompts, c.token_count,
               c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
               c.forked_from, c.slug, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
               c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria
        FROM agents c
        INNER JOIN user_agent_preferences ucp ON c.id = ucp.agent_id
        WHERE ucp.user_id = $1 AND ucp.is_hidden = TRUE AND c.tenant_id = $2
        ORDER BY c.title
        ",
    )
    .bind(user_id)
    .bind(tenant_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to list hidden coaches: {e}")))?;

    rows.iter().map(row_to_agent_pg).collect()
}

/// Check if an agent can be hidden by a user
///
/// An agent is hideable if it's a system agent or assigned to the user,
/// but NOT if it's a personal agent created by the user.
async fn is_agent_hideable(
    pool: &PgPool,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<bool> {
    // Check if it's a system agent (system agents are visible across all tenants)
    let is_system = sqlx::query(
        r"
        SELECT 1 FROM agents
        WHERE id = $1 AND is_system = TRUE
        ",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to check system agent: {e}")))?
    .is_some();

    if is_system {
        return Ok(true);
    }

    // Assigned to the user AND owned by the caller's tenant — without the
    // tenant join, an agent id from another tenant answered differently from
    // a nonexistent one, a cross-tenant existence oracle.
    let is_assigned = sqlx::query(
        r"
        SELECT 1 FROM agent_assignments ca
        INNER JOIN agents c ON c.id = ca.agent_id
        WHERE ca.agent_id = $1 AND ca.user_id = $2 AND c.tenant_id = $3
        ",
    )
    .bind(agent_id)
    .bind(user_id)
    .bind(tenant_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?
    .is_some();

    Ok(is_assigned)
}
