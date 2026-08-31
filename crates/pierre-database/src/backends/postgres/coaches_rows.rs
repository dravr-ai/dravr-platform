// ABOUTME: PostgreSQL row → coach domain mappers plus the coach content/request hashes
// ABOUTME: Reads PG-native types (UUID, BOOLEAN, TIMESTAMPTZ) and parses the JSON-encoded columns

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{
    Coach, CoachCategory, CoachListItem, CoachPrerequisites, CoachVersion, CoachVisibility,
    CreateCoachRequest, DataRequirements,
};
use sqlx::postgres::PgRow;
use sqlx::Row;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

/// Convert a u32 token count to i32 for binding to `PostgreSQL` INTEGER columns.
/// Token counts are bounded well within i32 range (max ~25K tokens for 100K chars).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
pub(super) const fn token_count_as_i32(count: u32) -> i32 {
    count as i32
}

/// Compute hash of content for version tracking
pub(super) fn compute_content_hash(content: &serde_json::Value) -> String {
    let mut hasher = DefaultHasher::new();
    content.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Compute a content hash from a `CreateCoachRequest` using `DefaultHasher`.
///
/// Hashes the title, `system_prompt`, tags, and all structured section fields
/// to produce a deterministic 16-character hex string for deduplication.
pub(super) fn compute_request_hash(request: &CreateCoachRequest) -> String {
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

/// Convert a `PostgreSQL` row to a `Coach` struct
///
/// Reads PG-native types directly: `UUID` for `user_id`/`tenant_id`, `BOOLEAN`
/// for `is_system`, `TIMESTAMPTZ` for `created_at`/`updated_at`. Coach id is
/// stored as TEXT in PG but the model uses `Uuid`, so it is read as String and
/// parsed; `tenant_id` is the reverse — a UUID column feeding a String field.
pub(super) fn row_to_coach_pg(row: &PgRow) -> AppResult<Coach> {
    let id_str: String = row.get("id");
    let user_id: Uuid = row.get("user_id");
    let tenant_id: Uuid = row.get("tenant_id");
    let category_str: String = row.get("category");
    let tags_json: Option<String> = row.get("tags");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");
    let token_count: i32 = row.get("token_count");

    let is_system: bool = row.try_get("is_system").unwrap_or(false);
    let visibility_str: String = row
        .try_get("visibility")
        .unwrap_or_else(|_| "private".to_owned());
    let sample_prompts_json: Option<String> = row.try_get("sample_prompts").ok().flatten();
    let prerequisites_json: Option<String> = row.try_get("prerequisites").ok().flatten();
    let forked_from: Option<Uuid> = row
        .try_get::<Option<String>, _>("forked_from")
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok());
    let max_tool_iterations: Option<i32> = row.try_get("max_tool_iterations").ok().flatten();
    let temperature: Option<f32> = row.try_get("temperature").ok().flatten();
    let startup_query: Option<String> = row.try_get("startup_query").ok().flatten();
    let data_requirements_json: Option<String> = row.try_get("data_requirements").ok().flatten();
    let data_requirements: Option<DataRequirements> =
        data_requirements_json.and_then(|json| serde_json::from_str(&json).ok());
    let output_schema: Option<String> = row.try_get("output_schema").ok().flatten();

    // Structured sections (nullable columns populated by seeder or structured API)
    let purpose: Option<String> = row.try_get("purpose").ok().flatten();
    let when_to_use: Option<String> = row.try_get("when_to_use").ok().flatten();
    let instructions: Option<String> = row.try_get("instructions").ok().flatten();
    let example_inputs: Option<String> = row.try_get("example_inputs").ok().flatten();
    let example_outputs: Option<String> = row.try_get("example_outputs").ok().flatten();
    let success_criteria: Option<String> = row.try_get("success_criteria").ok().flatten();
    let source: String = row
        .try_get("source")
        .unwrap_or_else(|_| "custom".to_owned());
    let handle: Option<String> = row.try_get("slug").ok().flatten();

    let tags: Vec<String> = tags_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let sample_prompts: Vec<String> = sample_prompts_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let prerequisites: CoachPrerequisites = prerequisites_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();

    #[allow(clippy::cast_sign_loss)]
    Ok(Coach {
        id: Uuid::parse_str(&id_str)
            .map_err(|e| AppError::internal(format!("Invalid UUID: {e}")))?,
        user_id,
        tenant_id: tenant_id.to_string(),
        title: row.get("title"),
        description: row.get("description"),
        system_prompt: row.get("system_prompt"),
        category: CoachCategory::parse(&category_str),
        tags,
        sample_prompts,
        token_count: token_count as u32,
        created_at,
        updated_at,
        is_system,
        visibility: CoachVisibility::parse(&visibility_str),
        prerequisites,
        forked_from,
        max_tool_iterations,
        temperature,
        startup_query,
        data_requirements,
        output_schema,
        purpose,
        when_to_use,
        instructions,
        example_inputs,
        example_outputs,
        success_criteria,
        source,
        handle,
    })
}

/// Convert a `PostgreSQL` row to a `CoachListItem` (with preference fields from `coach_assignments`)
///
/// Assignment fields (`is_assigned`, `is_favorite`, `is_active`) are read as booleans directly
/// from CASE WHEN / COALESCE expressions that return BOOLEAN in the PG query.
pub(super) fn row_to_coach_list_item_pg(row: &PgRow) -> AppResult<CoachListItem> {
    let coach = row_to_coach_pg(row)?;
    let is_assigned: bool = row.try_get("is_assigned").unwrap_or(false);
    let is_favorite: bool = row.try_get("is_favorite").unwrap_or(false);
    let is_active: bool = row.try_get("is_active").unwrap_or(false);
    let use_count: i32 = row.try_get("use_count").unwrap_or(0);
    let last_used_at: Option<DateTime<Utc>> = row.try_get("last_used_at").ok().flatten();

    #[allow(clippy::cast_sign_loss)]
    Ok(CoachListItem {
        coach,
        is_assigned,
        is_favorite,
        is_active,
        use_count: use_count as u32,
        last_used_at,
    })
}

/// Convert a `PostgreSQL` row to a `CoachVersion` struct
pub(super) fn row_to_coach_version_pg(row: &PgRow) -> AppResult<CoachVersion> {
    let id: String = row.get("id");
    let coach_id: String = row.get("coach_id");
    let version: i32 = row.get("version");
    let content_hash: String = row.get("content_hash");
    let content_snapshot_str: String = row.get("content_snapshot");
    let change_summary: Option<String> = row.get("change_summary");
    let created_at: DateTime<Utc> = row.get("created_at");
    let created_by: Option<Uuid> = row.get("created_by");

    let content_snapshot: serde_json::Value = serde_json::from_str(&content_snapshot_str)
        .map_err(|e| AppError::internal(format!("Invalid JSON in version snapshot: {e}")))?;

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
