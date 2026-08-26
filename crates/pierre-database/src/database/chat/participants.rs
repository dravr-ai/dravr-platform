// ABOUTME: Participant operations for chat conversations — add, remove, list membership rows
// ABOUTME: The ChatManager methods plus the Database impl hops the ChatRepository delegations call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConversationParticipant, ParticipantRole, TenantId};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::super::Database;
use super::ChatManager;

impl ChatManager {
    /// Add a member to a conversation, idempotently. The conversation must
    /// exist in this tenant.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` when the conversation is not in this tenant, or an
    /// error if the database operation fails.
    pub async fn add_participant(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
        user_id: &str,
        added_by: &str,
    ) -> AppResult<ConversationParticipant> {
        let now = chrono::Utc::now().to_rfc3339();

        // INSERT OR IGNORE keeps an existing row (the owner's included)
        // untouched; the WHERE EXISTS gate refuses a conversation outside
        // this tenant instead of writing a dangling membership.
        sqlx::query(
            r"
            INSERT OR IGNORE INTO conversation_participants
                (conversation_id, user_id, tenant_id, role, added_by, added_at)
            SELECT $1, $2, $3, $4, $5, $6
            WHERE EXISTS (
                SELECT 1 FROM chat_conversations WHERE id = $1 AND tenant_id = $3
            )
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(ParticipantRole::Member.as_str())
        .bind(added_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to add participant: {e}")))?;

        let row = sqlx::query(
            r"
            SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
            FROM conversation_participants
            WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read participant: {e}")))?;

        row.as_ref()
            .map(map_participant_row)
            .transpose()?
            .ok_or_else(|| AppError::not_found("Conversation not found"))
    }

    /// Remove a member from a conversation. The owner's row is never removed.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn remove_participant(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM conversation_participants
            WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3 AND role = $4
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(ParticipantRole::Member.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to remove participant: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Every participant of a conversation, owner first.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_participants(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ConversationParticipant>> {
        let rows = sqlx::query(
            r"
            SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
            FROM conversation_participants
            WHERE conversation_id = $1 AND tenant_id = $2
            ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, added_at ASC, user_id ASC
            ",
        )
        .bind(conversation_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list participants: {e}")))?;

        rows.iter().map(map_participant_row).collect()
    }
}

/// Map a `conversation_participants` row to its record. The role column is
/// CHECK-constrained to the two known values, so an unknown one is a schema
/// breach surfaced as a database error rather than silently defaulted.
fn map_participant_row(r: &SqliteRow) -> AppResult<ConversationParticipant> {
    let role: String = r.get("role");
    let role = ParticipantRole::from_column(&role).ok_or_else(|| {
        AppError::database(format!(
            "conversation_participants.role holds unknown value {role}"
        ))
    })?;
    Ok(ConversationParticipant {
        conversation_id: r.get("conversation_id"),
        user_id: r.get("user_id"),
        tenant_id: r.get("tenant_id"),
        role,
        added_by: r.get("added_by"),
        added_at: r.get("added_at"),
    })
}

impl Database {
    /// Add a participant to a conversation (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database write fails or the conversation is not in the tenant.
    pub async fn chat_add_participant_impl(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
        user_id: &str,
        added_by: &str,
    ) -> AppResult<ConversationParticipant> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .add_participant(conversation_id, tenant_id, user_id, added_by)
            .await
    }

    /// Remove a participant from a conversation (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database write fails.
    pub async fn chat_remove_participant_impl(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .remove_participant(conversation_id, tenant_id, user_id)
            .await
    }

    /// List a conversation's participants (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub async fn chat_list_participants_impl(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ConversationParticipant>> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .list_participants(conversation_id, tenant_id)
            .await
    }
}
