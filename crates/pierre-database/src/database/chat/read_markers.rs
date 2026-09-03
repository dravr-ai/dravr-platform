// ABOUTME: Per-participant read markers on SQLite — advance (monotonic) and clear (mark unread)
// ABOUTME: The marker is conversation_participants.last_read_at; unread counts are read against it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;

use super::super::Database;
use super::ChatManager;

/// The instant the marker should advance to, gated on membership.
///
/// Answers for a participant only — a stranger gets no row at all, which the
/// caller reports as `false`. `$4` is the message the athlete has seen; when
/// it is absent the newest `user`/`assistant` row is the target, and an empty
/// conversation yields a `NULL` target with the membership row still present.
const TARGET_SQL: &str = r"
    SELECT (
        SELECT MAX(m.created_at) FROM chat_messages m
        WHERE m.conversation_id = p.conversation_id
          AND (($4 IS NULL AND m.role IN ('user', 'assistant')) OR m.id = $4)
    ) AS target
    FROM conversation_participants p
    JOIN chat_conversations c ON c.id = p.conversation_id
    WHERE p.conversation_id = $1 AND p.user_id = $2
      AND p.tenant_id = c.tenant_id
      AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)
";

/// Advance the marker, never backwards: a stale client re-marking an older
/// row leaves a newer marker where it is.
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

impl ChatManager {
    /// Advance the caller's read marker — see
    /// `ChatRepository::mark_conversation_read` for the contract.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn mark_conversation_read(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        up_to_message_id: Option<&str>,
    ) -> AppResult<bool> {
        let row = sqlx::query(TARGET_SQL)
            .bind(conversation_id)
            .bind(user_id)
            .bind(tenant_id)
            .bind(up_to_message_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to resolve read marker: {e}")))?;

        // No row: not a participant. A row with no target: either a named
        // message that is not in this conversation (refused), or nothing
        // written yet (nothing to mark, and nothing wrong).
        let Some(row) = row else {
            return Ok(false);
        };
        let target: Option<String> = row.get("target");
        let Some(target) = target else {
            return Ok(up_to_message_id.is_none());
        };

        sqlx::query(ADVANCE_SQL)
            .bind(conversation_id)
            .bind(user_id)
            .bind(tenant_id)
            .bind(&target)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to advance read marker: {e}")))?;
        Ok(true)
    }

    /// Clear the caller's read marker (mark unread).
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn clear_conversation_read_marker(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(CLEAR_SQL)
            .bind(conversation_id)
            .bind(user_id)
            .bind(tenant_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to clear read marker: {e}")))?;
        Ok(result.rows_affected() > 0)
    }
}

impl Database {
    /// Advance a participant's read marker (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database write fails.
    pub async fn chat_mark_conversation_read_impl(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        up_to_message_id: Option<&str>,
    ) -> AppResult<bool> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .mark_conversation_read(conversation_id, user_id, tenant_id, up_to_message_id)
            .await
    }

    /// Clear a participant's read marker (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database write fails.
    pub async fn chat_clear_conversation_read_marker_impl(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .clear_conversation_read_marker(conversation_id, user_id, tenant_id)
            .await
    }
}
