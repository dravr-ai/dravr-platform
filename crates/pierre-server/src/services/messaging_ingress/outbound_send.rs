// ABOUTME: The two outbound paths every non-pipeline messaging reply leaves through
// ABOUTME: Loads channel config, splits a body past the channel ceiling, and spawns delivery

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use std::sync::Arc;

use pierre_core::models::messaging::{ChannelConfig, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use tracing::error;

use super::block_render;
use super::dispatch::load_channel_config;
use super::outbound_persist::{persist_outbound_row, OutboundRowParams};

/// What the spawned delivery writes to the `messaging_messages` ledger per
/// part, when the caller has a resolved session to attach the rows to.
///
/// `None` at a send site means no ledger row: the pre-session auth flows
/// (link code, OTP, logout, the unlinked prompt, the auth denial) have no
/// `messaging_sessions` row yet, so there is nothing for a row to belong to.
pub struct OutboundPersistSpec {
    /// Clone of the messaging repository `Arc`, moved into the delivery task.
    pub db: Arc<dyn MessagingRepository>,
    /// Tenant that owns the session/conversation the rows read under.
    pub session_tenant_id: TenantId,
    /// The `messaging_sessions` row the send belongs to.
    pub session_id: String,
    /// The assistant chat row this reply delivers, when the transcript policy
    /// persisted one — the join a reaction rating resolves through.
    pub chat_message_id: Option<String>,
}

/// How the spawned task hands a part to the channel.
enum DeliveryMode {
    /// `MessagingChannel::send`, addressed to the room/DM the message names.
    Room,
    /// `MessagingChannel::send_private_reply` to this channel-native user id.
    Private { recipient: String },
}

/// Deliver the split parts sequentially, writing one ledger row per part.
///
/// A failure part-way stops the rest rather than posting a tail with no head;
/// the failed attempt still lands in the ledger (`failed-…`) so the wire stays
/// observable. Slash replies are synchronous request/response and are NOT
/// queued for retry: the retry worker re-sends through `send`, addressed to
/// the room, which would post a privately-redirected answer publicly.
async fn deliver_and_persist(
    adapter: Arc<dyn MessagingChannel>,
    config: ChannelConfig,
    parts: Vec<OutgoingMessage>,
    mode: DeliveryMode,
    channel: String,
    persist: Option<OutboundPersistSpec>,
) {
    for message in parts {
        let sent = match &mode {
            DeliveryMode::Room => adapter.send(&message, &config).await,
            DeliveryMode::Private { recipient } => {
                adapter
                    .send_private_reply(&message, recipient, &config)
                    .await
            }
        };
        match sent {
            Ok(receipt) => {
                if let Some(spec) = &persist {
                    persist_outbound_row(
                        spec.db.as_ref(),
                        &OutboundRowParams {
                            session_tenant_id: spec.session_tenant_id,
                            session_id: &spec.session_id,
                            channel: &channel,
                            receipt_id: receipt.channel_message_id.as_deref(),
                            delivered: true,
                            chat_message_id: spec.chat_message_id.as_deref(),
                        },
                        &message,
                    )
                    .await;
                }
            }
            Err(e) => {
                error!(error = %e, channel = %channel, "Failed to send channel response");
                if let Some(spec) = &persist {
                    persist_outbound_row(
                        spec.db.as_ref(),
                        &OutboundRowParams {
                            session_tenant_id: spec.session_tenant_id,
                            session_id: &spec.session_id,
                            channel: &channel,
                            receipt_id: None,
                            delivered: false,
                            chat_message_id: spec.chat_message_id.as_deref(),
                        },
                        &message,
                    )
                    .await;
                }
                return;
            }
        }
    }
}

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
pub async fn send_channel_response(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
    persist: Option<OutboundPersistSpec>,
) {
    let ceiling = block_render::channel_ceiling(message.channel_type);
    let messages = block_render::fan_out(message, ceiling);
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        tokio::spawn(deliver_and_persist(
            Arc::clone(adapter),
            cfg,
            messages,
            DeliveryMode::Room,
            channel.to_owned(),
            persist,
        ));
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
pub async fn send_private_channel_response(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
    recipient_user_id: &str,
    persist: Option<OutboundPersistSpec>,
) {
    let ceiling = block_render::channel_ceiling(message.channel_type);
    let messages = block_render::fan_out(message, ceiling);
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        tokio::spawn(deliver_and_persist(
            Arc::clone(adapter),
            cfg,
            messages,
            DeliveryMode::Private {
                recipient: recipient_user_id.to_owned(),
            },
            channel.to_owned(),
            persist,
        ));
    } else {
        error!(
            channel = %channel,
            "No channel config; private reply dropped without being sent"
        );
    }
}
