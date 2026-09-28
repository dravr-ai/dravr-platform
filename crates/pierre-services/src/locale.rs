// ABOUTME: One resolver for "what language does this athlete read" — tenant-scoped, with a default
// ABOUTME: Also the channel chain: per-channel link override, then the athlete's profile, then the default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete locale resolution.
//!
//! Ten call sites asked the same question — REST handlers, chat routes, store
//! and memory tools, the persona cards — and answered it ten times. The copies
//! were not identical: only the agents one treated a stored empty string as
//! "no preference", so every other site would have handed `""` to the string
//! registry as if it were a locale.
//!
//! The read is global rather than tenant-scoped, which is the one place these
//! copies disagreed and the disagreement mattered. `locale` is a column on the
//! user row: one athlete, one stored language, whatever tenant they are acting
//! in. The tenant-scoped read is an `INNER JOIN tenant_users`, so a missing
//! membership row does not return "no tenant access" — it returns "no
//! preference", and the athlete silently reads French. Every caller here
//! resolves the language of the *authenticated caller themselves*, so there is
//! no cross-tenant read to guard against, and answering the question with the
//! join risks the wrong language for no isolation gain.

use pierre_contremaitre::messaging_strings::DEFAULT_LOCALE;
use pierre_core::models::{TenantId, User};
use pierre_database::repositories::{MessagingRepository, UserRepository};
use uuid::Uuid;

/// The locale an athlete reads, or [`DEFAULT_LOCALE`] when they have no usable
/// preference.
///
/// A missing row, a failed lookup and a stored empty string all mean the same
/// thing to a caller — "we do not know, use the default" — so all three land on
/// the default rather than three different behaviours per call site.
pub async fn resolve_user_locale(users: &dyn UserRepository, user_id: Uuid) -> String {
    let user = users.get_global(user_id).await.ok().flatten();
    user_locale(user.as_ref())
}

/// The locale of a user row the caller already holds — the rule
/// [`resolve_user_locale`] applies, for a caller that read many users in one
/// batch query instead of one lookup each.
#[must_use]
pub fn user_locale(user: Option<&User>) -> String {
    user.map(|user| user.locale.as_str())
        .filter(|locale| !locale.trim().is_empty())
        .unwrap_or(DEFAULT_LOCALE)
        .to_owned()
}

/// The locale an athlete reads on one messaging channel.
///
/// Walks the one fallback chain every channel surface uses:
///
/// 1. `messaging_channel_links.locale` for `(tenant, channel, channel_user_id)`
///    — an explicit per-channel override (Telegram in English while the web
///    app stays in French, for example)
/// 2. the profile-wide preference, through [`resolve_user_locale`], when the
///    Pierre user behind the channel identity is known (`user_id`)
/// 3. [`DEFAULT_LOCALE`]
///
/// Never fails: an unreadable rung degrades to the next. This is the athlete's
/// *stored* preference for the channel, which is what a platform string
/// outside a turn (an OTP prompt, an error apology, a connect card, a
/// scheduled notice) is written in; a coaching turn refines it from the
/// language of the message itself.
pub async fn resolve_channel_locale(
    messaging: &dyn MessagingRepository,
    users: &dyn UserRepository,
    tenant_id: TenantId,
    channel_type: &str,
    channel_user_id: &str,
    user_id: Option<Uuid>,
) -> String {
    if let Ok(Some(override_locale)) = messaging
        .get_channel_link_locale(tenant_id, channel_type, channel_user_id)
        .await
    {
        if !override_locale.trim().is_empty() {
            return override_locale;
        }
    }
    match user_id {
        Some(user_id) => resolve_user_locale(users, user_id).await,
        None => DEFAULT_LOCALE.to_owned(),
    }
}
