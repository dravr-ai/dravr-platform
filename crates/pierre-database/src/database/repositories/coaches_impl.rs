// ABOUTME: Direct CoachesRepository impl on Database (SQLite coaches catalogue)
// ABOUTME: Split out of repositories/direct_impls.rs to mirror per-domain PG backend shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! #[async_trait] impl CoachesRepository for Database — coach CRUD, versioning, assignments, and overlays.

use super::coaches_assignments as assignments;
use super::coaches_versions as versions;
use super::CoachesRepository;
use crate::database::coaches::{compute_request_hash, row_to_coach, row_to_coach_list_item};
use crate::database::Database;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{
    Coach, CoachAssignment, CoachCategory, CoachFieldOverlay, CoachHandle, CoachListItem,
    CoachPrerequisites, CoachVersion, CoachVisibility, CreateCoachRequest,
    CreateSystemCoachRequest, ListCoachesFilter, UpdateCoachRequest,
};
use pierre_core::models::TenantId;
use pierre_core::models::{split_visuals, CoachRuntimeContext};
use pierre_core::tokens::estimate_prompt_tokens;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::fmt::Write as _;
use uuid::Uuid;

// ============================================================================
// CoachesRepository — helper functions
// ============================================================================

/// Ensure a `coach_assignments` row exists for a user+coach pair.
///
/// Uses `INSERT OR IGNORE` so it is safe to call multiple times.
/// Needed for operations like `toggle_favorite`, `record_usage`,
/// and `activate_coach` that need an assignment row to update.
async fn ensure_assignment_exists(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<()> {
    let id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        r"
        INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
        VALUES ($1, $2, $3, $3, $4, 0, 0, NULL)
        ",
    )
    .bind(id.to_string())
    .bind(coach_id)
    .bind(user_id.to_string())
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to ensure coach assignment: {e}")))?;

    Ok(())
}

/// Check if a coach can be hidden by a user.
///
/// A coach is hideable if it's a system coach or assigned to the user,
/// but NOT if it's a personal coach created by the user.
pub(super) async fn is_coach_hideable(
    pool: &SqlitePool,
    coach_id: &str,
    user_id: Uuid,
) -> AppResult<bool> {
    let is_system = sqlx::query("SELECT 1 FROM coaches WHERE id = $1 AND is_system = 1")
        .bind(coach_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to check system coach: {e}")))?
        .is_some();

    if is_system {
        return Ok(true);
    }

    let is_assigned =
        sqlx::query("SELECT 1 FROM coach_assignments WHERE coach_id = $1 AND user_id = $2")
            .bind(coach_id)
            .bind(user_id.to_string())
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to check assignment: {e}")))?
            .is_some();

    Ok(is_assigned)
}

// ============================================================================
// CoachesRepository
// ============================================================================

#[async_trait]
impl CoachesRepository for Database {
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateCoachRequest,
    ) -> AppResult<Coach> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;

        let effective_system_prompt = request
            .instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&request.system_prompt);

        let coach_for_tokens = Coach {
            id,
            user_id,
            tenant_id: tenant_id.to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            system_prompt: effective_system_prompt.to_owned(),
            category: request.category,
            tags: request.tags.clone(),
            sample_prompts: request.sample_prompts.clone(),
            token_count: 0,
            created_at: now,
            updated_at: now,
            is_system: false,
            visibility: CoachVisibility::Private,
            prerequisites: CoachPrerequisites::default(),
            forked_from: None,
            handle: None,
            max_tool_iterations: request.max_tool_iterations,
            temperature: None,
            startup_query: request.startup_query.clone(),
            data_requirements: request.data_requirements.clone(),
            output_schema: None,
            purpose: request.purpose.clone(),
            when_to_use: request.when_to_use.clone(),
            instructions: request.instructions.clone(),
            example_inputs: request.example_inputs.clone(),
            example_outputs: request.example_outputs.clone(),
            success_criteria: request.success_criteria.clone(),
            source: "custom".to_owned(),
        };
        let token_count = coach_for_tokens.compute_token_count();

        let data_requirements_json = request
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok());
        let content_hash = compute_request_hash(request);

        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria,
                content_hash
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
            ",
        )
        .bind(id.to_string()).bind(user_id.to_string()).bind(tenant_id)
        .bind(&request.title).bind(&request.description).bind(effective_system_prompt)
        .bind(request.category.as_str()).bind(&tags_json).bind(&sample_prompts_json)
        .bind(i64::from(token_count)).bind(now.to_rfc3339())
        .bind(0i64).bind(CoachVisibility::Private.as_str())
        .bind(Option::<String>::None).bind(Option::<String>::None).bind(request.max_tool_iterations).bind(Option::<f32>::None)
        .bind(&request.startup_query).bind(&data_requirements_json)
        .bind(&request.purpose).bind(&request.when_to_use).bind(&request.instructions)
        .bind(&request.example_inputs).bind(&request.example_outputs)
        .bind(&request.success_criteria).bind(&content_hash)
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create coach: {e}")))?;

        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string()).bind(id.to_string())
        .bind(user_id.to_string()).bind(now.to_rfc3339())
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

        Ok(Coach {
            id,
            user_id,
            tenant_id: tenant_id.to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            system_prompt: effective_system_prompt.to_owned(),
            category: request.category,
            tags: request.tags.clone(),
            sample_prompts: request.sample_prompts.clone(),
            token_count,
            created_at: now,
            updated_at: now,
            is_system: false,
            visibility: CoachVisibility::Private,
            prerequisites: CoachPrerequisites::default(),
            forked_from: None,
            handle: None,
            max_tool_iterations: request.max_tool_iterations,
            temperature: None,
            startup_query: request.startup_query.clone(),
            data_requirements: request.data_requirements.clone(),
            output_schema: None,
            purpose: request.purpose.clone(),
            when_to_use: request.when_to_use.clone(),
            instructions: request.instructions.clone(),
            example_inputs: request.example_inputs.clone(),
            example_outputs: request.example_outputs.clone(),
            success_criteria: request.success_criteria.clone(),
            source: "custom".to_owned(),
        })
    }

    async fn get_by_id(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND (
                (user_id = $2 AND tenant_id = $3)
                OR is_system = 1
                OR id IN (SELECT coach_id FROM coach_assignments WHERE user_id = $2)
            )",
        )
        .bind(coach_id).bind(user_id.to_string()).bind(tenant_id)
        .fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn find_installed_by_handle(
        &self,
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
            WHERE c.slug = $2 AND (c.tenant_id = $3 OR c.is_system = 1)
            ORDER BY CASE WHEN c.user_id = $1 THEN 0 ELSE 1 END, c.created_at ASC
            LIMIT 1",
        )
        .bind(user_id.to_string())
        .bind(handle.as_str())
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to resolve coach by handle: {e}")))?;
        row.as_ref().map(row_to_coach).transpose()
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);
        let user_id_str = user_id.to_string();
        let category_filter = filter
            .category
            .as_ref()
            .map(|c| format!("AND c.category = '{}'", c.as_str()))
            .unwrap_or_default();
        let favorites_filter = if filter.favorites_only {
            "AND ca.is_favorite = 1"
        } else {
            ""
        };
        let hidden_filter = if filter.include_hidden {
            ""
        } else {
            "AND c.id NOT IN (SELECT coach_id FROM user_coach_preferences WHERE user_id = $1 AND is_hidden = 1)"
        };
        let system_condition = if filter.include_system {
            "OR c.is_system = 1"
        } else {
            ""
        };

        let query = format!(
            r"SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.slug, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria,
                   CASE WHEN ca.coach_id IS NOT NULL THEN 1 ELSE 0 END as is_assigned,
                   COALESCE(ca.is_favorite, 0) as is_favorite,
                   CASE WHEN tu.selected_coach_id = c.id THEN 1 ELSE 0 END as is_active,
                   COALESCE(ca.use_count, 0) as use_count,
                   ca.last_used_at
            FROM coaches c
            LEFT JOIN coach_assignments ca ON c.id = ca.coach_id AND ca.user_id = $1
            LEFT JOIN tenant_users tu ON tu.user_id = $1 AND tu.tenant_id = $2
            WHERE (
                (c.user_id = $1 AND c.is_system = 0 AND c.tenant_id = $2)
                {system_condition}
                OR c.id IN (SELECT coach_id FROM coach_assignments WHERE user_id = $1)
            )
            {category_filter} {favorites_filter} {hidden_filter}
            ORDER BY c.updated_at DESC LIMIT $3 OFFSET $4"
        );
        let rows = sqlx::query(&query)
            .bind(&user_id_str)
            .bind(tenant_id)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to list coaches: {e}")))?;
        rows.iter().map(row_to_coach_list_item).collect()
    }

    async fn apply_translations(
        &self,
        coaches: &mut [CoachListItem],
        locale: &str,
    ) -> AppResult<()> {
        // English is canonical — skip the round-trip entirely.
        if locale == "en" || coaches.is_empty() {
            return Ok(());
        }

        // SQLite lacks `= ANY($2)`; build an IN list with one bind per coach.
        // Coach counts per turn are bounded by ListCoachesFilter::limit (default
        // 50) so the placeholder growth stays well under SQLite's 32766 cap.
        let mut query_str = String::from(
            "SELECT coach_id, title, description, purpose, instructions \
             FROM coach_translations WHERE locale = ?1 AND coach_id IN (",
        );
        for i in 0..coaches.len() {
            if i > 0 {
                query_str.push(',');
            }
            // SQLite positional binds are 1-indexed; reserve slot 1 for locale.
            let _ = write!(query_str, "?{}", i + 2);
        }
        query_str.push(')');

        let mut q = sqlx::query(&query_str).bind(locale);
        for item in coaches.iter() {
            q = q.bind(item.coach.id.to_string());
        }
        let rows = q
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("Failed to load coach translations: {e}")))?;

        if rows.is_empty() {
            return Ok(());
        }

        let mut overlays: HashMap<String, CoachFieldOverlay> = HashMap::with_capacity(rows.len());
        for row in &rows {
            let id: String = row.get("coach_id");
            overlays.insert(
                id,
                CoachFieldOverlay {
                    title: row.try_get("title").ok(),
                    description: row.try_get("description").ok(),
                    purpose: row.try_get("purpose").ok(),
                    instructions: row.try_get("instructions").ok(),
                },
            );
        }

        for item in coaches.iter_mut() {
            if let Some(ov) = overlays.get(&item.coach.id.to_string()) {
                ov.apply(&mut item.coach);
            }
        }
        Ok(())
    }

    async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
        change_summary: Option<&str>,
    ) -> AppResult<Option<Coach>> {
        let existing = CoachesRepository::get_by_id(self, coach_id, user_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        self.create_version(coach_id, user_id, change_summary)
            .await?;

        let now = Utc::now();
        let title = request.title.as_ref().unwrap_or(&existing.title);
        let description = request.description.clone().or(existing.description);
        let system_prompt = request
            .system_prompt
            .as_ref()
            .unwrap_or(&existing.system_prompt);
        let category = request.category.unwrap_or(existing.category);
        let tags = request.tags.as_ref().unwrap_or(&existing.tags);
        let sample_prompts = request
            .sample_prompts
            .as_ref()
            .unwrap_or(&existing.sample_prompts);
        let tags_json = serde_json::to_string(tags)?;
        let sample_prompts_json = serde_json::to_string(sample_prompts)?;
        let token_count = estimate_prompt_tokens(system_prompt);

        let startup_query: Option<String> = if request.startup_query.is_some() {
            request
                .startup_query
                .as_ref()
                .filter(|q| !q.is_empty())
                .cloned()
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT startup_query FROM coaches WHERE id = $1")
                    .bind(coach_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get startup_query: {e}")))?;
            existing_row.and_then(|(q,)| q)
        };
        let data_requirements_json: Option<String> = if request.data_requirements.is_some() {
            request
                .data_requirements
                .as_ref()
                .and_then(|dr| serde_json::to_string(dr).ok())
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT data_requirements FROM coaches WHERE id = $1")
                    .bind(coach_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get data_requirements: {e}"))
                    })?;
            existing_row.and_then(|(dr,)| dr)
        };

        let purpose = request.purpose.clone().or(existing.purpose);
        let when_to_use = request.when_to_use.clone().or(existing.when_to_use);
        let instructions = request.instructions.clone().or(existing.instructions);
        let example_inputs = request.example_inputs.clone().or(existing.example_inputs);
        let example_outputs = request.example_outputs.clone().or(existing.example_outputs);
        let success_criteria = request
            .success_criteria
            .clone()
            .or(existing.success_criteria);
        // Three-way, not a coalesce: an absent field keeps the stored budget,
        // an explicit null clears it back to inheriting the admin value.
        let max_tool_iterations = request
            .max_tool_iterations
            .resolve(existing.max_tool_iterations);
        let system_prompt = instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(system_prompt);

        let result = sqlx::query(
            r"UPDATE coaches SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8,
                startup_query = $12, data_requirements = $13,
                purpose = $14, when_to_use = $15, instructions = $16,
                example_inputs = $17, example_outputs = $18, success_criteria = $19,
                max_tool_iterations = $20
            WHERE id = $9 AND user_id = $10 AND tenant_id = $11",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&startup_query)
        .bind(&data_requirements_json)
        .bind(&purpose)
        .bind(&when_to_use)
        .bind(&instructions)
        .bind(&example_inputs)
        .bind(&example_outputs)
        .bind(&success_criteria)
        .bind(max_tool_iterations)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }
        CoachesRepository::get_by_id(self, coach_id, user_id, tenant_id).await
    }

    async fn delete(&self, coach_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        let result =
            sqlx::query("DELETE FROM coaches WHERE id = $1 AND user_id = $2 AND tenant_id = $3")
                .bind(coach_id)
                .bind(user_id.to_string())
                .bind(tenant_id)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to delete coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();
        // System coaches (is_system = 1) are pinned to the seed tenant but
        // exposed to every tenant via the catalog, so accept them
        // unconditionally — otherwise non-seed-tenant users silently skip
        // usage tracking when chatting with a builtin coach.
        let exists = sqlx::query(
            "SELECT 1 FROM coaches WHERE id = $1 AND (tenant_id = $2 OR is_system = 1)",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if exists.is_none() {
            return Ok(false);
        }
        ensure_assignment_exists(self.pool(), coach_id, user_id).await?;
        let result = sqlx::query(
            "UPDATE coach_assignments SET use_count = use_count + 1, last_used_at = $1 WHERE coach_id = $2 AND user_id = $3",
        ).bind(&now).bind(coach_id).bind(user_id.to_string())
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to record coach usage: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>> {
        // Same reasoning as record_usage: accept system coaches so favorites
        // toggle for non-seed-tenant users on builtin coaches.
        let coach_exists = sqlx::query(
            "SELECT 1 FROM coaches WHERE id = $1 AND (tenant_id = $2 OR is_system = 1)",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if coach_exists.is_none() {
            return Ok(None);
        }
        ensure_assignment_exists(self.pool(), coach_id, user_id).await?;
        let row = sqlx::query("SELECT ca.is_favorite FROM coach_assignments ca WHERE ca.coach_id = $1 AND ca.user_id = $2")
            .bind(coach_id).bind(user_id.to_string()).fetch_optional(self.pool()).await
            .map_err(|e| AppError::database(format!("Failed to get favorite status: {e}")))?;
        let current: i64 = row.map_or(0, |r| r.get("is_favorite"));
        let new_value = i64::from(current != 1);
        sqlx::query(
            "UPDATE coach_assignments SET is_favorite = $1 WHERE coach_id = $2 AND user_id = $3",
        )
        .bind(new_value)
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to toggle favorite: {e}")))?;
        Ok(Some(new_value == 1))
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Coach>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let offset_val = i32::try_from(offset.unwrap_or(0)).unwrap_or(0);
        let search_pattern = format!("%{query}%");
        let rows = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE user_id = $1 AND tenant_id = $2 AND (title LIKE $3 OR description LIKE $3 OR tags LIKE $3)
            ORDER BY updated_at DESC LIMIT $4 OFFSET $5",
        ).bind(user_id.to_string()).bind(tenant_id).bind(&search_pattern)
        .bind(limit_val).bind(offset_val).fetch_all(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to search coaches: {e}")))?;
        rows.iter().map(row_to_coach).collect()
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let row = sqlx::query(
            "SELECT COUNT(*) as count FROM coaches WHERE user_id = $1 AND tenant_id = $2",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_one(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to count coaches: {e}")))?;
        let count: i64 = row.get("count");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as u32)
    }

    // -- User methods --

    async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        let source = self
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
        let source_data_requirements_json = source
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok());

        sqlx::query(
            r"INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria, slug
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)",
        )
        .bind(id.to_string()).bind(user_id.to_string()).bind(tenant_id)
        .bind(&source.title).bind(&source.description).bind(&source.system_prompt)
        .bind(source.category.as_str()).bind(&tags_json).bind(&sample_prompts_json)
        .bind(i64::from(source.token_count)).bind(now.to_rfc3339())
        .bind(0i64).bind(CoachVisibility::Private.as_str())
        .bind(&prerequisites_json).bind(source_coach_id)
        .bind(source.max_tool_iterations).bind(source.temperature).bind(&source.startup_query)
        .bind(&source_data_requirements_json)
        .bind(&source.purpose).bind(&source.when_to_use).bind(&source.instructions)
        .bind(&source.example_inputs).bind(&source.example_outputs).bind(&source.success_criteria)
        .bind(&source.handle)
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to fork coach: {e}")))?;

        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, NULL)",
        )
        .bind(assignment_id.to_string()).bind(id.to_string())
        .bind(user_id.to_string()).bind(now.to_rfc3339())
        .execute(self.pool()).await
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

    async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        // Same reasoning as record_usage / toggle_favorite: accept system coaches
        // unconditionally so non-seed-tenant users can pick a builtin coach as
        // their active default.
        let coach_exists = sqlx::query(
            "SELECT 1 FROM coaches WHERE id = $1 AND (tenant_id = $2 OR is_system = 1)",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;
        if coach_exists.is_none() {
            return Ok(None);
        }
        // The roster row still records entitlement, favourites and usage.
        ensure_assignment_exists(self.pool(), coach_id, user_id).await?;
        // Selection itself is one pointer on the membership row. The pair of
        // UPDATEs this replaced (clear every row, then set one) was not atomic
        // and could leave a user with zero or two active coaches.
        sqlx::query(
            "UPDATE tenant_users SET selected_coach_id = $1 WHERE user_id = $2 AND tenant_id = $3",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to activate coach: {e}")))?;
        CoachesRepository::get_by_id(self, coach_id, user_id, tenant_id).await
    }

    async fn deactivate_coach(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        // Clears the selection, leaving the roster row intact: the user still has
        // the coach available, they just are not talking to it.
        let result = sqlx::query(
            "UPDATE tenant_users SET selected_coach_id = NULL \
             WHERE user_id = $1 AND tenant_id = $2 AND selected_coach_id IS NOT NULL",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn get_active_coach(
        &self,
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
            JOIN tenant_users tu ON c.id = tu.selected_coach_id
            WHERE tu.user_id = $1 AND tu.tenant_id = $2",
        ).bind(user_id.to_string()).bind(tenant_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get active coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE content_hash = $1 AND user_id = $2 AND tenant_id = $3 LIMIT 1",
        ).bind(content_hash).bind(user_id.to_string()).bind(tenant_id)
        .fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to find coach by content hash: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    // -- Admin methods --

    async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;
        let token_count = estimate_prompt_tokens(&request.system_prompt);
        sqlx::query(
            r"INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19)",
        )
        .bind(id.to_string()).bind(admin_user_id.to_string()).bind(tenant_id)
        .bind(&request.title).bind(&request.description).bind(&request.system_prompt)
        .bind(request.category.as_str()).bind(&tags_json).bind(&sample_prompts_json)
        .bind(i64::from(token_count)).bind(now.to_rfc3339())
        .bind(1i64).bind(request.visibility.as_str())
        .bind(Option::<String>::None).bind(Option::<String>::None)
        .bind(Option::<i32>::None).bind(Option::<f32>::None).bind(Option::<String>::None).bind(Option::<String>::None)
        .execute(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to create system coach: {e}")))?;

        Ok(Coach {
            id,
            user_id: admin_user_id,
            tenant_id: tenant_id.to_string(),
            title: request.title.clone(),
            description: request.description.clone(),
            system_prompt: request.system_prompt.clone(),
            category: request.category,
            tags: request.tags.clone(),
            sample_prompts: request.sample_prompts.clone(),
            token_count,
            created_at: now,
            updated_at: now,
            is_system: true,
            visibility: request.visibility,
            prerequisites: CoachPrerequisites::default(),
            forked_from: None,
            handle: None,
            max_tool_iterations: None,
            temperature: None,
            startup_query: None,
            data_requirements: None,
            output_schema: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            source: "custom".to_owned(),
        })
    }

    async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE tenant_id = $1 AND is_system = 1 ORDER BY created_at DESC",
        ).bind(tenant_id).fetch_all(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to list system coaches: {e}")))?;
        rows.iter().map(row_to_coach).collect()
    }

    async fn get_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND tenant_id = $2 AND is_system = 1",
        ).bind(coach_id).bind(tenant_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get system coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches WHERE id = $1 AND is_system = 1",
        ).bind(coach_id).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get system coach: {e}")))?;
        row.map(|r| row_to_coach(&r)).transpose()
    }

    async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        let existing = self.get_system_coach(coach_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        self.create_version(coach_id, existing.user_id, None)
            .await?;
        let now = Utc::now();
        let title = request.title.as_ref().unwrap_or(&existing.title);
        let description = request.description.clone().or(existing.description);
        let system_prompt = request
            .system_prompt
            .as_ref()
            .unwrap_or(&existing.system_prompt);
        let category = request.category.unwrap_or(existing.category);
        let tags = request.tags.as_ref().unwrap_or(&existing.tags);
        let sample_prompts = request
            .sample_prompts
            .as_ref()
            .unwrap_or(&existing.sample_prompts);
        let tags_json = serde_json::to_string(tags)?;
        let sample_prompts_json = serde_json::to_string(sample_prompts)?;
        let token_count = estimate_prompt_tokens(system_prompt);
        let result = sqlx::query(
            r"UPDATE coaches SET title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND tenant_id = $10 AND is_system = 1",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(tenant_id)
        .execute(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to update system coach: {e}")))?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_system_coach(coach_id, tenant_id).await
    }

    async fn delete_system_coach(&self, coach_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let result =
            sqlx::query("DELETE FROM coaches WHERE id = $1 AND tenant_id = $2 AND is_system = 1")
                .bind(coach_id)
                .bind(tenant_id)
                .execute(self.pool())
                .await
                .map_err(|e| AppError::database(format!("Failed to delete system coach: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    // -- Assignment methods --

    async fn get_user_preferences(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, u32, Option<DateTime<Utc>>)> {
        let row = sqlx::query(
            "SELECT is_favorite, use_count, last_used_at FROM coach_assignments WHERE coach_id = $1 AND user_id = $2",
        ).bind(coach_id).bind(user_id.to_string()).fetch_optional(self.pool()).await
        .map_err(|e| AppError::database(format!("Failed to get user preferences: {e}")))?;
        row.map_or(Ok((false, 0, None)), |r| {
            let is_favorite: i64 = r.get("is_favorite");
            let use_count: i64 = r.get("use_count");
            let last_used_at_str: Option<String> = r.get("last_used_at");
            let last_used_at = last_used_at_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok((is_favorite == 1, use_count as u32, last_used_at))
        })
    }

    async fn assign_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool> {
        assignments::assign_coach(self.pool(), coach_id, user_id, assigned_by).await
    }

    async fn unassign_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        assignments::unassign_coach(self.pool(), coach_id, user_id).await
    }

    async fn list_assignments(&self, coach_id: &str) -> AppResult<Vec<CoachAssignment>> {
        assignments::list_assignments(self.pool(), coach_id).await
    }

    async fn list_assignments_for_tenant(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<CoachAssignment>> {
        assignments::list_assignments_for_tenant(self.pool(), coach_id, tenant_id).await
    }

    async fn hide_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        assignments::hide_coach(self.pool(), coach_id, user_id).await
    }

    async fn show_coach(&self, coach_id: &str, user_id: Uuid) -> AppResult<bool> {
        assignments::show_coach(self.pool(), coach_id, user_id).await
    }

    async fn list_hidden_coaches(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Coach>> {
        assignments::list_hidden_coaches(self.pool(), user_id, tenant_id).await
    }

    // -- Version methods (bodies in coaches_versions.rs) --

    async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        versions::create_version(self.pool(), coach_id, user_id, change_summary).await
    }

    async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>> {
        versions::get_versions(self.pool(), coach_id, tenant_id, limit).await
    }

    async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>> {
        versions::get_version(self.pool(), coach_id, version, tenant_id).await
    }

    async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        versions::revert_to_version(self.pool(), coach_id, version, user_id, tenant_id).await
    }

    async fn get_current_version(&self, coach_id: &str) -> AppResult<i32> {
        versions::get_current_version(self.pool(), coach_id).await
    }

    async fn get_coach_runtime_context(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachRuntimeContext>> {
        // Column order matches `CoachesManager::get_coach_runtime_context`
        // in `coaches/versions.rs` — keep in lock-step.
        type Row = (
            Option<String>,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i32>,
            Option<f32>,
            String,
        );
        let row: Option<Row> = sqlx::query_as(
            r"SELECT slug, source, system_prompt, startup_query, data_requirements, output_schema, visuals, max_tool_iterations, temperature, category
            FROM coaches WHERE id = $1 AND (tenant_id = $2 OR is_system = 1) LIMIT 1",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach runtime context: {e}")))?;
        Ok(row.map(
            |(
                slug,
                source,
                system_prompt,
                startup_query,
                data_requirements,
                output_schema,
                visuals,
                max_tool_iterations,
                temperature,
                category,
            )| {
                CoachRuntimeContext {
                    slug: slug.unwrap_or_default(),
                    source,
                    system_prompt,
                    startup_query,
                    data_requirements,
                    output_schema,
                    visuals: split_visuals(visuals.as_deref()),
                    max_tool_iterations,
                    temperature,
                    category: CoachCategory::parse(&category),
                }
            },
        ))
    }
}
