// ABOUTME: Whether a messaging session's chat is a direct message or a room, as the ingress records it each turn
// ABOUTME: The reads the room classifier asks of a conversation's sessions, emitted per backend inside impl_messaging_repository!

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Session chats.
//!
//! A messaging session writes one chat's turns into one Pierre conversation.
//! The chat is a direct message or a room (a group chat), which the channel
//! reports on every inbound message and the ingress records on the session
//! (`messaging_sessions.is_direct_message`, carnet#1422). A conversation row
//! alone cannot tell: a group chat whose group binding failed keeps no group,
//! exactly as a DM's thread does.

/// What one session that writes into a conversation says about its chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionChat {
    /// The session of a direct message: its owner alone reads the chat.
    DirectMessage,
    /// The session of a group chat: everyone in the chat reads it.
    Room,
    /// A session no turn has stamped since the column arrived.
    ///
    /// Which chat it serves is not on record.
    Unrecorded,
}

impl SessionChat {
    /// The record a session's `is_direct_message` column holds.
    #[must_use]
    pub const fn from_column(is_direct_message: Option<bool>) -> Self {
        match is_direct_message {
            Some(true) => Self::DirectMessage,
            Some(false) => Self::Room,
            None => Self::Unrecorded,
        }
    }
}

/// The chat record of every session writing into a conversation.
///
/// Within one tenant: `$1` the tenant, `$2` the conversation id.
pub(crate) const SESSION_CHATS_SQL: &str = "SELECT is_direct_message FROM messaging_sessions \
     WHERE tenant_id = $1 AND pierre_conversation_id = $2";

/// Record whether a session's chat is a direct message.
///
/// Within one tenant, as every read of the sessions is: `$1` the record, `$2`
/// the session id, `$3` the tenant the session lives under.
pub(crate) const STAMP_SESSION_CHAT_SQL: &str =
    "UPDATE messaging_sessions SET is_direct_message = $1 WHERE id = $2 AND tenant_id = $3";

/// Emit the session-chat reads for one backend type.
///
/// The bodies behind `MessagingRepository::session_chats`,
/// `MessagingRepository::record_session_chat` and
/// `MessagingRepository::get_session`, invoked inside
/// `impl_messaging_repository!`.
macro_rules! impl_session_chat {
    ($ty:ty) => {
        impl $ty {
            /// The chat record of every session writing into `conversation_id`.
            async fn chats_of_sessions(
                &self,
                tenant_id: TenantId,
                conversation_id: &str,
            ) -> AppResult<Vec<$crate::repositories::messaging_session_chat::SessionChat>> {
                let rows = sqlx::query(
                    $crate::repositories::messaging_session_chat::SESSION_CHATS_SQL,
                )
                .bind(tenant_id)
                .bind(conversation_id)
                .fetch_all(self.pool())
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to read a thread's sessions: {e}"))
                })?;
                rows.iter()
                    .map(|row| {
                        row.try_get::<Option<bool>, _>("is_direct_message")
                            .map(
                                $crate::repositories::messaging_session_chat::SessionChat::from_column,
                            )
                            .map_err(|e| messaging_column_error("is_direct_message", &e))
                    })
                    .collect()
            }

            /// The session `session_id` filed under `tenant_id`, which a
            /// message row names, as the other session reads project it.
            async fn session_by_id(
                &self,
                tenant_id: TenantId,
                session_id: &str,
            ) -> AppResult<Option<Value>> {
                let row = sqlx::query(SESSION_BY_ID_SQL)
                    .bind(tenant_id)
                    .bind(session_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get session: {e}")))?;
                row.as_ref().map(session_json).transpose()
            }

            /// Record whether `session_id`'s chat, under `tenant_id`, is a
            /// direct message.
            async fn stamp_session_chat(
                &self,
                tenant_id: TenantId,
                session_id: &str,
                is_direct_message: bool,
            ) -> AppResult<()> {
                sqlx::query($crate::repositories::messaging_session_chat::STAMP_SESSION_CHAT_SQL)
                    .bind(is_direct_message)
                    .bind(session_id)
                    .bind(tenant_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a session's chat: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_session_chat;
