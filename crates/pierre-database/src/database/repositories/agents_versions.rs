// ABOUTME: Agent version history — snapshot, list, fetch, revert, and current-version lookup
// ABOUTME: Split from agents_impl.rs; the AgentsRepository impl delegates its version methods here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every edit to an agent snapshots the prior state, so a prompt that used to
//! work can always be recovered.
//!
//! Reverting is itself an edit: [`revert_to_version`] snapshots the current
//! state before restoring the old one, which is why a revert can be undone and
//! why the history never loses a branch.
//!
//! Free functions over the pool rather than a second `impl` block, because
//! `AgentsRepository` is a trait impl and Rust does not allow one to be split
//! across modules. The version methods in `agents_impl.rs` are one-line
//! delegations.

use chrono::Utc;
use sqlx::{Pool, Row, Sqlite};
use uuid::Uuid;

use crate::database::agents::{compute_content_hash, row_to_agent, row_to_agent_version};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{Agent, AgentVersion};
use pierre_core::models::TenantId;
use pierre_core::tokens::estimate_prompt_tokens;

pub(super) async fn create_version(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    user_id: Uuid,
    change_summary: Option<&str>,
) -> AppResult<i32> {
    let row = sqlx::query(
        r"SELECT id, user_id, tenant_id, title, description, system_prompt,
               category, tags, sample_prompts, token_count,
               created_at, updated_at, is_system, visibility, prerequisites,
               forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
               purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
        FROM agents WHERE id = $1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get coach for versioning: {e}")))?
    .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;
    let agent = row_to_agent(&row)?;

    let version_row = sqlx::query(
        "SELECT COALESCE(MAX(version), 0) as max_version FROM agent_versions WHERE agent_id = $1",
    )
    .bind(agent_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get max version: {e}")))?;
    let max_version: i32 = version_row.get("max_version");
    let new_version = max_version + 1;

    let content_snapshot = serde_json::json!({
        "title": agent.title, "description": agent.description,
        "system_prompt": agent.system_prompt, "category": agent.category.as_str(),
        "tags": agent.tags, "sample_prompts": agent.sample_prompts,
        "token_count": agent.token_count, "visibility": agent.visibility.as_str(),
        "prerequisites": agent.prerequisites,
    });
    let content_hash = compute_content_hash(&content_snapshot);
    let id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        r"INSERT INTO agent_versions (id, agent_id, version, content_hash, content_snapshot, change_summary, created_at, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    ).bind(id.to_string()).bind(agent_id).bind(new_version)
    .bind(&content_hash).bind(content_snapshot.to_string())
    .bind(change_summary).bind(now.to_rfc3339()).bind(user_id.to_string())
    .execute(pool).await
    .map_err(|e| AppError::database(format!("Failed to create version: {e}")))?;
    Ok(new_version)
}

pub(super) async fn get_versions(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    tenant_id: TenantId,
    limit: u32,
) -> AppResult<Vec<AgentVersion>> {
    let exists = sqlx::query("SELECT 1 FROM agents WHERE id = $1 AND tenant_id = $2")
        .bind(agent_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
    if exists.is_none() {
        return Err(AppError::not_found(format!("Coach {agent_id}")));
    }
    let limit_val = i32::try_from(limit).unwrap_or(50);
    let rows = sqlx::query(
        r"SELECT cv.id, cv.agent_id, cv.version, cv.content_hash, cv.content_snapshot,
               cv.change_summary, cv.created_at, cv.created_by
        FROM agent_versions cv WHERE cv.agent_id = $1 ORDER BY cv.version DESC LIMIT $2",
    )
    .bind(agent_id)
    .bind(limit_val)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get versions: {e}")))?;
    rows.iter().map(row_to_agent_version).collect()
}

pub(super) async fn get_version(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    version: i32,
    tenant_id: TenantId,
) -> AppResult<Option<AgentVersion>> {
    let exists = sqlx::query("SELECT 1 FROM agents WHERE id = $1 AND tenant_id = $2")
        .bind(agent_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
    if exists.is_none() {
        return Err(AppError::not_found(format!("Coach {agent_id}")));
    }
    let row = sqlx::query(
        r"SELECT id, agent_id, version, content_hash, content_snapshot, change_summary, created_at, created_by
        FROM agent_versions WHERE agent_id = $1 AND version = $2",
    ).bind(agent_id).bind(version).fetch_optional(pool).await
    .map_err(|e| AppError::database(format!("Failed to get version: {e}")))?;
    row.map(|r| row_to_agent_version(&r)).transpose()
}

pub(super) async fn revert_to_version(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    version: i32,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Agent> {
    let target_version = get_version(pool, agent_id, version, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Version {version} for coach {agent_id}")))?;
    let snapshot = &target_version.content_snapshot;
    let title = snapshot
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::internal("Missing title in version snapshot"))?;
    let description = snapshot
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let system_prompt = snapshot
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::internal("Missing system_prompt in version snapshot"))?;
    let category_str = snapshot
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("custom");
    let tags: Vec<String> = snapshot
        .get("tags")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let sample_prompts: Vec<String> = snapshot
        .get("sample_prompts")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let now = Utc::now();
    let tags_json = serde_json::to_string(&tags)?;
    let sample_prompts_json = serde_json::to_string(&sample_prompts)?;
    let token_count = estimate_prompt_tokens(system_prompt);

    // Owner-gated write: mirror `update()`'s `WHERE id AND user_id AND
    // tenant_id` predicate so only the agent owner can revert. A non-owner
    // (even within the same tenant) matches zero rows and is denied below,
    // closing the version-revert IDOR.
    let result = sqlx::query(
        r"UPDATE agents SET title = $1, description = $2, system_prompt = $3,
            category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
        WHERE id = $9 AND user_id = $10 AND tenant_id = $11",
    )
    .bind(title)
    .bind(&description)
    .bind(system_prompt)
    .bind(category_str)
    .bind(&tags_json)
    .bind(&sample_prompts_json)
    .bind(i64::from(token_count))
    .bind(now.to_rfc3339())
    .bind(agent_id)
    .bind(user_id.to_string())
    .bind(tenant_id)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to revert coach: {e}")))?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found(format!("Coach {agent_id}")));
    }

    let revert_summary = format!("Reverted to version {version}");
    create_version(pool, agent_id, user_id, Some(&revert_summary)).await?;

    let row = sqlx::query(
        r"SELECT id, user_id, tenant_id, title, description, system_prompt,
               category, tags, sample_prompts, token_count,
               created_at, updated_at, is_system, visibility, prerequisites,
               forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
               purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
        FROM agents WHERE id = $1 AND tenant_id = $2",
    )
    .bind(agent_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to get reverted coach: {e}")))?
    .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;
    row_to_agent(&row)
}

pub(super) async fn get_current_version(pool: &Pool<Sqlite>, agent_id: &str) -> AppResult<i32> {
    let row = sqlx::query(
        "SELECT COALESCE(MAX(version), 0) as current_version FROM agent_versions WHERE agent_id = $1",
    ).bind(agent_id).fetch_one(pool).await
    .map_err(|e| AppError::database(format!("Failed to get current version: {e}")))?;
    Ok(row.get("current_version"))
}
