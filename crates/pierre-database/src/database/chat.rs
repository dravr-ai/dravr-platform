// ABOUTME: Database operations for AI chat conversations and messages
// ABOUTME: Handles CRUD operations with multi-tenant isolation and conversation history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::repositories::ChatRepository;
use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::Database;
use pierre_core::models::TenantId;

// Re-export DTOs from pierre-core (canonical definitions)
pub use pierre_core::models::{
    AddMessageParams, ConversationRecord, ConversationSummary, MessageFeedbackRecord,
    MessageRecord, UpsertMessageFeedbackParams,
};

// ============================================================================
// Chat Manager
// ============================================================================

/// Chat database operations manager
pub struct ChatManager {
    pool: SqlitePool,
}

impl ChatManager {
    /// Create a new chat manager
    #[must_use]
    pub const fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ========================================================================
    // Conversation Operations
    // ========================================================================

    /// Create a new conversation
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        coach_id: Option<&str>,
        group_id: Option<&str>,
    ) -> AppResult<ConversationRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        sqlx::query(
            r"
            INSERT INTO chat_conversations (id, user_id, tenant_id, title, model, coach_id, group_id, total_tokens, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, $8)
            ",
        )
        .bind(&id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(title)
        .bind(model)
        .bind(coach_id)
        .bind(group_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create conversation: {e}")))?;

        Ok(ConversationRecord {
            id,
            user_id: user_id.to_owned(),
            tenant_id: tenant_id.to_string(),
            title: title.to_owned(),
            model: model.to_owned(),
            coach_id: coach_id.map(ToOwned::to_owned),
            session_id: None,
            total_tokens: 0,
            created_at: now.clone(),
            updated_at: now,
            group_id: group_id.map(ToOwned::to_owned),
        })
    }

    /// Get a conversation by ID with tenant isolation
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, model, coach_id, session_id, total_tokens, created_at, updated_at, group_id
            FROM chat_conversations
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get conversation: {e}")))?;

        Ok(row.map(|r| ConversationRecord {
            id: r.get("id"),
            user_id: r.get("user_id"),
            tenant_id: r.get("tenant_id"),
            title: r.get("title"),
            model: r.get("model"),
            coach_id: r.get("coach_id"),
            session_id: r.get("session_id"),
            total_tokens: r.get("total_tokens"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
            group_id: r.get("group_id"),
        }))
    }

    /// List conversations for a user with pagination
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>> {
        let rows = sqlx::query(
            r"
            SELECT c.id, c.title, c.model, c.total_tokens, c.coach_id, c.created_at, c.updated_at,
                   COUNT(m.id) as message_count
            FROM chat_conversations c
            LEFT JOIN chat_messages m ON m.conversation_id = c.id
            WHERE c.user_id = $1 AND c.tenant_id = $2
            GROUP BY c.id
            ORDER BY c.updated_at DESC
            LIMIT $3 OFFSET $4
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list conversations: {e}")))?;

        let summaries = rows
            .into_iter()
            .map(|r| ConversationSummary {
                id: r.get("id"),
                title: r.get("title"),
                model: r.get("model"),
                message_count: r.get("message_count"),
                total_tokens: r.get("total_tokens"),
                coach_id: r.get("coach_id"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect();

        Ok(summaries)
    }

    /// Update conversation title
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool> {
        let now = chrono::Utc::now().to_rfc3339();

        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET title = $1, updated_at = $2
            WHERE id = $3 AND user_id = $4 AND tenant_id = $5
            ",
        )
        .bind(title)
        .bind(&now)
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update conversation title: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a conversation and all its messages (cascade)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM chat_conversations
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete conversation: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    // ========================================================================
    // Message Operations
    // ========================================================================

    /// Add a message to a conversation
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let role_str = params.role;

        // Insert message only if the conversation belongs to the user in this tenant
        let result = sqlx::query(
            r"
            INSERT INTO chat_messages (id, conversation_id, role, content, token_count, finish_reason, created_at, prompt_tokens, model, structured_content)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $12
            WHERE EXISTS (
                SELECT 1 FROM chat_conversations WHERE id = $2 AND user_id = $10 AND tenant_id = $11
            )
            ",
        )
        .bind(&id)
        .bind(params.conversation_id)
        .bind(role_str)
        .bind(params.content)
        .bind(params.token_count.map(i64::from))
        .bind(params.finish_reason)
        .bind(&now)
        .bind(params.prompt_tokens.map(i64::from))
        .bind(params.model)
        .bind(params.user_id)
        .bind(params.tenant_id)
        .bind(params.structured_content)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to add message: {e}")))?;

        if result.rows_affected() == 0 {
            return Err(AppError::not_found(
                "Conversation not found or access denied",
            ));
        }

        // Update conversation's updated_at and total_tokens (ownership already verified above)
        if let Some(tokens) = params.token_count {
            sqlx::query(
                r"
                UPDATE chat_conversations
                SET updated_at = $1, total_tokens = total_tokens + $2
                WHERE id = $3 AND user_id = $4 AND tenant_id = $5
                ",
            )
            .bind(&now)
            .bind(i64::from(tokens))
            .bind(params.conversation_id)
            .bind(params.user_id)
            .bind(params.tenant_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to update conversation tokens: {e}"))
            })?;
        } else {
            sqlx::query(
                r"
                UPDATE chat_conversations
                SET updated_at = $1
                WHERE id = $2 AND user_id = $3 AND tenant_id = $4
                ",
            )
            .bind(&now)
            .bind(params.conversation_id)
            .bind(params.user_id)
            .bind(params.tenant_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to update conversation timestamp: {e}"))
            })?;
        }

        Ok(MessageRecord {
            id,
            conversation_id: params.conversation_id.to_owned(),
            role: role_str.to_owned(),
            content: params.content.to_owned(),
            token_count: params.token_count.map(i64::from),
            prompt_tokens: params.prompt_tokens.map(i64::from),
            model: params.model.map(ToOwned::to_owned),
            finish_reason: params.finish_reason.map(ToOwned::to_owned),
            structured_content: params.structured_content.map(ToOwned::to_owned),
            created_at: now,
        })
    }

    /// Get all messages for a conversation in chronological order
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r"
            SELECT m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, m.model, m.finish_reason, m.structured_content, m.created_at
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2 AND c.tenant_id = $3
            ORDER BY m.created_at ASC
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get messages: {e}")))?;

        let messages = rows
            .into_iter()
            .map(|r| MessageRecord {
                id: r.get("id"),
                conversation_id: r.get("conversation_id"),
                role: r.get("role"),
                content: r.get("content"),
                token_count: r.get("token_count"),
                prompt_tokens: r.get("prompt_tokens"),
                model: r.get("model"),
                finish_reason: r.get("finish_reason"),
                structured_content: r.get("structured_content"),
                created_at: r.get("created_at"),
            })
            .collect();

        Ok(messages)
    }

    /// Get the last N messages for a conversation (for context window)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r"
            SELECT m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, m.model, m.finish_reason, m.structured_content, m.created_at
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2 AND c.tenant_id = $3
            ORDER BY m.created_at DESC
            LIMIT $4
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get recent messages: {e}")))?;

        // Reverse to get chronological order
        let mut messages: Vec<MessageRecord> = rows
            .into_iter()
            .map(|r| MessageRecord {
                id: r.get("id"),
                conversation_id: r.get("conversation_id"),
                role: r.get("role"),
                content: r.get("content"),
                token_count: r.get("token_count"),
                prompt_tokens: r.get("prompt_tokens"),
                model: r.get("model"),
                finish_reason: r.get("finish_reason"),
                structured_content: r.get("structured_content"),
                created_at: r.get("created_at"),
            })
            .collect();
        messages.reverse();

        Ok(messages)
    }

    /// Get message count for a conversation
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2 AND c.tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get message count: {e}")))?;

        Ok(row.get("count"))
    }

    // ========================================================================
    // Feedback Operations
    // ========================================================================

    /// Upsert the caller's thumbs up/down feedback on a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operation fails, or `NotFound` if the
    /// message does not belong to a conversation the caller owns in this tenant.
    pub async fn upsert_message_feedback(
        &self,
        params: &UpsertMessageFeedbackParams<'_>,
    ) -> AppResult<MessageFeedbackRecord> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // Insert keyed on (message_id, user_id); on a repeat rating, overwrite
        // the rating + comment and bump updated_at. The WHERE EXISTS gate lands
        // the row only when the message belongs to a conversation the caller
        // owns in this tenant — a forged message_id never inserts.
        sqlx::query(
            r"
            INSERT INTO chat_message_feedback
                (id, message_id, conversation_id, user_id, tenant_id, rating, comment, created_at, updated_at)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $8
            WHERE EXISTS (
                SELECT 1 FROM chat_messages m
                JOIN chat_conversations c ON m.conversation_id = c.id
                WHERE m.id = $2 AND m.conversation_id = $3 AND c.user_id = $4 AND c.tenant_id = $5
            )
            ON CONFLICT(message_id, user_id) DO UPDATE SET
                rating = excluded.rating,
                comment = excluded.comment,
                updated_at = excluded.updated_at
            ",
        )
        .bind(&id)
        .bind(params.message_id)
        .bind(params.conversation_id)
        .bind(params.user_id)
        .bind(params.tenant_id)
        .bind(params.rating)
        .bind(params.comment)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert message feedback: {e}")))?;

        // Read back the canonical row: on conflict the stored id/created_at stay
        // the original values, not the ones generated above.
        self.get_message_feedback(params.message_id, params.user_id, params.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Message not found or access denied"))
    }

    /// Fetch the caller's feedback on a single message, if any.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_message_feedback(
        &self,
        message_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<MessageFeedbackRecord>> {
        let row = sqlx::query(
            r"
            SELECT id, message_id, conversation_id, user_id, tenant_id, rating, comment, created_at, updated_at
            FROM chat_message_feedback
            WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(message_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get message feedback: {e}")))?;

        Ok(row.map(|r| MessageFeedbackRecord {
            id: r.get("id"),
            message_id: r.get("message_id"),
            conversation_id: r.get("conversation_id"),
            user_id: r.get("user_id"),
            tenant_id: r.get("tenant_id"),
            rating: r.get("rating"),
            comment: r.get("comment"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    /// Delete the caller's feedback on a message (thumbs toggle-off).
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn delete_message_feedback(
        &self,
        message_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            DELETE FROM chat_message_feedback
            WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(message_id)
        .bind(user_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete message feedback: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    /// Load all of the caller's feedback rows for a conversation.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn get_conversation_feedback(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageFeedbackRecord>> {
        let rows = sqlx::query(
            r"
            SELECT id, message_id, conversation_id, user_id, tenant_id, rating, comment, created_at, updated_at
            FROM chat_message_feedback
            WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get conversation feedback: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|r| MessageFeedbackRecord {
                id: r.get("id"),
                message_id: r.get("message_id"),
                conversation_id: r.get("conversation_id"),
                user_id: r.get("user_id"),
                tenant_id: r.get("tenant_id"),
                rating: r.get("rating"),
                comment: r.get("comment"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    /// Count total conversations for a user in a tenant
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64> {
        let row = sqlx::query(
            r"
            SELECT COUNT(*) as count
            FROM chat_conversations
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count conversations: {e}")))?;

        Ok(row.get("count"))
    }

    /// Delete all conversations for a user (for account cleanup)
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let result = sqlx::query(
            r"
            DELETE FROM chat_conversations
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(user_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete user conversations: {e}")))?;

        #[allow(clippy::cast_possible_wrap)]
        Ok(result.rows_affected() as i64)
    }
}

#[async_trait]
impl ChatRepository for Database {
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        coach_id: Option<&str>,
        group_id: Option<&str>,
    ) -> AppResult<ConversationRecord> {
        Self::chat_create_conversation_impl(
            self, user_id, tenant_id, title, model, coach_id, group_id,
        )
        .await
    }
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>> {
        Self::chat_get_conversation_impl(self, conversation_id, user_id, tenant_id).await
    }
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>> {
        Self::chat_list_conversations_impl(self, user_id, tenant_id, limit, offset).await
    }
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool> {
        Self::chat_update_conversation_title_impl(self, conversation_id, user_id, tenant_id, title)
            .await
    }
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        Self::chat_delete_conversation_impl(self, conversation_id, user_id, tenant_id).await
    }
    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord> {
        Self::chat_add_message_impl(self, params).await
    }
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageRecord>> {
        Self::chat_get_messages_impl(self, conversation_id, user_id, tenant_id).await
    }
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>> {
        Self::chat_get_recent_messages_impl(self, conversation_id, user_id, tenant_id, limit).await
    }
    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        Self::chat_get_message_count_impl(self, conversation_id, user_id, tenant_id).await
    }
    async fn upsert_message_feedback(
        &self,
        params: &UpsertMessageFeedbackParams<'_>,
    ) -> AppResult<MessageFeedbackRecord> {
        Self::chat_upsert_message_feedback_impl(self, params).await
    }
    async fn delete_message_feedback(
        &self,
        message_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        Self::chat_delete_message_feedback_impl(self, message_id, user_id, tenant_id).await
    }
    async fn get_conversation_feedback(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageFeedbackRecord>> {
        Self::chat_get_conversation_feedback_impl(self, conversation_id, user_id, tenant_id).await
    }
    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64> {
        Self::chat_count_conversations_impl(self, user_id, tenant_id).await
    }
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        Self::chat_delete_all_user_conversations_impl(self, user_id, tenant_id).await
    }

    async fn get_recent_conversations_admin(
        &self,
        limit: i64,
    ) -> AppResult<Vec<ConversationRecord>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, model, coach_id, session_id,
                   total_tokens, created_at, updated_at, group_id
            FROM chat_conversations
            ORDER BY updated_at DESC
            LIMIT $1
            ",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to query recent conversations: {e}")))?;

        Ok(rows
            .iter()
            .map(|row| ConversationRecord {
                id: row.get("id"),
                user_id: row.get("user_id"),
                tenant_id: row.get("tenant_id"),
                title: row.get("title"),
                model: row.get("model"),
                coach_id: row.get("coach_id"),
                session_id: row.get("session_id"),
                total_tokens: row.get("total_tokens"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                group_id: row.get("group_id"),
            })
            .collect())
    }

    async fn count_active_conversations_since(&self, since: &str) -> AppResult<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            r"
            SELECT COUNT(*) FROM chat_conversations WHERE updated_at >= $1
            ",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count active conversations: {e}")))?;

        Ok(row.0)
    }

    async fn set_conversation_session_id(
        &self,
        conversation_id: &str,
        session_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET session_id = $1
            WHERE id = $2 AND tenant_id = $3
            ",
        )
        .bind(session_id)
        .bind(conversation_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set conversation session_id: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_conversation_group_id(
        &self,
        conversation_id: &str,
        group_id: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET group_id = $1
            WHERE id = $2 AND tenant_id = $3
            ",
        )
        .bind(group_id)
        .bind(conversation_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set conversation group_id: {e}")))?;
        Ok(result.rows_affected() > 0)
    }
}
