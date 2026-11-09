// ABOUTME: A conversation set aside by /reset and brought back: archiving it, reading the flag, reactivating it under the cap
// ABOUTME: The statements and the bodies behind them, emitted per backend inside impl_chat_repository!, like chat_windows

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Archive a conversation the caller owns, when it is still active.
pub(crate) const ARCHIVE_CONVERSATION_SQL: &str = r"
    UPDATE chat_conversations
    SET archived_at = $1
    WHERE id = $2 AND user_id = $3 AND tenant_id = $4 AND archived_at IS NULL";

/// Return an archived conversation the caller owns to the active set. The
/// cap, when there is one, was checked under the owner slot lock in the same
/// transaction.
pub(crate) const REACTIVATE_CONVERSATION_SQL: &str = r"
    UPDATE chat_conversations
    SET archived_at = NULL
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3 AND archived_at IS NOT NULL";

/// Whether a conversation the caller owns is archived; no row when it is not
/// theirs in this tenant.
pub(crate) const IS_ARCHIVED_SQL: &str = r"
    SELECT archived_at IS NOT NULL
    FROM chat_conversations
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// [`IS_ARCHIVED_SQL`], and whether the row holds one of its owner's
/// `max_active_conversations` slots once active: the personal thread and a
/// coach's thread about one athlete never do, so neither is refused at the cap.
pub(crate) const ARCHIVED_SLOT_SQL: &str = r"
    SELECT archived_at IS NOT NULL AS archived,
           (personal = FALSE AND subject_user_id IS NULL) AS holds_slot
    FROM chat_conversations
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Emit the archive writes and reads for one backend type: the bodies
/// behind [`ChatRepository::archive_conversation`],
/// [`ChatRepository::is_conversation_archived`] and
/// [`ChatRepository::reactivate_conversation`]. Invoked inside
/// `impl_chat_repository!`, whose slot transaction and helpers it names
/// unqualified, and whose shell must `use` this module's statements.
///
/// [`ChatRepository::archive_conversation`]: super::chat::ChatRepository::archive_conversation
/// [`ChatRepository::is_conversation_archived`]: super::chat::ChatRepository::is_conversation_archived
/// [`ChatRepository::reactivate_conversation`]: super::chat::ChatRepository::reactivate_conversation
macro_rules! impl_chat_archive {
    ($ty:ty, $ids:ident) => {
        impl $ty {
            /// Archive an active conversation the caller owns.
            async fn archive(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(ARCHIVE_CONVERSATION_SQL)
                    .bind(Utc::now())
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to archive conversation: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            /// Whether a conversation the caller owns is archived.
            async fn archived(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                sqlx::query_scalar::<_, bool>(IS_ARCHIVED_SQL)
                    .bind(conversation_id)
                    .bind($ids::bind_text(user_id)?)
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map(|archived| archived.unwrap_or(false))
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to read conversation archive state: {e}"
                        ))
                    })
            }

            /// Return an archived conversation the caller owns to the active set,
            /// under the owner slot lock when there is a cap.
            async fn reactivate(
                &self,
                conversation_id: &str,
                user_id: &str,
                tenant_id: TenantId,
                cap: Option<i64>,
            ) -> AppResult<Reactivation> {
                let user = $ids::bind_text(user_id)?;
                let tenant = tenant_id.to_string();
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to reactivate conversation: {e}"))
                };

                let (mut tx, at_cap) = self.open_slot_transaction(user_id, &tenant, cap).await?;
                // Read under the lock: a concurrent turn may have reactivated
                // the row since the caller looked.
                let state = sqlx::query_as::<_, (bool, bool)>(ARCHIVED_SLOT_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(failed)?;
                let Some((true, holds_slot)) = state else {
                    return Ok(Reactivation::NotArchived);
                };
                // A row that holds no slot once active takes none of the
                // owner's, so the count does not decide it.
                if let Some(current) = at_cap.filter(|_| holds_slot) {
                    // Dropping the transaction rolls it back; the row stays archived.
                    return Ok(Reactivation::AtCap { current });
                }
                sqlx::query(REACTIVATE_CONVERSATION_SQL)
                    .bind(conversation_id)
                    .bind(&user)
                    .bind(&tenant)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(Reactivation::Reactivated)
            }
        }
    };
}
pub(crate) use impl_chat_archive;
