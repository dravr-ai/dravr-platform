// ABOUTME: Constructor helpers for outbound channel messages (canot OutgoingMessage)
// ABOUTME: Collapses the repeated proactive text-message struct literal into one helper

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use pierre_core::models::messaging::{ChannelType, MessageContent, OutgoingMessage};
use pierre_messaging::turn::ConversationTurnId;

/// Build a proactive text message: a fresh conversation turn with no reply or
/// thread linkage.
///
/// "Proactive" means the platform initiates the turn — a backfill-ready push, an
/// account-approved notice, an OTP/link prompt, or a session-reset confirmation —
/// rather than replying to an inbound utterance. So `turn_id` is a fresh
/// [`ConversationTurnId`] and both `reply_to` and `thread_id` are `None`. Reply
/// messages, which carry the inbound turn id, a `reply_to`, or a `thread_id`,
/// construct [`OutgoingMessage`] inline.
pub fn proactive_text(
    channel_type: ChannelType,
    recipient_id: String,
    body: String,
) -> OutgoingMessage {
    OutgoingMessage {
        channel_type,
        recipient_id,
        content: MessageContent::Text { body },
        turn_id: ConversationTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}
