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
//! The push is worded from the catalogue in the athlete's locale. Every
//! reconnect goes through the provider's hosted sign-in, a dead OAuth grant's
//! included (`backend_resolver::reconnect_target`, carnet#1494). A session
//! that dies while its live grant keeps the provider syncing is told so, in
//! the chat half's wording. A grant that dies beside a session that serves
//! is not surfaced, since the session serves the provider, unless only that
//! grant writes what the athlete asks of the provider (Garmin's workouts and
//! calendar): the athlete is then told, once, that the reads go on and the
//! writes need the grant again, with the grant's own authorization link
//! (JF's directive 8). A grant that dies beside a session that died first
//! leaves nothing serving, and that session's notice said the grant kept the
//! provider syncing: the athlete is told the ordinary reconnect, once.
//!
//! [`flag_needs_reauth`] is the flip plus the notice, for the paths that flag
//! through [`ProviderConnectionRepository::mark_needs_reauth`]. The OAuth
//! refresh path flips through a token-guarded write of its own and calls
//! [`notify_needs_reauth`] directly when it flipped.
//!
//! [`ProviderConnectionRepository::mark_needs_reauth`]: pierre_database::repositories::ProviderConnectionRepository::mark_needs_reauth

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_auth::connect_order::{order_and_seats, sign_in_falls_back_to_oauth};
use pierre_auth::garmin_seats::garmin_seat_summary;
#[cfg(feature = "client-notifications")]
use pierre_contremaitre::reauth_strings::{
    KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_BODY, KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_TITLE,
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
#[cfg(feature = "client-notifications")]
use pierre_providers::backend_resolver::brand_name;
use pierre_providers::backend_resolver::{
    backend_pair_for, grant_writes_beside_serving_session, is_mirror_backend, reconnect_target,
    session_answers_for_grant, user_facing_name,
};
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
/// A dead OAuth grant the session beside it answers for
/// ([`session_answers_for_grant`]) is not surfaced: a session that serves
/// keeps the provider syncing, and one flagged after the grant was already
/// told the ordinary reconnect. Nothing is claimed or sent for it; its flag
/// stands, and it is never elected while it does (carnet#1494). The
/// exception is a grant that writes what its session cannot (Garmin's
/// workout and calendar push) beside a session that serves
/// ([`grant_writes_beside_serving_session`]): its athlete is told once,
/// through the same claim, that the reads go on and the writes need the
/// grant again, with the grant's own authorization link (JF's directive 8).
/// Such a write asked for meanwhile is still refused with that link
/// (`backend_resolver::grant_owed_refusal`). A grant that dies beside a
/// session flagged before it is told the ordinary reconnect, once: the
/// session's own notice said the grant kept the provider syncing, and
/// nothing serves the athlete any more. A session that dies beside a live
/// grant is told, once: the grant keeps the provider syncing for now, and
/// the chat half says so.
pub async fn notify_needs_reauth(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> ReauthNotice {
    let copy = if session_answers_for_grant(runtime.repos(), user_id, tenant_id, provider).await {
        if !writes_owed(runtime, user_id, tenant_id, provider).await {
            info!(
                user_id = %user_id,
                provider = %provider,
                "Reconnect notice: the session beside the grant answers for it; nothing sent"
            );
            return ReauthNotice::default();
        }
        NoticeCopy::GrantWrites
    } else {
        NoticeCopy::Reconnect
    };
    send_claimed_notice(runtime, user_id, tenant_id, provider, copy).await
}

/// Which wording a notice carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeCopy {
    /// The connection must be reconnected for the provider to keep syncing.
    Reconnect,
    /// The reads go on through the athlete's session; the writes only the
    /// grant makes need the grant again.
    GrantWrites,
}

/// Whether the dead or withdrawn grant `grant` stands beside a session that
/// serves its provider while only the grant writes the athlete's workouts
/// and calendar ([`grant_writes_beside_serving_session`]).
async fn writes_owed(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    grant: &str,
) -> bool {
    grant_writes_beside_serving_session(
        runtime.repos(),
        runtime.provider_registry(),
        user_id,
        tenant_id,
        grant,
    )
    .await
}

/// Claim the connection's one notice and, on the won claim, send it in the
/// app and on every linked chat, worded as `copy`.
async fn send_claimed_notice(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    copy: NoticeCopy,
) -> ReauthNotice {
    if !claim_notice(runtime, user_id, tenant_id, provider).await {
        return ReauthNotice::default();
    }

    // Without the notification backend there is no app half, and nothing to
    // suppress the chat half.
    #[cfg(feature = "client-notifications")]
    let suppressed = suppressed_in_app(runtime, user_id, tenant_id, provider, copy).await;
    #[cfg(not(feature = "client-notifications"))]
    let suppressed = {
        let _ = copy;
        false
    };

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

/// Whether a sign-in hand-off is a new connection that can fall back to its
/// provider's OAuth.
///
/// True for a mirror `backend` when the athlete holds neither half of its
/// pair in `tenant_id` (no token row, no connection row) and the operator's
/// order for its provider puts the
/// sign-in first while a seat is free on the provider's OAuth
/// ([`sign_in_falls_back_to_oauth`], carnet#1477). Its link is then the
/// connect picker, which carries that fallback; the bare sign-in page does
/// not. A provider with no OAuth Pierre can call has no seat, so it never
/// does (directive 5: the order applies to every provider).
///
/// A lookup that fails reads as a connection held, so the athlete keeps the
/// sign-in link they had.
pub async fn new_sign_in_falls_back(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    if !is_mirror_backend(backend) {
        return false;
    }
    let repos = runtime.repos();
    let pair = backend_pair_for(backend);
    for held in &pair {
        if !matches!(
            repos.oauth_tokens.get_token(user_id, tenant_id, held).await,
            Ok(None)
        ) {
            return false;
        }
    }
    let connected = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
        .map_or(true, |connections| {
            connections
                .iter()
                .any(|connection| pair.contains(&connection.provider))
        });
    if connected {
        return false;
    }
    let (order, seats) = order_and_seats(
        repos.system_settings.as_ref(),
        repos.oauth_tokens.as_ref(),
        repos.provider_connections.as_ref(),
        &runtime.config().oauth,
        user_facing_name(backend),
    )
    .await;
    sign_in_falls_back_to_oauth(order, seats)
}

/// `last_error` of a grant connection a withdrawal flags to claim its notice.
const GRANT_WITHDRAWN: &str = "grant_withdrawn";

/// Tell an athlete whose OAuth grant of `grant` is being withdrawn (a seat
/// reclaim, the provider's own revocation) to reconnect through the hosted
/// sign-in, when no session of that provider serves them.
///
/// Called before the withdrawal deletes the grant's rows (carnet#1494): the
/// grant's connection is flagged `needs_reauth` and its one notice claimed,
/// as any dead connection's is, and the notice is sent on the won claim. A
/// withdrawal that fails after its token delete has therefore already told
/// the athlete, and a redelivered event, a retried pass or a concurrent
/// delivery finds the claim taken and sends nothing. A grant with no
/// connection row left has nothing to claim and is told nothing. The notice
/// is the ordinary one, app half and chat half, with the hosted sign-in
/// link, and it is what a grant withdrawn beside a session that is itself
/// flagged sends: that session's notice said the grant kept the provider
/// syncing. A grant beside a session that serves
/// ([`session_answers_for_grant`]) asks nothing of the athlete, unless only
/// that grant writes their workouts and calendar (Garmin's)
/// ([`grant_writes_beside_serving_session`]): they are then told once that
/// the reads go on and the writes need the grant again, with its own
/// authorization link, while a seat is free on Garmin's client for them to
/// take ([`garmin_seat_free`]); with none, nothing is sent.
/// A backend that is no mirrored provider's grant is never told here. A chat
/// send that fails is queued as any reconnect notice is, guarded on a
/// connection the withdrawal then deletes, so it is not retried.
pub async fn notify_grant_withdrawn(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    grant: &str,
) -> ReauthNotice {
    if is_mirror_backend(grant) || reconnect_target(grant).is_none() {
        return ReauthNotice::default();
    }
    let copy = if session_answers_for_grant(runtime.repos(), user_id, tenant_id, grant).await {
        if !writes_owed(runtime, user_id, tenant_id, grant).await
            || !garmin_seat_free(runtime).await
        {
            info!(
                user_id = %user_id,
                provider = %grant,
                "Withdrawn grant: a signed-in session serves the provider; nothing sent"
            );
            return ReauthNotice::default();
        }
        NoticeCopy::GrantWrites
    } else {
        NoticeCopy::Reconnect
    };
    let mark = runtime
        .repos()
        .provider_connections
        .mark_needs_reauth(user_id, tenant_id, grant, Some(GRANT_WITHDRAWN), Utc::now())
        .await;
    match mark {
        Ok(ReauthMark::Flagged | ReauthMark::AlreadyFlagged) => {}
        Ok(ReauthMark::ReconnectedSince | ReauthMark::NoConnection) => {
            return ReauthNotice::default();
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %grant,
                error = %e,
                "Withdrawn grant: its connection could not be flagged; nothing sent"
            );
            return ReauthNotice::default();
        }
    }
    send_claimed_notice(runtime, user_id, tenant_id, grant, copy).await
}

/// Whether a seat of the server's Garmin client is free for an athlete
/// whose Garmin grant is being withdrawn.
///
/// Read before the withdrawal deletes the grant, so a grant that holds a
/// seat still counts as using it: a pool full only of the athlete's own
/// seat reads as full, and they are told nothing rather than sent to race
/// others for the seat their withdrawal frees. A client that is not
/// configured, or seats that cannot be counted, read as none free: the
/// athlete is never sent to an authorization that cannot start. The seat is
/// weighed again when the link is opened.
async fn garmin_seat_free(runtime: &Arc<dyn ToolRuntime>) -> bool {
    match garmin_seat_summary(
        runtime.repos().provider_connections.as_ref(),
        &runtime.config().oauth,
    )
    .await
    {
        Ok(summary) => summary.left() > 0,
        Err(e) => {
            info!(error = %e, "Withdrawn grant: Garmin's seats could not be counted; none free");
            false
        }
    }
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
    copy: NoticeCopy,
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
    let locale = resolve_user_locale(runtime.repos().users.as_ref(), user_id).await;
    let strings = runtime.messaging_strings_registry();
    let (title_key, body_key) = match copy {
        NoticeCopy::Reconnect => (
            KEY_NOTIFICATION_PROVIDER_REAUTH_TITLE,
            KEY_NOTIFICATION_PROVIDER_REAUTH_BODY,
        ),
        NoticeCopy::GrantWrites => (
            KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_TITLE,
            KEY_NOTIFICATION_PROVIDER_GRANT_WRITES_BODY,
        ),
    };
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

/// Whether the user holds no token row for the session `mirror` in this
/// tenant. A row that cannot be read counts as held: a grant is never flagged
/// on a guess.
async fn holds_no_session(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    mirror: &str,
) -> bool {
    matches!(
        runtime
            .repos()
            .oauth_tokens
            .get_token(user_id, tenant_id, mirror)
            .await,
        Ok(None)
    )
}

/// [`flag_needs_reauth`] for the connection `refused` an attempt found dead,
/// recording `reason` and when the attempt began.
///
/// A mirror the athlete holds neither a connection nor a session for stands
/// for the connection of its pair they do hold: a read of a grant with no usable token goes to
/// the mirror's sign-in (`backend_resolver::read_plan`, carnet#1494), and the
/// grant's own row is the one that must read as needing the reconnect. A
/// flip that cannot be written is logged: the caller serves what it holds
/// either way.
pub(crate) async fn flag_refused(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    refused: &str,
    reason: &str,
    attempt_started_at: DateTime<Utc>,
) {
    let flagged = match flag_needs_reauth(
        runtime,
        user_id,
        tenant_id,
        refused,
        reason,
        attempt_started_at,
    )
    .await
    {
        Ok((ReauthMark::NoConnection, _))
            if is_mirror_backend(refused)
                && holds_no_session(runtime, user_id, tenant_id, refused).await =>
        {
            flag_needs_reauth(
                runtime,
                user_id,
                tenant_id,
                user_facing_name(refused),
                reason,
                attempt_started_at,
            )
            .await
            .map(|_| ())
        }
        other => other.map(|_| ()),
    };
    if let Err(e) = flagged {
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
