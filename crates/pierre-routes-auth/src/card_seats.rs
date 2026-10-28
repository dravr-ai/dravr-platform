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

use tracing::{debug, warn};
use uuid::Uuid;

use crate::AuthRoutesContext;
use pierre_auth::{garmin_seats, strava_pool};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

/// Seats left on each capped OAuth client.
pub(crate) struct CardSeats {
    strava: u32,
    garmin: u32,
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
        Self { strava, garmin }
    }

    /// The `recommended_backend` and `seats_left` of `provider_name`'s card.
    ///
    /// The Sciotte entries are the user-facing "Strava" and "Garmin" cards:
    /// `oauth` (the card's `hosted_login_target`) while seats remain, else
    /// `mirror`. Any other card carries neither.
    pub(crate) fn recommendation(&self, provider_name: &str) -> (Option<String>, Option<u32>) {
        let left = match provider_name {
            oauth_providers::SCIOTTE => self.strava,
            oauth_providers::SCIOTTE_GARMIN => self.garmin,
            _ => return (None, None),
        };
        let backend = if left > 0 { "oauth" } else { "mirror" };
        (Some(backend.to_owned()), Some(left))
    }
}

/// Refuse to start Garmin's OAuth for `user_id` in `tenant_id` when the
/// Developer Program client has no seat for them: the pool is full (or cannot
/// be counted, or no client is configured) and they hold no Garmin grant in
/// that tenant whose seat a reconnect would re-sign.
///
/// The card already recommends the usr/pwd connection once the pool is full,
/// but it is drawn before the connect starts, so two athletes shown the last
/// seat could both start the flow. This check, at every authorize start,
/// holds the cap. Any other provider passes.
///
/// # Errors
/// A [`AppError::resource_unavailable`] refusal when no seat is left for
/// this user; the caller routes them to the usr/pwd connection.
pub(crate) async fn require_garmin_seat(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> AppResult<()> {
    if provider != oauth_providers::GARMIN {
        return Ok(());
    }
    match resources
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, oauth_providers::GARMIN)
        .await
    {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, "Garmin grant lookup failed; weighing the seat pool alone");
        }
    }
    let left = match garmin_seats::garmin_seat_summary(
        resources.repos.provider_connections.as_ref(),
        &resources.config.oauth,
    )
    .await
    {
        Ok(summary) => summary.left(),
        Err(e) => {
            debug!(error = %e, "Garmin seats could not be counted; no OAuth start");
            0
        }
    };
    if left > 0 {
        return Ok(());
    }
    Err(AppError::resource_unavailable(
        "Garmin OAuth refused: the Developer Program client has no seat left; \
         connect with the Garmin email and password instead",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_with_a_seat_recommends_oauth_and_a_full_one_its_mirror() {
        let seats = CardSeats {
            strava: 0,
            garmin: 3,
        };
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE),
            (Some("mirror".to_owned()), Some(0)),
            "bug: a full Strava pool recommends the mirror"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::SCIOTTE_GARMIN),
            (Some("oauth".to_owned()), Some(3)),
            "bug: a Garmin pool with seats recommends OAuth"
        );
        assert_eq!(
            seats.recommendation(oauth_providers::WHOOP),
            (None, None),
            "bug: an uncapped card carries no recommendation"
        );
    }
}
