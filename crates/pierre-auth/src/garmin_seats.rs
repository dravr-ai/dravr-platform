// ABOUTME: Seat accounting for Dravr's Garmin Developer Program client, capped per client id
// ABOUTME: Counts the users whose live Garmin grant the server-level client signs, and refuses an OAuth start with none left
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
//! A connect offers Garmin's OAuth only while a seat is free: under the
//! operator's OAuth-first order ([`crate::connect_order`]), and as the
//! fallback of a sign-in that fails while the sign-in goes first.

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::backends::{OAuthTokenRepository, ProviderConnectionRepository};
use serde_json::{json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::config::oauth::OAuthConfig;

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

#[cfg(test)]
mod tests {
    use super::{is_no_seat_refusal, GarminSeatSummary, NO_SEAT_REASON};
    use pierre_core::errors::AppError;
    use serde_json::json;

    #[test]
    fn only_the_seat_refusal_reads_as_one() {
        let mut seat = AppError::resource_unavailable("no seat");
        seat.details = Some(Box::new(json!({ "reason": NO_SEAT_REASON })));
        assert!(is_no_seat_refusal(&seat), "bug: the seat refusal");
        let mut other = AppError::resource_unavailable("busy");
        other.details = Some(Box::new(json!({ "reason": "busy" })));
        assert!(!is_no_seat_refusal(&other), "bug: another reason");
        assert!(
            !is_no_seat_refusal(&AppError::invalid_input("bad")),
            "bug: no details is no refusal"
        );
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
