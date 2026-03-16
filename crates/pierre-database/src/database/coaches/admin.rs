// ABOUTME: System/admin coach management operations (create, list, get, update, delete)
// ABOUTME: Handles admin-created coaches visible to tenant users with elevated permissions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::CoachPrerequisites;
use pierre_core::models::TenantId;
use uuid::Uuid;

use super::types::{Coach, CreateSystemCoachRequest, UpdateCoachRequest};
use super::{row_to_coach, CoachesManager};

impl CoachesManager {
    // ============================================
    // System Coach Methods (Admin Operations)
    // ============================================

    /// Create a system coach (admin-created, visible to tenant users)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn create_system_coach(
        &self,
        admin_user_id: Uuid,
        tenant_id: TenantId,
        request: &CreateSystemCoachRequest,
    ) -> AppResult<Coach> {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let tags_json = serde_json::to_string(&request.tags)?;
        let sample_prompts_json = serde_json::to_string(&request.sample_prompts)?;
        let token_count = Self::estimate_tokens(&request.system_prompt);

        sqlx::query(
            r"
            INSERT INTO coaches (
                id, user_id, tenant_id, title, description, system_prompt,
                category, tags, sample_prompts, token_count,
                created_at, updated_at, is_system, visibility, prerequisites,
                forked_from, max_tool_iterations, startup_query, data_requirements
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, $12, $13, $14, $15, $16, $17, $18)
            ",
        )
        .bind(id.to_string())
        .bind(admin_user_id.to_string())
        .bind(tenant_id)
        .bind(&request.title)
        .bind(&request.description)
        .bind(&request.system_prompt)
        .bind(request.category.as_str())
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(1i64) // is_system = true
        .bind(request.visibility.as_str())
        .bind(Option::<String>::None) // prerequisites (system coaches may have this set later)
        .bind(Option::<String>::None) // forked_from (system coaches are originals)
        .bind(Option::<i32>::None) // max_tool_iterations
        .bind(Option::<String>::None) // startup_query
        .bind(Option::<String>::None) // data_requirements
        .execute(&self.pool)
        .await
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
            max_tool_iterations: None,
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
        })
    }

    /// List all system coaches in a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_system_coaches(&self, tenant_id: TenantId) -> AppResult<Vec<Coach>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE tenant_id = $1 AND is_system = 1
            ORDER BY created_at DESC
            ",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list system coaches: {e}")))?;

        rows.iter().map(row_to_coach).collect()
    }

    /// Get a system coach by ID
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_system_coach(
        &self,
        coach_id: &str,
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
            WHERE id = $1 AND tenant_id = $2 AND is_system = 1
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get system coach: {e}")))?;

        row.map(|r| row_to_coach(&r)).transpose()
    }

    /// Get a system coach by ID without tenant filtering
    ///
    /// System coaches are platform-wide resources visible to all users.
    /// Used by `fork_coach` where any user can fork any system coach.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_system_coach_any_tenant(&self, coach_id: &str) -> AppResult<Option<Coach>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements,
                   purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria
            FROM coaches
            WHERE id = $1 AND is_system = 1
            ",
        )
        .bind(coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get system coach: {e}")))?;

        row.map(|r| row_to_coach(&r)).transpose()
    }

    /// Update a system coach
    ///
    /// Automatically creates a version snapshot before applying changes.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn update_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
    ) -> AppResult<Option<Coach>> {
        self.update_system_coach_with_summary(coach_id, tenant_id, request, None)
            .await
    }

    /// Update a system coach with a change summary
    ///
    /// Automatically creates a version snapshot before applying changes.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn update_system_coach_with_summary(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        request: &UpdateCoachRequest,
        change_summary: Option<&str>,
    ) -> AppResult<Option<Coach>> {
        // First get the existing coach
        let existing = self.get_system_coach(coach_id, tenant_id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        // Create a version snapshot BEFORE applying changes
        // Use the existing coach's user_id (admin who created it) for the version record
        self.create_version(coach_id, existing.user_id, change_summary)
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

        let result = sqlx::query(
            r"
            UPDATE coaches SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND tenant_id = $10 AND is_system = 1
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
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update system coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        // Return updated coach
        self.get_system_coach(coach_id, tenant_id).await
    }

    /// Delete a system coach
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn delete_system_coach(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM coaches
            WHERE id = $1 AND tenant_id = $2 AND is_system = 1
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete system coach: {e}")))?;

        Ok(result.rows_affected() > 0)
    }
}
