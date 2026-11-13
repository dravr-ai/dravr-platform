// ABOUTME: Deleting a conversation its owner holds, outright or only while no message is in it, with the A2A tasks run in it
// ABOUTME: The statements and the bodies behind them, emitted per backend inside impl_chat_repository!, like chat_archive

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Delete a conversation; owner-only, the messages cascade.
pub(crate) const DELETE_CONVERSATION_SQL: &str = r"
    DELETE FROM chat_conversations
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Delete a conversation while no message is in it; owner-only. Run under
/// [`conversation_row_lock_sql!`], so a message insert cannot commit between
/// this statement's read of `chat_messages` and the cascade.
pub(crate) const DELETE_EMPTY_CONVERSATION_SQL: &str = r"
    DELETE FROM chat_conversations
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3
      AND NOT EXISTS (
        SELECT 1 FROM chat_messages m WHERE m.conversation_id = chat_conversations.id
      )";

/// Lock an owner's conversation row for a transaction that deletes it only
/// while it holds no message. `$lock` is the backend's clause — `FOR UPDATE`
/// on `PostgreSQL`: a message insert's foreign-key check holds a key-share
/// lock on the row, so this waits out an insert in flight and holds off the
/// next until the transaction ends, and the delete after it reads every
/// message committed before; nothing on `SQLite`, whose `BEGIN IMMEDIATE`
/// already holds the one write lock.
macro_rules! conversation_row_lock_sql {
    ($lock:literal) => {
        concat!(
            "SELECT id FROM chat_conversations WHERE id = $1 AND user_id = $2 AND tenant_id = $3 ",
            $lock
        )
    };
}
pub(crate) use conversation_row_lock_sql;

/// The A2A tasks a deleted conversation's owner ran in it. `a2a_tasks` has no
/// tenant, and a tool task may name any `contextId`, so the owner's task key
/// (`user_task_key`) is what holds this to their rows.
pub(crate) const DELETE_CONVERSATION_TASKS_SQL: &str = r"
    DELETE FROM a2a_tasks
    WHERE context_id = $1 AND session_token = $2";

/// Emit the conversation deletes for one backend type: the bodies behind
/// [`ChatRepository::delete_conversation`] and
/// [`ChatRepository::delete_conversation_if_empty`]. `$begin` and `$row_lock`
/// are [`impl_chat_repository!`](super::chat_backend::impl_chat_repository)'s,
/// which invokes this and names its helpers and types unqualified, so its
/// shell must `use` this module's statements.
///
/// [`ChatRepository::delete_conversation`]: super::chat::ChatRepository::delete_conversation
/// [`ChatRepository::delete_conversation_if_empty`]: super::chat::ChatRepository::delete_conversation_if_empty
macro_rules! impl_chat_delete {
    ($ty:ty, $row:ty, $ids:ident, $begin:literal, $row_lock:literal) => {
        impl $ty {
            /// Delete a conversation `user_id` owns in `tenant_id`.
            async fn delete_owned_conversation(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let tx = self.pool().begin_with($begin).await.map_err(|e| {
                    AppError::database(format!("Failed to delete conversation: {e}"))
                })?;
                Self::delete_owned_conversation_in(
                    tx,
                    DELETE_CONVERSATION_SQL,
                    conversation_id,
                    user_id,
                    tenant_id,
                )
                .await
            }

            /// [`Self::delete_owned_conversation`], only while no message is
            /// in it: the row lock first, then the delete that reads
            /// `chat_messages`.
            async fn delete_owned_conversation_if_empty(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let mut tx = self.pool().begin_with($begin).await.map_err(|e| {
                    AppError::database(format!("Failed to delete conversation: {e}"))
                })?;
                let owned: Option<String> =
                    sqlx::query_scalar(conversation_row_lock_sql!($row_lock))
                        .bind(conversation_id)
                        .bind($ids::bind_text(user_id)?)
                        .bind(tenant_id.to_string())
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to delete conversation: {e}"))
                        })?;
                if owned.is_none() {
                    // Dropping the transaction rolls it back; nothing was written.
                    return Ok(false);
                }
                Self::delete_owned_conversation_in(
                    tx,
                    DELETE_EMPTY_CONVERSATION_SQL,
                    conversation_id,
                    user_id,
                    tenant_id,
                )
                .await
            }

            /// Delete `user_id`'s conversation with `delete_sql` in `tx`,
            /// and the A2A tasks they ran in it with it, and commit. `false`,
            /// with nothing written, when the statement matched no row.
            async fn delete_owned_conversation_in(
                mut tx: sqlx::Transaction<'static, <$row as sqlx::Row>::Database>,
                delete_sql: &'static str,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let task_key = Uuid::parse_str(user_id)
                    .map(user_task_key)
                    .map_err(|e| AppError::invalid_input(format!("Invalid user id: {e}")))?;
                let result = sqlx::query(delete_sql)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete conversation: {e}"))
                    })?;
                if result.rows_affected() == 0 {
                    // Dropping the transaction rolls it back; nothing was written.
                    return Ok(false);
                }
                sqlx::query(DELETE_CONVERSATION_TASKS_SQL)
                    .bind(conversation_id)
                    .bind(task_key)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete conversation tasks: {e}"))
                    })?;
                tx.commit().await.map_err(|e| {
                    AppError::database(format!("Failed to delete conversation: {e}"))
                })?;
                Ok(true)
            }
        }
    };
}
pub(crate) use impl_chat_delete;
