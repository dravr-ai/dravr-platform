// ABOUTME: The reply to an inbound message the agent cannot read yet: a voice note, a bare photo
// ABOUTME: Ledgered like any reply, answered in the sender's locale, never forwarded to the model
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use pierre_contremaitre::messaging_strings::KEY_MEDIA_TEXT_ONLY;
use pierre_core::models::messaging::{ChannelType, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_services::messaging_broadcast::proactive_text;

use super::content::needs_text_only_reply;
use super::otp::apply_conversation_recipient;
use super::outbound_send::{send_channel_response, OutboundPersistSpec};
use super::ResolvedSession;
use crate::mcp::resources::ServerContext;

/// What [`send_text_only_reply`] needs to answer one unreadable inbound message.
pub(super) struct TextOnlyReply<'a> {
    pub(super) tenant_id: TenantId,
    /// The tenant the session's ledger rows file under.
    pub(super) session_tenant_id: TenantId,
    pub(super) channel: &'a str,
    pub(super) channel_type: ChannelType,
    pub(super) adapter: &'a Arc<dyn MessagingChannel>,
    pub(super) message: &'a IncomingMessage,
    pub(super) session: &'a ResolvedSession,
    pub(super) thread_id: Option<String>,
    /// The sender's resolved locale, the one every other reply this turn uses.
    pub(super) locale: &'a str,
}

/// Tell the sender the agent only reads text for now.
///
/// The reply answers the inbound turn (it shares its id), is ledgered as an outbound row
/// like any other reply, and never reaches the model. Sends nothing when the message
/// has no text for a reason that owes the sender no answer (see [`needs_text_only_reply`]).
pub(super) async fn send_text_only_reply(
    resources: &Arc<ServerContext>,
    reply: &TextOnlyReply<'_>,
) {
    let message = reply.message;
    let addressed = message.is_direct_message || message.addressed_to_bot;
    if !needs_text_only_reply(&message.content, reply.channel_type, addressed) {
        return;
    }
    let body = resources
        .mcp
        .messaging_strings_registry
        .get(KEY_MEDIA_TEXT_ONLY, reply.locale);
    let mut outgoing = proactive_text(reply.channel_type, message.sender_id.clone(), body);
    outgoing.turn_id = message.turn_id;
    outgoing.thread_id.clone_from(&reply.thread_id);
    apply_conversation_recipient(&mut outgoing, message.conversation_id.as_deref());
    let persist = OutboundPersistSpec {
        db: Arc::clone(&resources.common.repos.messaging),
        session_tenant_id: reply.session_tenant_id,
        session_id: reply.session.session_id.clone(),
        chat_message_id: None,
    };
    send_channel_response(
        resources,
        reply.tenant_id,
        reply.channel,
        reply.adapter,
        outgoing,
        Some(persist),
    )
    .await;
}
