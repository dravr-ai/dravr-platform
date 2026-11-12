// ABOUTME: Seat accounting for Dravr's Garmin Developer Program client, capped per client id
// ABOUTME: Counts the users whose live Garmin grant the server-level client signs, and the one rule for moving off usr/pwd
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Seats of the server's own Garmin client (carnet#1155).
//!
//! Garmin's Developer Program caps how many users one client id may connect
//! (1,000 on the Starter tier), the way Strava caps the athletes of one app.
//! A connect therefore offers Garmin's OAuth while a seat remains and falls
//! back to the connection using usr/pwd from Dravr once the pool is full —
//! the same decision the Strava card makes from
//! [`crate::strava_pool::strava_seat_summary`].
//!
//! A seat is one Garmin user whose grant the server-level client signs and
//! whose connection is still live, counted once however many tenants hold it:
//! [`ProviderConnectionRepository::count_server_level_grants`], the count that
//! sizes a provider app's request budget by the athletes who granted it.
//!
//! Garmin connects through its OAuth only while a seat is free
//! (carnet#1454): [`garmin_oauth_required`] is the one rule every surface
//! reads, so the card's move to Garmin's official connection, the refusal of
//! a new usr/pwd sign-in and the reconnect notice's link agree. With the pool
//! full, an athlete with no grant connects and reconnects by usr/pwd as
//! before, until a seat frees. The operator's connect order
//! ([`crate::connect_order`]) can put the usr/pwd sign-in first instead: no
//! one is then asked to move to the OAuth, whatever the seats (carnet#1472).

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::backends::{OAuthTokenRepository, ProviderConnectionRepository};
use pierre_database::repositories::SystemSettingsRepository;
use serde_json::{json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::config::oauth::OAuthConfig;
use crate::connect_order::{connect_order, ConnectOrder};

/// Seat capacity of the server's Garmin client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarminSeatSummary {
    /// The client's cap ([`OAuthConfig::garmin_seat_cap`]).
    pub total: u32,
    /// Users holding a live grant the client signs.
    pub used: u32,
}

impl GarminSeatSummary {
    /// Seats still free.
    #[must_use]
    pub const fn left(&self) -> u32 {
        self.total.saturating_sub(self.used)
    }
}

/// Count the seats of the server's Garmin client.
///
/// # Errors
///
/// Refuses when the server has no Garmin client configured — no seat can be
/// offered then, and a connect must not be sent into an OAuth flow that cannot
/// start — and returns the store's error when the grants cannot be counted.
/// A caller deciding a connect path treats either as "no seat left".
pub async fn garmin_seat_summary(
    grants: &dyn ProviderConnectionRepository,
    server: &OAuthConfig,
) -> AppResult<GarminSeatSummary> {
    if !server.server_app(oauth_providers::GARMIN).enabled {
        return Err(AppError::invalid_input(
            "Garmin OAuth client not configured (GARMIN_CLIENT_ID/GARMIN_CLIENT_SECRET)",
        ));
    }
    let used = grants
        .count_server_level_grants(oauth_providers::GARMIN)
        .await?;
    Ok(GarminSeatSummary {
        total: server.garmin_seat_cap(),
        used,
    })
}

/// Whether an athlete must connect Garmin through its OAuth rather than by usr/pwd.
///
/// Given the `seats_left` on the server's Garmin client, whether they already
/// hold a Garmin grant (`grant_held`), and the operator's Garmin connect
/// `order`.
///
/// True only while Garmin's OAuth goes first and a seat is free for an
/// athlete with no grant: they are
/// offered Garmin's official connection, a new usr/pwd sign-in is refused,
/// and a dead usr/pwd session's notice points to the OAuth. A grant holder is
/// on the OAuth already. With no seat left, the usr/pwd connection stays the
/// way in until one frees, and with the sign-in first it is the way in
/// whatever the seats. The one predicate to change if that rule does.
#[must_use]
pub fn garmin_oauth_required(seats_left: u32, grant_held: bool, order: ConnectOrder) -> bool {
    order == ConnectOrder::OAuthFirst && !grant_held && seats_left > 0
}

/// [`garmin_oauth_required`] for `user_id` in `tenant_id`, read from the store.
///
/// Whether they hold a Garmin token row, the seats left under `server`, and
/// the Garmin connect order in `settings`.
///
/// A seat count that fails, or a server with no Garmin client, reads as no
/// seat, and a grant lookup that fails as a grant held: either answers
/// `false`, which keeps the usr/pwd connection the athlete has rather than
/// sending them into an OAuth flow that may not finish.
pub async fn garmin_oauth_required_for(
    tokens: &dyn OAuthTokenRepository,
    grants: &dyn ProviderConnectionRepository,
    settings: &dyn SystemSettingsRepository,
    server: &OAuthConfig,
    user_id: Uuid,
    tenant_id: TenantId,
) -> bool {
    let grant_held = !matches!(
        tokens
            .get_token(user_id, tenant_id, oauth_providers::GARMIN)
            .await,
        Ok(None)
    );
    if grant_held {
        return false;
    }
    let order = connect_order(settings, oauth_providers::GARMIN).await;
    if order == ConnectOrder::SignInFirst {
        return false;
    }
    let seats_left = match garmin_seat_summary(grants, server).await {
        Ok(summary) => summary.left(),
        Err(e) => {
            debug!(error = %e, "Garmin seats could not be counted; usr/pwd stays the way in");
            0
        }
    };
    garmin_oauth_required(seats_left, grant_held, order)
}

/// `details.reason` of a Garmin OAuth start refused for want of a seat, and
/// the `error` an OAuth return carries for it.
pub const NO_SEAT_REASON: &str = "no_seat";

/// Refuse to start Garmin's OAuth for `user_id` in `tenant_id` when the
/// Developer Program client has no seat for them.
///
/// No seat means the pool is full (or cannot be counted, or no client is
/// configured) and they hold no Garmin grant in that tenant whose seat a
/// reconnect would re-sign.
///
/// The card recommends the usr/pwd connection once the pool is full, but it
/// is drawn before the connect starts, so two athletes shown the last seat
/// could both start the flow. Every surface that starts Garmin's OAuth (the
/// authorize routes, the hosted picker, the chat's minted authorization URL)
/// checks this, which holds the cap. Any other provider passes.
///
/// # Errors
///
/// A [`AppError::resource_unavailable`] refusal, with `details.reason`
/// [`NO_SEAT_REASON`] ([`is_no_seat_refusal`]), when no seat is left for this
/// user; the caller routes them to the usr/pwd connection.
pub async fn require_garmin_seat(
    tokens: &dyn OAuthTokenRepository,
    grants: &dyn ProviderConnectionRepository,
    server: &OAuthConfig,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> AppResult<()> {
    if provider != oauth_providers::GARMIN {
        return Ok(());
    }
    match tokens
        .get_token(user_id, tenant_id, oauth_providers::GARMIN)
        .await
    {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, "Garmin grant lookup failed; weighing the seat pool alone");
        }
    }
    let left = match garmin_seat_summary(grants, server).await {
        Ok(summary) => summary.left(),
        Err(e) => {
            debug!(error = %e, "Garmin seats could not be counted; no OAuth start");
            0
        }
    };
    if left > 0 {
        return Ok(());
    }
    let mut refusal = AppError::resource_unavailable(
        "Garmin OAuth refused: the Developer Program client has no seat left; \
         connect with the Garmin email and password instead",
    );
    refusal.details = Some(Box::new(json!({ "reason": NO_SEAT_REASON })));
    Err(refusal)
}

/// Whether `error` is [`require_garmin_seat`]'s refusal for want of a seat.
#[must_use]
pub fn is_no_seat_refusal(error: &AppError) -> bool {
    error
        .details
        .as_ref()
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str)
        == Some(NO_SEAT_REASON)
}

/// Whether `user_id` holds a usr/pwd Garmin connection of their own in
/// `tenant_id`: the session's token row, or a connection row they made
/// themselves. A delegated row is a coach's account, not theirs.
///
/// # Errors
///
/// The store's error when either lookup fails. Each caller maps it to the
/// answer that keeps the usr/pwd way in: the sign-in gate counts it as held
/// (nothing is refused), the reconnect counts it as not held (the ordinary
/// sign-in link is sent, not a move to the OAuth).
pub async fn holds_own_usr_pwd_garmin_connection(
    tokens: &dyn OAuthTokenRepository,
    grants: &dyn ProviderConnectionRepository,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<bool> {
    if tokens
        .get_token(user_id, tenant_id, oauth_providers::SCIOTTE_GARMIN)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    Ok(grants
        .get_for_user(user_id, Some(tenant_id))
        .await?
        .iter()
        .any(|c| {
            c.provider == oauth_providers::SCIOTTE_GARMIN
                && c.connection_type != ConnectionType::Delegated
        }))
}

/// Whether the athlete reconnects `backend` through Garmin's OAuth rather
/// than its usr/pwd sign-in.
///
/// True for a `sciotte_garmin` session of their own
/// ([`holds_own_usr_pwd_garmin_connection`]) while
/// [`garmin_oauth_required_for`] holds for them. Every reconnect surface (the
/// chat reply, the app push, the detached chat notice) reads this one rule.
///
/// A store that cannot be read answers `false`: the ordinary sign-in keeps
/// the way in the athlete has.
pub async fn reconnects_through_garmin_oauth(
    tokens: &dyn OAuthTokenRepository,
    grants: &dyn ProviderConnectionRepository,
    settings: &dyn SystemSettingsRepository,
    server: &OAuthConfig,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> bool {
    if backend != oauth_providers::SCIOTTE_GARMIN {
        return false;
    }
    match holds_own_usr_pwd_garmin_connection(tokens, grants, user_id, tenant_id).await {
        Ok(true) => {}
        Ok(false) => return false,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Garmin connection lookup failed; the reconnect is the ordinary sign-in");
            return false;
        }
    }
    garmin_oauth_required_for(tokens, grants, settings, server, user_id, tenant_id).await
}

#[cfg(test)]
mod tests {
    use super::{garmin_oauth_required, is_no_seat_refusal, GarminSeatSummary, NO_SEAT_REASON};
    use crate::connect_order::ConnectOrder;
    use pierre_core::errors::AppError;
    use serde_json::json;

    #[test]
    fn only_the_seat_refusal_reads_as_one() {
        let mut seat = AppError::resource_unavailable("no seat");
        seat.details = Some(Box::new(json!({ "reason": NO_SEAT_REASON })));
        assert!(is_no_seat_refusal(&seat), "bug: the seat refusal");
        let mut other = AppError::resource_unavailable("busy");
        other.details = Some(Box::new(json!({ "reason": "oauth_required" })));
        assert!(!is_no_seat_refusal(&other), "bug: another reason");
        assert!(
            !is_no_seat_refusal(&AppError::invalid_input("bad")),
            "bug: no details is no refusal"
        );
    }

    #[test]
    fn oauth_is_required_only_while_a_seat_is_free_for_an_athlete_with_no_grant() {
        assert!(
            garmin_oauth_required(3, false, ConnectOrder::OAuthFirst),
            "bug: a free seat moves a usr/pwd-only athlete to the OAuth"
        );
        assert!(
            !garmin_oauth_required(0, false, ConnectOrder::OAuthFirst),
            "bug: a full pool keeps the usr/pwd connection the way in"
        );
        assert!(
            !garmin_oauth_required(3, true, ConnectOrder::OAuthFirst),
            "bug: a grant holder is on the OAuth already"
        );
        assert!(
            !garmin_oauth_required(0, true, ConnectOrder::OAuthFirst),
            "bug: a grant holder is never asked to move"
        );
    }

    /// With the sign-in first (carnet#1472), no one is sent to the OAuth,
    /// whatever the seats.
    #[test]
    fn oauth_is_never_required_while_the_sign_in_goes_first() {
        for (seats, grant) in [(3, false), (0, false), (3, true)] {
            assert!(
                !garmin_oauth_required(seats, grant, ConnectOrder::SignInFirst),
                "bug: the sign-in-first order sent a connection ({seats} seats, grant {grant}) to the OAuth"
            );
        }
    }

    #[test]
    fn a_full_or_overfull_pool_has_no_seat_left() {
        let full = GarminSeatSummary {
            total: 1_000,
            used: 1_000,
        };
        assert_eq!(full.left(), 0, "bug: a full pool has no seat");
        let over = GarminSeatSummary {
            total: 10,
            used: 12,
        };
        assert_eq!(over.left(), 0, "bug: an overfull pool must not underflow");
        let open = GarminSeatSummary {
            total: 1_000,
            used: 999,
        };
        assert_eq!(open.left(), 1, "bug: the free seats");
    }
}
