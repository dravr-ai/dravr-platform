// ABOUTME: PostgreSQL implementation of AgentsRepository for agent CRUD, assignments, and versioning
// ABOUTME: Uses PG-native types (TIMESTAMPTZ, BOOLEAN, UUID) and parameterized queries throughout
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::AgentsRepository;
use super::agents_assignments as assignments;
use super::agents_copies as copies;
use super::agents_rows::{
    compute_content_hash, compute_request_hash, row_to_agent_list_item_pg, row_to_agent_pg,
    row_to_agent_version_pg, token_count_as_i32,
};
use super::PostgresDatabase;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{
    Agent, AgentAssignment, AgentCategory, AgentHandle, AgentListItem, AgentPrerequisites,
    AgentVersion, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest, ListAgentsFilter,
    UpdateAgentRequest,
};
use pierre_core::models::TenantId;
use pierre_core::models::{split_visuals, AgentRuntimeContext};
use pierre_core::tokens::estimate_prompt_tokens;
use sqlx::Row;
use uuid::Uuid;

impl PostgresDatabase {
    /// Ensure a `agent_assignments` row exists for a user+agent pair.
    ///
    /// Uses `INSERT ... ON CONFLICT DO NOTHING` so it is safe to call multiple times.
    /// This is needed for operations like `toggle_favorite`, `record_usage`,
    /// and `activate_agent` that need an assignment row to update.
    async fn ensure_agent_assignment_exists(&self, agent_id: &str, user_id: Uuid) -> AppResult<()> {
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, FALSE, 0, NULL)
            ON CONFLICT (agent_id, user_id) DO NOTHING
            ",
        )
        .bind(id.to_string())
        .bind(agent_id)
        .bind(user_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to ensure coach assignment: {e}")))?;

        Ok(())
    }

    /// Create a version snapshot for an agent (internal helper shared by update methods)
    ///
    /// Reads the current agent state, computes a content hash, and inserts a new
    /// version record with an incremented version number.
    async fn create_agent_version_pg(
        &self,
        agent_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        // Get the current agent to snapshot
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM agents WHERE id = $1
            ",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach for versioning: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;

        let agent = row_to_agent_pg(&row)?;

        // Get the next version number
        let version_row = sqlx::query(
            r"
            SELECT COALESCE(MAX(version), 0) as max_version
            FROM agent_versions WHERE agent_id = $1
            ",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get max version: {e}")))?;

        let max_version: i32 = version_row.get("max_version");
        let new_version = max_version + 1;

        // Create content snapshot as JSON
        let content_snapshot = serde_json::json!({
            "title": agent.title,
            "description": agent.description,
            "system_prompt": agent.system_prompt,
            "category": agent.category.as_str(),
            "tags": agent.tags,
            "sample_prompts": agent.sample_prompts,
            "token_count": agent.token_count,
            "visibility": agent.visibility.as_str(),
            "prerequisites": agent.prerequisites,
        });

        let content_hash = compute_content_hash(&content_snapshot);

        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO agent_versions (
                id, agent_id, version, content_hash, content_snapshot,
                change_summary, created_at, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ",
        )
        .bind(id.to_string())
        .bind(agent_id)
        .bind(new_version)
        .bind(&content_hash)
        .bind(content_snapshot.to_string())
        .bind(change_summary)
        .bind(now)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create version: {e}")))?;

        Ok(new_version)
    }
}

#[async_trait]
impl AgentsRepository for PostgresDatabase {
    // ============================================
    // User Agent Methods (CRUD)
    // ============================================

    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateAgentRequest,
    ) -> AppResult<Agent> {
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

        // Section-aware token count
        let token_count = if request.instructions.is_some() || request.purpose.is_some() {
            let combined = [
                request.purpose.as_deref().unwrap_or_default(),
                request.instructions.as_deref().unwrap_or_default(),
                request.example_inputs.as_deref().unwrap_or_default(),
                request.example_outputs.as_deref().unwrap_or_default(),
                request.success_criteria.as_deref().unwrap_or_default(),
            ]
            .concat();
            estimate_prompt_tokens(&combined)
        } else {
            estimate_prompt_tokens(&request.system_prompt)
        };

        // Serialize data_requirements to JSON if present
        let data_requirements_json = request
            .data_requirements
            .as_ref()
            .and_then(|dr| serde_json::to_string(dr).ok());

        // Compute content hash from request fields for deduplication
        let content_hash = compute_request_hash(request);

        sqlx::query(
            r"
            INSERT INTO agents (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
                purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria,
                content_hash
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)
            ",
        )
        .bind(id.to_string())
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .bind(&request.title)
        .bind(&request.description)
        .bind(effective_system_prompt)
        .bind(request.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(token_count_as_i32(token_count))
        .bind(now)
        .bind(false) // is_system (user-created agents are not system)
        .bind(AgentVisibility::Private.as_str())
        .bind(Option::<String>::None) // prerequisites
        .bind(Option::<String>::None) // forked_from
        .bind(request.max_tool_iterations)
        .bind(Option::<f32>::None) // temperature
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

        // Create self-assignment row for the creator
        let assignment_id = Uuid::new_v4();
        sqlx::query(
            r"
            INSERT INTO agent_assignments (id, agent_id, user_id, assigned_by, created_at, is_favorite, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, FALSE, 0, NULL)
            ",
        )
        .bind(assignment_id.to_string())
        .bind(id.to_string())
        .bind(user_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create coach assignment: {e}")))?;

        Ok(Agent {
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
            visibility: AgentVisibility::Private,
            prerequisites: AgentPrerequisites::default(),
            forked_from: None,
            handle: None,
            max_tool_iterations: request.max_tool_iterations,
            temperature: None,
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

    async fn get_by_id(
        &self,
        agent_id: &str,
        user_id: Uuid,
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
            WHERE id = $1 AND (
                (user_id = $2 AND tenant_id = $3)
                OR is_system = TRUE
                OR id IN (SELECT agent_id FROM agent_assignments WHERE user_id = $2)
            )
            ",
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach: {e}")))?;

        row.map(|r| row_to_agent_pg(&r)).transpose()
    }

    async fn find_installed_by_handle(
        &self,
        handle: &AgentHandle,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>> {
        copies::find_installed_by_handle(&self.pool, handle, user_id, tenant_id).await
    }

    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        filter: &ListAgentsFilter,
    ) -> AppResult<Vec<AgentListItem>> {
        let limit_val = i32::try_from(filter.limit.unwrap_or(50)).unwrap_or(50);
        let offset_val = i32::try_from(filter.offset.unwrap_or(0)).unwrap_or(0);

        // Build dynamic query parts based on filters (static conditions only, no user values)
        let category_filter = filter
            .category
            .as_ref()
            .map(|c| format!("AND c.category = '{}'", c.as_str()))
            .unwrap_or_default();
        let favorites_filter = if filter.favorites_only {
            "AND ca.is_favorite = TRUE"
        } else {
            ""
        };
        let hidden_filter = if filter.include_hidden {
            ""
        } else {
            "AND c.id NOT IN (SELECT agent_id FROM user_agent_preferences WHERE user_id = $1 AND is_hidden = TRUE)"
        };

        // System agents (is_system=TRUE) are platform-wide resources visible to all users
        let system_condition = if filter.include_system {
            "OR c.is_system = TRUE"
        } else {
            ""
        };

        let query = format!(
            r"
            SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.slug, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria,
                   CASE WHEN ca.agent_id IS NOT NULL THEN TRUE ELSE FALSE END as is_assigned,
                   COALESCE(ca.is_favorite, FALSE) as is_favorite,
                   (tu.selected_agent_id = c.id) as is_active,
                   COALESCE(ca.use_count, 0) as use_count,
                   ca.last_used_at
            FROM agents c
            LEFT JOIN agent_assignments ca ON c.id = ca.agent_id AND ca.user_id = $1
            LEFT JOIN tenant_users tu ON tu.user_id = $1 AND tu.tenant_id = $2::uuid
            WHERE (
                -- Personal agents: owned by user
                (c.user_id = $1 AND c.is_system = FALSE AND c.tenant_id = $2)
                -- System agents visible to tenant
                {system_condition}
                -- Assigned agents: explicitly assigned to user
                OR c.id IN (SELECT agent_id FROM agent_assignments WHERE user_id = $1)
            )
            {category_filter}
            {favorites_filter}
            {hidden_filter}
            ORDER BY c.updated_at DESC
            LIMIT $3 OFFSET $4
            "
        );

        let rows = sqlx::query(&query)
            .bind(user_id)
            .bind(tenant_id.as_uuid())
            .bind(limit_val)
            .bind(offset_val)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list coaches: {e}")))?;

        rows.iter().map(row_to_agent_list_item_pg).collect()
    }

    async fn apply_translations(
        &self,
        agents: &mut [AgentListItem],
        locale: &str,
    ) -> AppResult<()> {
        let ids: Vec<String> = agents.iter().map(|it| it.agent.id.to_string()).collect();
        let overlays = self.agent_translation_overlays(&ids, locale).await?;
        for item in agents.iter_mut() {
            if let Some(ov) = overlays.get(&item.agent.id.to_string()) {
                ov.apply(&mut item.agent);
            }
        }
        Ok(())
    }

    async fn translate_agents(&self, agents: &mut [Agent], locale: &str) -> AppResult<()> {
        let ids: Vec<String> = agents.iter().map(|c| c.id.to_string()).collect();
        let overlays = self.agent_translation_overlays(&ids, locale).await?;
        for agent in agents.iter_mut() {
            if let Some(ov) = overlays.get(&agent.id.to_string()) {
                ov.apply(agent);
            }
        }
        Ok(())
    }

    async fn update(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        request: &UpdateAgentRequest,
        change_summary: Option<&str>,
    ) -> AppResult<Option<Agent>> {
        // First get the existing agent
        let existing = self.get_by_id(agent_id, user_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        // Snapshot BEFORE applying changes, carrying the caller's summary.
        self.create_agent_version_pg(agent_id, user_id, change_summary)
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

        // Resolve startup_query: use new value if provided, otherwise keep existing via COALESCE
        let startup_query: Option<String> = if request.startup_query.is_some() {
            request
                .startup_query
                .as_ref()
                .filter(|q| !q.is_empty())
                .cloned()
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT startup_query FROM agents WHERE id = $1")
                    .bind(agent_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get startup_query: {e}")))?;
            existing_row.and_then(|(q,)| q)
        };

        // Resolve data_requirements
        let data_requirements_json: Option<String> = if request.data_requirements.is_some() {
            request
                .data_requirements
                .as_ref()
                .and_then(|dr| serde_json::to_string(dr).ok())
        } else {
            let existing_row: Option<(Option<String>,)> =
                sqlx::query_as("SELECT data_requirements FROM agents WHERE id = $1")
                    .bind(agent_id)
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
        // Three-way, not a coalesce: an absent field keeps the stored budget,
        // an explicit null clears it back to inheriting the admin value.
        let max_tool_iterations = request
            .max_tool_iterations
            .resolve(existing.max_tool_iterations);

        // When instructions is updated, also update system_prompt for runtime compatibility
        let system_prompt = instructions
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(system_prompt);

        let result = sqlx::query(
            r"
            UPDATE agents SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8,
                startup_query = $12, data_requirements = $13,
                purpose = $14, when_to_use = $15, instructions = $16,
                example_inputs = $17, example_outputs = $18, success_criteria = $19,
                max_tool_iterations = $20
            WHERE id = $9 AND user_id = $10 AND tenant_id = $11
            ",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(token_count_as_i32(token_count))
        .bind(now)
        .bind(agent_id)
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .bind(&startup_query)
        .bind(&data_requirements_json)
        .bind(&purpose)
        .bind(&when_to_use)
        .bind(&instructions)
        .bind(&example_inputs)
        .bind(&example_outputs)
        .bind(&success_criteria)
        .bind(max_tool_iterations)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        // Return updated agent
        self.get_by_id(agent_id, user_id, tenant_id).await
    }

    async fn delete(&self, agent_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM agents
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn fork_agent(
        &self,
        source_agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Agent> {
        copies::fork_agent(self, source_agent_id, user_id, tenant_id).await
    }

    async fn record_usage(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let now = Utc::now();

        // Verify the agent is reachable to the caller. System agents (is_system = TRUE)
        // are pinned to the seed tenant but exposed to every tenant via the catalog,
        // so we accept them unconditionally — otherwise any non-seed-tenant user
        // chatting with a builtin agent silently skips usage tracking.
        let exists = sqlx::query(
            r"
            SELECT 1 FROM agents WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE)
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if exists.is_none() {
            return Ok(false);
        }

        // Ensure assignment row exists
        self.ensure_agent_assignment_exists(agent_id, user_id)
            .await?;

        // Update usage in agent_assignments
        let result = sqlx::query(
            r"
            UPDATE agent_assignments SET
                use_count = use_count + 1,
                last_used_at = $1
            WHERE agent_id = $2 AND user_id = $3
            ",
        )
        .bind(now)
        .bind(agent_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to record coach usage: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn toggle_favorite(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<bool>> {
        // Same reasoning as record_usage: accept system agents so favorites
        // toggle for non-seed-tenant users on builtin agents.
        let agent_exists = sqlx::query(
            r"
            SELECT 1 FROM agents WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE)
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if agent_exists.is_none() {
            return Ok(None);
        }

        // Ensure assignment row exists
        self.ensure_agent_assignment_exists(agent_id, user_id)
            .await?;

        // Get current favorite status
        let row = sqlx::query(
            r"
            SELECT ca.is_favorite FROM agent_assignments ca
            WHERE ca.agent_id = $1 AND ca.user_id = $2
            ",
        )
        .bind(agent_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get favorite status: {e}")))?;

        let current: bool = row.is_some_and(|r| r.get("is_favorite"));
        let new_value = !current;

        // Update in agent_assignments
        sqlx::query(
            r"
            UPDATE agent_assignments SET is_favorite = $1
            WHERE agent_id = $2 AND user_id = $3
            ",
        )
        .bind(new_value)
        .bind(agent_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to toggle favorite: {e}")))?;

        Ok(Some(new_value))
    }

    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count FROM agents
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count coaches: {e}")))?;

        let count: i64 = row.get("count");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(count as u32)
    }

    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Agent>> {
        let limit_val = i32::try_from(limit.unwrap_or(20)).unwrap_or(20);
        let offset_val = i32::try_from(offset.unwrap_or(0)).unwrap_or(0);
        let search_pattern = format!("%{query}%");

        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM agents
            WHERE user_id = $1 AND tenant_id = $2 AND (
                title ILIKE $3 OR description ILIKE $3 OR tags ILIKE $3
            )
            ORDER BY updated_at DESC
            LIMIT $4 OFFSET $5
            ",
        )
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .bind(&search_pattern)
        .bind(limit_val)
        .bind(offset_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to search coaches: {e}")))?;

        rows.iter().map(row_to_agent_pg).collect()
    }

    async fn activate_agent(
        &self,
        agent_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>> {
        // Same reasoning as record_usage / toggle_favorite: accept system agents
        // unconditionally so non-seed-tenant users can pick a builtin agent as
        // their active default.
        let agent_exists = sqlx::query(
            r"
            SELECT 1 FROM agents WHERE id = $1 AND (tenant_id = $2 OR is_system = TRUE)
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if agent_exists.is_none() {
            return Ok(None);
        }

        // The roster row still records entitlement, favourites and usage.
        self.ensure_agent_assignment_exists(agent_id, user_id)
            .await?;

        // Selection is one pointer on the membership row. The pair of UPDATEs
        // this replaced (clear every row, then set one) was not atomic and could
        // leave a user with zero or two active agents.
        sqlx::query(
            r"
            UPDATE tenant_users SET selected_agent_id = $1
            WHERE user_id = $2 AND tenant_id = $3::uuid
            ",
        )
        .bind(agent_id)
        .bind(user_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to activate coach: {e}")))?;

        // Return the activated agent
        self.get_by_id(agent_id, user_id, tenant_id).await
    }

    async fn deactivate_agent(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        // Clears the selection, leaving the roster intact: the agent is still
        // available to the user, they just are not talking to it.
        let result = sqlx::query(
            r"
            UPDATE tenant_users SET selected_agent_id = NULL
            WHERE user_id = $1 AND tenant_id = $2::uuid AND selected_agent_id IS NOT NULL
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_active_agent(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Agent>> {
        let row = sqlx::query(
            r"
            SELECT c.id, c.user_id, c.tenant_id, c.title, c.description, c.system_prompt,
                   c.category, c.tags, c.sample_prompts, c.token_count,
                   c.created_at, c.updated_at, c.is_system, c.visibility, c.prerequisites,
                   c.forked_from, c.slug, c.max_tool_iterations, c.temperature, c.startup_query, c.data_requirements,
                   c.purpose, c.when_to_use, c.instructions, c.example_inputs, c.example_outputs, c.success_criteria
            FROM agents c
            JOIN tenant_users tu ON c.id = tu.selected_agent_id
            WHERE tu.user_id = $1 AND tu.tenant_id = $2::uuid
            ",
        )
        .bind(user_id)
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get active coach: {e}")))?;

        row.map(|r| row_to_agent_pg(&r)).transpose()
    }

    async fn find_by_content_hash(
        &self,
        content_hash: &str,
        user_id: Uuid,
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
            WHERE content_hash = $1 AND user_id = $2 AND tenant_id = $3
            LIMIT 1
            ",
        )
        .bind(content_hash)
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to find coach by content hash: {e}")))?;

        row.map(|r| row_to_agent_pg(&r)).transpose()
    }

    // ============================================
    // Admin Methods
    // ============================================

    async fn create_system_agent(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemAgentRequest,
    ) -> AppResult<Agent> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;
        let token_count = estimate_prompt_tokens(&request.system_prompt);

        sqlx::query(
            r"
            INSERT INTO agents (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, temperature, startup_query, data_requirements
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18, $19)
            ",
        )
        .bind(id.to_string())
        .bind(admin_user_id)
        .bind(tenant_id.as_uuid())
        .bind(&request.title)
        .bind(&request.description)
        .bind(&request.system_prompt)
        .bind(request.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(token_count_as_i32(token_count))
        .bind(now)
        .bind(true) // is_system = true
        .bind(request.visibility.as_str())
        .bind(Option::<String>::None) // prerequisites
        .bind(Option::<String>::None) // forked_from
        .bind(Option::<i32>::None) // max_tool_iterations
        .bind(Option::<f32>::None) // temperature
        .bind(Option::<String>::None) // startup_query
        .bind(Option::<String>::None) // data_requirements
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create system agent: {e}")))?;

        Ok(Agent {
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
            prerequisites: AgentPrerequisites::default(),
            forked_from: None,
            handle: None,
            max_tool_iterations: None,
            temperature: None,
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            source: "custom".to_owned(),
        })
    }

    async fn list_system_agents(&self, tenant_id: TenantId) -> AppResult<Vec<Agent>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM agents
            WHERE tenant_id = $1 AND is_system = TRUE
            ORDER BY created_at DESC
            ",
        )
        .bind(tenant_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list system agents: {e}")))?;

        rows.iter().map(row_to_agent_pg).collect()
    }

    async fn get_system_agent(
        &self,
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
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get system agent: {e}")))?;

        row.map(|r| row_to_agent_pg(&r)).transpose()
    }

    async fn get_system_agent_any_tenant(&self, agent_id: &str) -> AppResult<Option<Agent>> {
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
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get system agent: {e}")))?;

        row.map(|r| row_to_agent_pg(&r)).transpose()
    }

    async fn update_system_agent(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
        request: &UpdateAgentRequest,
    ) -> AppResult<Option<Agent>> {
        // First get the existing agent
        let existing = self.get_system_agent(agent_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        // Create a version snapshot BEFORE applying changes
        // Use the existing agent's user_id (admin who created it) for the version record
        self.create_agent_version_pg(agent_id, existing.user_id, None)
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
            r"
            UPDATE agents SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND tenant_id = $10 AND is_system = TRUE
            ",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(token_count_as_i32(token_count))
        .bind(now)
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update system agent: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        // Return updated agent
        self.get_system_agent(agent_id, tenant_id).await
    }

    async fn delete_system_agent(&self, agent_id: &str, tenant_id: TenantId) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM agents
            WHERE id = $1 AND tenant_id = $2 AND is_system = TRUE
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete system agent: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Assignment Methods
    // ============================================

    async fn get_user_preferences(
        &self,
        agent_id: &str,
        user_id: Uuid,
    ) -> AppResult<(bool, u32, Option<DateTime<Utc>>)> {
        let row = sqlx::query(
            r"
            SELECT is_favorite, use_count, last_used_at
            FROM agent_assignments
            WHERE agent_id = $1 AND user_id = $2
            ",
        )
        .bind(agent_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user preferences: {e}")))?;

        row.map_or(Ok((false, 0, None)), |r| {
            let is_favorite: bool = r.get("is_favorite");
            let use_count: i32 = r.get("use_count");
            let last_used_at: Option<DateTime<Utc>> = r.get("last_used_at");
            #[allow(clippy::cast_sign_loss)]
            Ok((is_favorite, use_count as u32, last_used_at))
        })
    }

    async fn assign_agent(
        &self,
        agent_id: &str,
        user_id: Uuid,
        assigned_by: Uuid,
    ) -> AppResult<bool> {
        assignments::assign_agent(&self.pool, agent_id, user_id, assigned_by).await
    }

    async fn unassign_agent(&self, agent_id: &str, user_id: Uuid) -> AppResult<bool> {
        assignments::unassign_agent(&self.pool, agent_id, user_id).await
    }

    async fn list_assignments(&self, agent_id: &str) -> AppResult<Vec<AgentAssignment>> {
        assignments::list_assignments(&self.pool, agent_id).await
    }

    async fn list_assignments_for_tenant(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<AgentAssignment>> {
        assignments::list_assignments_for_tenant(&self.pool, agent_id, tenant_id).await
    }

    async fn hide_agent(&self, id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool> {
        assignments::hide_agent(&self.pool, id, user_id, tenant_id).await
    }

    async fn show_agent(&self, agent_id: &str, user_id: Uuid) -> AppResult<bool> {
        assignments::show_agent(&self.pool, agent_id, user_id).await
    }

    async fn list_hidden_agents(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Vec<Agent>> {
        assignments::list_hidden_agents(&self.pool, user_id, tenant_id).await
    }

    // ============================================
    // Version Methods
    // ============================================

    async fn create_version(
        &self,
        agent_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        self.create_agent_version_pg(agent_id, user_id, change_summary)
            .await
    }

    async fn get_versions(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<AgentVersion>> {
        // Verify the agent exists and belongs to the tenant
        let exists = sqlx::query(
            r"
            SELECT 1 FROM agents WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if exists.is_none() {
            return Err(AppError::not_found(format!("Coach {agent_id}")));
        }

        let limit_val = i32::try_from(limit).unwrap_or(50);

        let rows = sqlx::query(
            r"
            SELECT cv.id, cv.agent_id, cv.version, cv.content_hash, cv.content_snapshot,
                   cv.change_summary, cv.created_at, cv.created_by
            FROM agent_versions cv
            WHERE cv.agent_id = $1
            ORDER BY cv.version DESC
            LIMIT $2
            ",
        )
        .bind(agent_id)
        .bind(limit_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get versions: {e}")))?;

        rows.iter().map(row_to_agent_version_pg).collect()
    }

    async fn get_version(
        &self,
        agent_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<AgentVersion>> {
        // Verify the agent exists and belongs to the tenant
        let exists = sqlx::query(
            r"
            SELECT 1 FROM agents WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to verify coach: {e}")))?;

        if exists.is_none() {
            return Err(AppError::not_found(format!("Coach {agent_id}")));
        }

        let row = sqlx::query(
            r"
            SELECT id, agent_id, version, content_hash, content_snapshot,
                   change_summary, created_at, created_by
            FROM agent_versions
            WHERE agent_id = $1 AND version = $2
            ",
        )
        .bind(agent_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get version: {e}")))?;

        row.map(|r| row_to_agent_version_pg(&r)).transpose()
    }

    async fn revert_to_version(
        &self,
        agent_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Agent> {
        // Get the target version
        let target_version = self
            .get_version(agent_id, version, tenant_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found(format!("Version {version} for coach {agent_id}"))
            })?;

        // Extract fields from the snapshot
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

        // Update the agent with the reverted content.
        // Owner-gated write: mirror `update()`'s `WHERE id AND user_id AND
        // tenant_id` predicate so only the agent owner can revert. A
        // non-owner (even within the same tenant) matches zero rows and is
        // denied below, closing the version-revert IDOR.
        let result = sqlx::query(
            r"
            UPDATE agents SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND user_id = $10 AND tenant_id = $11
            ",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category_str)
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(token_count_as_i32(token_count))
        .bind(now)
        .bind(agent_id)
        .bind(user_id)
        .bind(tenant_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to revert coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("Coach {agent_id}")));
        }

        // Create a new version recording this revert
        let change_summary = format!("Reverted to version {version}");
        self.create_agent_version_pg(agent_id, user_id, Some(&change_summary))
            .await?;

        // Return the updated agent
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, slug, max_tool_iterations, temperature, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM agents WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get reverted coach: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {agent_id}")))?;

        row_to_agent_pg(&row)
    }

    async fn get_current_version(&self, agent_id: &str) -> AppResult<i32> {
        let row = sqlx::query(
            r"
            SELECT COALESCE(MAX(version), 0) as current_version
            FROM agent_versions WHERE agent_id = $1
            ",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get current version: {e}")))?;

        Ok(row.get("current_version"))
    }

    async fn get_agent_runtime_context(
        &self,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<AgentRuntimeContext>> {
        // Column order matches the SQLite impl in
        // `database/repositories/coaches_impl.rs` — keep both in lock-step.
        type Row = (
            Option<String>,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i32>,
            Option<f32>,
            String,
        );
        let row: Option<Row> = sqlx::query_as(
            r"
            SELECT slug, source, system_prompt, startup_query, data_requirements, visuals, max_tool_iterations, temperature, category
            FROM agents
            WHERE id = $1
              AND (tenant_id = $2 OR is_system = TRUE)
            LIMIT 1
            ",
        )
        .bind(agent_id)
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach runtime context: {e}")))?;

        Ok(row.map(
            |(
                slug,
                source,
                system_prompt,
                startup_query,
                data_requirements,
                visuals,
                max_tool_iterations,
                temperature,
                category,
            )| {
                AgentRuntimeContext {
                    slug: slug.unwrap_or_default(),
                    source,
                    system_prompt,
                    startup_query,
                    data_requirements,
                    visuals: split_visuals(visuals.as_deref()),
                    max_tool_iterations,
                    temperature,
                    category: AgentCategory::parse(&category),
                }
            },
        ))
    }
}
