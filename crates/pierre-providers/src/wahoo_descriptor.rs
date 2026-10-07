// ABOUTME: Wahoo provider descriptor — identity, capabilities, OAuth endpoints and the terms Wahoo's agreement sets
// ABOUTME: Kept beside the provider (as Terra's is) rather than in spi.rs, whose built-in descriptors fill its budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::oauth::{
    WAHOO_API_BASE_URL, WAHOO_AUTH_URL, WAHOO_DEAUTHORIZE_URL, WAHOO_DEFAULT_SCOPES,
    WAHOO_TOKEN_URL,
};

use crate::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
};

/// Wahoo provider descriptor
///
/// Wahoo's Cloud API (ELEMNT bike computers, KICKR trainers, RIVAL watches):
/// completed workouts with their summary and FIT file, the athlete's power
/// zones, and a training calendar Dravr writes structured workouts into
/// (carnet#34). Read the legal assessment before widening anything here:
/// dravr-vault `Work Log/2026-08/Wahoo API Agreement — Legal Read and Sync
/// Assessment (2026-08-17).md`, Part 8.
pub struct WahooDescriptor;

impl ProviderDescriptor for WahooDescriptor {
    fn name(&self) -> &'static str {
        "wahoo"
    }

    fn display_name(&self) -> &'static str {
        "Wahoo"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Completed workouts (detail is one more HTTP GET), and the calendar:
        // a pushed session is a plan in the athlete's Wahoo library plus a
        // workout scheduled on its day, which the ELEMNT app and devices
        // show from today through six days out.
        ProviderCapabilities::OAUTH
            .union(ProviderCapabilities::ACTIVITIES)
            .union(ProviderCapabilities::CHEAP_ACTIVITY_DETAIL)
            .union(ProviderCapabilities::CALENDAR_WRITE)
    }

    /// Wahoo has no RFC 7009 revocation endpoint: a disconnect `DELETE`s the
    /// athlete's permissions with their access token as `Bearer`.
    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        Some(OAuthEndpoints {
            auth_url: WAHOO_AUTH_URL,
            token_url: WAHOO_TOKEN_URL,
            revoke_url: Some(WAHOO_DEAUTHORIZE_URL),
        })
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        Some(OAuthParams {
            // `user_read workouts_read …`, as the Cloud API documents it.
            scope_separator: " ",
            // Wahoo supports S256 PKCE. The app stays confidential in the
            // portal, so the refresh grant authenticates with the client
            // secret and needs no code verifier.
            use_pkce: true,
            additional_auth_params: &[],
        })
    }

    /// Client credentials in the form body; every refresh returns a new
    /// refresh token, which the platform's compare-and-swap write stores.
    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        Some(OAuthRefresh::STANDARD)
    }

    /// Wahoo's token response carries no user id, and the workout webhook
    /// names the athlete by it: it is served at `user`.
    fn owner_id_from_api(&self) -> bool {
        true
    }

    /// Wahoo's only webhook event is `workout_summary`: an athlete revoking
    /// Dravr in their Wahoo settings surfaces as a refused refresh and
    /// nothing else, and the agreement requires their data deleted on
    /// revocation (carnet#34).
    fn refused_refresh_is_revocation(&self) -> bool {
        true
    }

    fn api_base_url(&self) -> &'static str {
        WAHOO_API_BASE_URL
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        WAHOO_DEFAULT_SCOPES
    }

    /// Restriction (iii) of the Wahoo API Agreement bars using the API "to
    /// aggregate, cache, or store geographic location information or other
    /// user information": per-athlete coaching reads Wahoo data, learning
    /// from it across athletes waits for a lawyer's reading (legal read, W7).
    fn bars_cross_athlete_learning(&self) -> bool {
        true
    }
}
