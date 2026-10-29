// ABOUTME: The seam through which a group's own bound Telegram, Slack or Discord chat is posted into
// ABOUTME: Implemented in pierre-server over the messaging adapters, which no crate below it compiles in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Posting into a group's chat.
//!
//! A group bootstrapped from a Telegram group, a Slack channel or a Discord
//! channel is bound to that chat (`channel_type` + `channel_chat_id`), and
//! what the whole group should read belongs there: its weekly digest, a
//! coach's correction of a reply the agent sent there, and the post a coach's
//! plan publish leaves in the room (carnet#1249). The adapters that can send
//! into a chat live behind `pierre-server`'s messaging feature, which the
//! crates that decide those posts are built without, so they take the send as
//! a trait — the same shape as [`crate::commitment_sweep::CommitmentReporter`].
//! The implementation is `pierre_mcp_server::services::group_chat_poster`.
//!
//! Only plain text crosses the seam: the caller renders its text itself, so
//! no messaging type has to be visible on this side.

use async_trait::async_trait;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;

/// Sends one text into a group's bound chat.
#[async_trait]
pub trait GroupChatPoster: Send + Sync {
    /// Post `text` into the chat `chat_id` on `channel_type`, through the
    /// channel configuration `tenant_id` owns, and report how many message
    /// parts the channel accepted.
    ///
    /// A text longer than the channel carries goes out as several parts, in
    /// order, stopping at the first one refused. Zero means nothing reached
    /// the chat: the channel is unconfigured, the bot was removed from it, or
    /// the first part failed. Best-effort by contract — a failure is logged
    /// by the implementation, never raised.
    async fn post(
        &self,
        tenant_id: TenantId,
        channel_type: ChannelType,
        chat_id: &str,
        text: &str,
    ) -> usize;
}

/// The chat `group` is bound to, when the platform may post into it unasked.
///
/// That is a Telegram group, a Slack channel or a Discord channel, returned
/// with its channel-native id. `None` for a group bound to no chat, and for
/// one bound to `WhatsApp` or Messenger, which refuse a text the platform
/// starts outside the 24-hour window after a member's last message.
#[must_use]
pub fn proactive_chat(group: &CoachingGroup) -> Option<(ChannelType, &str)> {
    let channel = group.channel_type.as_deref()?.parse::<ChannelType>().ok()?;
    let chat_id = group
        .channel_chat_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())?;
    matches!(
        channel,
        ChannelType::Telegram | ChannelType::Slack | ChannelType::Discord
    )
    .then_some((channel, chat_id))
}
