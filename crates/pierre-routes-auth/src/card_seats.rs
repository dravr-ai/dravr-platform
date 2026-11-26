// ABOUTME: The seats left on the server's Strava and Garmin OAuth clients, and each sign-in card's order, read once per list
// ABOUTME: A new connection follows the admin's order and the seats; a reconnect always signs in through the mirror

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which backend the Strava and Garmin cards recommend.
//!
//! Each registered client of either provider serves a bounded number of
//! athletes. A new connection signs in through the mirror by default, and goes through the provider's
//! OAuth while a seat remains only under the admin's OAuth-first order
//! (carnet#1472). A reconnect signs in through the mirror whatever the order
//! and the seats: a dead grant reconnects only through the hosted sign-in
//! (carnet#1494). A count that fails, or a server with no client configured,
//! reads as no seat left, so a connect is never sent into an OAuth flow that
//! cannot finish.
//!
//! Every account sign-in card (Strava, Garmin, TrainingPeaks, COROS) also
//! carries its provider's order itself (`connect_order`), so a web client
//! with the sign-in extension installed knows whether to start it before the
//! credentials form ([`ConnectOrder::ExtensionFirst`]).

use tracing::debug;
use uuid::Uuid;

use crate::AuthRoutesContext;
use pierre_auth::connect_order::{
    connect_order, new_connection_through_oauth, ConnectOrder, DEFAULT_ORDER,
};
use pierre_auth::{garmin_seats, strava_pool};
use pierre_core::constants::oauth::providers::{self as oauth_providers, MIRROR_PAIRS};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;

/// Seats left on each capped OAuth client, and the admin's connect order
/// of every account sign-in card (carnet#1472).
pub(crate) struct CardSeats {
    strava: u32,
    garmin: u32,
    /// Each sign-in card's order, by card key: the mirror half of every
    /// [`MIRROR_PAIRS`] entry (`sciotte`, `sciotte_garmin`, ...).
    orders: [(&'static str, ConnectOrder); MIRROR_PAIRS.len()],
}

impl CardSeats {
    /// Count both pools once, for every card of one provider list.
    pub(crate) async fn count(resources: &AuthRoutesContext) -> Self {
        // Strava's pool is the env-default STRAVA_CLIENT_ID app plus any
        // enabled DB pool apps. Each application serves a bounded number of
        // athletes, a number no API header reports, so the seats are counted
        // here: a NEW Strava connection is offered OAuth while a seat remains.
        let strava = strava_pool::strava_seat_summary(
            resources.repos.oauth_tokens.as_ref(),
            &resources.config.oauth,
        )
        .await
        .map_or(0, |s| s.left());
        // Garmin's Developer Program client serves a bounded number of users.
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
        let mut orders = MIRROR_PAIRS.map(|(_, card)| (card, DEFAULT_ORDER));
        let slots = orders.iter_mut().zip(MIRROR_PAIRS);
        for ((_, order), (provider, _)) in slots {
            *order = connect_order(settings, provider).await;
        }
        Self {
            strava,
            garmin,
            orders,
        }
    }

    /// The `connect_order` of `provider_name`'s card: its provider's order
    /// for an account sign-in card (`sciotte`, `sciotte_garmin`,
    /// `sciotte_trainingpeaks`, `sciotte_coros`), whether it connects,
    /// reconnects or imports its history; `None` for any other card.
    pub(crate) fn connect_order(&self, provider_name: &str) -> Option<ConnectOrder> {
        self.orders
            .iter()
            .find(|(card, _)| *card == provider_name)
            .map(|(_, order)| *order)
    }

    /// The `recommended_backend` and `seats_left` of `provider_name`'s card,
    /// for an athlete whose card asks for a `reconnect`.
    ///
    /// The session sign-in entries are the user-facing "Strava" and "Garmin" cards:
    /// `oauth` (the card's provider's authorization) only while the
    /// admin's order puts the OAuth first ([`ConnectOrder::OAuthFirst`])
    /// and a seat remains, else `mirror`. A card asking for a reconnect
    /// recommends `mirror` whatever the order and the seats: a dead grant
    /// reconnects through the hosted sign-in, as a dead session does. Any
    /// other card carries neither.
    pub(crate) fn recommendation(
        &self,
        provider_name: &str,
        reconnect: bool,
    ) -> (Option<String>, Option<u32>) {
        let left = match provider_name {
            oauth_providers::SCIOTTE => self.strava,
            oauth_providers::SCIOTTE_GARMIN => self.garmin,
            _ => return (None, None),
        };
        let Some(order) = self.connect_order(provider_name) else {
            return (None, None);
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

    /// Seats on the Strava and Garmin clients, Strava's and Garmin's orders,
    /// and the default order on every other sign-in card.
    fn card_seats(
        strava: u32,
        garmin: u32,
        strava_order: ConnectOrder,
        garmin_order: ConnectOrder,
    ) -> CardSeats {
        CardSeats {
            strava,
            garmin,
            orders: [
                (oauth_providers::SCIOTTE, strava_order),
                (oauth_providers::SCIOTTE_GARMIN, garmin_order),
                (
                    oauth_providers::SCIOTTE_TRAININGPEAKS,
                    ConnectOrder::ExtensionFirst,
                ),
                (oauth_providers::SCIOTTE_COROS, ConnectOrder::SignInFirst),
            ],
        }
    }

    #[test]
    fn an_oauth_first_card_with_a_seat_recommends_oauth_and_a_full_one_its_mirror() {
        let seats = card_seats(0, 3, ConnectOrder::OAuthFirst, ConnectOrder::OAuthFirst);
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
        let seats = card_seats(5, 5, ConnectOrder::OAuthFirst, ConnectOrder::OAuthFirst);
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
        let seats = card_seats(5, 5, ConnectOrder::SignInFirst, ConnectOrder::OAuthFirst);
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

    /// The extension-first order (the default) routes a new connection as
    /// the sign-in-first order does: to the account sign-in, seats or not.
    #[test]
    fn an_extension_first_order_recommends_the_mirror_whatever_the_seats() {
        let seats = card_seats(
            5,
            0,
            ConnectOrder::ExtensionFirst,
            ConnectOrder::ExtensionFirst,
        );
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE, false),
            (Some("mirror".to_owned()), Some(5)),
            "bug: an extension-first Strava card sends a new connection to the OAuth"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE_GARMIN, false),
            (Some("mirror".to_owned()), Some(0)),
            "bug: an extension-first Garmin card with a full pool"
        );
    }

    /// Every account sign-in card carries its own provider's order, and no
    /// other card carries one.
    #[test]
    fn every_sign_in_card_carries_its_own_order_and_no_other_card_does() {
        let seats = card_seats(5, 5, ConnectOrder::OAuthFirst, ConnectOrder::ExtensionFirst);
        for (card, expected) in [
            (oauth_providers::SCIOTTE, ConnectOrder::OAuthFirst),
            (
                oauth_providers::SCIOTTE_GARMIN,
                ConnectOrder::ExtensionFirst,
            ),
            (
                oauth_providers::SCIOTTE_TRAININGPEAKS,
                ConnectOrder::ExtensionFirst,
            ),
            (oauth_providers::SCIOTTE_COROS, ConnectOrder::SignInFirst),
        ] {
            assert_eq!(
                seats.connect_order(card),
                Some(expected),
                "bug: the {card} card carries another order"
            );
        }
        for card in [
            oauth_providers::WHOOP,
            oauth_providers::STRAVA,
            oauth_providers::GARMIN,
        ] {
            assert_eq!(
                seats.connect_order(card),
                None,
                "bug: the {card} card, with no account sign-in of its own, carries an order"
            );
        }
    }
}
