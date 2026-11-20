// ABOUTME: Connecting a coaching group to a messaging chat from the web app (carnet#1683): trait, statements, shared body
// ABOUTME: Connect codes, the connect/disconnect writes, released chats and the one-time non-member prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! [`GroupChatConnectRepository`], written once for both backends.
//!
//! A supertrait of [`super::CoachingGroupRepository`]: the same repository
//! object answers it, and the registry needs no second field. `group_id` and
//! `created_by` are uuid columns (native on Postgres, `TEXT` on `SQLite`) and
//! bind through the backend's uuid codec; every tenant id is `TEXT` on both.
//! Timestamps bind as `DateTime<Utc>`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{
    ConnectGroupChat, GroupChatConnectCode, GroupChatConnectOutcome, ReleasedGroupChat,
};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Connecting a group to a messaging chat from the web app (carnet#1683).
#[async_trait]
pub trait GroupChatConnectRepository: Send + Sync {
    /// Store a freshly minted connect code.
    async fn create_chat_connect_code(
        &self,
        code: &GroupChatConnectCode,
        now: DateTime<Utc>,
    ) -> AppResult<()>;

    /// The connect code `code` when the bot of `channel_tenant_id` may still
    /// redeem it on `channel_type` at `now`: unspent and unexpired. Reading
    /// never spends it.
    async fn live_chat_connect_code(
        &self,
        code: &str,
        channel_type: &str,
        channel_tenant_id: TenantId,
        now: DateTime<Utc>,
    ) -> AppResult<Option<GroupChatConnectCode>>;

    /// Spend the code and bind the group to the chat, in one transaction.
    ///
    /// Releases `release_group_id` from the chat first when it is named,
    /// and forgets that the chat was released by an earlier disconnect.
    /// Every refusal leaves everything as it was, the code unspent included,
    /// except [`GroupChatConnectOutcome::CodeSpent`], where it already was.
    async fn connect_group_chat(
        &self,
        connect: &ConnectGroupChat<'_>,
    ) -> AppResult<GroupChatConnectOutcome>;

    /// Clear the group's chat binding, whichever way it was made, and record
    /// the chat as released so its next message creates no group. Returns
    /// the binding removed, `None` when the group was bound to no chat.
    /// `tenant_id` is the tenant that owns the group; the write matches the
    /// group only under it.
    async fn disconnect_group_chat(
        &self,
        group_id: Uuid,
        tenant_id: TenantId,
        now: DateTime<Utc>,
    ) -> AppResult<Option<ReleasedGroupChat>>;

    /// Whether the chat was released by a disconnect and not connected since.
    async fn is_group_chat_released(
        &self,
        channel_tenant_id: TenantId,
        channel_type: &str,
        channel_chat_id: &str,
    ) -> AppResult<bool>;

    /// Record that a non-member sender in the group's chat was told how to
    /// join. `true` the first time, `false` when they already were.
    async fn record_chat_invite_prompt(
        &self,
        group_id: Uuid,
        channel_type: &str,
        channel_user_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool>;
}

/// A freshly minted connect code. `$8` is the creation instant.
pub(crate) const INSERT_CHAT_CONNECT_CODE_SQL: &str = r"INSERT INTO group_chat_connect_codes
              (id, code, group_id, channel_type, channel_tenant_id, created_by, expires_at, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";

/// A connect code that the bot of tenant `$3` may still redeem on channel
/// `$2` at instant `$4`: unspent and unexpired. Reading never spends it.
pub(crate) const LIVE_CHAT_CONNECT_CODE_SQL: &str = r"SELECT code, group_id, channel_type, channel_tenant_id, created_by, expires_at
              FROM group_chat_connect_codes
              WHERE code = $1 AND channel_type = $2 AND channel_tenant_id = $3
                AND used_at IS NULL AND expires_at > $4";

/// Spend connect code `$1` at `$2`, only while it is still live. Zero rows
/// means another redemption got there first or it expired.
pub(crate) const SPEND_CHAT_CONNECT_CODE_SQL: &str = r"UPDATE group_chat_connect_codes SET used_at = $2
              WHERE code = $1 AND used_at IS NULL AND expires_at > $2";

/// Clear group `$1`'s chat binding when it is still bound to chat `$3` of
/// channel `$2` served by tenant `$5` (the served-chat expression the lookup
/// found it by): the group the chat moves off when its owner connects it to
/// another group. The group itself, its members and its history stay.
pub(crate) const RELEASE_GROUP_CHAT_FOR_MOVE_SQL: &str = r"UPDATE coaching_groups
              SET channel_type = NULL, channel_chat_id = NULL, channel_tenant_id = NULL,
                  channel_chat_title = NULL, channel_connected_at = NULL, updated_at = $4
              WHERE id = $1 AND COALESCE(channel_tenant_id, tenant_id) = $5
                AND channel_type = $2 AND channel_chat_id = $3";

/// Bind active group `$1` of tenant `$7`, which is bound to no chat, to chat
/// `$3` of channel `$2` served by tenant `$4`. Zero rows means the group was
/// connected to a chat in the meantime (or archived); the served-chat unique
/// index refuses a chat another group still holds.
pub(crate) const CONNECT_GROUP_CHAT_SQL: &str = r"UPDATE coaching_groups
              SET channel_type = $2, channel_chat_id = $3, channel_tenant_id = $4,
                  channel_chat_title = $5, channel_connected_at = $6, updated_at = $6
              WHERE id = $1 AND tenant_id = $7 AND channel_type IS NULL AND is_active = TRUE";

/// Forget that chat `$3` of channel `$2`, served by tenant `$1`, was released:
/// it is being connected again.
pub(crate) const FORGET_GROUP_CHAT_RELEASE_SQL: &str = r"DELETE FROM group_chat_releases
              WHERE channel_tenant_id = $1 AND channel_type = $2 AND channel_chat_id = $3";

/// The chat group `$1` of tenant `$2` is bound to, with the tenant serving
/// it, read before a disconnect clears it.
pub(crate) const GROUP_CHAT_BINDING_SQL: &str = r"SELECT channel_type, channel_chat_id,
              COALESCE(channel_tenant_id, tenant_id) AS serving_tenant_id
              FROM coaching_groups
              WHERE id = $1 AND tenant_id = $2
                AND channel_type IS NOT NULL AND channel_chat_id IS NOT NULL";

/// Clear the chat binding of group `$1` of tenant `$2`, whichever way it was
/// made, at `$3`.
pub(crate) const DISCONNECT_GROUP_CHAT_SQL: &str = r"UPDATE coaching_groups
              SET channel_type = NULL, channel_chat_id = NULL, channel_tenant_id = NULL,
                  channel_chat_title = NULL, channel_connected_at = NULL, updated_at = $3
              WHERE id = $1 AND tenant_id = $2 AND channel_type IS NOT NULL";

/// Record that chat `$3` of channel `$2`, served by tenant `$1`, was released
/// at `$4`, so its next message auto-creates no group. A chat released twice
/// keeps the newer instant.
pub(crate) const RECORD_GROUP_CHAT_RELEASE_SQL: &str = r"INSERT INTO group_chat_releases
              (channel_tenant_id, channel_type, channel_chat_id, released_at)
              VALUES ($1, $2, $3, $4)
              ON CONFLICT (channel_tenant_id, channel_type, channel_chat_id)
              DO UPDATE SET released_at = excluded.released_at";

/// Whether chat `$3` of channel `$2`, served by tenant `$1`, was released.
pub(crate) const GROUP_CHAT_RELEASED_SQL: &str = r"SELECT 1 AS released FROM group_chat_releases
              WHERE channel_tenant_id = $1 AND channel_type = $2 AND channel_chat_id = $3";

/// Record that sender `$3` on channel `$2` was told once how to join group
/// `$1`; a second record is a no-op, which is how the caller knows it already
/// told them.
pub(crate) const RECORD_CHAT_INVITE_PROMPT_SQL: &str = r"INSERT INTO group_chat_invite_prompts
              (group_id, channel_type, channel_user_id, prompted_at)
              VALUES ($1, $2, $3, $4)
              ON CONFLICT (group_id, channel_type, channel_user_id) DO NOTHING";

/// Emit the [`GroupChatConnectRepository`] implementation for one backend
/// type. `$ids` is the backend's uuid codec from [`super::uuid_columns`]; the
/// invoking shell must `use` every statement above and the decode helpers
/// (`column`, `instant`) of [`super::coaching_groups`].
macro_rules! impl_group_chat_connect_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl GroupChatConnectRepository for $ty {
            async fn create_chat_connect_code(
                &self,
                code: &GroupChatConnectCode,
                now: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(INSERT_CHAT_CONNECT_CODE_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind(&code.code)
                    .bind($ids::bind(code.group_id))
                    .bind(&code.channel_type)
                    .bind(code.channel_tenant_id.to_string())
                    .bind($ids::bind(code.created_by))
                    .bind(code.expires_at)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store chat connect code: {e}"))
                    })?;
                Ok(())
            }

            async fn live_chat_connect_code(
                &self,
                code: &str,
                channel_type: &str,
                channel_tenant_id: TenantId,
                now: DateTime<Utc>,
            ) -> AppResult<Option<GroupChatConnectCode>> {
                let row = sqlx::query(LIVE_CHAT_CONNECT_CODE_SQL)
                    .bind(code)
                    .bind(channel_type)
                    .bind(channel_tenant_id.to_string())
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read chat connect code: {e}"))
                    })?;
                let Some(r) = row else {
                    return Ok(None);
                };
                let serving: String = column(&r, "channel_tenant_id")?;
                Ok(Some(GroupChatConnectCode {
                    code: column(&r, "code")?,
                    group_id: $ids::read(&r, "group_id")?,
                    channel_type: column(&r, "channel_type")?,
                    channel_tenant_id: TenantId::parse_str(&serving).map_err(|e| {
                        AppError::database(format!(
                            "group_chat_connect_codes column `channel_tenant_id`: {e}"
                        ))
                    })?,
                    created_by: $ids::read(&r, "created_by")?,
                    expires_at: instant(&r, "expires_at")?,
                }))
            }

            async fn connect_group_chat(
                &self,
                connect: &ConnectGroupChat<'_>,
            ) -> AppResult<GroupChatConnectOutcome> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to connect group chat: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(failed)?;
                // Spent first, so a code redeemed twice at once connects once:
                // the second redemption waits on the row and then matches none.
                let spent = sqlx::query(SPEND_CHAT_CONNECT_CODE_SQL)
                    .bind(connect.code)
                    .bind(connect.now)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                if spent.rows_affected() == 0 {
                    return Ok(GroupChatConnectOutcome::CodeSpent);
                }
                if let Some(release) = connect.release_group_id {
                    sqlx::query(RELEASE_GROUP_CHAT_FOR_MOVE_SQL)
                        .bind($ids::bind(release))
                        .bind(connect.channel_type)
                        .bind(connect.channel_chat_id)
                        .bind(connect.now)
                        .bind(connect.channel_tenant_id.to_string())
                        .execute(&mut *tx)
                        .await
                        .map_err(failed)?;
                }
                let bound = sqlx::query(CONNECT_GROUP_CHAT_SQL)
                    .bind($ids::bind(connect.group_id))
                    .bind(connect.channel_type)
                    .bind(connect.channel_chat_id)
                    .bind(connect.channel_tenant_id.to_string())
                    .bind(connect.chat_title)
                    .bind(connect.now)
                    .bind(connect.group_tenant_id.to_string())
                    .execute(&mut *tx)
                    .await;
                match bound {
                    // Dropping the transaction rolls the spend back too: the
                    // code stays redeemable for the chat the group should get.
                    Ok(done) if done.rows_affected() == 0 => {
                        return Ok(GroupChatConnectOutcome::GroupAlreadyConnected);
                    }
                    Ok(_) => {}
                    Err(e)
                        if e.as_database_error()
                            .is_some_and(|db| db.is_unique_violation()) =>
                    {
                        return Ok(GroupChatConnectOutcome::ChatTaken);
                    }
                    Err(e) => return Err(failed(e)),
                }
                sqlx::query(FORGET_GROUP_CHAT_RELEASE_SQL)
                    .bind(connect.channel_tenant_id.to_string())
                    .bind(connect.channel_type)
                    .bind(connect.channel_chat_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(GroupChatConnectOutcome::Connected)
            }

            async fn disconnect_group_chat(
                &self,
                group_id: Uuid,
                tenant_id: TenantId,
                now: DateTime<Utc>,
            ) -> AppResult<Option<ReleasedGroupChat>> {
                let failed = |e: sqlx::Error| {
                    AppError::database(format!("Failed to disconnect group chat: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(failed)?;
                let row = sqlx::query(GROUP_CHAT_BINDING_SQL)
                    .bind($ids::bind(group_id))
                    .bind(tenant_id.to_string())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(failed)?;
                let Some(r) = row else {
                    return Ok(None);
                };
                let channel_type: String = column(&r, "channel_type")?;
                let channel_chat_id: String = column(&r, "channel_chat_id")?;
                let serving: String = column(&r, "serving_tenant_id")?;
                let channel_tenant_id = TenantId::parse_str(&serving).map_err(|e| {
                    AppError::database(format!("coaching_groups serving tenant: {e}"))
                })?;
                let cleared = sqlx::query(DISCONNECT_GROUP_CHAT_SQL)
                    .bind($ids::bind(group_id))
                    .bind(tenant_id.to_string())
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                if cleared.rows_affected() == 0 {
                    return Ok(None);
                }
                sqlx::query(RECORD_GROUP_CHAT_RELEASE_SQL)
                    .bind(&serving)
                    .bind(&channel_type)
                    .bind(&channel_chat_id)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(failed)?;
                tx.commit().await.map_err(failed)?;
                Ok(Some(ReleasedGroupChat {
                    channel_type,
                    channel_chat_id,
                    channel_tenant_id,
                }))
            }

            async fn is_group_chat_released(
                &self,
                channel_tenant_id: TenantId,
                channel_type: &str,
                channel_chat_id: &str,
            ) -> AppResult<bool> {
                let row = sqlx::query(GROUP_CHAT_RELEASED_SQL)
                    .bind(channel_tenant_id.to_string())
                    .bind(channel_type)
                    .bind(channel_chat_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read group chat release: {e}"))
                    })?;
                Ok(row.is_some())
            }

            async fn record_chat_invite_prompt(
                &self,
                group_id: Uuid,
                channel_type: &str,
                channel_user_id: &str,
                now: DateTime<Utc>,
            ) -> AppResult<bool> {
                let done = sqlx::query(RECORD_CHAT_INVITE_PROMPT_SQL)
                    .bind($ids::bind(group_id))
                    .bind(channel_type)
                    .bind(channel_user_id)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record chat invite prompt: {e}"))
                    })?;
                Ok(done.rows_affected() == 1)
            }
        }
    };
}
pub(crate) use impl_group_chat_connect_repository;
