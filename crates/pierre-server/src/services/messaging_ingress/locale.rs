// ABOUTME: Messaging-turn locale resolution (channel link locale -> user profile -> default)
// ABOUTME: The surface's starting point; the turn service refines it from the message's language
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use pierre_services::locale::resolve_user_locale;

/// Resolve the user-facing locale for a messaging turn.
///
/// Walks the documented fallback chain:
///
/// 1. `messaging_channel_links.locale` for `(tenant, channel, channel_user_id)`
///    — explicit per-channel override (user set Telegram to EN while keeping
///    the web app in FR, for example)
/// 2. `users.locale` — the profile-wide preference edited from the Settings UI
/// 3. the platform default locale — the terminal rung, hard-coded
///    French fallback
///
/// Never fails: any DB error silently degrades to the next rung. Called once
/// per command/dispatch so handlers and chat-pipeline stages work with a
/// single resolved `String` instead of re-querying.
///
/// This is the athlete's *stored* preference for the channel, which is what a
/// platform string outside a turn (an OTP prompt, an error apology, a connect
/// card) is written in. A coaching turn refines it from the language of the
/// message itself — see
/// [`pierre_chat_pipeline::turn_service::detect_turn_locale`].
pub async fn resolve_messaging_locale(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: Uuid,
    channel_type: &str,
    channel_user_id: &str,
) -> String {
    if let Ok(Some(override_locale)) = resources
        .common
        .repos
        .messaging
        .get_channel_link_locale(tenant_id, channel_type, channel_user_id)
        .await
    {
        if !override_locale.trim().is_empty() {
            return override_locale;
        }
    }

    // Rungs 2 and 3 are the platform-wide question — "what language does this
    // athlete read" — so they are the shared resolver, not a second copy of it.
    resolve_user_locale(resources.common.repos.users.as_ref(), user_id).await
}
