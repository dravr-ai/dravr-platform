// ABOUTME: A provider 401 must surface as ProviderAuthRequired, not a generic external error
// ABOUTME: registre#25 — a revoked-but-unexpired token was replayed forever with no re-auth prompt
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Regression cover for registre#25.
//!
//! Token refresh used to key solely off the stored `expires_at`, so a
//! credential the provider had already rejected kept being replayed: the
//! platform believed it was healthy until its recorded expiry, which
//! revocation never moves.

use pierre_providers::errors::ErrorCode;
use pierre_providers::utils::auth_error_for_status;
use reqwest::StatusCode;

/// The provider is the only authority on whether a credential still works.
/// `expires_at` is a belief, and revocation does not move it — so a 401 has to
/// be the signal that re-authentication is needed, independent of the expiry.
#[test]
fn unauthorized_maps_to_provider_auth_required() {
    for slug in ["strava", "fitbit", "whoop", "coros", "garmin"] {
        let err = auth_error_for_status(StatusCode::UNAUTHORIZED, slug)
            .unwrap_or_else(|| panic!("401 must produce an auth error for {slug}"));

        assert_eq!(
            err.code,
            ErrorCode::ProviderAuthRequired,
            "{slug}: a 401 must carry ProviderAuthRequired so the chat pipeline can mint a reconnect link"
        );
        assert_eq!(
            err.provider_auth_required_provider().as_deref(),
            Some(slug),
            "{slug}: the provider slug must survive in details"
        );
    }
}

/// Only 401 means "this credential is not accepted". Everything else keeps its
/// own handling — a 500 is the provider being unwell, not the athlete needing to
/// reconnect, and prompting for re-auth there would be a false alarm.
#[test]
fn other_statuses_are_not_auth_errors() {
    for status in [
        StatusCode::OK,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
    ] {
        assert!(
            auth_error_for_status(status, "strava").is_none(),
            "{status} must not be treated as a re-authentication signal"
        );
    }
}
