// ABOUTME: A coaching group's chat connected from the web app (carnet#1683): the connection, its codes, its outcomes
// ABOUTME: Re-exported from models::groups, beside the CoachingGroup whose channel_type/channel_chat_id pair it qualifies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::groups::CoachingGroup;
use super::TenantId;

/// A group's chat binding made from the web app by its owner or coach
/// (carnet#1683), alongside the `channel_type` / `channel_chat_id` pair it
/// qualifies.
///
/// A group the messaging ingress created for a chat lives under the tenant
/// whose bot serves that chat, so its `tenant_id` names both. A group made in
/// the web app lives under its creator's tenant while another tenant's bot
/// serves its chat, so the connection records that tenant itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupChatConnection {
    /// Tenant whose bot serves the chat: the one the chat's messages arrive
    /// under, and the one whose channel configuration posts into it.
    pub channel_tenant_id: TenantId,
    /// The chat's title when it was connected, for « Connected to Telegram:
    /// <title> ». `None` when the platform sent none.
    pub chat_title: Option<String>,
    /// When the chat was connected.
    pub connected_at: DateTime<Utc>,
}

/// A connect code a group's owner or coach minted in the web app, which the
/// bot redeems in the chat to connect it (carnet#1683).
///
/// The code is a credential while it lives, so `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupChatConnectCode {
    /// The code itself, single use.
    pub code: String,
    /// The group the chat will be connected to.
    pub group_id: Uuid,
    /// The channel the code is redeemable on (`telegram`).
    pub channel_type: String,
    /// The tenant whose bot may redeem it.
    pub channel_tenant_id: TenantId,
    /// Who minted it.
    pub created_by: Uuid,
    /// When it stops being redeemable.
    pub expires_at: DateTime<Utc>,
}

/// The chat a group is being connected to, with the code that authorizes it.
/// `Debug` never prints the code.
#[derive(Clone, Copy)]
pub struct ConnectGroupChat<'a> {
    /// The live code being spent.
    pub code: &'a str,
    /// The group the chat is connected to.
    pub group_id: Uuid,
    /// Channel of the chat (`telegram`).
    pub channel_type: &'a str,
    /// Channel-native id of the chat.
    pub channel_chat_id: &'a str,
    /// Tenant whose bot serves the chat.
    pub channel_tenant_id: TenantId,
    /// The chat's title, when the platform sent one.
    pub chat_title: Option<&'a str>,
    /// Another group the chat is bound to now, released in the same
    /// transaction: the caller decided the sender may move the chat off it
    /// (they own it). `None` when the chat is bound to no other group.
    pub release_group_id: Option<Uuid>,
    /// The instant the connection is recorded at; the code must be live then.
    pub now: DateTime<Utc>,
}

impl fmt::Debug for GroupChatConnectCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupChatConnectCode")
            .field("code", &"<redacted>")
            .field("group_id", &self.group_id)
            .field("channel_type", &self.channel_type)
            .field("channel_tenant_id", &self.channel_tenant_id)
            .field("created_by", &self.created_by)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl fmt::Debug for ConnectGroupChat<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectGroupChat")
            .field("code", &"<redacted>")
            .field("group_id", &self.group_id)
            .field("channel_type", &self.channel_type)
            .field("channel_chat_id", &self.channel_chat_id)
            .field("channel_tenant_id", &self.channel_tenant_id)
            .field("release_group_id", &self.release_group_id)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// What [`ConnectGroupChat`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupChatConnectOutcome {
    /// The chat is now the group's; the code is spent.
    Connected,
    /// The code was spent, expired or unknown by the time it was redeemed;
    /// nothing changed.
    CodeSpent,
    /// The group was connected to a chat in the meantime; nothing changed.
    GroupAlreadyConnected,
    /// Another group holds the chat and was not released; nothing changed.
    ChatTaken,
}

/// The binding a disconnect removed from a group, so the caller can tell the
/// chat it is no longer the group's room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasedGroupChat {
    /// Channel of the chat that was released.
    pub channel_type: String,
    /// Channel-native id of the chat.
    pub channel_chat_id: String,
    /// Tenant whose bot serves it.
    pub channel_tenant_id: TenantId,
}

impl CoachingGroup {
    /// The tenant whose bot serves the group's chat: the one recorded when it
    /// was connected from the web app, else the group's own (a group the chat
    /// created lives under its bot's tenant). `None` when the group's tenant
    /// is not a valid id and nothing was recorded.
    #[must_use]
    pub fn chat_serving_tenant(&self) -> Option<TenantId> {
        self.chat_connection.as_ref().map_or_else(
            || TenantId::parse_str(&self.tenant_id).ok(),
            |connection| Some(connection.channel_tenant_id),
        )
    }

    /// Whether the person `user_id` may connect this group to a chat, or
    /// disconnect it: its owner or its human coach.
    #[must_use]
    pub fn chat_manager(&self, user_id: Uuid) -> bool {
        self.owner_id == user_id || self.coach_user_id == Some(user_id)
    }
}
