// ABOUTME: In-chat provider-connect — mint a connect link-token in-process and build a tappable Card
// ABOUTME: Shared by the no-provider auto-prompt, the /connect command, and the connect_provider tool
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! In-chat provider connection entry point.
//!
//! The platform's own messaging pipeline already resolves the sender to a
//! `user_id` + the user's own `active_tenant_id` on every turn, so it can mint a
//! generic connect link-token **in-process** (no admin-authed HTTP round-trip)
//! and hand the user a `/providers/connect?token=...` URL as a tappable Card
//! button. The hosted page collects the password / OAuth consent over TLS — it
//! never enters the chat transcript.
//!
//! **Security: direct-message only.** A connect link is scoped to one `user_id`;
//! anyone who opens it within the 20-minute window could attach *their* provider
//! to that account. We therefore only emit the tokenized link in a direct
//! message (never a shared group), and the caller falls back to the plain web
//! link for group contexts.

use pierre_core::models::messaging::{CardAction, ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::TenantRepository;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use pierre_middleware::provider_link_token::mint_connect_link_token;
use tracing::warn;
use uuid::Uuid;

use pierre_contremaitre::messaging_strings::{KEY_CONNECT_BUTTON, KEY_CONNECT_PROMPT};

use crate::mcp::resources::ServerContext;

/// Resolve `(user_id, active_tenant_id)` for a channel sender, mirroring
/// `McpAuthMiddleware::authenticate_channel`: the channel link gives the
/// `user_id`, and the user's first/active tenant is the scope under which a
/// provider connection must be stored. Falls back to the inbound tenant if the
/// per-user tenant lookup is empty.
async fn resolve_user_and_tenant(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
) -> Option<(Uuid, Uuid)> {
    let link = resources
        .common
        .repos
        .messaging
        .get_channel_link(tenant_id, channel, sender_id)
        .await
        .ok()
        .flatten()?;
    let user_id = Uuid::parse_str(link.get("user_id")?.as_str()?).ok()?;

    let tenants_repo: &dyn TenantRepository = resources.common.repos.tenants.as_ref();
    let active_tenant = tenants_repo
        .list_for_user(user_id)
        .await
        .ok()
        .and_then(|ts| ts.first().map(|t| t.id.as_uuid()))
        .unwrap_or_else(|| tenant_id.as_uuid());
    Some((user_id, active_tenant))
}

/// Mint a connect link-token (for an already-resolved identity) and build the
/// hosted connect URL, or `None` if signing fails.
fn mint_connect_url(
    resources: &ServerContext,
    user_id: Uuid,
    active_tenant: Uuid,
    channel: &str,
    channel_thread: Option<&str>,
) -> Option<String> {
    let token = mint_connect_link_token(
        user_id,
        active_tenant,
        channel,
        channel_thread,
        &resources.auth.admin_jwt_secret,
    )
    .map_err(|e| warn!(error = %e, "Failed to mint connect link-token"))
    .ok()?;

    let base_url = &resources.common.config.base_url;
    Some(format!(
        "{base_url}/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
}

/// Build a tappable "Connect your account" Card for a **known** identity (the
/// `/connect` command + the connect tool both already hold the authenticated
/// `user_id` + the user's own tenant). Returns `None` for group contexts
/// (security: never post a user-scoped link in a shared room) or when minting
/// fails (the caller falls back to the plain web link).
///
/// `body` is the localized prompt; the URL rides a `url` [`CardAction`], which
/// renders as a native button on Telegram/Slack/Discord/Messenger and degrades
/// to a tappable link on `WhatsApp`.
#[allow(clippy::too_many_arguments)]
pub fn build_connect_card_direct(
    resources: &ServerContext,
    user_id: Uuid,
    active_tenant: Uuid,
    channel: &str,
    channel_type: ChannelType,
    recipient_id: &str,
    thread_id: Option<String>,
    is_direct_message: bool,
    locale: &str,
) -> Option<OutgoingMessage> {
    if !is_direct_message {
        return None;
    }
    let connect_url = mint_connect_url(
        resources,
        user_id,
        active_tenant,
        channel,
        thread_id.as_deref(),
    )?;

    let registry = &resources.mcp.messaging_strings_registry;
    let body = registry.get(KEY_CONNECT_PROMPT, locale);
    let button_label = registry.get(KEY_CONNECT_BUTTON, locale);

    Some(OutgoingMessage {
        channel_type,
        recipient_id: recipient_id.to_owned(),
        content: MessageContent::Card {
            title: String::new(),
            body,
            actions: vec![CardAction {
                label: button_label,
                action_type: "url".to_owned(),
                value: connect_url,
            }],
        },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id,
    })
}

/// Build a connect Card for the auth-denial path, which only has the sender's
/// channel id + the bot/channel `tenant_id` (the authenticated `AuthResult` was
/// already replaced by the `NoProviderConnected` error). Resolves the identity
/// from the channel link, then delegates to [`build_connect_card_direct`].
#[allow(clippy::too_many_arguments)]
pub async fn try_build_connect_card(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    channel_type: ChannelType,
    sender_id: &str,
    thread_id: Option<String>,
    is_direct_message: bool,
    locale: &str,
) -> Option<OutgoingMessage> {
    if !is_direct_message {
        return None;
    }
    let (user_id, active_tenant) =
        resolve_user_and_tenant(resources, tenant_id, channel, sender_id).await?;
    build_connect_card_direct(
        resources,
        user_id,
        active_tenant,
        channel,
        channel_type,
        sender_id,
        thread_id,
        is_direct_message,
        locale,
    )
}
