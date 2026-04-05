// ABOUTME: User-facing coach CRUD operations (create, get, list, update, delete, fork)
// ABOUTME: Handles personal coach management, usage tracking, favorites, activation, and search
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::CoachPrerequisites;
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use super::types::{
    Coach, CoachListItem, CoachVisibility, CreateCoachRequest, ListCoachesFilter,
    UpdateCoachRequest,
};
use super::{row_to_coach, row_to_coach_list_item, CoachesManager};

impl CoachesManager {
    /// Create a new coach in the database
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateCoachRequest,
    ) -> AppResult<Coach> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;

        // When structured `instructions` is provided, use it as the runtime system_prompt
        let effective_system_prompt = request
            .instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&request.system_prompt);

        // Build a temporary Coach to compute section-aware token count
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
            max_tool_iterations: None,
            startup_query: request.startup_query.clone(),
            data_requirements: request.data_requirements.clone(),
            purpose: request.purpose.clone(),
            when_to_use: request.when_to_use.clone(),
            instructions: request.instructions.clone(),
            example_inputs: request.example_inputs.clone(),
            example_outputs: request.example_outputs.clone(),
            success_criteria: request.success_criteria.clone(),
            source: "custom".to_owned(),
        };
        let token_count = coach_for_tokens.compute_token_count();

        // Serialize data_requirements to JSON if present
        let data_requirements_json = request
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok());

        // Compute content hash from request fields for deduplication
        let content_hash = compute_request_hash(request);

        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria,
                content_hash
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&request.title)
        .bind(&request.description)
        .bind(effective_system_prompt)
        .bind(request.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(0i64) // is_system (user-created coaches are not system)
        .bind(CoachVisibility::Private.as_str()) // visibility
        .bind(Option::<String>::None) // prerequisites (user-created coaches don't have prerequisites)
        .bind(Option::<String>::None) // forked_from (not a fork)
        .bind(Option::<i32>::None) // max_tool_iterations
        .bind(&request.startup_query)
        .bind(&data_requirements_json)
        .bind(&request.purpose)
        .bind(&request.when_to_use)
        .bind(&request.instructions)
        .bind(&request.example_inputs)
        .bind(&request.example_outputs)
        .bind(&request.success_criteria)
        .bind(&content_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create coach: {e}")))?;

        // Create self-assignment row in coach_assignments for the creator
        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await
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
            max_tool_iterations: None,
            startup_query: request.startup_query.clone(),
            data_requirements: request.data_requirements.clone(),
            purpose: request.purpose.clone(),
            when_to_use: request.when_to_use.clone(),
            instructions: request.instructions.clone(),
            example_inputs: request.example_inputs.clone(),
            example_outputs: request.example_outputs.clone(),
            success_criteria: request.success_criteria.clone(),
            source: "custom".to_owned(),
        })
    }

    /// Get a coach by ID for a specific user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?;

        row.map(|r| row_to_coach(&r)).transpose()
    }

    /// List coaches for a user with optional filtering
    ///
    /// Returns coaches from three sources:
    /// 1. Personal coaches: created by the user (`is_system = 0`)
    /// 2. System coaches: visible to tenant (`is_system = 1 AND visibility = 'tenant'`)
    /// 3. Assigned coaches: explicitly assigned to the user via `coach_assignments`
    ///
    /// Hidden coaches are excluded unless `include_hidden` is true.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListCoachesFilter,
    ) -> AppResult<Vec<CoachListItem>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);
        let user_id_str = user_id.to_string();

        // Build dynamic query parts based on filters
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

        // Build system coaches condition
        // System coaches (is_system=1) are always visible to all users
        // regardless of their visibility setting - they're platform-wide resources
        let system_condition = if filter.include_system {
            "OR c.is_system = 1"
        } else {
            ""
        };

        // Build the unified query
        // Uses a subquery to identify assigned coaches for the is_assigned flag
        let query = format!(
            r"
            SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria,
                   CASE WHEN ca.coach_id IS NOT NULL THEN 1 ELSE 0 END as is_assigned,
                   COALESCE(ca.is_favorite, 0) as is_favorite,
                   COALESCE(ca.is_active, 0) as is_active,
                   COALESCE(ca.use_count, 0) as use_count,
                   ca.last_used_at
            FROM coaches c
            LEFT JOIN coach_assignments ca ON c.id = ca.coach_id AND ca.user_id = $1
            WHERE (
                -- Personal coaches: owned by user
                (c.user_id = $1 AND c.is_system = 0 AND c.tenant_id = $2)
                -- System coaches visible to tenant
                {system_condition}
                -- Assigned coaches: explicitly assigned to user
                OR c.id IN (SELECT coach_id FROM coach_assignments WHERE user_id = $1)
            )
            {category_filter}
            {favorites_filter}
            {hidden_filter}
            ORDER BY c.updated_at DESC
            LIMIT $3 OFFSET $4
            "
        );

        let rows = sqlx::query(&query)
            .bind(&user_id_str)
            .bind(tenant_id)
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list coaches: {e}")))?;

        rows.iter().map(row_to_coach_list_item).collect()
    }

    /// Update an existing coach
    ///
    /// Automatically creates a version snapshot before applying changes.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails or coach not found
    pub async fn update(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        self.update_with_summary(coach_id, user_id, tenant_id, request, None)
            .await
    }

    /// Update an existing coach with a change summary
    ///
    /// Automatically creates a version snapshot before applying changes.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails or coach not found
    pub async fn update_with_summary(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
        change_summary: Option<&str>,
    ) -> AppResult<Option<Coach>> {
        // First get the existing coach
        let existing = self.get(coach_id, user_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        // Create a version snapshot BEFORE applying changes
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
        let token_count = Self::estimate_tokens(system_prompt);

        // Resolve startup_query: use new value if provided, otherwise keep existing
        let startup_query: Option<String> = if request.startup_query.is_some() {
            // Empty string means clear; non-empty means set
            request
                .startup_query
                .as_ref()
                .filter(|q| !q.is_empty())
                .cloned()
        } else {
            // Not provided in update — fetch existing from DB
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT startup_query FROM coaches WHERE id = $1")
                    .bind(coach_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get startup_query: {e}")))?;
            existing_row.and_then(|(q,)| q)
        };

        // Resolve data_requirements: serialize new if provided, otherwise keep existing
        let data_requirements_json: Option<String> = if request.data_requirements.is_some() {
            request
                .data_requirements
                .as_ref()
                .and_then(|dr| serde_json::to_string(dr).ok())
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT data_requirements FROM coaches WHERE id = $1")
                    .bind(coach_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get data_requirements: {e}"))
                    })?;
            existing_row.and_then(|(dr,)| dr)
        };

        // Resolve structured sections: use new value if provided, otherwise keep existing
        let purpose = request.purpose.clone().or(existing.purpose);
        let when_to_use = request.when_to_use.clone().or(existing.when_to_use);
        let instructions = request.instructions.clone().or(existing.instructions);
        let example_inputs = request.example_inputs.clone().or(existing.example_inputs);
        let example_outputs = request.example_outputs.clone().or(existing.example_outputs);
        let success_criteria = request
            .success_criteria
            .clone()
            .or(existing.success_criteria);

        // When instructions is updated, also update system_prompt for runtime compatibility
        let system_prompt = instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(system_prompt);

        let result = sqlx::query(
            r"
            UPDATE coaches SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8,
                startup_query = $12, data_requirements = $13,
                purpose = $14, when_to_use = $15, instructions = $16,
                example_inputs = $17, example_outputs = $18, success_criteria = $19
            WHERE id = $9 AND user_id = $10 AND tenant_id = $11
            ",
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
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        // Return updated coach
        self.get(coach_id, user_id, tenant_id).await
    }

    /// Delete a coach
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn delete(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM coaches
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Fork a system coach to create a user-owned copy
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Source coach is not found
    /// - Source coach is not a system coach
    /// - Database operation fails
    pub async fn fork_coach(
        &self,
        source_coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        // Get the source coach (must be a system coach)
        // System coaches are platform-wide, so no tenant filter — any user can fork them
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
                forked_from, max_tool_iterations, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
            ",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&source.title)
        .bind(&source.description)
        .bind(&source.system_prompt)
        .bind(source.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(source.token_count))
        .bind(now.to_rfc3339())
        .bind(0i64) // is_system = false (user's copy)
        .bind(CoachVisibility::Private.as_str()) // visibility = private
        .bind(&prerequisites_json) // prerequisites
        .bind(source_coach_id) // forked_from
        .bind(source.max_tool_iterations) // max_tool_iterations (inherit from source)
        .bind(&source.startup_query) // startup_query (inherit from source)
        .bind(&source_data_requirements_json) // data_requirements (inherit from source)
        .bind(&source.purpose)
        .bind(&source.when_to_use)
        .bind(&source.instructions)
        .bind(&source.example_inputs)
        .bind(&source.example_outputs)
        .bind(&source.success_criteria)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to fork coach: {e}")))?;

        // Create self-assignment row in coach_assignments for the forking user
        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
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
            forked_from: Some(source_coach_id.to_owned()),
            max_tool_iterations: source.max_tool_iterations,
            startup_query: source.startup_query,
            data_requirements: source.data_requirements,
            purpose: source.purpose,
            when_to_use: source.when_to_use,
            instructions: source.instructions,
            example_inputs: source.example_inputs,
            example_outputs: source.example_outputs,
            success_criteria: source.success_criteria,
            source: "custom".to_owned(),
        })
    }

    /// Record coach usage (increment `use_count` and update `last_used_at`)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn record_usage(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now().to_rfc3339();

        // Verify the coach exists and belongs to the user/tenant
        let exists = sqlx::query(
            r"
            SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if exists.is_none() {
            return Ok(false);
        }

        // Ensure assignment row exists (upsert)
        self.ensure_assignment_exists(coach_id, user_id).await?;

        // Update usage in coach_assignments
        let result = sqlx::query(
            r"
            UPDATE coach_assignments SET
                use_count = use_count + 1,
                last_used_at = $1
            WHERE coach_id = $2 AND user_id = $3
            ",
        )
        .bind(&now)
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record coach usage: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Toggle favorite status for a coach
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn toggle_favorite(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>> {
        // Verify the coach exists and is accessible to the user's tenant
        // System coaches (is_system = 1) are accessible to all tenants
        let coach_exists = sqlx::query(
            r"
            SELECT 1 FROM coaches WHERE id = $1 AND (tenant_id = $2 OR is_system = 1)
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if coach_exists.is_none() {
            return Ok(None);
        }

        // Ensure assignment row exists (upsert)
        self.ensure_assignment_exists(coach_id, user_id).await?;

        // Get current favorite status from coach_assignments
        let row = sqlx::query(
            r"
            SELECT ca.is_favorite FROM coach_assignments ca
            WHERE ca.coach_id = $1 AND ca.user_id = $2
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get favorite status: {e}")))?;

        let current: i64 = row.map_or(0, |r| r.get("is_favorite"));
        let new_value = i64::from(current != 1);

        // Update in coach_assignments
        sqlx::query(
            r"
            UPDATE coach_assignments SET is_favorite = $1
            WHERE coach_id = $2 AND user_id = $3
            ",
        )
        .bind(new_value)
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to toggle favorite: {e}")))?;

        Ok(Some(new_value == 1))
    }

    /// Count coaches for a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count FROM coaches
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count coaches: {e}")))?;

        let count: i64 = row.get("count");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as u32)
    }

    /// Search coaches by title, description, or tags
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn search(
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
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE user_id = $1 AND tenant_id = $2 AND (
                title LIKE $3 OR description LIKE $3 OR tags LIKE $3
            )
            ORDER BY updated_at DESC
            LIMIT $4 OFFSET $5
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .bind(&search_pattern)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to search coaches: {e}")))?;

        rows.iter().map(row_to_coach).collect()
    }

    /// Activate a coach (deactivates all other coaches for the user first)
    ///
    /// Only one coach can be active per user at a time. This method
    /// deactivates any currently active coach before activating the new one.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn activate_coach(
        &self,
        coach_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        // Verify the coach exists
        let coach_exists = sqlx::query(
            r"
            SELECT 1 FROM coaches WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if coach_exists.is_none() {
            return Ok(None);
        }

        // Ensure assignment row exists (upsert)
        self.ensure_assignment_exists(coach_id, user_id).await?;

        // Deactivate all coaches for this user in coach_assignments
        sqlx::query(
            r"
            UPDATE coach_assignments SET is_active = 0
            WHERE user_id = $1
            ",
        )
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate coaches: {e}")))?;

        // Activate the target coach
        sqlx::query(
            r"
            UPDATE coach_assignments SET is_active = 1
            WHERE coach_id = $1 AND user_id = $2
            ",
        )
        .bind(coach_id)
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to activate coach: {e}")))?;

        // Return the activated coach
        self.get(coach_id, user_id, tenant_id).await
    }

    /// Deactivate the currently active coach for a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn deactivate_coach(&self, user_id: Uuid, _tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE coach_assignments SET is_active = 0
            WHERE user_id = $1 AND is_active = 1
            ",
        )
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Get the currently active coach for a user
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_active_coach(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"
            SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.max_tool_iterations, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria
            FROM coaches c
            JOIN coach_assignments ca ON c.id = ca.coach_id AND ca.user_id = $1
            WHERE ca.is_active = 1 AND c.tenant_id = $2
            ",
        )
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get active coach: {e}")))?;

        row.map(|r| row_to_coach(&r)).transpose()
    }

    /// Find a coach by content hash for deduplication within a user's coaches.
    ///
    /// Returns the first coach matching the given `content_hash` for the specified
    /// user and tenant, or None if no match exists.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE content_hash = $1 AND user_id = $2 AND tenant_id = $3
            LIMIT 1
            ",
        )
        .bind(content_hash)
        .bind(user_id.to_string())
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to find coach by content hash: {e}")))?;

        row.map(|r| row_to_coach(&r)).transpose()
    }
}

/// Compute a content hash from a `CreateCoachRequest` using `DefaultHasher`.
///
/// Hashes the title, `system_prompt`, tags, and all structured section fields
/// to produce a deterministic 16-character hex string for deduplication.
#[must_use]
pub fn compute_request_hash(request: &CreateCoachRequest) -> String {
    let mut hasher = DefaultHasher::new();
    request.title.hash(&mut hasher);
    request.system_prompt.hash(&mut hasher);
    request.tags.hash(&mut hasher);
    request.purpose.hash(&mut hasher);
    request.instructions.hash(&mut hasher);
    request.example_inputs.hash(&mut hasher);
    request.example_outputs.hash(&mut hasher);
    request.success_criteria.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
