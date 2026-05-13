// ABOUTME: Messaging-turn locale resolution (channel link locale -> user profile -> default)
// ABOUTME: Plus content-language detection used by status placeholders + scope-refusal text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::contremaitre::messaging_strings::DEFAULT_LOCALE;
use crate::mcp::resources::ServerContext;

/// Resolve the user-facing locale for a messaging turn.
///
/// Walks the documented fallback chain:
///
/// 1. `messaging_channel_links.locale` for `(tenant, channel, channel_user_id)`
///    — explicit per-channel override (user set Telegram to EN while keeping
///    the web app in FR, for example)
/// 2. `users.locale` — the profile-wide preference edited from the Settings UI
/// 3. [`crate::contremaitre::messaging_strings::DEFAULT_LOCALE`] — hard-coded
///    French fallback
///
/// Never fails: any DB error silently degrades to the next rung. Called once
/// per command/dispatch so handlers and chat-pipeline stages work with a
/// single resolved `String` instead of re-querying.
pub async fn resolve_messaging_locale(
    resources: &ServerContext,
    tenant_id: TenantId,
    user_id: Uuid,
    channel_type: &str,
    channel_user_id: &str,
) -> String {
    if let Ok(Some(override_locale)) = resources
        .repos
        .messaging
        .get_channel_link_locale(tenant_id, channel_type, channel_user_id)
        .await
    {
        if !override_locale.trim().is_empty() {
            return override_locale;
        }
    }

    if let Ok(Some(user)) = resources.repos.users.get_global(user_id).await {
        if !user.locale.trim().is_empty() {
            return user.locale;
        }
    }

    DEFAULT_LOCALE.to_owned()
}

/// Detect the turn's *content* locale from the raw user text.
///
/// Used for text that must match the LLM reply's language — status
/// placeholders ("thinking…" / "réflexion…"), scope-refusal
/// interpolation, guardrail fallbacks. When detection succeeds and the
/// language is one of our supported locales (`fr`, `en`, `es`, `de`,
/// `pt`), returns that BCP-47 short code. Otherwise returns `fallback`
/// (normally the user's stored `users.locale`, which is already the
/// right default for OTP/error flows that should stay consistent).
///
/// Short messages (<12 chars) skip detection because whatlang's signal
/// is unreliable on tiny samples ("ok", "oui") and the fallback is
/// almost always what the user wants anyway.
#[must_use]
pub fn detect_turn_locale(text: &str, fallback: &str) -> String {
    const MIN_LEN: usize = 12;
    if text.trim().chars().count() < MIN_LEN {
        return fallback.to_owned();
    }
    let Some(info) = whatlang::detect(text) else {
        return fallback.to_owned();
    };
    if !info.is_reliable() {
        return fallback.to_owned();
    }
    match info.lang() {
        whatlang::Lang::Fra => "fr".to_owned(),
        whatlang::Lang::Eng => "en".to_owned(),
        whatlang::Lang::Spa => "es".to_owned(),
        whatlang::Lang::Deu => "de".to_owned(),
        whatlang::Lang::Por => "pt".to_owned(),
        _ => fallback.to_owned(),
    }
}
