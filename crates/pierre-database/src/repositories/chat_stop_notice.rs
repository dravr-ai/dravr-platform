// ABOUTME: The one rewrite a stored chat row takes: a stopped notice gains the envelope notice its turn's writes earned
// ABOUTME: The statement and the body behind it, emitted per backend inside impl_chat_repository!, like chat_member_rows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Replace the `content_blocks` of one stopped notice, for a participant of
/// its conversation in the caller's tenant.
///
/// The stop request writes the notice before the turn learns of it, so the
/// envelope notice for the writes the turn made before the stop can only
/// reach that row afterwards (carnet#1519). The statement matches only an
/// assistant row stamped with the stopped finish reason (`$6`): every other
/// row stays append-only.
pub(crate) const SET_STOPPED_NOTICE_BLOCKS_SQL: &str = r"
    UPDATE chat_messages
    SET content_blocks = $1
    WHERE id = $2 AND conversation_id = $3 AND role = 'assistant' AND finish_reason = $6
      AND EXISTS (
        SELECT 1 FROM chat_conversations c
        JOIN conversation_participants p ON p.conversation_id = c.id
        WHERE c.id = $3 AND c.tenant_id = $5 AND p.user_id = $4 AND p.tenant_id = $5
      )";

/// Emit the body behind
/// [`ChatRepository::set_stopped_notice_blocks`] for one backend type.
/// Invoked inside `impl_chat_repository!`, whose helpers it names
/// unqualified, and whose shell must `use` this module's statement.
///
/// [`ChatRepository::set_stopped_notice_blocks`]: super::chat::ChatRepository::set_stopped_notice_blocks
macro_rules! impl_chat_stop_notice {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// Replace a stopped notice's `content_blocks`; `false` when no
            /// stopped notice of the caller's matches.
            async fn replace_stopped_notice_blocks(
                &self,
                message_id: &str,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                content_blocks: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(SET_STOPPED_NOTICE_BLOCKS_SQL)
                    .bind(content_blocks)
                    .bind(message_id)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(STOPPED_TURN_FINISH_REASON)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update a stopped notice: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_chat_stop_notice;
