// ABOUTME: Channel-linking commands ("/start <code>", "LINK <code>") for messaging ingress
// ABOUTME: Plus analytics-consent cache hydration triggered when a freshly linked user appears

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;

use pierre_core::models::messaging::{ChannelType, MessageContent, OutgoingMessage};
use pierre_core::models::TenantId;
use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_LINK_IDENTITY_COLLISION, KEY_LINK_SESSION_EXPIRED, KEY_LINK_SUCCESS,
};
use crate::mcp::resources::ServerContext;
use crate::services::analytics::{analytics, hash_id};
use crate::services::user_status_gate::messaging_key_for_status;

use super::locale::resolve_messaging_locale;

/// Result of checking an inbound message for a channel linking command
pub(super) enum LinkingAction {
    /// Message contains a linking command — handle it and do not dispatch to LLM
    LinkCode(String),
    /// Normal message — proceed with standard routing
    Normal,
}

/// Detect if an inbound message contains a channel linking command
///
/// `Telegram`: `/start {code}` — bot deep link with verification code
/// `WhatsApp`: `LINK {code}` — text message with verification code
pub(super) fn detect_linking_code(
    channel_type: ChannelType,
    content: &MessageContent,
) -> LinkingAction {
    let text = match content {
        MessageContent::Text { body } => body.as_str(),
        _ => return LinkingAction::Normal,
    };

    match channel_type {
        ChannelType::Telegram => {
            if let Some(code) = text.strip_prefix("/start ") {
                let code = code.trim();
                if !code.is_empty() {
                    return LinkingAction::LinkCode(code.to_owned());
                }
            }
        }
        ChannelType::WhatsApp => {
            if let Some(code) = text.strip_prefix("LINK ") {
                let code = code.trim();
                if !code.is_empty() {
                    return LinkingAction::LinkCode(code.to_owned());
                }
            }
        }
        _ => {}
    }

    LinkingAction::Normal
}

/// Enumerated failure reasons from [`execute_link_code`].
///
/// Held as a structured variant (rather than a pre-formatted `String`) so the
/// user-facing translation can happen at the outer boundary where the
/// `MessagingStringsRegistry` is available.
enum DeepLinkError {
    /// `consume_link_state` returned an error — expired or already used code.
    SessionExpired(String),
    /// Creating the channel link row failed (most commonly a uniqueness
    /// collision on the `(tenant, channel_type, channel_user_id)` triple).
    IdentityCollision(String),
}

/// Consume a link code and create the permanent channel link, returning the user ID
async fn execute_link_code(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> Result<String, DeepLinkError> {
    let link_state = db
        .consume_link_state(code, tenant_id)
        .await
        .map_err(|e| DeepLinkError::SessionExpired(e.to_string()))?;

    let user_id = link_state["user_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let link_id = Uuid::new_v4().to_string();

    let link_params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id,
        user_id: &user_id,
        channel_type: channel,
        channel_user_id: sender_id,
        display_name: None,
    };

    db.create_channel_link(&link_params)
        .await
        .map_err(|e| DeepLinkError::IdentityCollision(e.to_string()))?;

    Ok(user_id)
}

/// Consume a link code and create the permanent channel link
///
/// Returns a user-facing message describing the result, translated through
/// the messaging-strings registry.
/// Build the reply body for the moment a fresh channel link gets created.
///
/// Calls the unified channel-authentication path (same call every inbound
/// message after this will hit) and translates the outcome into a link-time
/// reply:
/// - `Ok(_)` → [`KEY_LINK_SUCCESS`] (locale resolved from the auth result).
/// - `AccountPending` / `AccountSuspended` → translated denial copy.
/// - Any other auth error → falls back to [`KEY_LINK_SUCCESS`] (the link did
///   persist, the transient auth-side error doesn't justify scaring the user
///   off; the next inbound message will retry the gate).
pub(super) async fn link_time_reply(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
) -> String {
    let reg = &resources.messaging_strings_registry;
    match resources
        .auth_middleware
        .authenticate_channel(tenant_id, channel, sender_id)
        .await
    {
        Ok(auth_result) => {
            let locale = resolve_messaging_locale(
                resources,
                tenant_id,
                auth_result.user_id,
                channel,
                sender_id,
            )
            .await;
            reg.get(KEY_LINK_SUCCESS, &locale)
        }
        Err(e) => {
            if let Some(key) = messaging_key_for_status(e.code) {
                let locale = resources
                    .repos
                    .messaging
                    .get_channel_link_locale(tenant_id, channel, sender_id)
                    .await
                    .ok()
                    .flatten()
                    .filter(|l| !l.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_LOCALE.to_owned());
                reg.get(key, &locale)
            } else {
                // Operator-category failure (DB blip, etc.). Log and fall
                // back to the link-success template — the link is real, the
                // next inbound message hits the gate again.
                warn!(error = %e, channel = %channel, "Link-time auth check failed; using success template");
                reg.get(KEY_LINK_SUCCESS, DEFAULT_LOCALE)
            }
        }
    }
}

async fn consume_and_link(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> String {
    let reg = &resources.messaging_strings_registry;
    match execute_link_code(db, tenant_id, channel, sender_id, code).await {
        Ok(user_id) => {
            info!(channel = %channel, user_id = %user_id, channel_user_id = %sender_id, "Channel linked via deep link");
            let hashed_tenant = hash_id(&tenant_id.to_string());
            let hashed_user = hash_id(&user_id);
            let hashed_channel_id = hash_id(&format!("{channel}:{sender_id}"));
            analytics().alias(&hashed_channel_id, &hashed_user);
            analytics().track_linking_completed(channel, &hashed_tenant, &hashed_user, "deep_link");
            // Run the unified channel-auth path on the freshly-minted link so
            // the immediate reply reflects approval state exactly as every
            // subsequent inbound message will.
            link_time_reply(resources, tenant_id, channel, sender_id).await
        }
        Err(err) => {
            let (key, detail) = match &err {
                DeepLinkError::SessionExpired(d) => (KEY_LINK_SESSION_EXPIRED, d.as_str()),
                DeepLinkError::IdentityCollision(d) => (KEY_LINK_IDENTITY_COLLISION, d.as_str()),
            };
            warn!(error = %detail, "Channel linking failed");
            analytics().track_linking_failed(channel, &hash_id(&tenant_id.to_string()), detail);
            reg.get(key, DEFAULT_LOCALE)
        }
    }
}

/// Handle a channel linking command: consume the code and create the link
///
/// Returns an outgoing confirmation or error message to send back to the user.
pub(super) async fn handle_linking_command(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel: &str,
    sender_id: &str,
    code: &str,
) -> OutgoingMessage {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let channel_type = ChannelType::from_str(channel).unwrap_or(ChannelType::Telegram);
    let response_text = consume_and_link(resources, db, tenant_id, channel, sender_id, code).await;

    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text {
            body: response_text,
        },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}

/// Hydrate the analytics consent cache for a messaging user on cache miss
///
/// The cache is in-memory and empties on every Cloud Run cold start, so each
/// fresh pod needs to learn each user's durable `analytics_consent` value from
/// the database before their events will be captured. Once hydrated the entry
/// persists for the life of the pod and `/privacy on|off` commands keep it
/// current via `set_consent`.
pub(super) async fn hydrate_analytics_consent(resources: &ServerContext, user_id: &str) {
    let hashed_user = hash_id(user_id);
    if analytics().has_consent_cached(&hashed_user) {
        return;
    }
    let Ok(parsed) = Uuid::parse_str(user_id) else {
        return;
    };
    match resources.repos.users.get_global(parsed).await {
        Ok(Some(user)) => {
            analytics().hydrate_consent(&hashed_user, user.analytics_consent);
        }
        Ok(None) => {}
        Err(e) => {
            error!(error = %e, user_id = %user_id, "Failed to load user for analytics consent hydration");
        }
    }
}
