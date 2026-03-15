// ABOUTME: Coach database module providing CRUD, assignments, versioning, and admin operations
// ABOUTME: Organizes coach operations into focused sub-modules with shared helpers and type re-exports
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// System/admin coach management methods
mod admin;
/// Coach assignment and user preference methods
mod assignments;
/// Type definitions for coach database operations
mod types;
/// User-facing coach CRUD and query methods
mod user;
/// Coach version history and startup query methods
mod versions;

pub use types::*;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{CoachPrerequisites, DataRequirements};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};
use uuid::Uuid;

/// Token estimation constant: average characters per token for system prompts
const CHARS_PER_TOKEN: usize = 4;

/// Coach database operations manager
pub struct CoachesManager {
    pool: SqlitePool,
}

impl CoachesManager {
    /// Create a new coaches manager
    #[must_use]
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Estimate token count for a system prompt
    ///
    /// Uses conservative estimate of ~4 characters per token
    #[allow(clippy::cast_possible_truncation)]
    pub(super) const fn estimate_tokens(text: &str) -> u32 {
        let char_count = text.len();
        let tokens = char_count / CHARS_PER_TOKEN;
        // Token count bounded by reasonable system prompt size (< 100K chars = < 25K tokens)
        tokens as u32
    }

    /// Ensure a `coach_assignments` row exists for a user+coach pair.
    ///
    /// Uses `INSERT OR IGNORE` so it is safe to call multiple times.
    /// This is needed for operations like `toggle_favorite`, `record_usage`,
    /// and `activate_coach` that need an assignment row to update.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub(super) async fn ensure_assignment_exists(
        &self,
        coach_id: &str,
        user_id: Uuid,
    ) -> AppResult<()> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at, is_favorite, is_active, use_count, last_used_at)
            VALUES ($1, $2, $3, $3, $4, 0, 0, 0, NULL)
            ",
        )
        .bind(id.to_string())
        .bind(coach_id)
        .bind(user_id.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to ensure coach assignment: {e}")))?;

        Ok(())
    }
}

/// Convert a database row to a `Coach` struct
///
/// # Errors
///
/// Returns an error if row fields cannot be parsed (invalid UUID, datetime, or JSON)
pub fn row_to_coach(row: &SqliteRow) -> AppResult<Coach> {
    let id_str: String = row.get("id");
    let user_id_str: String = row.get("user_id");
    let category_str: String = row.get("category");
    let tags_json: String = row.get("tags");
    let created_at_str: String = row.get("created_at");
    let updated_at_str: String = row.get("updated_at");
    let token_count: i64 = row.get("token_count");

    // Fields with defaults when columns are null or missing
    let is_system: i64 = row.try_get("is_system").unwrap_or(0);
    let visibility_str: String = row
        .try_get("visibility")
        .unwrap_or_else(|_| "private".to_owned());
    let sample_prompts_json: String = row
        .try_get("sample_prompts")
        .unwrap_or_else(|_| "[]".to_owned());
    let prerequisites_json: String = row
        .try_get("prerequisites")
        .unwrap_or_else(|_| "{}".to_owned());
    let forked_from: Option<String> = row.try_get("forked_from").ok();

    let tags: Vec<String> = serde_json::from_str(&tags_json)?;
    let sample_prompts: Vec<String> = serde_json::from_str(&sample_prompts_json)?;
    let prerequisites: CoachPrerequisites =
        serde_json::from_str(&prerequisites_json).unwrap_or_default();

    let max_tool_iterations: Option<i32> = row.try_get("max_tool_iterations").ok().flatten();
    let startup_query: Option<String> = row.try_get("startup_query").ok().flatten();
    let data_requirements_json: Option<String> = row.try_get("data_requirements").ok().flatten();
    let data_requirements: Option<DataRequirements> =
        data_requirements_json.and_then(|json| serde_json::from_str(&json).ok());

    Ok(Coach {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?,
        user_id: Uuid::parse_str(&user_id_str)
            .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?,
        tenant_id: row.get("tenant_id"),
        title: row.get("title"),
        description: row.get("description"),
        system_prompt: row.get("system_prompt"),
        category: CoachCategory::parse(&category_str),
        tags,
        sample_prompts,
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        token_count: token_count as u32,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| AppError::internal(format!("Invalid datetime: {e}")))?
            .with_timezone(&Utc),
        is_system: is_system == 1,
        visibility: CoachVisibility::parse(&visibility_str),
        prerequisites,
        forked_from,
        max_tool_iterations,
        startup_query,
        data_requirements,
    })
}

/// Convert a database row to a `CoachListItem` (with preference fields from `coach_assignments`)
pub(super) fn row_to_coach_list_item(row: &SqliteRow) -> AppResult<CoachListItem> {
    let coach = row_to_coach(row)?;
    let is_assigned: i64 = row.try_get("is_assigned").unwrap_or(0);
    let is_favorite: i64 = row.try_get("is_favorite").unwrap_or(0);
    let is_active: i64 = row.try_get("is_active").unwrap_or(0);
    let use_count: i64 = row.try_get("use_count").unwrap_or(0);
    let last_used_at_str: Option<String> = row.try_get("last_used_at").unwrap_or(None);

    Ok(CoachListItem {
        coach,
        is_assigned: is_assigned == 1,
        is_favorite: is_favorite == 1,
        is_active: is_active == 1,
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        use_count: use_count as u32,
        last_used_at: last_used_at_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc)),
    })
}

/// Compute hash of content for version tracking
pub(super) fn compute_content_hash(content: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
