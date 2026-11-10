// ABOUTME: The seats left on the server's Strava and Garmin OAuth clients, read once per provider list
// ABOUTME: A card with a seat recommends its provider's OAuth for a new connection, a full one its mirror

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which backend the Strava and Garmin cards recommend for a new connection.
//!
//! Both providers cap the athletes one client may connect, so a new
//! connection goes through the provider's OAuth while a seat remains and
//! through the mirror once the pool is full (carnet#1155 for Garmin). A count
//! that fails, or a server with no client configured, reads as no seat left,
//! so a connect is never sent into an OAuth flow that cannot finish.

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

    /// Seats left on the Garmin client, as counted for this provider list.
    pub(crate) const fn garmin_left(&self) -> u32 {
        self.garmin
    }

    /// The operator's Garmin connect order, as read for this provider list.
    pub(crate) const fn garmin_order(&self) -> ConnectOrder {
        self.garmin_order
    }

    /// The `recommended_backend` and `seats_left` of `provider_name`'s card,
    /// for an athlete whose own grant serves it when `grant_serves`.
    ///
    /// The Sciotte entries are the user-facing "Strava" and "Garmin" cards:
    /// `oauth` (the card's `hosted_login_target`) while seats remain, else
    /// `mirror`; with the operator's sign-in-first order
    /// ([`ConnectOrder::SignInFirst`]) `mirror` whatever the seats. A card the
    /// athlete's own grant serves recommends `oauth`
    /// whatever the pool holds: their reconnect re-signs the seat they hold
    /// ([`require_garmin_seat`] lets it through), while a usr/pwd session
    /// signed in beside that grant would serve nothing (carnet#1386). Any
    /// other card carries neither.
    pub(crate) fn recommendation(
        &self,
        provider_name: &str,
        grant_serves: bool,
    ) -> (Option<String>, Option<u32>) {
        let (left, order) = match provider_name {
            oauth_providers::SCIOTTE => (self.strava, self.strava_order),
            oauth_providers::SCIOTTE_GARMIN => (self.garmin, self.garmin_order),
            _ => return (None, None),
        };
        let oauth_first = new_connection_through_oauth(order, left);
        let backend = if oauth_first || grant_serves {
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
    fn a_card_with_a_seat_recommends_oauth_and_a_full_one_its_mirror() {
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
            "bug: a Garmin pool with seats recommends OAuth"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::WHOOP, false),
            (None, None),
            "bug: an uncapped card carries no recommendation"
        );
    }

    /// A grant holder reconnects through the OAuth whose seat they hold: a
    /// usr/pwd session signed in beside their grant serves nothing.
    #[test]
    fn a_card_its_own_grant_serves_recommends_oauth_with_the_pool_full() {
        let seats = CardSeats {
            strava: 0,
            garmin: 0,
            strava_order: ConnectOrder::SignInFirst,
            garmin_order: ConnectOrder::SignInFirst,
        };
        for card in [oauth_providers::SCIOTTE, oauth_providers::SCIOTTE_GARMIN] {
            assert_eq!(
                seats.recommendation(card, true),
                (Some("oauth".to_owned()), Some(0)),
                "bug: the {card} card of a grant holder sends a reconnect to the usr/pwd login"
            );
        }
    }

    /// The operator's sign-in-first order (carnet#1472) sends a new
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
