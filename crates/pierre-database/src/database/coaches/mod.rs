// ABOUTME: Coach row mappers and hash helpers shared by the SQLite coach repositories
// ABOUTME: Re-exports the coach type definitions so `database::coaches::*` stays one import path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Type definitions for coach database operations
mod types;

pub use types::*;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::coaches::{CoachPrerequisites, DataRequirements};
use sqlx::{sqlite::SqliteRow, Row};
use uuid::Uuid;

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
    let forked_from: Option<Uuid> = row
        .try_get::<Option<String>, _>("forked_from")
        .ok()
        .flatten()
        .and_then(|s| Uuid::parse_str(&s).ok());

    let tags: Vec<String> = serde_json::from_str(&tags_json)?;
    let sample_prompts: Vec<String> = serde_json::from_str(&sample_prompts_json)?;
    let prerequisites: CoachPrerequisites =
        serde_json::from_str(&prerequisites_json).unwrap_or_default();

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

/// Convert a database row to a `CoachVersion` struct
///
/// # Errors
///
/// Returns an error if row fields cannot be parsed (invalid UUID, datetime, or JSON)
pub fn row_to_coach_version(row: &SqliteRow) -> AppResult<CoachVersion> {
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
