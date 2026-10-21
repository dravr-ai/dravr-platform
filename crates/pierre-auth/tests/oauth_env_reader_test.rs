// ABOUTME: Provider OAuth settings have one environment reader, OAuthConfig::from_env, run once at startup
// ABOUTME: The PIERRE_ aliases, the unset redirect, default scopes, the seat cap and endpoint overrides read there
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `ServerConfig.oauth` is built once by `OAuthConfig::from_env`, and every
//! runtime lookup of a provider's server-level app (`OAuthConfig::server_app`),
//! the Strava seat cap or a provider's endpoint override reads that value.
//! This binary pins what the one reader reads.
//!
//! Every assertion lives in one test: the environment is process-global, and
//! this binary runs nothing else that reads it.

use std::env;

use pierre_auth::config::{OAuthConfig, ProviderEndpointOverride};
use pierre_core::constants::provider_seats::STRAVA_OAUTH_SEAT_CAP_DEFAULT;
use secrecy::ExposeSecret;

#[test]
fn the_startup_reader_resolves_provider_settings() {
    for key in [
        "STRAVA_CLIENT_ID",
        "STRAVA_CLIENT_SECRET",
        "STRAVA_REDIRECT_URI",
        "PIERRE_STRAVA_SCOPES",
        "STRAVA_SCOPES",
        "GARMIN_CLIENT_ID",
        "GARMIN_CLIENT_SECRET",
        "PIERRE_GARMIN_CLIENT_ID",
        "PIERRE_GARMIN_CLIENT_SECRET",
        "PIERRE_GARMIN_SCOPES",
        "GARMIN_SCOPES",
        "PIERRE_WHOOP_SCOPES",
        "WHOOP_SCOPES",
        "STRAVA_OAUTH_SEAT_CAP",
        "PIERRE_WHOOP_TOKEN_URL",
        "PIERRE_WHOOP_API_BASE_URL",
        "PIERRE_WHOOP_AUTH_URL",
        "PIERRE_WHOOP_REVOKE_URL",
        "PIERRE_STRAVA_TOKEN_URL",
        "PIERRE_STRAVA_AUTH_URL",
        "PIERRE_STRAVA_API_BASE_URL",
        "PIERRE_STRAVA_REVOKE_URL",
    ] {
        env::remove_var(key);
    }
    // A deployment that names the Strava app by its PIERRE_ aliases only.
    env::set_var("PIERRE_STRAVA_CLIENT_ID", "alias-client");
    env::set_var("PIERRE_STRAVA_CLIENT_SECRET", "alias-secret");

    let server = OAuthConfig::from_env();

    assert_eq!(server.strava.client_id.as_deref(), Some("alias-client"));
    assert_eq!(
        server
            .strava
            .client_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("alias-secret")
    );
    assert!(server.strava.enabled, "both credentials are set");
    assert_eq!(
        server.server_app("strava").client_id.as_deref(),
        Some("alias-client"),
        "the runtime lookup answers what startup read"
    );

    // No redirect is configured: the reader invents none, and the caller
    // builds the server's own callback URL.
    assert_eq!(server.strava.redirect_uri, None);

    // Default scopes are the provider's.
    assert_eq!(server.strava.scopes, vec!["activity:read_all"]);
    // Garmin's scope is fixed server-side: none is requested by default.
    assert_eq!(server.garmin.scopes, [] as [String; 0]);
    assert_eq!(
        server.whoop.scopes,
        vec![
            "offline",
            "read:profile",
            "read:body_measurement",
            "read:workout",
            "read:sleep",
            "read:recovery",
            "read:cycles",
        ]
    );

    // A provider with no credentials is disabled, and so is one the server
    // carries no app for at all.
    assert!(!server.garmin.enabled);
    assert!(!server.server_app("garmin").enabled);
    assert!(!server.server_app("no-such-provider").enabled);
    assert_eq!(server.server_app("no-such-provider").client_id, None);

    // No seat cap and no endpoint override is configured.
    assert_eq!(server.strava_seat_cap, None);
    assert_eq!(server.strava_seat_cap(), STRAVA_OAUTH_SEAT_CAP_DEFAULT);
    assert!(
        !server.endpoints.contains_key("whoop"),
        "a provider that overrides nothing carries no entry"
    );

    // The scope override still reaches a provider that has no default; the
    // seat cap and the endpoint overrides are read at the same time.
    env::set_var("PIERRE_GARMIN_SCOPES", "CONNECT_READ");
    env::set_var("STRAVA_OAUTH_SEAT_CAP", "7");
    env::set_var("PIERRE_WHOOP_TOKEN_URL", "http://127.0.0.1:9/oauth/token");
    env::set_var("PIERRE_WHOOP_SCOPES", "offline read:workout");
    let overridden = OAuthConfig::from_env();
    assert_eq!(overridden.garmin.scopes, vec!["CONNECT_READ"]);
    assert_eq!(overridden.strava_seat_cap(), 7);
    assert_eq!(
        overridden.endpoints.get("whoop"),
        Some(&ProviderEndpointOverride {
            token_url: Some("http://127.0.0.1:9/oauth/token".to_owned()),
            scopes: Some(vec!["offline".to_owned(), "read:workout".to_owned()]),
            ..ProviderEndpointOverride::default()
        })
    );

    // A zero cap would wedge every athlete onto the scraper: it reads as the
    // default.
    env::set_var("STRAVA_OAUTH_SEAT_CAP", "0");
    assert_eq!(
        OAuthConfig::from_env().strava_seat_cap(),
        STRAVA_OAUTH_SEAT_CAP_DEFAULT
    );

    for key in [
        "PIERRE_GARMIN_SCOPES",
        "STRAVA_OAUTH_SEAT_CAP",
        "PIERRE_WHOOP_TOKEN_URL",
        "PIERRE_WHOOP_SCOPES",
        "PIERRE_STRAVA_CLIENT_ID",
        "PIERRE_STRAVA_CLIENT_SECRET",
    ] {
        env::remove_var(key);
    }
}
