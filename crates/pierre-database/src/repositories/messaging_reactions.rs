// ABOUTME: Shared SQL and body for the reaction -> chat-message lookup both backends serve
// ABOUTME: One statement, one row mapper; each backend shell supplies only its own cast suffix

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reaction → chat-message resolution, written once.
//!
//! A reaction webhook carries only the channel's own message id, while
//! feedback is keyed on `chat_messages.id`. The outbound persist stamps the
//! assistant message id onto the sent row, so this lookup is the reverse of
//! that stamp: given the channel identity of a message, hand back the chat
//! message it delivered plus the session identity the caller needs to decide
//! whether the reactor is allowed to rate it.
//!
//! The two backends differ in one respect only. `messaging_sessions.user_id`
//! is a `uuid` column on Postgres and `TEXT` on `SQLite`, so Postgres needs an
//! explicit `::text` on the selected column and on the nullable chat-id
//! parameter, where `SQLite` needs none. That suffix is the macro's single
//! argument; everything else — the join, the filters, the ordering and the
//! row decode — exists once.

/// The lookup statement, with `$cast` appended wherever Postgres needs a text
/// coercion and `SQLite` needs nothing.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both drivers.
macro_rules! reaction_target_sql {
    ($cast:literal) => {
        concat!(
            "
        SELECT m.chat_message_id, s.tenant_id, s.user_id",
            $cast,
            " AS user_id, s.channel_user_id,
               s.pierre_conversation_id
        FROM messaging_messages m
        JOIN messaging_sessions s ON s.id = m.session_id
        WHERE m.direction = 'outbound'
          AND m.channel_type = $1
          AND m.channel_message_id = $2
          AND m.chat_message_id IS NOT NULL
          AND s.pierre_conversation_id IS NOT NULL
          AND ($3",
            $cast,
            " IS NULL OR COALESCE(s.channel_conversation_id, '') = $3",
            $cast,
            ")
        ORDER BY m.created_at DESC
        LIMIT 1
        "
        )
    };
}
pub(crate) use reaction_target_sql;

/// Emit `find_reaction_feedback_target` for one backend.
///
/// `$db` is the sqlx database type the pool is parameterised on; `$sql` is
/// that backend's resolved statement, which its shell builds by expanding
/// [`reaction_target_sql`] with the text coercion its `user_id` column needs
/// (`"::text"` on Postgres, `""` on `SQLite`).
///
/// The body is written once here. Each backend module invokes the macro, and
/// sqlx resolves the driver from the pool type at that expansion.
macro_rules! impl_find_reaction_feedback_target {
    ($db:ty, $sql:ident) => {
        /// Resolve the assistant chat message a reacted-to channel message
        /// delivered.
        ///
        /// Matches on channel identity — channel type plus the channel's own
        /// message id — rather than tenant: the reaction webhook authenticates
        /// as the bot's tenant while DM message rows live under the athlete's
        /// own. `channel_conversation_id`, when the reaction carries one,
        /// narrows the match to the session bound to that chat, because
        /// Telegram message ids and Slack timestamps are unique only within a
        /// chat. The chat filter compares `COALESCE`d ids so a legacy session
        /// with a NULL chat id still matches a reaction that carries none.
        ///
        /// Only outbound rows stamped with a `chat_message_id` resolve;
        /// everything else yields `Ok(None)`, so a reaction on a message the
        /// platform never sent (or on one that carried no ratable coaching
        /// reply) is a no-op rather than an error.
        ///
        /// # Errors
        ///
        /// Returns an error if the database query or column decode fails.
        pub async fn find_reaction_feedback_target(
            pool: &Pool<$db>,
            channel_type: &str,
            channel_message_id: &str,
            channel_conversation_id: Option<&str>,
        ) -> AppResult<Option<ReactionFeedbackTarget>> {
            let row = sqlx::query($sql)
                .bind(channel_type)
                .bind(channel_message_id)
                .bind(channel_conversation_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to resolve reaction target: {e}"))
                })?;

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
    };
}
pub(crate) use impl_find_reaction_feedback_target;
