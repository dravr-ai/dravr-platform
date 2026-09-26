// ABOUTME: Single-agent reads by id on PostgreSQL, one per scope: a tenant's system agent, any system agent, a tenant's runnable agent
// ABOUTME: Split out of agents.rs so the trait impl file stays within its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::agents_rows::row_to_agent_pg;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::Agent;
use pierre_core::models::TenantId;
use sqlx::PgPool;

/// A system agent of one tenant — see `AgentsRepository::get_system_agent`.
pub(super) async fn system_agent(
    pool: &PgPool,
    agent_id: &str,
    tenant_id: TenantId,
) -> AppResult<Option<Agent>> {
    let row = sqlx::query(
        r"
        SELECT id, user_id, tenant_id, title, description, system_prompt,
               category, tags, sample_prompts, token_count,
               created_at, updated_at, is_system, visibility, prerequisites,
               forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
               purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
        FROM agents
        WHERE id = $1 AND tenant_id = $2 AND is_system = TRUE
        ",
    )
    .bind(agent_id)
    .bind(tenant_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get system agent: {e}")))?;

    row.map(|r| row_to_agent_pg(&r)).transpose()
}

/// A system agent whatever its tenant — see
/// `AgentsRepository::get_system_agent_any_tenant`.
pub(super) async fn system_agent_any_tenant(
    pool: &PgPool,
    agent_id: &str,
) -> AppResult<Option<Agent>> {
    let row = sqlx::query(
        r"
        SELECT id, user_id, tenant_id, title, description, system_prompt,
               category, tags, sample_prompts, token_count,
               created_at, updated_at, is_system, visibility, prerequisites,
               forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
               purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
        FROM agents
        WHERE id = $1 AND is_system = TRUE
        ",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get system agent: {e}")))?;

    row.map(|r| row_to_agent_pg(&r)).transpose()
}

/// An agent one tenant can run, its own or a system agent — see
/// `AgentsRepository::get_in_tenant`.
pub(super) async fn in_tenant(
    pool: &PgPool,
    agent_id: &str,
    tenant_id: TenantId,
) -> AppResult<Option<Agent>> {
    let row = sqlx::query(
        r"
        SELECT id, user_id, tenant_id, title, description, system_prompt,
               category, tags, sample_prompts, token_count,
               created_at, updated_at, is_system, visibility, prerequisites,
               forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
               purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
        FROM agents
        WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE)
        ",
    )
    .bind(agent_id)
    .bind(tenant_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get tenant coach: {e}")))?;

    row.map(|r| row_to_agent_pg(&r)).transpose()
}
