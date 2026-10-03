// ABOUTME: The room and private outbound paths every non-pipeline messaging reply leaves through
// ABOUTME: Splits past the channel ceiling and sends a batch in order from one drain-tracked task

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-messaging")]

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use pierre_core::models::messaging::{ChannelConfig, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use tracing::error;

use super::block_render;
use super::dispatch::load_channel_config;
use super::outbound_persist::{persist_outbound_row, OutboundRowParams};
use crate::mcp::resources::ServerContext;

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

/// One message of an ordered send: its split parts and the ledger they write.
struct Delivery {
    parts: Vec<OutgoingMessage>,
    persist: Option<OutboundPersistSpec>,
}

/// Deliver each message's split parts in order, writing one ledger row per
/// part under that message's spec.
///
/// A failure part-way stops the rest of that message rather than posting a
/// tail with no head; the next message still goes, since each stands on its
/// own. The failed attempt lands in the ledger (`failed-…`) so the wire stays
/// observable. Slash replies are synchronous request/response and are NOT
/// queued for retry: the retry worker re-sends through `send`, addressed to
/// the room, which would post a privately-redirected answer publicly.
async fn deliver_and_persist(
    adapter: Arc<dyn MessagingChannel>,
    config: ChannelConfig,
    deliveries: Vec<Delivery>,
    mode: DeliveryMode,
    channel: String,
) {
    for Delivery { parts, persist } in deliveries {
        for message in parts {
            let sent = match &mode {
                DeliveryMode::Room => adapter.send(&message, &config).await,
                DeliveryMode::Private { recipient } => {
                    adapter
                        .send_private_reply(&message, recipient, &config)
                        .await
                }
            };
            let receipt = match sent {
                Ok(receipt) => Some(receipt),
                Err(e) => {
                    error!(error = %e, channel = %channel, "Failed to send channel response");
                    None
                }
            };
            if let Some(spec) = &persist {
                persist_outbound_row(
                    spec.db.as_ref(),
                    &OutboundRowParams {
                        session_tenant_id: spec.session_tenant_id,
                        session_id: &spec.session_id,
                        channel: &channel,
                        receipt_id: receipt
                            .as_ref()
                            .and_then(|r| r.channel_message_id.as_deref()),
                        delivered: receipt.is_some(),
                        chat_message_id: spec.chat_message_id.as_deref(),
                    },
                    &message,
                )
                .await;
            }
            if receipt.is_none() {
                break;
            }
        }
    }
}

/// Send an outgoing reply to a channel user, loading config and spawning
/// delivery.
///
/// The delivery task is spawned through `resources.common.turns`, the tracker
/// the shutdown drain waits on. A slash, intake or prompt reply queued while
/// the instance is draining is work the athlete is waiting for; spawned
/// outside the tracker it would get only the runtime's one-second teardown
/// once the server future returned, and a slow channel API would lose it.
///
/// A body past the channel's ceiling is split here, at the one point every
/// non-pipeline reply in this module passes through: an over-limit message is
/// rejected outright by the channel API, so a `/plan`, an intake question or a
/// agent list that outgrew Discord's 2000 characters used to arrive truncated
/// or not at all. The parts are sent sequentially inside one spawned task so a
/// split answer never arrives out of order, and a failure part-way stops the
/// rest rather than posting a tail with no head.
pub async fn send_channel_response(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
    persist: Option<OutboundPersistSpec>,
) {
    send_channel_responses(
        resources,
        tenant_id,
        channel,
        adapter,
        vec![(message, persist)],
    )
    .await;
}

/// Send several replies to the room, in order, from one delivery task.
///
/// Each reply spawned on its own would race the others to the channel, so a
/// command's answer and the agent welcome that follows it (carnet#750) could
/// arrive welcome first. One task sends them in order; a message that fails
/// does not hold back the next. Each message is split, and the task
/// drain-tracked, as [`send_channel_response`] describes — that function is
/// this one with a single reply.
pub async fn send_channel_responses(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    messages: Vec<(OutgoingMessage, Option<OutboundPersistSpec>)>,
) {
    let deliveries = messages
        .into_iter()
        .map(|(message, persist)| {
            let ceiling = block_render::channel_ceiling(message.channel_type);
            Delivery {
                parts: block_render::fan_out(message, ceiling),
                persist,
            }
        })
        .collect();
    let db = resources.common.repos.messaging.as_ref();
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        resources.common.turns.spawn(deliver_and_persist(
            Arc::clone(adapter),
            cfg,
            deliveries,
            DeliveryMode::Room,
            channel.to_owned(),
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
/// originating room. Delivery is drain-tracked like [`send_channel_response`].
pub async fn send_private_channel_response(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    adapter: &Arc<dyn MessagingChannel>,
    message: OutgoingMessage,
    recipient_user_id: &str,
    persist: Option<OutboundPersistSpec>,
) {
    let ceiling = block_render::channel_ceiling(message.channel_type);
    let messages = block_render::fan_out(message, ceiling);
    let db = resources.common.repos.messaging.as_ref();
    let config = load_channel_config(db, tenant_id, channel).await;
    if let Some(cfg) = config {
        resources.common.turns.spawn(deliver_and_persist(
            Arc::clone(adapter),
            cfg,
            vec![Delivery {
                parts: messages,
                persist,
            }],
            DeliveryMode::Private {
                recipient: recipient_user_id.to_owned(),
            },
            channel.to_owned(),
        ));
    } else {
        error!(
            channel = %channel,
            "No channel config; private reply dropped without being sent"
        );
    }
}
