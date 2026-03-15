// ABOUTME: Coach version history and startup query operations
// ABOUTME: Handles version snapshots, history retrieval, reverts, and coach lookup by system prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::{sqlite::SqliteRow, Row};
use uuid::Uuid;

use super::types::{Coach, CoachVersion};
use super::{compute_content_hash, row_to_coach, CoachesManager};

impl CoachesManager {
    // ============================================
    // Coach Version History Methods (ASY-153)
    // ============================================

    /// Create a new version snapshot for a coach
    ///
    /// This is called automatically when a coach is updated to track version history.
    /// Returns the new version number.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn create_version(
        &self,
        coach_id: &str,
        user_id: Uuid,
        change_summary: Option<&str>,
    ) -> AppResult<i32> {
        // Get the current coach to snapshot
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements
            FROM coaches WHERE id = $1
            ",
        )
        .bind(coach_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get coach for versioning: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        let coach = row_to_coach(&row)?;

        // Get the next version number
        let version_row = sqlx::query(
            r"
            SELECT COALESCE(MAX(version), 0) as max_version
            FROM coach_versions WHERE coach_id = $1
            ",
        )
        .bind(coach_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get max version: {e}")))?;

        let max_version: i32 = version_row.get("max_version");
        let new_version = max_version + 1;

        // Create content snapshot as JSON
        let content_snapshot = serde_json::json!({
            "title": coach.title,
            "description": coach.description,
            "system_prompt": coach.system_prompt,
            "category": coach.category.as_str(),
            "tags": coach.tags,
            "sample_prompts": coach.sample_prompts,
            "token_count": coach.token_count,
            "visibility": coach.visibility.as_str(),
            "prerequisites": coach.prerequisites,
        });

        // Compute content hash
        let content_hash = compute_content_hash(&content_snapshot);

        // Insert the version record
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO coach_versions (
                id, coach_id, version, content_hash, content_snapshot,
                change_summary, created_at, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ",
        )
        .bind(id.to_string())
        .bind(coach_id)
        .bind(new_version)
        .bind(&content_hash)
        .bind(content_snapshot.to_string())
        .bind(change_summary)
        .bind(now.to_rfc3339())
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create version: {e}")))?;

        Ok(new_version)
    }

    /// Get version history for a coach
    ///
    /// Returns versions in descending order (newest first).
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_versions(
        &self,
        coach_id: &str,
        tenant_id: TenantId,
        limit: u32,
    ) -> AppResult<Vec<CoachVersion>> {
        // Verify the coach exists and belongs to the tenant
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
            return Err(AppError::not_found(format!("Coach {coach_id}")));
        }

        let limit_val = i32::try_from(limit).unwrap_or(50);

        let rows = sqlx::query(
            r"
            SELECT cv.id, cv.coach_id, cv.version, cv.content_hash, cv.content_snapshot,
                   cv.change_summary, cv.created_at, cv.created_by
            FROM coach_versions cv
            WHERE cv.coach_id = $1
            ORDER BY cv.version DESC
            LIMIT $2
            ",
        )
        .bind(coach_id)
        .bind(limit_val)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get versions: {e}")))?;

        rows.iter().map(row_to_coach_version).collect()
    }

    /// Get a specific version of a coach
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails or version not found
    pub async fn get_version(
        &self,
        coach_id: &str,
        version: i32,
        tenant_id: TenantId,
    ) -> AppResult<Option<CoachVersion>> {
        // Verify the coach exists and belongs to the tenant
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
            return Err(AppError::not_found(format!("Coach {coach_id}")));
        }

        let row = sqlx::query(
            r"
            SELECT id, coach_id, version, content_hash, content_snapshot,
                   change_summary, created_at, created_by
            FROM coach_versions
            WHERE coach_id = $1 AND version = $2
            ",
        )
        .bind(coach_id)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get version: {e}")))?;

        row.map(|r| row_to_coach_version(&r)).transpose()
    }

    /// Revert a coach to a previous version
    ///
    /// This creates a NEW version with the content from the specified version,
    /// preserving the complete history (doesn't delete any versions).
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails or version not found
    pub async fn revert_to_version(
        &self,
        coach_id: &str,
        version: i32,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Coach> {
        // Get the target version
        let target_version = self
            .get_version(coach_id, version, tenant_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found(format!("Version {version} for coach {coach_id}"))
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
        let token_count = Self::estimate_tokens(system_prompt);

        // Update the coach with the reverted content
        let result = sqlx::query(
            r"
            UPDATE coaches SET
                title = $1, description = $2, system_prompt = $3,
                category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
            WHERE id = $9 AND tenant_id = $10
            ",
        )
        .bind(title)
        .bind(&description)
        .bind(system_prompt)
        .bind(category_str)
        .bind(&tags_json)
        .bind(&sample_prompts_json)
        .bind(i64::from(token_count))
        .bind(now.to_rfc3339())
        .bind(coach_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to revert coach: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(format!("Coach {coach_id}")));
        }

        // Create a new version recording this revert
        let change_summary = format!("Reverted to version {version}");
        self.create_version(coach_id, user_id, Some(&change_summary))
            .await?;

        // Return the updated coach
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, description, system_prompt,
                   category, tags, sample_prompts, token_count,
                   created_at, updated_at, is_system, visibility, prerequisites,
                   forked_from, max_tool_iterations, startup_query, data_requirements
            FROM coaches WHERE id = $1 AND tenant_id = $2
            ",
        )
        .bind(coach_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get reverted coach: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        row_to_coach(&row)
    }

    /// Get the current version number for a coach
    ///
    /// Returns 0 if no versions exist yet.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_current_version(&self, coach_id: &str) -> AppResult<i32> {
        let row = sqlx::query(
            r"
            SELECT COALESCE(MAX(version), 0) as current_version
            FROM coach_versions WHERE coach_id = $1
            ",
        )
        .bind(coach_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get current version: {e}")))?;

        Ok(row.get("current_version"))
    }

    // ============================================
    // Startup Query Methods
    // ============================================

    // NOTE: Store methods (submit_for_review, approve_coach, reject_coach,
    // get_published_coaches, install_from_store, uninstall_coach, etc.)
    // have been moved to StoreListingsManager in store_listings.rs

    /// Get the startup query for a coach by matching its system prompt.
    ///
    /// This is used to automatically inject a startup query when a user
    /// starts a conversation with a coach that has one configured.
    /// The `system_prompt` is stored in conversations when a coach is selected.
    ///
    /// # Arguments
    ///
    /// * `system_prompt` - The system prompt to match against coach instructions
    /// * `tenant_id` - The tenant ID to scope the search
    ///
    /// # Returns
    ///
    /// The `startup_query` if found, None otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    /// Get startup context (query + data requirements) by system prompt.
    ///
    /// Returns both the startup query and the JSON-serialized data requirements
    /// for deterministic activity pre-fetching.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails.
    pub async fn get_startup_context_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<(Option<String>, Option<String>)>> {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            r"
            SELECT startup_query, data_requirements
            FROM coaches
            WHERE system_prompt = $1
              AND (startup_query IS NOT NULL OR data_requirements IS NOT NULL)
              AND (tenant_id = $2 OR is_system = 1)
            LIMIT 1
            ",
        )
        .bind(system_prompt)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get startup context: {e}")))?;

        Ok(row)
    }

    /// Convenience method: get startup query only.
    /// Delegates to `get_startup_context_by_system_prompt` for the query portion.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails.
    pub async fn get_startup_query_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<String>> {
        let context = self
            .get_startup_context_by_system_prompt(system_prompt, tenant_id)
            .await?;
        Ok(context.and_then(|(q, _)| q))
    }

    /// Get the `max_tool_iterations` override for a coach by matching its system prompt.
    ///
    /// Used to resolve per-coach tool iteration limits in the chat tool loop.
    /// Returns `None` if no coach matches or the coach has no override set.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_max_tool_iterations_by_system_prompt(
        &self,
        system_prompt: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<i32>> {
        let row: Option<(Option<i32>,)> = sqlx::query_as(
            r"
            SELECT max_tool_iterations
            FROM coaches
            WHERE system_prompt = $1
              AND (tenant_id = $2 OR is_system = 1)
            LIMIT 1
            ",
        )
        .bind(system_prompt)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get max_tool_iterations: {e}")))?;

        Ok(row.and_then(|(v,)| v))
    }
}

/// Convert a database row to a `CoachVersion` struct
fn row_to_coach_version(row: &SqliteRow) -> AppResult<CoachVersion> {
    let id: String = row.get("id");
    let coach_id: String = row.get("coach_id");
    let version: i32 = row.get("version");
    let content_hash: String = row.get("content_hash");
    let content_snapshot_str: String = row.get("content_snapshot");
    let change_summary: Option<String> = row.get("change_summary");
    let created_at_str: String = row.get("created_at");
    let created_by_str: Option<String> = row.get("created_by");

    let content_snapshot: serde_json::Value = serde_json::from_str(&content_snapshot_str)
        .map_err(|e| AppError::internal(format!("Invalid JSON in version snapshot: {e}")))?;

    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
        .with_timezone(&Utc);

    let created_by = created_by_str
        .map(|s| Uuid::parse_str(&s))
        .transpose()
        .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?;

    Ok(CoachVersion {
        id,
        coach_id,
        version,
        content_hash,
        content_snapshot,
        change_summary,
        created_at,
        created_by,
    })
}
