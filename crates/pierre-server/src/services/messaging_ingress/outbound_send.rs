// ABOUTME: The two outbound paths every non-pipeline messaging reply leaves through
// ABOUTME: Loads channel config, splits a body past the channel ceiling, and spawns delivery

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use std::sync::Arc;

use pierre_core::models::messaging::OutgoingMessage;
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use tracing::error;

use super::block_render;
use super::dispatch::load_channel_config;

/// Send an outgoing reply to a channel user, loading config and spawning
/// delivery.
///
/// A body past the channel's ceiling is split here, at the one point every
/// non-pipeline reply in this module passes through: an over-limit message is
/// rejected outright by the channel API, so a `/plan`, an intake question or a
/// coach list that outgrew Discord's 2000 characters used to arrive truncated
/// or not at all. The parts are sent sequentially inside one spawned task so a
/// split answer never arrives out of order, and a failure part-way stops the
/// rest rather than posting a tail with no head.
pub(super) async fn send_channel_response(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
) {
    let ceiling = block_render::channel_ceiling(message.channel_type);
    let messages = block_render::fan_out(message, ceiling);
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        let adapter_clone = Arc::clone(adapter);
        tokio::spawn(async move {
            for message in messages {
                if let Err(e) = adapter_clone.send(&message, &cfg).await {
                    error!(error = %e, "Failed to send channel response");
                    return;
                }
            }
        });
    } else {
        // A missing config dropped the message with no trace at all, which is
        // indistinguishable from a channel that stayed quiet on purpose.
        error!(
            channel = %channel,
            "No channel config; outbound message dropped without being sent"
        );
    }
}

/// Deliver a slash-command reply privately to the caller instead of to the
/// room it arrived in, loading config and spawning delivery.
///
/// Each channel applies its own private mechanism inside canot's
/// `send_private_reply`: a 1:1 DM for Telegram/`WhatsApp`/Messenger, an ephemeral
/// message for Slack, an opened DM channel for Discord. `recipient_user_id` is
/// the channel-native id of the caller; `message` is the reply addressed to the
/// originating room.
pub(super) async fn send_private_channel_response(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
    recipient_user_id: &str,
) {
    let ceiling = block_render::channel_ceiling(message.channel_type);
    let messages = block_render::fan_out(message, ceiling);
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        let adapter_clone = Arc::clone(adapter);
        let recipient = recipient_user_id.to_owned();
        tokio::spawn(async move {
            for message in messages {
                if let Err(e) = adapter_clone
                    .send_private_reply(&message, &recipient, &cfg)
                    .await
                {
                    error!(error = %e, "Failed to send private slash-command reply");
                    return;
                }
            }
        });
    } else {
        error!(
            channel = %channel,
            "No channel config; private reply dropped without being sent"
        );
    }
}
