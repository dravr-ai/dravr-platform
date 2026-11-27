// ABOUTME: The owner's own threads on the chat repository: the personal thread (carnet#840) and the plan thread (carnet#1841)
// ABOUTME: Emitted per backend inside impl_chat_repository!, whose ChatRepository impl delegates the four methods here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Emit the reads and designations of an owner's personal and plan threads
/// for one backend type: the bodies behind
/// [`ChatRepository::latest_personal_conversation`],
/// [`ChatRepository::personal_conversation`],
/// [`ChatRepository::designate_personal_conversation`],
/// [`ChatRepository::plan_thread_conversation`]. Invoked inside
/// `impl_chat_repository!`, whose shell must `use` the statements of
/// `chat::personal_sql` and the row parser this body names unqualified.
///
/// [`ChatRepository::latest_personal_conversation`]: super::chat::ChatRepository::latest_personal_conversation
/// [`ChatRepository::personal_conversation`]: super::chat::ChatRepository::personal_conversation
/// [`ChatRepository::designate_personal_conversation`]: super::chat::ChatRepository::designate_personal_conversation
/// [`ChatRepository::plan_thread_conversation`]: super::chat::ChatRepository::plan_thread_conversation
macro_rules! impl_chat_owner_threads {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// The thread a personal one is adopted from, when none is designated yet.
            async fn latest_personal(
                &self,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<ConversationRecord>> {
                // The predicate compares `CAST(user_id AS TEXT)`, so the id
                // binds as its hyphenated text on both backends.
                let user_id = Uuid::parse_str(user_id)
                    .map_err(|e| AppError::invalid_input(format!("Invalid user id: {e}")))?
                    .to_string();
                let row = sqlx::query(LATEST_PERSONAL_CONVERSATION_SQL)
                    .bind(user_id)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to find the personal conversation: {e}"))
                    })?;
                row.as_ref().map(conversation_from_row).transpose()
            }

            /// The owner's designated personal thread.
            async fn read_personal(
                &self,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<ConversationRecord>> {
                let row = sqlx::query(PERSONAL_CONVERSATION_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the personal thread: {e}"))
                    })?;
                row.as_ref().map(conversation_from_row).transpose()
            }

            /// Designate a thread its owner's personal one.
            async fn designate_personal(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                match sqlx::query(DESIGNATE_PERSONAL_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                {
                    Ok(result) => Ok(result.rows_affected() > 0),
                    // Another thread holds the designation: the one-per-owner
                    // index, reported by the driver as such.
                    Err(e)
                        if e.as_database_error()
                            .is_some_and(|db| db.is_unique_violation()) =>
                    {
                        Ok(false)
                    }
                    Err(e) => Err(AppError::database(format!(
                        "Failed to designate the personal thread: {e}"
                    ))),
                }
            }

            /// The owner's designated plan thread (carnet#1841).
            async fn read_plan_thread(
                &self,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<Option<ConversationRecord>> {
                let row = sqlx::query(PLAN_THREAD_CONVERSATION_SQL)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the plan thread: {e}"))
                    })?;
                row.as_ref().map(conversation_from_row).transpose()
            }
        }
    };
}
pub(crate) use impl_chat_owner_threads;
