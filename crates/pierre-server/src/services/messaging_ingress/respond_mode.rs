// ABOUTME: The group respond-mode gate: which room messages are ambient chatter the agent stays out of
// ABOUTME: A mentions-mode room answers only what addresses the bot, a slash command, or a member mid-walk
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::groups::GroupRespondMode;
use pierre_core::models::messaging::IncomingMessage;
use pierre_core::models::TenantId;
use tracing::warn;

use super::content::content_body_text;
use super::session;
use crate::mcp::resources::ServerContext;

/// Resolve the respond mode of the coaching group bound to a channel chat.
///
/// No chat id, no group binding, or a lookup failure all resolve to
/// [`GroupRespondMode::All`] — the pre-feature behavior — so a transient DB
/// error can only ever make the agent chattier, never mute it.
async fn channel_group_respond_mode(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    conversation_id: Option<&str>,
) -> GroupRespondMode {
    let Some(chat_id) = conversation_id.filter(|c| !c.is_empty()) else {
        return GroupRespondMode::All;
    };
    match resources
        .common
        .repos
        .groups
        .get_group_by_channel(tenant_id, channel, chat_id)
        .await
    {
        Ok(Some(group)) => group.respond_mode,
        Ok(None) => GroupRespondMode::All,
        Err(e) => {
            warn!(
                error = %e,
                channel = %channel,
                "respond-mode lookup failed; answering (fail-open to pre-feature behavior)"
            );
            GroupRespondMode::All
        }
    }
}

/// `true` when this inbound is ambient room conversation the agent must not
/// answer: a group-chat message in a mentions-mode group that neither
/// addresses the bot nor invokes a slash command.
pub(super) async fn is_ambient_group_message(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    message: &IncomingMessage,
) -> bool {
    if message.is_direct_message || message.addressed_to_bot {
        return false;
    }
    // Slash commands stay live unaddressed — `/group respond all` must work
    // without a mention, or the owner could never leave mentions mode from
    // inside the chat. Their replies are already delivered privately in
    // shared rooms, so honoring them adds no room noise.
    if content_body_text(&message.content).is_some_and(|t| t.trim_start().starts_with('/')) {
        return false;
    }
    if channel_group_respond_mode(
        resources,
        tenant_id,
        channel,
        message.conversation_id.as_deref(),
    )
    .await
        != GroupRespondMode::Mentions
    {
        return false;
    }
    // A member mid guided walk answers the agent's question unaddressed — an
    // interview that demanded an @-mention per answer would shed its athlete
    // by question two. Only the walker is exempted: the lookup resolves the
    // SENDER's own room session (rooms are per-member), so everyone else's
    // unaddressed chatter stays silently captured.
    !session::sender_room_walk_is_active(
        resources,
        tenant_id,
        channel,
        &message.sender_id,
        message.conversation_id.as_deref(),
    )
    .await
}
