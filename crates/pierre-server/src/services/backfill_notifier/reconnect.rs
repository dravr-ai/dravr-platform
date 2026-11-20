// ABOUTME: The reconnect notice's wording and link for one linked chat: the hosted sign-in, or a writing grant's authorization
// ABOUTME: Chooses the catalogue keys and mints the one link a notice carries, with the instant a queued copy may still go
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The reconnect notice's body for one linked chat (carnet#1494).
//!
//! A dead connection of a provider with a mirror signs in again on the
//! hosted login page; a session its live grant still serves is told the
//! provider keeps syncing for now. A grant only it writes beside a session
//! that serves (Garmin's) is told the reads go on and links its own
//! authorization instead (JF's directive 8).
//!
//! A chat whose window is closed is sent none of it (carnet#1672): a linked
//! chat is left to the notice's app half, and the conversation that asked is
//! answered in the app with the linkless sentence — once while it stands, as
//! the thread's latest assistant turn or for a day after it was written. A
//! turn the platform runs on its own (a check-in, a landing reaction) reads
//! the provider too, and on a dead session each one would otherwise add the
//! same sentence to the thread.

use chrono::{DateTime, Duration, Utc};
use pierre_contremaitre::messaging_strings::{
    KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
};
use pierre_contremaitre::reauth_strings::{
    KEY_PROVIDER_REAUTH_FALLBACK_SERVED, KEY_PROVIDER_REAUTH_FALLBACK_SERVED_NO_LINK,
    KEY_PROVIDER_REAUTH_GRANT_WRITES, KEY_PROVIDER_REAUTH_GRANT_WRITES_NO_LINK,
};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::MessagingRepository;
use pierre_database::repositories::shorten_url;
use pierre_middleware::provider_link_token::{
    mint_connect_oauth_init_url, mint_hosted_signin_url, PROVIDER_LINK_TOKEN_TTL_MINUTES,
};
use pierre_providers::backend_resolver;
use pierre_providers::registry::global_registry;
use pierre_services::messaging_broadcast::LinkedChannelTarget;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use super::ServerBackfillNotifier;
use crate::services::backfill_delivery::{InAppDelivery, ResolvedRoute, PURPOSE_PROVIDER_REAUTH};
use crate::services::post_delivery::{report_held, HELD_WINDOW_CLOSED};

/// How long a reconnect link must still have to run for a queued notice to be
/// sent with it. A retry that would hand the athlete a link with less time
/// than this left is given up instead: a link that dies before they read it
/// is worse than the app notice they already have.
const RECONNECT_LINK_MIN_REMAINING_MINUTES: i64 = 60;

/// How many of a conversation's newest rows are read to find a reconnect
/// sentence still standing there.
const REAUTH_NOTE_LOOKBACK: i64 = 20;

/// How long a reconnect sentence written in the app stands: the same one is
/// not written into that conversation again within it.
const REAUTH_NOTE_QUIET: Duration = Duration::hours(24);

/// How long a grant's authorization link must still have to run for a
/// queued notice to be sent with it. The link lasts an hour
/// (`CONNECT_LINK_TOKEN_TTL_MINUTES`), so the reconnect margin above would
/// leave no retry at all; a quarter of an hour still lets the athlete open
/// it and consent.
const GRANT_LINK_MIN_REMAINING_MINUTES: i64 = 15;

/// A one-time hosted-login link, and the last instant a message carrying it
/// may still be sent.
pub(super) struct ReconnectLink {
    /// The (shortened) link.
    pub(super) url: String,
    /// The link's expiry less [`RECONNECT_LINK_MIN_REMAINING_MINUTES`].
    pub(super) send_by: DateTime<Utc>,
}

/// A rendered reconnect notice, and the instant after which it must not be
/// sent because the link in it expires; `None` when it carries no link.
pub(super) struct ReconnectNotice {
    /// The localized notice text.
    pub(super) body: String,
    /// See [`ReconnectLink::send_by`].
    pub(super) send_by: Option<DateTime<Utc>>,
}

/// The last instant a reconnect notice whose hosted sign-in link is minted
/// now may still be sent: the link's expiry less
/// [`RECONNECT_LINK_MIN_REMAINING_MINUTES`]. Also the bound of a notice whose
/// link could not be minted, when nothing else ends its retry.
pub(super) fn hosted_send_by() -> DateTime<Utc> {
    Utc::now() + Duration::minutes(PROVIDER_LINK_TOKEN_TTL_MINUTES)
        - Duration::minutes(RECONNECT_LINK_MIN_REMAINING_MINUTES)
}

impl ServerBackfillNotifier {
    /// A one-time hosted-login link that reconnects `provider`, minted for
    /// `channel` and shortened to a dot-free `<base>/r/<code>` (the raw JWT's
    /// dots make `WhatsApp` truncate linkification mid-token; the full URL is
    /// kept when the shortener store write fails). Mirrors
    /// `auth_recovery::mint_reconnect_url`'s hosted-login arm.
    ///
    /// Either half of a mirror pair reconnects through its mirror's sign-in
    /// ([`backend_resolver::reconnect_target`]): a dead Strava or Garmin grant
    /// included (carnet#1494). `None`, logged, when `provider` has no hosted
    /// login or the token cannot be minted.
    pub(super) async fn hosted_login_url(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        channel: &str,
    ) -> Option<ReconnectLink> {
        let target = backend_resolver::reconnect_target(provider)?;
        // Taken before the mint, so it is never later than the token's own
        // expiry; the short link is created after the token and outlives it.
        let send_by = hosted_send_by();
        let full_url = match mint_hosted_signin_url(
            &self.base_url,
            user_id,
            tenant_id.as_uuid(),
            target,
            channel,
            &self.admin_jwt_secret,
        ) {
            Ok(url) => url,
            Err(e) => {
                warn!(error = %e, provider = %provider, "Reconnect link: link-token mint failed");
                return None;
            }
        };
        let url = shorten_url(
            self.repos.short_links.as_ref(),
            &self.base_url,
            &full_url,
            &tenant_id.as_uuid().to_string(),
            &user_id.to_string(),
        )
        .await;
        Some(ReconnectLink { url, send_by })
    }

    /// A link to the OAuth authorization of the grant `grant`, minted for
    /// `channel` and shortened as [`Self::hosted_login_url`]'s is, for a
    /// grant only it writes beside a session that serves (JF's directive 8).
    ///
    /// The link starts the authorization when it is opened
    /// (`mint_connect_oauth_init_url`), so it lasts its connect token's hour
    /// rather than the ten minutes of an authorization URL's state; its
    /// `send_by` is that expiry less [`GRANT_LINK_MIN_REMAINING_MINUTES`].
    /// The seat cap is weighed when it is opened. `None`, logged, when the
    /// token cannot be minted.
    async fn grant_authorization_url(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        grant: &str,
        channel: &str,
    ) -> Option<ReconnectLink> {
        let (full_url, expires_at) = match mint_connect_oauth_init_url(
            &self.base_url,
            user_id,
            tenant_id.as_uuid(),
            grant,
            channel,
            &self.admin_jwt_secret,
        ) {
            Ok(minted) => minted,
            Err(e) => {
                warn!(error = %e, provider = %grant, "Grant link: link-token mint failed");
                return None;
            }
        };
        let url = shorten_url(
            self.repos.short_links.as_ref(),
            &self.base_url,
            &full_url,
            &tenant_id.as_uuid().to_string(),
            &user_id.to_string(),
        )
        .await;
        Some(ReconnectLink {
            url,
            send_by: expires_at - Duration::minutes(GRANT_LINK_MIN_REMAINING_MINUTES),
        })
    }

    /// The catalogue keys of `provider`'s reconnect sentence, with and without
    /// a link.
    ///
    /// A session whose live grant keeps the provider syncing
    /// ([`backend_resolver::fallback_grant`]) is told it still syncs for now,
    /// and asked to sign in again to keep it that way (carnet#1494); any
    /// other dead connection, the ordinary reconnect.
    pub(super) async fn reauth_keys(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> (&'static str, &'static str) {
        if backend_resolver::fallback_grant(&self.repos, user_id, tenant_id, provider)
            .await
            .is_some()
        {
            (
                KEY_PROVIDER_REAUTH_FALLBACK_SERVED,
                KEY_PROVIDER_REAUTH_FALLBACK_SERVED_NO_LINK,
            )
        } else {
            (
                KEY_PROVIDER_REAUTH_REQUIRED,
                KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
            )
        }
    }

    /// The DM session of the linked chat `target`, stored under the
    /// athlete's own tenant, never the bot's: when they last wrote there.
    /// `None` (logged on error) when they never have.
    pub(super) async fn linked_session(
        &self,
        tenant_id: TenantId,
        target: &LinkedChannelTarget,
    ) -> Option<Value> {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        db.get_session_by_channel_identity(
            tenant_id,
            &target.channel_type.to_string(),
            &target.recipient_id,
            None,
        )
        .await
        .inspect_err(|e| warn!(error = %e, "Reconnect notice: linked chat session lookup failed"))
        .ok()
        .flatten()
    }

    /// Answer the conversation that asked with the reconnect sentence in the
    /// app, for a route whose window is closed: the channel would refuse it
    /// unprompted (carnet#1672).
    ///
    /// The linkless sentence — the athlete reconnects from the app's
    /// settings — so no one-time link is minted for a message that never
    /// travels. Stored without the completion push's app notification, whose
    /// "N activities loaded" would misstate it: the turn's unread badge is the
    /// ping, as the reconnect notice's own app half is.
    ///
    /// Written once while it stands ([`Self::reauth_note_standing`]): an
    /// automatic turn on a dead session asks again without the athlete doing
    /// anything, and the hold is still reported each time.
    pub(super) async fn reauth_in_app(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        provider: &str,
        route: &ResolvedRoute,
    ) {
        info!(
            channel = %route.channel_str,
            provider = %provider,
            "Reauth nudge: the channel's window is closed; answering in the app"
        );
        report_held(
            tenant_id,
            route.channel_type,
            PURPOSE_PROVIDER_REAUTH,
            HELD_WINDOW_CLOSED,
        );
        let locale = self.resolve_locale(user_id, tenant_id, None).await;
        let display =
            backend_resolver::brand_name(&global_registry(), provider).unwrap_or(provider);
        let (_, bare_key) = self.reauth_keys(user_id, tenant_id, provider).await;
        let body = self.strings.render(bare_key, &locale, &[display]);
        if self
            .reauth_note_standing(user_id, tenant_id, conversation_id, &body, Utc::now())
            .await
        {
            info!(provider = %provider, "Reauth nudge: the same sentence still stands in the app; not written again");
            return;
        }
        // A provider's name and a pointer to settings: no data of any session.
        InAppDelivery {
            repos: &self.repos,
            strings: &self.strings,
            #[cfg(feature = "client-notifications")]
            notifications: self.notifications.as_ref(),
            transport_policy: TransportPolicy::AnyTransport,
        }
        .persist(user_id, tenant_id, conversation_id, body, 0)
        .await;
    }

    /// Whether `body` already stands in `conversation_id` at `now`: among the
    /// thread's newest [`REAUTH_NOTE_LOOKBACK`] rows, it is the latest
    /// assistant turn, or one written within [`REAUTH_NOTE_QUIET`]. The
    /// sentence names its provider's brand, so an equal body tells the athlete
    /// the same thing whichever of that brand's backends asked. A thread that
    /// cannot be read answers `false`, and the sentence is written.
    async fn reauth_note_standing(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        body: &str,
        now: DateTime<Utc>,
    ) -> bool {
        let recent = match self
            .repos
            .chat
            .get_recent_messages(
                conversation_id,
                &user_id.to_string(),
                tenant_id,
                REAUTH_NOTE_LOOKBACK,
                TransportPolicy::FirstPartyOnly,
            )
            .await
        {
            Ok(recent) => recent,
            Err(e) => {
                warn!(error = %e, "Reauth nudge: the conversation could not be read; writing the sentence");
                return false;
            }
        };
        // Oldest first: the latest assistant turn is the last one.
        let assistant: Vec<_> = recent
            .iter()
            .rev()
            .filter(|message| message.role == "assistant")
            .collect();
        if assistant
            .first()
            .is_some_and(|latest| latest.content == body)
        {
            return true;
        }
        assistant.iter().any(|message| {
            message.content == body
                && DateTime::parse_from_rfc3339(&message.created_at)
                    .is_ok_and(|at| now - at.with_timezone(&Utc) < REAUTH_NOTE_QUIET)
        })
    }

    /// The reconnect notice for one linked chat on `channel`, in `locale`.
    ///
    /// A provider with a mirror signs in again on the hosted login page,
    /// through a one-time link minted for this channel: a dead session and a
    /// dead OAuth grant alike (carnet#1494). An OAuth provider with no mirror
    /// has no link this notice can carry: its authorization URL and the
    /// connect picker's token both expire within the hour, well before a
    /// notice read the next morning, and the sentence promises a day. It is
    /// named without a link, and reconnected from the app's settings — as is
    /// a mirror whose link could not be minted.
    ///
    /// A grant only it writes beside a session that serves
    /// ([`backend_resolver::grant_writes_beside_serving_session`]) is told the
    /// reads go on and the writes need it again, and links its own
    /// authorization, never the sign-in the session already holds (JF's
    /// directive 8).
    pub(super) async fn reauth_notice_body(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        channel: &str,
        locale: &str,
    ) -> ReconnectNotice {
        let registry = global_registry();
        let display = backend_resolver::brand_name(&registry, provider).unwrap_or(provider);
        let writes_owed = backend_resolver::grant_writes_beside_serving_session(
            &self.repos,
            &registry,
            user_id,
            tenant_id,
            provider,
        )
        .await;
        let ((linked_key, bare_key), link) = if writes_owed {
            (
                (
                    KEY_PROVIDER_REAUTH_GRANT_WRITES,
                    KEY_PROVIDER_REAUTH_GRANT_WRITES_NO_LINK,
                ),
                self.grant_authorization_url(user_id, tenant_id, provider, channel)
                    .await,
            )
        } else {
            (
                self.reauth_keys(user_id, tenant_id, provider).await,
                self.hosted_login_url(user_id, tenant_id, provider, channel)
                    .await,
            )
        };
        link.map_or_else(
            || ReconnectNotice {
                body: self.strings.render(bare_key, locale, &[display]),
                send_by: None,
            },
            |link| ReconnectNotice {
                body: self
                    .strings
                    .render(linked_key, locale, &[display, &link.url]),
                send_by: Some(link.send_by),
            },
        )
    }
}
