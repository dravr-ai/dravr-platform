// ABOUTME: The reconnect copy of a detached notice for a dead usr/pwd-only Garmin session while a seat is free on Garmin's OAuth
// ABOUTME: Names the move to Garmin's official connection, with no link: the connect picker's token outlives no notice

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reconnecting through the provider's OAuth (carnet#1454).
//!
//! A usr/pwd Garmin session of the athlete's own that served them alone is
//! reconnected through Garmin's OAuth while a seat is free there
//! ([`reconnects_through_garmin_oauth`], the one rule): the notice says so and
//! links nothing, since the connect picker's token lasts an hour and a
//! detached notice may be read the next morning. With the pool full, or for
//! any other backend, the notice is the ordinary sign-in again.

use pierre_auth::garmin_seats::reconnects_through_garmin_oauth;
use pierre_contremaitre::reauth_strings::KEY_PROVIDER_REAUTH_OAUTH_REQUIRED_NO_LINK;
use pierre_core::models::TenantId;
use uuid::Uuid;

use super::ServerBackfillNotifier;

impl ServerBackfillNotifier {
    /// The notice for `provider` in `locale`, displayed as `display`, when
    /// the athlete reconnects it through its provider's OAuth; `None` when
    /// the ordinary reconnect applies.
    pub(super) async fn oauth_reconnect_body(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        display: &str,
        locale: &str,
    ) -> Option<String> {
        if !reconnects_through_garmin_oauth(
            self.repos.oauth_tokens.as_ref(),
            self.repos.provider_connections.as_ref(),
            self.repos.system_settings.as_ref(),
            &self.oauth,
            user_id,
            tenant_id,
            provider,
        )
        .await
        {
            return None;
        }
        Some(self.strings.render(
            KEY_PROVIDER_REAUTH_OAUTH_REQUIRED_NO_LINK,
            locale,
            &[display],
        ))
    }
}
