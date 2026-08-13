// ABOUTME: What happens after a channel message authenticates — or is refused
// ABOUTME: Split from messaging_ingress/mod.rs; usage recording plus the denial reply per status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The tail of channel authentication: given an [`AuthResult`], record the
//! usage a successful turn implies, or compose the reply a refused one needs.
//!
//! Refusal is not one case. A suspended account, a pending one, an expired JWT
//! and a sender with no linked provider each need a different thing said to
//! them, in their own locale — which is why this is a module rather than an
//! `else` branch. Getting it wrong strands a real person in a chat window with
//! no idea what to do next.

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::messaging::{
    ChannelType, IncomingMessage, MessageContent, OutgoingMessage,
};
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use pierre_middleware::auth::record_jwt_usage_for_request;
use pierre_services::channel_error_reply::ChannelErrorReply;
use pierre_services::user_status_gate::messaging_key_for_status;
use tracing::{debug, error, warn};
use uuid::Uuid;

use super::otp::apply_conversation_recipient;
use super::{
    connect, extract_thread_id, send_channel_response, send_unlinked_user_prompt, ServerContext,
};

/// Inputs for [`handle_channel_auth_outcome`].
pub(super) struct ChannelAuthOutcomeInputs<'a> {
    pub(super) resources: &'a ServerContext,
    pub(super) db: &'a dyn MessagingRepository,
    pub(super) tenant_id: TenantId,
    pub(super) channel: &'a str,
    pub(super) channel_type: ChannelType,
    pub(super) adapter: &'a Arc<dyn MessagingChannel>,
    pub(super) message: &'a IncomingMessage,
    pub(super) outcome: Result<AuthResult, AppError>,
}

/// Refresh `users.last_active` after a successful channel authentication.
///
/// Mirrors the JWT/MCP tool-handler behavior so admin "last seen" views and
/// activity reports treat messaging-only users as live. Best-effort: a
/// failure here is logged but does not block dispatch — activity tracking is
/// observability, not correctness.
pub(super) async fn refresh_channel_last_active(
    resources: &ServerContext,
    user_id: Uuid,
    channel_type: ChannelType,
) {
    if let Err(e) = resources
        .common
        .repos
        .users
        .update_last_active(user_id)
        .await
    {
        warn!(
            user_id = %user_id,
            channel = %channel_type,
            error = %e,
            "Failed to update last_active on channel auth (activity tracking impacted)"
        );
    }
}

/// Record a `JwtUsage` row for a successful channel-authenticated turn.
///
/// Delegates to [`pierre_middleware::auth::record_jwt_usage_for_request`] so
/// the JWT (`HTTP` cookie / Bearer / `MCP`) path and the channel-link path
/// share **one** write site for the rate-limit counter — no symmetry gap
/// between transports. The endpoint label is rendered as
/// `messaging:<channel>` (e.g. `messaging:telegram`) so admin usage reports
/// can disaggregate by transport.
pub(super) async fn record_channel_usage(resources: &ServerContext, user_id: Uuid, channel: &str) {
    let endpoint = format!("messaging:{channel}");
    record_jwt_usage_for_request(&resources.common.repos, user_id, &endpoint, "WEBHOOK").await;
}

/// Branch on the channel-authentication outcome, surfacing the right reply
/// for each terminal state and returning the [`AuthResult`] only on success.
pub(super) async fn handle_channel_auth_outcome(
    inputs: ChannelAuthOutcomeInputs<'_>,
) -> Result<Option<AuthResult>, ()> {
    match inputs.outcome {
        Ok(r) => {
            refresh_channel_last_active(inputs.resources, r.user_id, inputs.channel_type).await;
            record_channel_usage(inputs.resources, r.user_id, inputs.channel).await;
            Ok(Some(r))
        }
        Err(e) if e.code == ErrorCode::AuthInvalid => {
            debug!(
                sender_id = %inputs.message.sender_id,
                channel = %inputs.channel_type,
                "Sender not linked, prompting"
            );
            send_unlinked_user_prompt(
                inputs.resources,
                inputs.db,
                inputs.tenant_id,
                inputs.channel,
                inputs.channel_type,
                inputs.adapter,
                inputs.message,
            )
            .await;
            Ok(None)
        }
        Err(e)
            if matches!(
                e.code,
                ErrorCode::AccountPending
                    | ErrorCode::AccountSuspended
                    | ErrorCode::RateLimitExceeded
                    | ErrorCode::NoProviderConnected
            ) =>
        {
            let reply = build_auth_denial_reply(AuthDenialReplyInputs {
                resources: inputs.resources,
                tenant_id: inputs.tenant_id,
                channel: inputs.channel,
                channel_type: inputs.channel_type,
                sender_id: &inputs.message.sender_id,
                conversation_id: inputs.message.conversation_id.as_deref(),
                thread_id: extract_thread_id(&inputs.message.metadata),
                is_direct_message: inputs.message.is_direct_message,
                err: &e,
            })
            .await;
            send_channel_response(
                inputs.db,
                inputs.tenant_id,
                inputs.channel,
                inputs.adapter,
                reply,
            )
            .await;
            Ok(None)
        }
        Err(e) => {
            // Operator-category failure — drop the message, let dravr-tronc
            // page on-call via the ERROR subscriber.
            error!(
                error = %e,
                sender_id = %inputs.message.sender_id,
                channel = %inputs.channel_type,
                "Channel authentication failed, dropping message"
            );
            Err(())
        }
    }
}

/// Inputs for [`build_auth_denial_reply`].
pub(super) struct AuthDenialReplyInputs<'a> {
    pub(super) resources: &'a ServerContext,
    pub(super) tenant_id: TenantId,
    pub(super) channel: &'a str,
    pub(super) channel_type: ChannelType,
    pub(super) sender_id: &'a str,
    pub(super) conversation_id: Option<&'a str>,
    pub(super) thread_id: Option<String>,
    /// Direct message vs group — gates the tokenized connect Card (a user-scoped
    /// connect link must never be posted into a shared room).
    pub(super) is_direct_message: bool,
    pub(super) err: &'a AppError,
}

/// Resolve the user's account email for a `(channel, channel_user_id)`
/// pair via the channel-link → user-id → user-row chain.
///
/// Used by the `NoProviderConnected` denial path so the chat reply can tell
/// the user *which* email to sign in with. Returns `None` on any failure
/// (channel link missing, malformed JSON, user-id parse failure, DB hiccup,
/// user row deleted) so the caller can fall back to the URL-only template.
pub(super) async fn resolve_channel_user_email(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    channel_user_id: &str,
) -> Option<String> {
    let link = resources
        .common
        .repos
        .messaging
        .get_channel_link(tenant_id, channel, channel_user_id)
        .await
        .ok()
        .flatten()?;
    let user_id_str = link.get("user_id")?.as_str()?;
    let user_id = Uuid::parse_str(user_id_str).ok()?;
    let user = resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .ok()
        .flatten()?;
    Some(user.email)
}

/// Build a localized "denied" reply for the authentication outcomes that need
/// to surface user-facing text (`Pending`, `Suspended`, `RateLimitExceeded`).
pub(super) async fn build_auth_denial_reply(inputs: AuthDenialReplyInputs<'_>) -> OutgoingMessage {
    let locale = inputs
        .resources
        .common
        .repos
        .messaging
        .get_channel_link_locale(inputs.tenant_id, inputs.channel, inputs.sender_id)
        .await
        .ok()
        .flatten()
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LOCALE.to_owned());

    // A direct-message user with no provider gets a tappable "Connect your
    // account" Card (in-process-minted link → hosted picker). Group contexts and
    // mint failures fall through to the plain web-link text below.
    if inputs.err.code == ErrorCode::NoProviderConnected {
        if let Some(mut card) = connect::try_build_connect_card(
            inputs.resources,
            inputs.tenant_id,
            inputs.channel,
            inputs.channel_type,
            inputs.sender_id,
            inputs.thread_id.clone(),
            inputs.is_direct_message,
            &locale,
        )
        .await
        {
            apply_conversation_recipient(&mut card, inputs.conversation_id);
            return card;
        }
    }

    let body = if let Some(key) = messaging_key_for_status(inputs.err.code) {
        // NoProviderConnected carries a `{0}` placeholder for the dravr web
        // connect URL, and `{1}` for the account email when we can resolve
        // it from the channel link. Telling the user *which* email to sign
        // in with removes a guessing step (multi-account households, etc.).
        // Other status denials (Pending/Suspended) have no template args.
        if inputs.err.code == ErrorCode::NoProviderConnected {
            let connect_url = format!(
                "{}/providers",
                inputs
                    .resources
                    .common
                    .config
                    .frontend_url
                    .as_deref()
                    .unwrap_or(&inputs.resources.common.config.base_url)
            );
            let email = resolve_channel_user_email(
                inputs.resources,
                inputs.tenant_id,
                inputs.channel,
                inputs.sender_id,
            )
            .await;
            let registry = &inputs.resources.mcp.messaging_strings_registry;
            email.map_or_else(
                || registry.render(key, &locale, &[&connect_url]),
                |email| {
                    registry.render(
                        KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
                        &locale,
                        &[&connect_url, &email],
                    )
                },
            )
        } else {
            inputs
                .resources
                .mcp
                .messaging_strings_registry
                .get(key, &locale)
        }
    } else {
        inputs
            .err
            .to_channel_reply(
                &inputs.resources.mcp.messaging_strings_registry,
                DEFAULT_LOCALE,
                "channel_auth",
            )
            .0
    };

    let mut message = OutgoingMessage {
        channel_type: inputs.channel_type,
        recipient_id: inputs.sender_id.to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: inputs.thread_id,
    };
    apply_conversation_recipient(&mut message, inputs.conversation_id);
    message
}
