// ABOUTME: Per-participant read markers on PostgreSQL — advance (monotonic) and clear (mark unread)
// ABOUTME: Free functions over the pool, mirroring database/chat/read_markers.rs with TIMESTAMPTZ
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::uuid_utils::parse_uuid;
use sqlx::{PgPool, Row};

/// The instant the marker should advance to, gated on membership — the same
/// question the `SQLite` statement asks. `$4::TEXT` pins the parameter's type
/// for the planner, which a bare `IS NULL` leaves open.
const TARGET_SQL: &str = r"
    SELECT (
        SELECT MAX(m.created_at) FROM chat_messages m
        WHERE m.conversation_id = p.conversation_id
          AND (($4::TEXT IS NULL AND m.role IN ('user', 'assistant')) OR m.id = $4::TEXT)
    ) AS target
    FROM conversation_participants p
    JOIN chat_conversations c ON c.id = p.conversation_id
    WHERE p.conversation_id = $1 AND p.user_id = $2
      AND p.tenant_id = c.tenant_id
      AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)
";

const ADVANCE_SQL: &str = r"
    UPDATE conversation_participants
    SET last_read_at = $4
    WHERE conversation_id = $1 AND user_id = $2
      AND EXISTS (
        SELECT 1 FROM chat_conversations c
        WHERE c.id = conversation_participants.conversation_id
          AND conversation_participants.tenant_id = c.tenant_id
          AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)
      )
      AND (last_read_at IS NULL OR last_read_at < $4)
";

const CLEAR_SQL: &str = r"
    UPDATE conversation_participants
    SET last_read_at = NULL
    WHERE conversation_id = $1 AND user_id = $2
      AND EXISTS (
        SELECT 1 FROM chat_conversations c
        WHERE c.id = conversation_participants.conversation_id
          AND conversation_participants.tenant_id = c.tenant_id
          AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)
      )
";

/// Advance the caller's read marker — see
/// `ChatRepository::mark_conversation_read` for the contract.
pub(super) async fn mark_conversation_read(
    pool: &PgPool,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    up_to_message_id: Option<&str>,
) -> AppResult<bool> {
    let user_uuid = parse_uuid(user_id)?;
    let tenant_str = tenant_id.to_string();
    let row = sqlx::query(TARGET_SQL)
        .bind(conversation_id)
        .bind(user_uuid)
        .bind(&tenant_str)
        .bind(up_to_message_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to resolve read marker: {e}")))?;

    // No row: not a participant. A row with no target: either a named message
    // that is not in this conversation (refused), or nothing written yet
    // (nothing to mark, and nothing wrong).
    let Some(row) = row else {
        return Ok(false);
    };
    let target: Option<DateTime<Utc>> = row.get("target");
    let Some(target) = target else {
        return Ok(up_to_message_id.is_none());
    };

    sqlx::query(ADVANCE_SQL)
        .bind(conversation_id)
        .bind(user_uuid)
        .bind(&tenant_str)
        .bind(target)
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to advance read marker: {e}")))?;
    Ok(true)
}

/// Clear the caller's read marker (mark unread).
pub(super) async fn clear_conversation_read_marker(
    pool: &PgPool,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<bool> {
    let result = sqlx::query(CLEAR_SQL)
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to clear read marker: {e}")))?;
    Ok(result.rows_affected() > 0)
}
