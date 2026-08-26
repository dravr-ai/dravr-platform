// ABOUTME: SQLite lookup resolving an inbound emoji reaction to the chat message it rates
// ABOUTME: Joins messaging_messages to messaging_sessions so the caller can authorise the feedback write
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reaction → chat-message resolution for the SQLite backend.
//!
//! A reaction webhook carries only the channel's own message id, while
//! feedback is keyed on `chat_messages.id`. The outbound persist stamps the
//! assistant message id onto the sent row, so this lookup is the reverse of
//! that stamp: given the channel identity of a message, hand back the chat
//! message it delivered plus the session identity the caller needs to decide
//! whether the reactor is allowed to rate it.
//!
//! It lives beside the reaction migration rather than inside the 1,600-line
//! messaging repository because it is one question asked one way, and reading
//! it should not mean scrolling past the outbound queue.

use pierre_core::errors::{AppError, AppResult};
use sqlx::{Pool, Row, Sqlite};

use crate::repositories::ReactionFeedbackTarget;

/// Resolve the assistant chat message a reacted-to channel message delivered.
///
/// Matches on channel identity — channel type plus the channel's own message
/// id — rather than tenant: the reaction webhook authenticates as the bot's
/// tenant while DM message rows live under the athlete's own.
/// `channel_conversation_id`, when the reaction carries one, narrows the match
/// to the session bound to that chat, because Telegram message ids and Slack
/// timestamps are unique only within a chat. The chat filter compares
/// `COALESCE`d ids so a legacy session with a NULL chat id still matches a
/// reaction that carries none.
///
/// Only outbound rows stamped with a `chat_message_id` resolve; everything
/// else yields `Ok(None)`, so a reaction on a message the platform never sent
/// (or on one that carried no ratable coaching reply) is a no-op rather than
/// an error.
///
/// # Errors
///
/// Returns an error if the database query or column decode fails.
pub async fn find_reaction_feedback_target(
    pool: &Pool<Sqlite>,
    channel_type: &str,
    channel_message_id: &str,
    channel_conversation_id: Option<&str>,
) -> AppResult<Option<ReactionFeedbackTarget>> {
    let row = sqlx::query(
        r"
        SELECT m.chat_message_id, m.tenant_id, s.user_id, s.channel_user_id,
               s.pierre_conversation_id
        FROM messaging_messages m
        JOIN messaging_sessions s ON s.id = m.session_id
        WHERE m.direction = 'outbound'
          AND m.channel_type = ?1
          AND m.channel_message_id = ?2
          AND m.chat_message_id IS NOT NULL
          AND s.pierre_conversation_id IS NOT NULL
          AND (?3 IS NULL OR COALESCE(s.channel_conversation_id, '') = ?3)
        ORDER BY m.created_at DESC
        LIMIT 1
        ",
    )
    .bind(channel_type)
    .bind(channel_message_id)
    .bind(channel_conversation_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::database(format!("Failed to resolve reaction target: {e}")))?;

    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(ReactionFeedbackTarget {
        chat_message_id: row
            .try_get("chat_message_id")
            .map_err(|e| AppError::database(format!("chat_message_id: {e}")))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("tenant_id: {e}")))?,
        user_id: row
            .try_get("user_id")
            .map_err(|e| AppError::database(format!("user_id: {e}")))?,
        channel_user_id: row
            .try_get("channel_user_id")
            .map_err(|e| AppError::database(format!("channel_user_id: {e}")))?,
        conversation_id: row
            .try_get("pierre_conversation_id")
            .map_err(|e| AppError::database(format!("pierre_conversation_id: {e}")))?,
    }))
}
