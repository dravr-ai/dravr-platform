// ABOUTME: Statements behind connecting a coaching group to a messaging chat from the web app (carnet#1683)
// ABOUTME: Connect codes, the connect/disconnect writes, released chats and the one-time non-member prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Statements for connecting a group to a chat, shared by both backends.
//!
//! `group_id` and `created_by` are uuid columns (native on Postgres, `TEXT`
//! on `SQLite`) and bind through the backend's uuid codec; every tenant id is
//! `TEXT` on both. Timestamps bind as `DateTime<Utc>`.

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
/// channel `$2`: the group the chat moves off when its owner connects it to
/// another group. The group itself, its members and its history stay.
pub(crate) const RELEASE_GROUP_CHAT_FOR_MOVE_SQL: &str = r"UPDATE coaching_groups
              SET channel_type = NULL, channel_chat_id = NULL, channel_tenant_id = NULL,
                  channel_chat_title = NULL, channel_connected_at = NULL, updated_at = $4
              WHERE id = $1 AND channel_type = $2 AND channel_chat_id = $3";

/// Bind active group `$1`, which is bound to no chat, to chat `$3` of channel
/// `$2` served by tenant `$4`. Zero rows means the group was connected to a
/// chat in the meantime (or archived); the served-chat unique index refuses a
/// chat another group still holds.
pub(crate) const CONNECT_GROUP_CHAT_SQL: &str = r"UPDATE coaching_groups
              SET channel_type = $2, channel_chat_id = $3, channel_tenant_id = $4,
                  channel_chat_title = $5, channel_connected_at = $6, updated_at = $6
              WHERE id = $1 AND channel_type IS NULL AND is_active = TRUE";

/// Forget that chat `$3` of channel `$2`, served by tenant `$1`, was released:
/// it is being connected again.
pub(crate) const FORGET_GROUP_CHAT_RELEASE_SQL: &str = r"DELETE FROM group_chat_releases
              WHERE channel_tenant_id = $1 AND channel_type = $2 AND channel_chat_id = $3";

/// The chat group `$1` is bound to, with the tenant serving it, read before a
/// disconnect clears it.
pub(crate) const GROUP_CHAT_BINDING_SQL: &str = r"SELECT channel_type, channel_chat_id,
              COALESCE(channel_tenant_id, tenant_id) AS serving_tenant_id
              FROM coaching_groups
              WHERE id = $1 AND channel_type IS NOT NULL AND channel_chat_id IS NOT NULL";

/// Clear group `$1`'s chat binding, whichever way it was made.
pub(crate) const DISCONNECT_GROUP_CHAT_SQL: &str = r"UPDATE coaching_groups
              SET channel_type = NULL, channel_chat_id = NULL, channel_tenant_id = NULL,
                  channel_chat_title = NULL, channel_connected_at = NULL, updated_at = $2
              WHERE id = $1 AND channel_type IS NOT NULL";

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
