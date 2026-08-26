// ABOUTME: Per-user copies of catalogue coaches on PostgreSQL: forking, and resolving a copy by @handle
// ABOUTME: Split out of coaches.rs so the trait impl file stays within its size budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::CoachesRepository;
use super::coaches_rows::{row_to_coach_pg, token_count_as_i32};
use super::PostgresDatabase;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{Coach, CoachHandle, CoachVisibility};
use pierre_core::models::TenantId;
use sqlx::PgPool;
use uuid::Uuid;

/// Resolve an installed coach by handle for one user — see
/// `CoachesRepository::find_installed_by_handle` for the contract.
pub(super) async fn find_installed_by_handle(
    pool: &PgPool,
    handle: &CoachHandle,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Option<Coach>> {
    let row = sqlx::query(
        r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
               c.category, c.tags, c.sample_prompts, c.token_count,
               c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
               c.forked_from, c.slug, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
               c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria
        FROM coaches c
        JOIN coach_assignments ca ON ca.coach_id = c.id AND ca.user_id = $1
        WHERE c.slug = $2 AND (c.tenant_id = $3 OR c.is_system = TRUE)
        ORDER BY CASE WHEN c.user_id = $1 THEN 0 ELSE 1 END, c.created_at ASC
        LIMIT 1",
    )
    .bind(user_id)
    .bind(handle.as_str())
    .bind(tenant_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to resolve coach by handle: {e}")))?;
    row.as_ref().map(row_to_coach_pg).transpose()
}

/// Fork a system coach into the user's own copy, carrying the origin's
/// handle so the copy resolves by the same `@handle`.
pub(super) async fn fork_coach(
    db: &PostgresDatabase,
    source_coach_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Coach> {
    // Get the source coach (must be a system coach)
    // System coaches are platform-wide, so no tenant filter
    let source = db
        .get_system_coach_any_tenant(source_coach_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("System coach {source_coach_id}")))?;

    if !source.is_system {
        return Err(AppError::invalid_input(
            "Only system coaches can be forked. Use duplicate for personal coaches.",
        ));
    }

    let now = Utc::now();
    let id = Uuid::new_v4();
    let tags_json = serde_json::to_string(&source.tags)?;
    let sample_prompts_json = serde_json::to_string(&source.sample_prompts)?;
    let prerequisites_json = serde_json::to_string(&source.prerequisites)?;

    // Serialize data_requirements from source coach for fork INSERT
    let source_data_requirements_json = source
        .data_requirements
        .as_ref()
        .and_then(|dr| serde_json::to_string(dr).ok());

    sqlx::query(
        r"
        INSERT INTO coaches (
            id, user_id, tenant_id, title, description, system_prompt,
            category, tags, sample_prompts, token_count,
            created_at, updated_at, is_system, visibility, prerequisites,
            forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
            purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria, slug
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
        ",
    )
    .bind(id.to_string())
    .bind(user_id)
    .bind(tenant_id.to_string())
    .bind(&source.title)
    .bind(&source.description)
    .bind(&source.system_prompt)
    .bind(source.category.as_str())
    .bind(&tags_json)
    .bind(&sample_prompts_json)
    .bind(token_count_as_i32(source.token_count))
    .bind(now)
    .bind(false) // is_system = false (user's copy)
    .bind(CoachVisibility::Private.as_str())
    .bind(&prerequisites_json)
    .bind(source_coach_id) // forked_from
    .bind(source.max_tool_iterations)
    .bind(source.temperature) // temperature (inherit from source)
    .bind(&source.startup_query) // startup_query (inherit from source)
    .bind(&source_data_requirements_json) // data_requirements (inherit from source)
    .bind(&source.purpose)
    .bind(&source.when_to_use)
    .bind(&source.instructions)
    .bind(&source.example_inputs)
    .bind(&source.example_outputs)
    .bind(&source.success_criteria)
    .bind(&source.handle)
    .execute(&db.pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to fork coach: {e}")))?;

    // Create self-assignment row for the forking user
    let assignment_id = Uuid::new_v4();
    sqlx::query(
        r"
        INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
        VALUES ($1, $2, $3, $3, $4, FALSE, 0, NULL)
        ",
    )
    .bind(assignment_id.to_string())
    .bind(id.to_string())
    .bind(user_id)
    .bind(now)
    .execute(&db.pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

    Ok(Coach {
        id,
        user_id,
        tenant_id: tenant_id.to_string(),
        title: source.title,
        description: source.description,
        system_prompt: source.system_prompt,
        category: source.category,
        tags: source.tags,
        sample_prompts: source.sample_prompts,
        token_count: source.token_count,
        created_at: now,
        updated_at: now,
        is_system: false,
        visibility: CoachVisibility::Private,
        prerequisites: source.prerequisites,
        forked_from: Some(source.id),
        handle: source.handle,
        max_tool_iterations: source.max_tool_iterations,
        temperature: source.temperature,
        startup_query: source.startup_query,
        data_requirements: source.data_requirements,
        output_schema: source.output_schema,
        purpose: source.purpose,
        when_to_use: source.when_to_use,
        instructions: source.instructions,
        example_inputs: source.example_inputs,
        example_outputs: source.example_outputs,
        success_criteria: source.success_criteria,
        source: "custom".to_owned(),
    })
}
