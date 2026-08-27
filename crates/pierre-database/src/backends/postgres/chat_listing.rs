// ABOUTME: The unified conversation list on PostgreSQL — one page of every thread a participant is in
// ABOUTME: Free functions over the pool, mirroring database/chat/listing.rs with PG-native types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConversationLastMessage, ConversationPage, ConversationSummary, TenantId,
};
use pierre_core::uuid_utils::parse_uuid;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

/// How many leading characters of the newest row travel with a list row —
/// the same bound as the `SQLite` backend, so a preview reads identically.
const CONTENT_HEAD_CHARS: i32 = 512;

/// One page of a participant's conversations — the same shape as the `SQLite`
/// statement (see `database/chat/listing.rs`), with `group_id` cast to text
/// because the column is a UUID here and `coaching_groups.id` joins on it
/// natively.
const PAGE_SQL: &str = r"
    SELECT c.id, c.title, c.model, c.total_tokens, c.coach_id, c.channel_type,
           c.created_at, c.updated_at, c.group_id::TEXT AS group_id,
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
    SELECT COUNT(*)
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    WHERE p.user_id = $1 AND p.tenant_id = $2 AND c.tenant_id = $2
";

/// Map one page row; timestamps are `TIMESTAMPTZ`, rendered to RFC 3339 like
/// every other chat read on this backend.
fn map_summary_row(r: &PgRow) -> ConversationSummary {
    let created_at: DateTime<Utc> = r.get("created_at");
    let updated_at: DateTime<Utc> = r.get("updated_at");
    let last_role: Option<String> = r.get("last_role");
    let last_message = last_role.map(|role| {
        let last_created_at: Option<DateTime<Utc>> = r.get("last_created_at");
        ConversationLastMessage {
            content_head: r
                .get::<Option<String>, _>("last_content_head")
                .unwrap_or_default(),
            role,
            created_at: last_created_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
        }
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
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    }
}

/// One page of a participant's conversations plus their total — see
/// `ChatRepository::list_conversations` for the row contract.
pub(super) async fn list_conversations(
    pool: &PgPool,
    user_id: &str,
    tenant_id: TenantId,
    limit: i64,
    offset: i64,
) -> AppResult<ConversationPage> {
    let user_uuid = parse_uuid(user_id)?;
    let rows = sqlx::query(PAGE_SQL)
        .bind(user_uuid)
        .bind(tenant_id.to_string())
        .bind(limit)
        .bind(offset)
        .bind(CONTENT_HEAD_CHARS)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to list conversations: {e}")))?;

    let total = count_participating_conversations(pool, user_id, tenant_id).await?;

    Ok(ConversationPage {
        items: rows.iter().map(map_summary_row).collect(),
        total,
    })
}

/// Count every conversation a user participates in, in this tenant.
pub(super) async fn count_participating_conversations(
    pool: &PgPool,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<i64> {
    let count: i64 = sqlx::query_scalar(TOTAL_SQL)
        .bind(parse_uuid(user_id)?)
        .bind(tenant_id.to_string())
        .fetch_one(pool)
        .await
        .map_err(|e| {
            AppError::database(format!("Failed to count participating conversations: {e}"))
        })?;
    Ok(count)
}
