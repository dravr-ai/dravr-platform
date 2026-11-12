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

use chrono::{DateTime, Duration, Utc};
use pierre_contremaitre::messaging_strings::{
    KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
};
use pierre_contremaitre::reauth_strings::{
    KEY_PROVIDER_REAUTH_FALLBACK_SERVED, KEY_PROVIDER_REAUTH_FALLBACK_SERVED_NO_LINK,
    KEY_PROVIDER_REAUTH_GRANT_WRITES, KEY_PROVIDER_REAUTH_GRANT_WRITES_NO_LINK,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::shorten_url;
use pierre_middleware::provider_link_token::{
    mint_connect_oauth_init_url, mint_hosted_signin_url, PROVIDER_LINK_TOKEN_TTL_MINUTES,
};
use pierre_providers::backend_resolver;
use pierre_providers::registry::global_registry;
use tracing::warn;
use uuid::Uuid;

use super::ServerBackfillNotifier;

/// How long a reconnect link must still have to run for a queued notice to be
/// sent with it. A retry that would hand the athlete a link with less time
/// than this left is given up instead: a link that dies before they read it
/// is worse than the app notice they already have.
const RECONNECT_LINK_MIN_REMAINING_MINUTES: i64 = 60;

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
        let send_by = Utc::now() + Duration::minutes(PROVIDER_LINK_TOKEN_TTL_MINUTES)
            - Duration::minutes(RECONNECT_LINK_MIN_REMAINING_MINUTES);
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
