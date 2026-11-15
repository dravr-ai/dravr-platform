// ABOUTME: The seats left on the server's Strava and Garmin OAuth clients, read once per provider list
// ABOUTME: A new connection follows the operator's order and the seats; a reconnect always signs in through the mirror

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which backend the Strava and Garmin cards recommend.
//!
//! Both providers cap the athletes one client may connect. A new connection
//! signs in through the mirror by default, and goes through the provider's
//! OAuth while a seat remains only under the operator's OAuth-first order
//! (carnet#1472). A reconnect signs in through the mirror whatever the order
//! and the seats: a dead grant reconnects only through the hosted sign-in
//! (carnet#1494). A count that fails, or a server with no client configured,
//! reads as no seat left, so a connect is never sent into an OAuth flow that
//! cannot finish.

use tracing::debug;
use uuid::Uuid;

use crate::AuthRoutesContext;
use pierre_auth::connect_order::{connect_order, new_connection_through_oauth, ConnectOrder};
use pierre_auth::{garmin_seats, strava_pool};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;

/// Seats left on each capped OAuth client, and the operator's connect order
/// for each (carnet#1472).
pub(crate) struct CardSeats {
    strava: u32,
    garmin: u32,
    strava_order: ConnectOrder,
    garmin_order: ConnectOrder,
}

impl CardSeats {
    /// Count both pools once, for every card of one provider list.
    pub(crate) async fn count(resources: &AuthRoutesContext) -> Self {
        // Strava's pool is the env-default STRAVA_CLIENT_ID app plus any
        // enabled DB pool apps. Strava enforces a per-application athlete cap
        // that no API header exposes, so a NEW Strava connection prefers
        // OAuth while ANY pool seat remains.
        let strava = strava_pool::strava_seat_summary(
            resources.repos.oauth_tokens.as_ref(),
            &resources.config.oauth,
        )
        .await
        .map_or(0, |s| s.left());
        // Garmin's Developer Program caps the users one client may connect.
        let garmin = match garmin_seats::garmin_seat_summary(
            resources.repos.provider_connections.as_ref(),
            &resources.config.oauth,
        )
        .await
        {
            Ok(summary) => summary.left(),
            Err(e) => {
                debug!(error = %e, "Garmin seats could not be counted; the Garmin card offers the usr/pwd connection");
                0
            }
        };
        let settings = resources.repos.system_settings.as_ref();
        Self {
            strava,
            garmin,
            strava_order: connect_order(settings, oauth_providers::STRAVA).await,
            garmin_order: connect_order(settings, oauth_providers::GARMIN).await,
        }
    }

    /// The `recommended_backend` and `seats_left` of `provider_name`'s card,
    /// for an athlete whose card asks for a `reconnect`.
    ///
    /// The Sciotte entries are the user-facing "Strava" and "Garmin" cards:
    /// `oauth` (the card's provider's authorization) only while the
    /// operator's order puts the OAuth first ([`ConnectOrder::OAuthFirst`])
    /// and a seat remains, else `mirror`. A card asking for a reconnect
    /// recommends `mirror` whatever the order and the seats: a dead grant
    /// reconnects through the hosted sign-in, as a dead session does. Any
    /// other card carries neither.
    pub(crate) fn recommendation(
        &self,
        provider_name: &str,
        reconnect: bool,
    ) -> (Option<String>, Option<u32>) {
        let (left, order) = match provider_name {
            oauth_providers::SCIOTTE => (self.strava, self.strava_order),
            oauth_providers::SCIOTTE_GARMIN => (self.garmin, self.garmin_order),
            _ => return (None, None),
        };
        let backend = if !reconnect && new_connection_through_oauth(order, left) {
            "oauth"
        } else {
            "mirror"
        };
        (Some(backend.to_owned()), Some(left))
    }
}

/// [`garmin_seats::require_garmin_seat`] over this server's stores: refuse to
/// start Garmin's OAuth for `user_id` in `tenant_id` when the Developer
/// Program client has no seat for them. Any other provider passes.
///
/// # Errors
/// A [`pierre_core::errors::AppError::resource_unavailable`] refusal, with `details.reason`
/// [`garmin_seats::NO_SEAT_REASON`], when no seat is left for this user; the caller routes
/// them to the usr/pwd connection.
pub(crate) async fn require_garmin_seat(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> AppResult<()> {
    garmin_seats::require_garmin_seat(
        resources.repos.oauth_tokens.as_ref(),
        resources.repos.provider_connections.as_ref(),
        &resources.config.oauth,
        user_id,
        tenant_id,
        provider,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oauth_first_card_with_a_seat_recommends_oauth_and_a_full_one_its_mirror() {
        let seats = CardSeats {
            strava: 0,
            garmin: 3,
            strava_order: ConnectOrder::OAuthFirst,
            garmin_order: ConnectOrder::OAuthFirst,
        };
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE, false),
            (Some("mirror".to_owned()), Some(0)),
            "bug: a full Strava pool recommends the mirror"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE_GARMIN, false),
            (Some("oauth".to_owned()), Some(3)),
            "bug: an OAuth-first Garmin pool with seats recommends OAuth"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::WHOOP, false),
            (None, None),
            "bug: an uncapped card carries no recommendation"
        );
    }

    /// A dead grant reconnects through the hosted sign-in only (carnet#1494),
    /// even where a new connection would take the OAuth.
    #[test]
    fn a_card_asking_for_a_reconnect_recommends_the_mirror_whatever_the_order_and_seats() {
        let seats = CardSeats {
            strava: 5,
            garmin: 5,
            strava_order: ConnectOrder::OAuthFirst,
            garmin_order: ConnectOrder::OAuthFirst,
        };
        for card in [oauth_providers::SCIOTTE, oauth_providers::SCIOTTE_GARMIN] {
            assert_eq!(
                seats.recommendation(card, true),
                (Some("mirror".to_owned()), Some(5)),
                "bug: the {card} card sends a reconnect to the OAuth"
            );
        }
    }

    /// The sign-in-first order (the default, carnet#1494) sends a new
    /// connection to the account sign-in with seats to spare, one provider at
    /// a time.
    #[test]
    fn a_sign_in_first_order_recommends_the_mirror_whatever_the_seats() {
        let seats = CardSeats {
            strava: 5,
            garmin: 5,
            strava_order: ConnectOrder::SignInFirst,
            garmin_order: ConnectOrder::OAuthFirst,
        };
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE, false),
            (Some("mirror".to_owned()), Some(5)),
            "bug: a sign-in-first Strava card sends a new connection to the OAuth"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE_GARMIN, false),
            (Some("oauth".to_owned()), Some(5)),
            "bug: Strava's order leaked into the Garmin card"
        );
    }
}
