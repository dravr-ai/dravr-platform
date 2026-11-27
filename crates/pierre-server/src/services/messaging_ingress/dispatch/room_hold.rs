// ABOUTME: A group chat's reply that draws on a member's data the room may not read stays in the app
// ABOUTME: Decided by room_relay::reply_held_from_chat over the reads the reply was stored with

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A messaging group chat bound to a coaching group is read by everyone in
//! it the moment a reply is posted, so no per-reader rule can withhold a row
//! there the way the app's transcript does. A reply stored about a member
//! the room may not read (one who shares with the coach alone, say) is
//! therefore not posted: the chat is told it stays in the app, and the
//! reply is kept in the athlete's conversation, served there by the
//! transcript's own rule. The rule is `room_relay::reply_held_from_chat`,
//! which decides each read the reply was stored with by the room reader's
//! rule (carnet#1511 design calls 6 and 19).
//!
//! The coach's member cards in such a chat are already the room's own
//! (`prompt_builder::resolve_group_context`), so what is held is a reply
//! whose turn read such a member some other way: a tool call naming them.

use pierre_chat_pipeline::room_relay::reply_held_from_chat;
use pierre_chat_pipeline::TurnEnvelope;
use pierre_contremaitre::messaging_strings::KEY_ROOM_REPLY_HELD;
use pierre_core::models::messaging::ChannelConfig;
use tracing::info;

use super::super::agui::MessagingAgUiWiring;
use super::super::PendingDispatch;
use super::send_plain_reply;

/// Whether `envelope`'s reply was held from `dispatch`'s group chat: a
/// group chat turn whose reply the room may not read. When it was, the chat
/// has been told so, closing the status placeholder when one is open.
pub(super) async fn held_from_the_room(
    dispatch: &PendingDispatch,
    messaging_agui: Option<&MessagingAgUiWiring>,
    channel_config: &ChannelConfig,
    envelope: &TurnEnvelope,
) -> bool {
    if !dispatch.is_group_chat {
        return false;
    }
    let ctx = dispatch.resources.chat_pipeline_context();
    let reply = &envelope.assistant.message;
    if !reply_held_from_chat(
        &ctx,
        &envelope.conversation,
        dispatch.auth_result.user_id,
        &reply.id,
        reply.about_user_id,
    )
    .await
    {
        return false;
    }
    info!(
        conversation_id = %envelope.conversation.id,
        "group chat reply draws on a member the room may not read; kept in the app"
    );
    let notice = dispatch
        .resources
        .mcp
        .messaging_strings_registry
        .get(KEY_ROOM_REPLY_HELD, &envelope.locale);
    // The notice is no answer: no assistant row is pointed at for a reaction.
    let finalized = match messaging_agui {
        Some(wiring) => wiring.finalize_reply(&notice, dispatch, None).await,
        None => false,
    };
    if !finalized {
        send_plain_reply(dispatch, channel_config, &notice).await;
    }
    true
}
