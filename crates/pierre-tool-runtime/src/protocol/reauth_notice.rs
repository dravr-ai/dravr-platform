// ABOUTME: The one notice telling an athlete a provider connection needs reconnecting, whichever path flagged it
// ABOUTME: Deduped per needs_reauth transition; reaches the app (push + notification row) and every linked chat

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The reconnect notice.
//!
//! Four paths can find a connection dead: an OAuth refresh the provider
//! refused, a delegated read that found the coach's session gone, a detached
//! historical backfill, and the capture sweep. Each flips the connection to
//! `needs_reauth`, and each owes the athlete the same thing: one notice that
//! the connection needs reconnecting, where they will see it.
//!
//! [`notify_needs_reauth`] is that notice, written once. It claims the
//! connection's `notified_at` marker first, so however many paths and replicas
//! observe the same dead connection, it is sent once per transition; the
//! marker is cleared when the connection returns to `active`, which re-arms
//! it. The notice goes to two places:
//!
//! - the app: a P1 push and the stored notification row both clients list,
//!   routed to the connections screen;
//! - every messaging channel the athlete linked, through the runtime's
//!   [`crate::runtime::BackfillNotifier`]: the reconnect sentence with a
//!   one-time sign-in link minted for that channel. A channel whose send fails
//!   is queued for the messaging outbound retry worker on its own — the app
//!   half and the channels already reached are not sent again — and retried
//!   with backoff while the connection is still `needs_reauth` and its link has
//!   time left to run.
//!
//! The app half is dispatched without the platform's chat fan-out, because
//! the chat half carries the link and a link token has no place in a stored
//! notification row. A notice the notification pipeline suppresses (the
//! athlete's own preferences, quiet hours, the daily cap) is suppressed in
//! chat as well, as the fan-out would have been.
//!
//! The push is worded from the catalogue in the athlete's locale. A dead
//! usr/pwd Garmin session that served the athlete alone, while a seat is free
//! on Garmin's OAuth ([`reconnects_through_oauth`]), is told to move to
//! Garmin's official connection instead of signing in again (carnet#1454).
//!
//! [`flag_needs_reauth`] is the flip plus the notice, for the paths that flag
//! through [`ProviderConnectionRepository::mark_needs_reauth`]. The OAuth
//! refresh path flips through a token-guarded write of its own and calls
//! [`notify_needs_reauth`] directly when it flipped.
//!
//! [`ProviderConnectionRepository::mark_needs_reauth`]: pierre_database::repositories::ProviderConnectionRepository::mark_needs_reauth

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_auth::garmin_seats::reconnects_through_garmin_oauth;
#[cfg(feature = "client-notifications")]
use pierre_contremaitre::reauth_strings::{
    KEY_NOTIFICATION_PROVIDER_OAUTH_REQUIRED_BODY, KEY_NOTIFICATION_PROVIDER_OAUTH_REQUIRED_TITLE,
    KEY_NOTIFICATION_PROVIDER_REAUTH_BODY, KEY_NOTIFICATION_PROVIDER_REAUTH_TITLE,
};
use pierre_core::errors::{AppError, AppResult};
#[cfg(feature = "client-notifications")]
use pierre_core::models::NotificationScreen;
use pierre_core::models::{ReauthMark, TenantId};
#[cfg(feature = "client-notifications")]
use pierre_notifications::models::{NotificationCategory, TransportPolicy as PushTransportPolicy};
#[cfg(feature = "client-notifications")]
use pierre_notifications::{DispatchOutcome, DispatchRequest, PushTier, TenantId as CommTenantId};
use pierre_providers::backend_resolver::{backend_pair_for, session_beside_grant};
#[cfg(feature = "client-notifications")]
use pierre_providers::backend_resolver::{brand_name, user_facing_name};
#[cfg(feature = "client-notifications")]
use pierre_services::locale::resolve_user_locale;
#[cfg(feature = "client-notifications")]
use serde_json::Value as JsonValue;
use tracing::{info, warn};
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// What one call to [`notify_needs_reauth`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReauthNotice {
    /// Whether this call won the connection's one notice for the current
    /// `needs_reauth` transition. `false` when another caller already sent it,
    /// or the connection is not `needs_reauth`.
    pub claimed: bool,
    /// Messaging channels the notice reached on its first send. Zero when it
    /// was not claimed, the pipeline suppressed it, the athlete linked no
    /// channel, or messaging is not wired. A channel whose send failed is not
    /// counted: it was queued for retry.
    pub chat_channels: usize,
}

/// Send the reconnect notice for `(user_id, tenant_id, provider)` once per
/// `needs_reauth` transition.
///
/// Claims the connection's notice marker first and sends nothing without it:
/// a connection that is not `needs_reauth`, or whose notice another caller
/// already sent, is left alone. With the claim, the notice goes to the app
/// (push and stored row) and to every linked messaging channel, as the module
/// doc describes. Best-effort: every failure is logged and the flag stands
/// either way.
///
/// A usr/pwd session beside a grant that serves its provider
/// ([`session_beside_grant`]) is never surfaced: the grant is the ongoing
/// source, so its session dying asks nothing of the athlete. Nothing is
/// claimed or sent for it; its flag stands (carnet#1386).
pub async fn notify_needs_reauth(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> ReauthNotice {
    if session_beside_grant(runtime.repos(), user_id, tenant_id, provider).await {
        info!(
            user_id = %user_id,
            provider = %provider,
            "Reconnect notice: the session stands beside a serving grant; nothing sent"
        );
        return ReauthNotice::default();
    }
    if !claim_notice(runtime, user_id, tenant_id, provider).await {
        return ReauthNotice::default();
    }

    // Without the notification backend there is no app half, and nothing to
    // suppress the chat half.
    #[cfg(feature = "client-notifications")]
    let suppressed = suppressed_in_app(runtime, user_id, tenant_id, provider).await;
    #[cfg(not(feature = "client-notifications"))]
    let suppressed = false;

    let chat_channels = if suppressed {
        info!(
            user_id = %user_id,
            provider = %provider,
            "Reconnect notice suppressed by the notification pipeline; no chat copy sent"
        );
        0
    } else {
        send_chat_half(runtime, user_id, tenant_id, provider).await
    };
    ReauthNotice {
        claimed: true,
        chat_channels,
    }
}

/// Whether the athlete reconnects `backend` through its provider's OAuth
/// rather than its usr/pwd sign-in (carnet#1454).
///
/// True for a `sciotte_garmin` session the athlete made themselves (never a
/// coach's delegated one) while they hold no Garmin grant in this tenant and
/// a seat is free on the server's Garmin client while its OAuth goes first
/// (carnet#1472) — the one rule
/// [`reconnects_through_garmin_oauth`] reads. With the pool full, or for any
/// other backend, the reconnect is the ordinary one.
pub async fn reconnects_through_oauth(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    reconnects_through_garmin_oauth(
        runtime.repos().oauth_tokens.as_ref(),
        runtime.repos().provider_connections.as_ref(),
        runtime.repos().system_settings.as_ref(),
        &runtime.config().oauth,
        user_id,
        tenant_id,
        backend,
    )
    .await
}

/// Claim the connection's one notice for its current `needs_reauth`
/// transition. A claim that cannot be written reads as lost, so nothing is
/// sent: a second notice is worse than a late one the next failure sends.
async fn claim_notice(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    runtime
        .repos()
        .provider_connections
        .claim_reauth_notification(user_id, tenant_id, provider)
        .await
        .unwrap_or_else(|e| {
            warn!(
                user_id = %user_id,
                provider = %provider,
                error = %e,
                "Reconnect notice: the notice claim failed; nothing sent"
            );
            false
        })
}

/// Send the chat half through the runtime's messaging notifier, and report how
/// many linked channels it reached; zero when messaging is not wired.
async fn send_chat_half(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> usize {
    let Some(notifier) = runtime.backfill_notifier() else {
        return 0;
    };
    let chat_channels = notifier
        .push_reauth_to_linked_channels(user_id, tenant_id, provider)
        .await;
    info!(
        user_id = %user_id,
        provider = %provider,
        chat_channels,
        "Reconnect notice sent"
    );
    chat_channels
}

/// Flag a connection an attempt found dead, and send its reconnect notice.
///
/// Flips `(user_id, tenant_id, provider)` to `needs_reauth` for an attempt
/// that began at `attempt_started_at`, recording `reason`. The notice follows [`ReauthMark::AlreadyFlagged`] as well as
/// [`ReauthMark::Flagged`]: the claim inside [`notify_needs_reauth`] is what
/// keeps it to one per transition, so a connection flagged by a path that did
/// not notify is told on the next failure, and one already told is not told
/// again. A connection reconnected since the attempt began, or removed, is
/// sent nothing.
///
/// # Errors
///
/// Returns the repository error when the flip itself cannot be written; no
/// notice is sent then.
pub async fn flag_needs_reauth(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    reason: &str,
    attempt_started_at: DateTime<Utc>,
) -> AppResult<(ReauthMark, ReauthNotice)> {
    let mark = runtime
        .repos()
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant_id,
            provider,
            Some(reason),
            attempt_started_at,
        )
        .await?;
    let notice = match mark {
        ReauthMark::Flagged | ReauthMark::AlreadyFlagged => {
            notify_needs_reauth(runtime, user_id, tenant_id, provider).await
        }
        ReauthMark::ReconnectedSince | ReauthMark::NoConnection => ReauthNotice::default(),
    };
    Ok((mark, notice))
}

/// Dispatch the app half of the notice — a P1 push and the stored row on the
/// connections screen — and report whether the pipeline suppressed it.
///
/// P1: an expired connection blocks every downstream feature, so it outranks
/// advisories, but it is recoverable, not break-glass P0. A failed dispatch is
/// logged and reads as not suppressed: the pipeline did not decide against
/// the notice, so the chat half still goes out.
#[cfg(feature = "client-notifications")]
async fn suppressed_in_app(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    let Some(service) = runtime.notification_service() else {
        return false;
    };
    let data = [
        ("type", "provider_needs_reauth"),
        ("provider", provider),
        // Named through the shared vocabulary, not as a loose string: the
        // clients resolve `data.screen` to a surface through the same enum,
        // so a token nothing routes cannot be emitted here.
        ("screen", NotificationScreen::Connections.as_str()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), JsonValue::String(v.to_owned())))
    .collect();
    // The body names the provider as the user knows it: a mirror backend's
    // slug (`sciotte_trainingpeaks`) is internal.
    let brand = brand_name(runtime.provider_registry(), provider)
        .unwrap_or_else(|| user_facing_name(provider));
    let (title_key, body_key) =
        if reconnects_through_oauth(runtime, user_id, tenant_id, provider).await {
            (
                KEY_NOTIFICATION_PROVIDER_OAUTH_REQUIRED_TITLE,
                KEY_NOTIFICATION_PROVIDER_OAUTH_REQUIRED_BODY,
            )
        } else {
            (
                KEY_NOTIFICATION_PROVIDER_REAUTH_TITLE,
                KEY_NOTIFICATION_PROVIDER_REAUTH_BODY,
            )
        };
    let locale = resolve_user_locale(runtime.repos().users.as_ref(), user_id).await;
    let strings = runtime.messaging_strings_registry();
    let request = DispatchRequest {
        user_id,
        tenant_id: CommTenantId(tenant_id.as_uuid()),
        category: NotificationCategory::System,
        notification_type: "provider_needs_reauth".to_owned(),
        title: strings.render(title_key, &locale, &[brand]),
        body: strings.render(body_key, &locale, &[brand]),
        data: Some(JsonValue::Object(data)),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
        transport_policy: PushTransportPolicy::AnyTransport,
    };
    match service
        .dispatch_in_app_with_tier(&request, PushTier::P1)
        .await
    {
        Ok(outcome) => matches!(outcome, DispatchOutcome::Suppressed(_)),
        Err(e) => {
            warn!("Failed to dispatch reauth push for user {user_id} provider {provider}: {e}");
            false
        }
    }
}

/// The connection an auth-shaped failure of a `provider_slug` read names, or
/// `None` when `error` is not one.
///
/// The backend the error carries when it is one of `provider_slug`'s own
/// ([`backend_pair_for`]): a `garmin` read the athlete's username-and-password
/// session served fails as `sciotte_garmin`, and that is the connection to
/// flag, not the Garmin grant beside it. Any other slug (the scraper's unnamed
/// `sciotte`) falls back to `provider_slug`.
#[must_use]
pub(crate) fn refused_backend(error: &AppError, provider_slug: &str) -> Option<String> {
    let named = error.provider_auth_required_provider()?;
    let own = backend_pair_for(provider_slug).contains(&named);
    Some(if own { named } else { provider_slug.to_owned() })
}

/// [`flag_needs_reauth`] for the connection `refused` an attempt found dead,
/// recording `reason` and when the attempt began.
///
/// A flip that cannot be written is logged: the caller serves what it holds
/// either way.
pub(crate) async fn flag_refused(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    refused: &str,
    reason: &str,
    attempt_started_at: DateTime<Utc>,
) {
    if let Err(e) = flag_needs_reauth(
        runtime,
        user_id,
        tenant_id,
        refused,
        reason,
        attempt_started_at,
    )
    .await
    {
        warn!(
            user_id = %user_id,
            provider = %refused,
            error = %e,
            "could not flag the refused connection"
        );
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::constants::oauth_providers;
    use pierre_core::errors::AppError;

    use super::refused_backend;

    /// The auth error names the backend that refused, which a `garmin`
    /// read's caller flags, prompts and mints its reconnect link for.
    #[test]
    fn a_garmin_read_the_session_refused_names_the_session() {
        let refused = AppError::provider_auth_required(oauth_providers::SCIOTTE_GARMIN);
        assert_eq!(
            refused_backend(&refused, oauth_providers::GARMIN).as_deref(),
            Some(oauth_providers::SCIOTTE_GARMIN),
            "bug: a refusal of Garmin's own session must name that session, not the grant"
        );
        // The scraper's unnamed slug is not one of Garmin's: the read's own
        // name stands.
        let unnamed = AppError::provider_auth_required("sciotte");
        assert_eq!(
            refused_backend(&unnamed, oauth_providers::GARMIN).as_deref(),
            Some(oauth_providers::GARMIN),
            "bug: a backend outside Garmin's pair must not be the one flagged"
        );
        let other = AppError::internal("boom");
        assert_eq!(
            refused_backend(&other, oauth_providers::GARMIN),
            None,
            "bug: an error that is no auth refusal must flag nothing"
        );
    }
}
