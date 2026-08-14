// ABOUTME: PostgreSQL chat repository implementation
// ABOUTME: Manages conversation records, messages, and chat history for user interactions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::ChatRepository;
use super::PostgresDatabase;
use crate::database::{
    ConversationRecord, ConversationSummary, MessageFeedbackRecord, MessageRecord,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AddMessageParams, TenantId, UpsertMessageFeedbackParams};
use pierre_core::uuid_utils::parse_uuid;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

/// Map a `chat_message_feedback` row to its record. `user_id` is a UUID column
/// (matching `chat_conversations`), surfaced as a string for the wire DTO;
/// timestamps are `TIMESTAMPTZ`, rendered to RFC 3339 like the other chat reads.
fn map_feedback_row(r: &PgRow) -> MessageFeedbackRecord {
    let user_id: Uuid = r.get("user_id");
    let created_at: DateTime<Utc> = r.get("created_at");
    let updated_at: DateTime<Utc> = r.get("updated_at");
    MessageFeedbackRecord {
        id: r.get("id"),
        message_id: r.get("message_id"),
        conversation_id: r.get("conversation_id"),
        user_id: user_id.to_string(),
        tenant_id: r.get("tenant_id"),
        rating: r.get("rating"),
        comment: r.get("comment"),
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    }
}

#[async_trait]
impl ChatRepository for PostgresDatabase {
    // ================================
    // Chat Conversations & Messages (PostgreSQL implementation)
    // ================================

    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        coach_id: Option<&str>,
        group_id: Option<&str>,
    ) -> AppResult<ConversationRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        // chat_conversations.group_id is UUID in Postgres (see migration
        // 20260326000001_fix_coaching_groups_uuid_types). Parse to Uuid before
        // binding so the type matches.
        let group_uuid: Option<Uuid> = group_id.map(parse_uuid).transpose()?;

        sqlx::query(
            r"
            INSERT INTO chat_conversations (id, user_id, tenant_id, title, model, coach_id, group_id, total_tokens, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, $8)
            ",
        )
        .bind(&id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .bind(title)
        .bind(model)
        .bind(coach_id)
        .bind(group_uuid)
        .bind(now)
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
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            group_id: group_uuid.map(|u| u.to_string()),
            onboarding_state: None,
        })
    }

    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, tenant_id, title, model, coach_id, session_id,
                   total_tokens, created_at, updated_at, group_id::TEXT AS group_id,
                   onboarding_state
            FROM chat_conversations
            WHERE id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get conversation: {e}")))?;

        Ok(row.map(|r| {
            let created_at: DateTime<Utc> = r.get("created_at");
            let updated_at: DateTime<Utc> = r.get("updated_at");
            let user_id_uuid: Uuid = r.get("user_id");

            ConversationRecord {
                id: r.get("id"),
                user_id: user_id_uuid.to_string(),
                tenant_id: r.get("tenant_id"),
                title: r.get("title"),
                model: r.get("model"),
                coach_id: r.get("coach_id"),
                session_id: r.get("session_id"),
                total_tokens: r.get("total_tokens"),
                created_at: created_at.to_rfc3339(),
                updated_at: updated_at.to_rfc3339(),
                group_id: r.get("group_id"),
                onboarding_state: r.get("onboarding_state"),
            }
        }))
    }

    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>> {
        let rows = sqlx::query(
            r"
            SELECT c.id, c.title, c.model, c.total_tokens, c.coach_id, c.channel_type, c.created_at, c.updated_at,
                   COUNT(m.id) as message_count
            FROM chat_conversations c
            LEFT JOIN chat_messages m ON m.conversation_id = c.id
            WHERE c.user_id = $1 AND c.tenant_id = $2
            GROUP BY c.id
            ORDER BY c.updated_at DESC
            LIMIT $3 OFFSET $4
            ",
        )
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list conversations: {e}")))?;

        let summaries = rows
            .into_iter()
            .map(|r| {
                let created_at: DateTime<Utc> = r.get("created_at");
                let updated_at: DateTime<Utc> = r.get("updated_at");

                ConversationSummary {
                    id: r.get("id"),
                    title: r.get("title"),
                    model: r.get("model"),
                    message_count: r.get("message_count"),
                    total_tokens: r.get("total_tokens"),
                    coach_id: r.get("coach_id"),
                    channel_type: r.get("channel_type"),
                    created_at: created_at.to_rfc3339(),
                    updated_at: updated_at.to_rfc3339(),
                }
            })
            .collect();

        Ok(summaries)
    }

    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool> {
        let now = Utc::now();

        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET title = $1, updated_at = $2
            WHERE id = $3 AND user_id = $4 AND tenant_id = $5
            ",
        )
        .bind(title)
        .bind(now)
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update conversation title: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn set_conversation_channel(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET channel_type = $1
            WHERE id = $2 AND user_id = $3 AND tenant_id = $4
            ",
        )
        .bind(channel_type)
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set conversation channel: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_conversation(
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
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete conversation: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let user_uuid = parse_uuid(params.user_id)?;
        let tenant_str = params.tenant_id.to_string();

        // Insert message only if the conversation belongs to the user in this tenant
        let result = sqlx::query(
            r"
            INSERT INTO chat_messages (id, conversation_id, role, content, token_count, finish_reason, created_at, prompt_tokens, model, structured_content, content_blocks)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $12, $13
            WHERE EXISTS (
                SELECT 1 FROM chat_conversations WHERE id = $2 AND user_id = $10 AND tenant_id = $11
            )
            ",
        )
        .bind(&id)
        .bind(params.conversation_id)
        .bind(params.role)
        .bind(params.content)
        .bind(params.token_count.map(i64::from))
        .bind(params.finish_reason)
        .bind(now)
        .bind(params.prompt_tokens.map(i64::from))
        .bind(params.model)
        .bind(user_uuid)
        .bind(&tenant_str)
        .bind(params.structured_content)
        .bind(params.content_blocks)
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
            .bind(now)
            .bind(i64::from(tokens))
            .bind(params.conversation_id)
            .bind(user_uuid)
            .bind(&tenant_str)
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
            .bind(now)
            .bind(params.conversation_id)
            .bind(user_uuid)
            .bind(&tenant_str)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to update conversation timestamp: {e}"))
            })?;
        }

        Ok(MessageRecord {
            id,
            conversation_id: params.conversation_id.to_owned(),
            role: params.role.to_owned(),
            content: params.content.to_owned(),
            token_count: params.token_count.map(i64::from),
            prompt_tokens: params.prompt_tokens.map(i64::from),
            model: params.model.map(ToOwned::to_owned),
            finish_reason: params.finish_reason.map(ToOwned::to_owned),
            structured_content: params.structured_content.map(ToOwned::to_owned),
            content_blocks: params.content_blocks.map(ToOwned::to_owned),
            created_at: now.to_rfc3339(),
        })
    }

    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r"
            SELECT m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, m.model, m.finish_reason, m.structured_content, m.content_blocks, m.created_at
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2 AND c.tenant_id = $3
            ORDER BY m.created_at ASC
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get messages: {e}")))?;

        let messages = rows
            .into_iter()
            .map(|r| {
                let created_at: DateTime<Utc> = r.get("created_at");

                MessageRecord {
                    id: r.get("id"),
                    conversation_id: r.get("conversation_id"),
                    role: r.get("role"),
                    content: r.get("content"),
                    token_count: r.get("token_count"),
                    prompt_tokens: r.get("prompt_tokens"),
                    model: r.get("model"),
                    finish_reason: r.get("finish_reason"),
                    structured_content: r.get("structured_content"),
                    content_blocks: r.get("content_blocks"),
                    created_at: created_at.to_rfc3339(),
                }
            })
            .collect();

        Ok(messages)
    }

    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r"
            SELECT m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, m.model, m.finish_reason, m.structured_content, m.content_blocks, m.created_at
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2 AND c.tenant_id = $3
            ORDER BY m.created_at DESC, m.id DESC
            LIMIT $4
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get recent messages: {e}")))?;

        // Reverse to get chronological order
        let mut messages: Vec<MessageRecord> = rows
            .into_iter()
            .map(|r| {
                let created_at: DateTime<Utc> = r.get("created_at");

                MessageRecord {
                    id: r.get("id"),
                    conversation_id: r.get("conversation_id"),
                    role: r.get("role"),
                    content: r.get("content"),
                    token_count: r.get("token_count"),
                    prompt_tokens: r.get("prompt_tokens"),
                    model: r.get("model"),
                    finish_reason: r.get("finish_reason"),
                    structured_content: r.get("structured_content"),
                    content_blocks: r.get("content_blocks"),
                    created_at: created_at.to_rfc3339(),
                }
            })
            .collect();
        messages.reverse();

        Ok(messages)
    }

    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*)
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2 AND c.tenant_id = $3
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get message count: {e}")))?;

        Ok(count)
    }

    async fn upsert_message_feedback(
        &self,
        params: &UpsertMessageFeedbackParams<'_>,
    ) -> AppResult<MessageFeedbackRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let user_uuid = parse_uuid(params.user_id)?;
        let tenant_str = params.tenant_id.to_string();

        // Insert keyed on (message_id, user_id); on a repeat rating, overwrite
        // the rating + comment and bump updated_at. The WHERE EXISTS gate lands
        // the row only when the message belongs to a conversation the caller
        // owns in this tenant.
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
            ON CONFLICT (message_id, user_id) DO UPDATE SET
                rating = EXCLUDED.rating,
                comment = EXCLUDED.comment,
                updated_at = EXCLUDED.updated_at
            ",
        )
        .bind(&id)
        .bind(params.message_id)
        .bind(params.conversation_id)
        .bind(user_uuid)
        .bind(&tenant_str)
        .bind(params.rating)
        .bind(params.comment)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to upsert message feedback: {e}")))?;

        // Read back the canonical row: on conflict the stored id/created_at stay
        // the original values, not the ones generated above.
        let row = sqlx::query(
            r"
            SELECT id, message_id, conversation_id, user_id, tenant_id, rating, comment, created_at, updated_at
            FROM chat_message_feedback
            WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3
            ",
        )
        .bind(params.message_id)
        .bind(user_uuid)
        .bind(&tenant_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to read back message feedback: {e}")))?;

        row.as_ref()
            .map(map_feedback_row)
            .ok_or_else(|| AppError::not_found("Message not found or access denied"))
    }

    async fn delete_message_feedback(
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
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete message feedback: {e}")))?;

        Ok(result.rows_affected() > 0)
    }

    async fn get_conversation_feedback(
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
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get conversation feedback: {e}")))?;

        Ok(rows.iter().map(map_feedback_row).collect())
    }

    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*)
            FROM chat_conversations
            WHERE user_id = $1 AND tenant_id = $2
            ",
        )
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count conversations: {e}")))?;

        Ok(count)
    }

    async fn delete_all_user_conversations(
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
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to delete user conversations: {e}")))?;

        #[allow(clippy::cast_possible_wrap)]
        Ok(result.rows_affected() as i64)
    }

    async fn get_recent_conversations_admin(
        &self,
        limit: i64,
    ) -> AppResult<Vec<ConversationRecord>> {
        let rows = sqlx::query(
            "SELECT id::TEXT, user_id::TEXT, tenant_id::TEXT, title, model, coach_id, session_id, \
                    total_tokens, TO_CHAR(created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as created_at, \
                    TO_CHAR(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as updated_at, \
                    group_id::TEXT, onboarding_state \
             FROM chat_conversations \
             ORDER BY updated_at DESC \
             LIMIT $1",
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
                onboarding_state: row.get("onboarding_state"),
            })
            .collect())
    }

    async fn count_active_conversations_since(&self, since: &str) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*) FROM chat_conversations WHERE updated_at >= $1::timestamptz
            ",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to count active conversations: {e}")))?;

        Ok(count)
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
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set conversation session_id: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn set_conversation_onboarding_state(
        &self,
        conversation_id: &str,
        onboarding_state: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET onboarding_state = $1
            WHERE id = $2 AND tenant_id = $3
            ",
        )
        .bind(onboarding_state)
        .bind(conversation_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to set conversation onboarding_state: {e}"))
        })?;
        Ok(result.rows_affected() > 0)
    }

    async fn compare_and_set_conversation_onboarding_state(
        &self,
        conversation_id: &str,
        expected: Option<&str>,
        onboarding_state: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        // `IS NOT DISTINCT FROM` is Postgres' NULL-safe equality: a conversation
        // that carried no state matches only an absent `expected`, never any
        // stored JSON. The cast pins the parameter type, which the planner
        // cannot infer from a NULL-safe comparison alone.
        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET onboarding_state = $1
            WHERE id = $2 AND tenant_id = $3
              AND onboarding_state IS NOT DISTINCT FROM $4::TEXT
            ",
        )
        .bind(onboarding_state)
        .bind(conversation_id)
        .bind(tenant_id.to_string())
        .bind(expected)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            AppError::database(format!(
                "Failed to compare-and-set conversation onboarding_state: {e}"
            ))
        })?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_user_onboarding_states(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            r"
            SELECT onboarding_state
            FROM chat_conversations
            WHERE user_id = $1 AND tenant_id = $2 AND onboarding_state IS NOT NULL
            ORDER BY updated_at DESC
            LIMIT $3
            ",
        )
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list user onboarding states: {e}")))?;
        Ok(rows
            .iter()
            .map(|r| r.get::<String, _>("onboarding_state"))
            .collect())
    }

    async fn set_conversation_group_id(
        &self,
        conversation_id: &str,
        group_id: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool> {
        // chat_conversations.group_id is UUID in Postgres — parse the str.
        let group_uuid: Option<Uuid> = group_id.map(parse_uuid).transpose()?;
        let result = sqlx::query(
            r"
            UPDATE chat_conversations
            SET group_id = $1
            WHERE id = $2 AND tenant_id = $3
            ",
        )
        .bind(group_uuid)
        .bind(conversation_id)
        .bind(tenant_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to set conversation group_id: {e}")))?;
        Ok(result.rows_affected() > 0)
    }
}
