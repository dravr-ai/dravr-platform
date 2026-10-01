// ABOUTME: ActivityConversationRepository trait plus the one shared implementation both backends emit
// ABOUTME: The thread an activity's view opened, per (tenant, user, provider, activity), only ever the owner's own conversation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity conversations.
//!
//! An activity's view carries a chat about the activity: its first question
//! opens a conversation, and every later one — on the same visit, a return,
//! another device — belongs in that thread. The link is kept in
//! `activity_conversations`, keyed by the provider key the cached activity is
//! stored under, like `activity_route_tracks`.
//!
//! Both statements reach the conversation through its owner: a link is
//! written only when the conversation is the caller's own, in the caller's
//! tenant, and read back only while it still is. A conversation deleted takes
//! its link along (`ON DELETE CASCADE`); a link whose conversation changed
//! hands would read as none.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres. The link's own id columns are `TEXT` on both engines;
//! `chat_conversations.user_id` is `uuid` on Postgres, so it is compared as
//! text.

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Persistence for the thread each activity's view opened.
#[async_trait]
pub trait ActivityConversationRepository: Send + Sync {
    /// The conversation linked to one activity, or `None` when none is linked
    /// or the linked one is no longer the caller's own.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn get_activity_conversation(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<Option<String>>;

    /// Link `conversation_id` to one activity, replacing any earlier link.
    ///
    /// Returns `false`, writing nothing, when the conversation is not one the
    /// caller owns in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn link_activity_conversation(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
        conversation_id: &str,
    ) -> AppResult<bool>;

    /// Remove one activity's link; the conversation itself is untouched.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn unlink_activity_conversation(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<()>;
}

/// Link a conversation to an activity when, and only when, the conversation
/// is `$2`'s own in tenant `$1`: the `SELECT` yields no row otherwise, so
/// nothing is written.
pub(crate) const LINK_ACTIVITY_CONVERSATION_SQL: &str = r"
    INSERT INTO activity_conversations (
        tenant_id, user_id, provider, activity_id, conversation_id, created_at
    )
    SELECT $1, $2, $3, $4, c.id, $6
    FROM chat_conversations c
    WHERE c.id = $5 AND c.tenant_id = $1 AND CAST(c.user_id AS TEXT) = $2
    ON CONFLICT (tenant_id, user_id, provider, activity_id) DO UPDATE SET
        conversation_id = EXCLUDED.conversation_id,
        created_at = EXCLUDED.created_at";

/// The linked conversation, joined back to its owner and tenant.
pub(crate) const GET_ACTIVITY_CONVERSATION_SQL: &str = r"
    SELECT l.conversation_id
    FROM activity_conversations l
    JOIN chat_conversations c
      ON c.id = l.conversation_id
     AND c.tenant_id = l.tenant_id
     AND CAST(c.user_id AS TEXT) = l.user_id
    WHERE l.tenant_id = $1 AND l.user_id = $2 AND l.provider = $3 AND l.activity_id = $4";

/// Remove one activity's link.
pub(crate) const UNLINK_ACTIVITY_CONVERSATION_SQL: &str = r"
    DELETE FROM activity_conversations
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND activity_id = $4";

/// Emit the whole [`ActivityConversationRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_activity_conversation_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ActivityConversationRepository for $ty {
            async fn get_activity_conversation(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<String>> {
                let row = sqlx::query(GET_ACTIVITY_CONVERSATION_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_activity_conversation: {e}")))?;
                row.map(|row| {
                    row.try_get::<String, _>("conversation_id")
                        .map_err(|e| AppError::database(format!("activity conversation id: {e}")))
                })
                .transpose()
            }

            async fn link_activity_conversation(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
                conversation_id: &str,
            ) -> AppResult<bool> {
                let written = sqlx::query(LINK_ACTIVITY_CONVERSATION_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .bind(conversation_id)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("link_activity_conversation: {e}")))?;
                Ok(written.rows_affected() > 0)
            }

            async fn unlink_activity_conversation(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<()> {
                sqlx::query(UNLINK_ACTIVITY_CONVERSATION_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("unlink_activity_conversation: {e}"))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_activity_conversation_repository;
