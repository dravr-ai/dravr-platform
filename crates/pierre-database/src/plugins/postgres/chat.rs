// ABOUTME: PostgreSQL chat repository implementation
// ABOUTME: Manages conversation records, messages, and chat history for user interactions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::ChatRepository;
use super::PostgresDatabase;
use crate::database::{ConversationRecord, ConversationSummary, MessageRecord};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_core::uuid_utils::parse_uuid;
use sqlx::Row;
use uuid::Uuid;

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
        system_prompt: Option<&str>,
    ) -> AppResult<ConversationRecord> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        sqlx::query(
            r"
            INSERT INTO chat_conversations (id, user_id, tenant_id, title, model, system_prompt, total_tokens, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, 0, $7, $7)
            ",
        )
        .bind(&id)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .bind(title)
        .bind(model)
        .bind(system_prompt)
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
            system_prompt: system_prompt.map(ToOwned::to_owned),
            total_tokens: 0,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            group_id: None,
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
            SELECT id, user_id, tenant_id, title, model, system_prompt, total_tokens, created_at, updated_at, group_id
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
                system_prompt: r.get("system_prompt"),
                total_tokens: r.get("total_tokens"),
                created_at: created_at.to_rfc3339(),
                updated_at: updated_at.to_rfc3339(),
                group_id: r.get("group_id"),
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
            SELECT c.id, c.title, c.model, c.total_tokens, c.created_at, c.updated_at,
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

        // Insert message only if the conversation belongs to the user
        let result = sqlx::query(
            r"
            INSERT INTO chat_messages (id, conversation_id, role, content, token_count, finish_reason, created_at, prompt_tokens, model)
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
            WHERE EXISTS (
                SELECT 1 FROM chat_conversations WHERE id = $2 AND user_id = $10
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
                WHERE id = $3 AND user_id = $4
                ",
            )
            .bind(now)
            .bind(i64::from(tokens))
            .bind(params.conversation_id)
            .bind(user_uuid)
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
                WHERE id = $2 AND user_id = $3
                ",
            )
            .bind(now)
            .bind(params.conversation_id)
            .bind(user_uuid)
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
            created_at: now.to_rfc3339(),
        })
    }

    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> AppResult<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r"
            SELECT m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, m.model, m.finish_reason, m.created_at
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2
            ORDER BY m.created_at ASC
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
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
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r"
            SELECT m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, m.model, m.finish_reason, m.created_at
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2
            ORDER BY m.created_at DESC
            LIMIT $3
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
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
                    created_at: created_at.to_rfc3339(),
                }
            })
            .collect();
        messages.reverse();

        Ok(messages)
    }

    async fn get_message_count(&self, conversation_id: &str, user_id: &str) -> AppResult<i64> {
        let count: i64 = sqlx::query_scalar(
            r"
            SELECT COUNT(*)
            FROM chat_messages m
            JOIN chat_conversations c ON m.conversation_id = c.id
            WHERE m.conversation_id = $1 AND c.user_id = $2
            ",
        )
        .bind(conversation_id)
        .bind(parse_uuid(user_id)?)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get message count: {e}")))?;

        Ok(count)
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
            "SELECT id::TEXT, user_id::TEXT, tenant_id::TEXT, title, model, system_prompt, \
                    total_tokens, TO_CHAR(created_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as created_at, \
                    TO_CHAR(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') as updated_at, \
                    group_id::TEXT \
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
                system_prompt: row.get("system_prompt"),
                total_tokens: row.get("total_tokens"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                group_id: row.get("group_id"),
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
}
