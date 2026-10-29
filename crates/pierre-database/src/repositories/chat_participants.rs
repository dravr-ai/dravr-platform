// ABOUTME: Who belongs to a conversation: adding and removing a member, and the roster read owner first
// ABOUTME: The statements and the bodies behind them, emitted per backend inside impl_chat_repository!, like chat_windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Add a member, idempotently. `ON CONFLICT DO NOTHING` keeps an existing
/// row (the owner's included) untouched; the WHERE EXISTS gate refuses a
/// conversation outside this tenant instead of writing a dangling
/// membership.
pub(crate) const ADD_PARTICIPANT_SQL: &str = r"
    INSERT INTO conversation_participants
        (conversation_id, user_id, tenant_id, role, added_by, added_at)
    SELECT $1, $2, $3, $4, $5, $6
    WHERE EXISTS (
        SELECT 1 FROM chat_conversations WHERE id = $1 AND tenant_id = $3
    )
    ON CONFLICT (conversation_id, user_id) DO NOTHING";

/// One membership row.
pub(crate) const GET_PARTICIPANT_SQL: &str = r"
    SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
    FROM conversation_participants
    WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3";

/// Remove a member; the owner's row is never removed.
pub(crate) const REMOVE_PARTICIPANT_SQL: &str = r"
    DELETE FROM conversation_participants
    WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3 AND role = $4";

/// Every participant of a conversation, owner first.
pub(crate) const LIST_PARTICIPANTS_SQL: &str = r"
    SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
    FROM conversation_participants
    WHERE conversation_id = $1 AND tenant_id = $2
    ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, added_at ASC, user_id ASC";

/// Emit the membership writes and reads for one backend type: the bodies
/// behind [`ChatRepository::add_participant`],
/// [`ChatRepository::remove_participant`] and
/// [`ChatRepository::list_participants`]. Invoked inside
/// `impl_chat_repository!`, whose row parsers and helpers it names
/// unqualified, and whose shell must `use` this module's statements.
///
/// [`ChatRepository::add_participant`]: super::chat::ChatRepository::add_participant
/// [`ChatRepository::remove_participant`]: super::chat::ChatRepository::remove_participant
/// [`ChatRepository::list_participants`]: super::chat::ChatRepository::list_participants
macro_rules! impl_chat_participants {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// Add a member to a conversation and read their row back.
            async fn insert_participant(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
                user_id: &str,
                added_by: &str,
            ) -> AppResult<ConversationParticipant> {
                let user = $ids::bind_text(user_id)?;
                let tenant = tenant_id.to_string();

                sqlx::query(ADD_PARTICIPANT_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .bind(ParticipantRole::Member.as_str())
                    .bind($ids::bind_text(added_by)?)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to add participant: {e}")))?;

                let row = sqlx::query(GET_PARTICIPANT_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read participant: {e}")))?;
                row.as_ref()
                    .map(participant_from_row)
                    .transpose()?
                    .ok_or_else(|| AppError::not_found("Conversation not found"))
            }

            /// Remove a member from a conversation.
            async fn delete_participant(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
                user_id: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(REMOVE_PARTICIPANT_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .bind(ParticipantRole::Member.as_str())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to remove participant: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            /// Every participant of a conversation, owner first.
            async fn select_participants(
                &self,
                conversation_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Vec<ConversationParticipant>> {
                let rows = sqlx::query(LIST_PARTICIPANTS_SQL)
                    .bind(conversation_id)
                    .bind(tenant_id.to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list participants: {e}")))?;
                rows.iter().map(participant_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_chat_participants;
