// ABOUTME: A thread's rows about a group member (carnet#1174): the prompt read that keeps only the readable ones
// ABOUTME: The statements and the reads behind them, emitted per backend inside impl_chat_repository!, like chat_windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::chat::{message_columns, participant_scope};

/// [`GET_RECENT_MESSAGES_SQL`] for a turn's prompt, keeping a row about a
/// group member only when the member is one of the `readable_about` ids bound
/// from `$6` on (carnet#1174); with none, only rows about nobody. One
/// placeholder per id, as `users_by_ids_sql` binds its batch, so the ids bind
/// through the codec on both drivers: the count is all that is formatted.
///
/// [`GET_RECENT_MESSAGES_SQL`]: super::chat::GET_RECENT_MESSAGES_SQL
pub(crate) fn recent_prompt_messages_sql(readable_about: usize) -> String {
    let about = if readable_about == 0 {
        "m.about_user_id IS NULL".to_owned()
    } else {
        let placeholders = (6..6 + readable_about)
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("(m.about_user_id IS NULL OR m.about_user_id IN ({placeholders}))")
    };
    format!(
        "SELECT {} FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         JOIN conversation_participants p ON p.conversation_id = c.id \
         WHERE m.conversation_id = $1 AND (m.first_party_only = FALSE OR $5) AND {about} AND {} \
         ORDER BY m.created_at DESC, m.id DESC \
         LIMIT $4",
        message_columns!(),
        participant_scope!(),
    )
}

/// The distinct members a conversation's rows are about, for a participant.
pub(crate) const MESSAGE_SUBJECTS_SQL: &str = concat!(
    "SELECT DISTINCT m.about_user_id \
     FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND m.about_user_id IS NOT NULL AND ",
    participant_scope!()
);

/// Emit the member-row reads for one backend type: the bodies behind
/// [`ChatRepository::get_recent_prompt_messages`] and
/// [`ChatRepository::list_message_subjects`]. Invoked inside
/// `impl_chat_repository!`, whose row parsers and helpers it names
/// unqualified, and whose shell must `use` this module's statements.
///
/// [`ChatRepository::get_recent_prompt_messages`]: super::chat::ChatRepository::get_recent_prompt_messages
/// [`ChatRepository::list_message_subjects`]: super::chat::ChatRepository::list_message_subjects
macro_rules! impl_chat_member_rows {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// The newest `limit` rows of a conversation a turn's prompt may replay,
            /// oldest first.
            async fn prompt_messages(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                limit: i64,
                readable: TransportPolicy,
                readable_about: &[Uuid],
            ) -> AppResult<Vec<MessageRecord>> {
                let sql = recent_prompt_messages_sql(readable_about.len());
                let mut query = sqlx::query(&sql)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(limit)
                    .bind(readable.is_first_party_only());
                for member in readable_about {
                    query = query.bind($ids::bind(*member));
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to get the prompt's recent messages: {e}"))
                })?;
                // Newest first from the statement; chronological for the caller.
                let mut messages = rows
                    .iter()
                    .map(message_from_row)
                    .collect::<AppResult<Vec<_>>>()?;
                messages.reverse();
                Ok(messages)
            }

            /// Every group member a conversation's rows are about, for a participant.
            async fn message_subjects(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<Uuid>> {
                let rows = sqlx::query(MESSAGE_SUBJECTS_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list a thread's members: {e}"))
                    })?;
                rows.iter()
                    .map(|row| $ids::read(row, "about_user_id"))
                    .collect()
            }
        }
    };
}
pub(crate) use impl_chat_member_rows;
