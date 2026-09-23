// ABOUTME: The seam through which the weekly-digest scheduler posts into a group's own bound chat
// ABOUTME: Implemented in pierre-server over the messaging adapters, which this crate does not compile in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Posting into a group's chat.
//!
//! A group bootstrapped from a Telegram group, a Slack channel or a Discord
//! channel is bound to that chat (`channel_type` + `channel_chat_id`), and its
//! weekly digest belongs there, where the whole group reads it. The adapters
//! that can send into a chat live behind `pierre-server`'s messaging feature,
//! which this crate is built without, so the scheduler takes the send as a
//! trait — the same shape as `pierre_services::commitment_sweep::CommitmentReporter`.
//! The implementation is `pierre_mcp_server::services::group_chat_poster`.
//!
//! Only plain text crosses the seam: the scheduler renders the digest itself,
//! so no messaging type has to be visible on this side.

use async_trait::async_trait;
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
