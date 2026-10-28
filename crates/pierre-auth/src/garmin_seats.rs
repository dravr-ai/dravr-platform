// ABOUTME: Seat accounting for Dravr's Garmin Developer Program client, capped per client id
// ABOUTME: Counts the users whose live Garmin grant the server-level client signs, across tenants
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

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::ProviderConnectionRepository;

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

#[cfg(test)]
mod tests {
    use super::GarminSeatSummary;

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
