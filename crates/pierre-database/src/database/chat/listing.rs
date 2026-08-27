// ABOUTME: The unified conversation list on SQLite — one page of every thread a participant is in
// ABOUTME: Row facts (coach, group, newest message, unread count) come from the same statement
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConversationLastMessage, ConversationPage, ConversationSummary, TenantId,
};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::super::Database;
use super::ChatManager;

/// How many leading characters of the newest row travel with a list row.
///
/// The route shapes the preview (marker strip, whitespace collapse, 120
/// characters); this bound keeps a thread whose last reply is a full training
/// plan from shipping the whole plan to draw one line.
const CONTENT_HEAD_CHARS: i64 = 512;

/// One page of a participant's conversations plus their total.
///
/// The statement asks the row-level questions inline — count of turns, count
/// of unread turns, and the newest turn — as correlated scalar subqueries over
/// `chat_messages`, each served by the `(conversation_id, created_at)` index.
/// A `GROUP BY` over a `LEFT JOIN chat_messages` counted tool rows and could
/// not say which row was newest; three scoped subqueries can.
///
/// `p` is the caller's own participant row, which is where their read marker
/// lives: "unread" is a question about one participant, so it is answered
/// from that row and never from the conversation.
const PAGE_SQL: &str = r"
    SELECT c.id, c.title, c.model, c.total_tokens, c.coach_id, c.channel_type,
           c.created_at, c.updated_at, c.group_id,
           g.name AS group_name, co.slug AS coach_handle, co.title AS coach_title,
           (SELECT COUNT(*) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')) AS message_count,
           (SELECT COUNT(*) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (p.last_read_at IS NULL OR m.created_at > p.last_read_at)) AS unread_count,
           (SELECT SUBSTR(m.content, 1, $5) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_content_head,
           (SELECT m.role FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_role,
           (SELECT m.created_at FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_created_at
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    LEFT JOIN coaching_groups g ON g.id = c.group_id
    LEFT JOIN coaches co ON co.id = c.coach_id
    WHERE p.user_id = $1 AND p.tenant_id = $2 AND c.tenant_id = $2
    ORDER BY c.updated_at DESC, c.id DESC
    LIMIT $3 OFFSET $4
";

/// The participant's total — the same membership predicate as the page.
const TOTAL_SQL: &str = r"
    SELECT COUNT(*) AS count
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    WHERE p.user_id = $1 AND p.tenant_id = $2 AND c.tenant_id = $2
";

/// Map one page row. The three `last_*` columns are all `NULL` or all set —
/// they come from the same newest row — so the preview is keyed on the role.
fn map_summary_row(r: &SqliteRow) -> ConversationSummary {
    let last_role: Option<String> = r.get("last_role");
    let last_message = last_role.map(|role| ConversationLastMessage {
        content_head: r
            .get::<Option<String>, _>("last_content_head")
            .unwrap_or_default(),
        role,
        created_at: r
            .get::<Option<String>, _>("last_created_at")
            .unwrap_or_default(),
    });
    ConversationSummary {
        id: r.get("id"),
        title: r.get("title"),
        model: r.get("model"),
        message_count: r.get("message_count"),
        total_tokens: r.get("total_tokens"),
        coach_id: r.get("coach_id"),
        coach_handle: r.get("coach_handle"),
        coach_title: r.get("coach_title"),
        group_id: r.get("group_id"),
        group_name: r.get("group_name"),
        channel_type: r.get("channel_type"),
        last_message,
        unread_count: r.get("unread_count"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

impl ChatManager {
    /// One page of the conversations a user participates in, newest activity
    /// first, with the participant's total — see
    /// `ChatRepository::list_conversations` for the row contract.
    ///
    /// # Errors
    ///
    /// Returns an error if either statement fails
    pub async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<ConversationPage> {
        let rows = sqlx::query(PAGE_SQL)
            .bind(user_id)
            .bind(tenant_id)
            .bind(limit)
            .bind(offset)
            .bind(CONTENT_HEAD_CHARS)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list conversations: {e}")))?;

        let total = self
            .count_participating_conversations(user_id, tenant_id)
            .await?;

        Ok(ConversationPage {
            items: rows.iter().map(map_summary_row).collect(),
            total,
        })
    }

    /// Count every conversation a user participates in, in this tenant.
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn count_participating_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let row = sqlx::query(TOTAL_SQL)
            .bind(user_id)
            .bind(tenant_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to count participating conversations: {e}"))
            })?;
        Ok(row.get("count"))
    }
}

impl Database {
    /// List conversations (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub async fn chat_list_conversations_impl(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<ConversationPage> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .list_conversations(user_id, tenant_id, limit, offset)
            .await
    }

    /// Count the conversations a user participates in (impl for trait)
    ///
    /// # Errors
    /// Returns an error if the database query fails.
    pub async fn chat_count_participating_conversations_impl(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64> {
        let chat_manager = ChatManager::new(self.pool.clone());
        chat_manager
            .count_participating_conversations(user_id, tenant_id)
            .await
    }
}
