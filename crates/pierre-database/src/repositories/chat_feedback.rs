// ABOUTME: A participant's thumbs up or down on one reply, kept once per message and user
// ABOUTME: The statements and the bodies behind them, emitted per backend inside impl_chat_repository!, like chat_windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The nine columns every feedback read returns.
macro_rules! feedback_columns {
    () => {
        "id, message_id, conversation_id, user_id, tenant_id, rating, comment, \
         created_at, updated_at"
    };
}

/// Insert keyed on (`message_id`, `user_id`); on a repeat rating, overwrite the
/// rating + comment and bump `updated_at`. The WHERE EXISTS gate lands the row
/// only when the message belongs to a conversation the caller participates
/// in, in this tenant — a forged `message_id` never inserts.
pub(crate) const UPSERT_FEEDBACK_SQL: &str = r"
    INSERT INTO chat_message_feedback
        (id, message_id, conversation_id, user_id, tenant_id, rating, comment, created_at, updated_at)
    SELECT $1, $2, $3, $4, $5, $6, $7, $8, $8
    WHERE EXISTS (
        SELECT 1 FROM chat_messages m
        JOIN chat_conversations c ON m.conversation_id = c.id
        JOIN conversation_participants p ON p.conversation_id = c.id
        WHERE m.id = $2 AND m.conversation_id = $3 AND p.user_id = $4 AND p.tenant_id = $5 AND c.tenant_id = $5
    )
    ON CONFLICT (message_id, user_id) DO UPDATE SET
        rating = EXCLUDED.rating,
        comment = EXCLUDED.comment,
        updated_at = EXCLUDED.updated_at";

/// The caller's feedback on one message: the canonical row, read back after
/// an upsert because on conflict the stored id and `created_at` stay the
/// original values.
pub(crate) const GET_FEEDBACK_SQL: &str = concat!(
    "SELECT ",
    feedback_columns!(),
    " FROM chat_message_feedback \
     WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3"
);

/// Delete the caller's feedback on a message (thumbs toggle-off).
pub(crate) const DELETE_FEEDBACK_SQL: &str = r"
    DELETE FROM chat_message_feedback
    WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3";

/// All of the caller's feedback rows for a conversation.
pub(crate) const CONVERSATION_FEEDBACK_SQL: &str = concat!(
    "SELECT ",
    feedback_columns!(),
    " FROM chat_message_feedback \
     WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3"
);

/// Emit the feedback writes and reads for one backend type: the bodies
/// behind [`ChatRepository::upsert_message_feedback`],
/// [`ChatRepository::delete_message_feedback`] and
/// [`ChatRepository::get_conversation_feedback`]. Invoked inside
/// `impl_chat_repository!`, whose row parsers and helpers it names
/// unqualified, and whose shell must `use` this module's statements.
///
/// [`ChatRepository::upsert_message_feedback`]: super::chat::ChatRepository::upsert_message_feedback
/// [`ChatRepository::delete_message_feedback`]: super::chat::ChatRepository::delete_message_feedback
/// [`ChatRepository::get_conversation_feedback`]: super::chat::ChatRepository::get_conversation_feedback
macro_rules! impl_chat_feedback {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// Upsert the caller's feedback on one message and read it back.
            async fn upsert_feedback(
                &self,
                params: &UpsertMessageFeedbackParams<'_>,
            ) -> AppResult<MessageFeedbackRecord> {
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();
                let user = $ids::bind_text(params.user_id)?;
                let tenant = params.tenant_id.to_string();

                sqlx::query(UPSERT_FEEDBACK_SQL)
                    .bind(&id)
                    .bind(params.message_id)
                    .bind(params.conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(params.rating)
                    .bind(params.comment)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to upsert message feedback: {e}"))
                    })?;

                let row = sqlx::query(GET_FEEDBACK_SQL)
                    .bind(params.message_id)
                    .bind(&user)
                    .bind(&tenant)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read back message feedback: {e}"))
                    })?;
                row.as_ref()
                    .map(feedback_from_row)
                    .transpose()?
                    .ok_or_else(|| AppError::not_found("Message not found or access denied"))
            }

            /// Delete the caller's feedback on one message.
            async fn delete_feedback(
                &self,
                message_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_FEEDBACK_SQL)
                    .bind(message_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete message feedback: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            /// The caller's feedback rows for one conversation.
            async fn conversation_feedback(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<MessageFeedbackRecord>> {
                let rows = sqlx::query(CONVERSATION_FEEDBACK_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get conversation feedback: {e}"))
                    })?;
                rows.iter().map(feedback_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_chat_feedback;
