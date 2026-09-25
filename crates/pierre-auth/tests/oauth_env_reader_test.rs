// ABOUTME: Provider OAuth settings have one environment reader, shared by ServerConfig and runtime lookups
// ABOUTME: The PIERRE_ credential aliases, the unset redirect and the default scopes read the same both ways
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `ServerConfig.oauth` (built once by `OAuthConfig::from_env`) and the
//! runtime lookup `get_oauth_config` used to read the environment separately:
//! the first ignored the `PIERRE_<P>_CLIENT_ID` aliases the second accepted,
//! and baked a `BASE_URL` redirect default the second left unset. The tenant
//! OAuth manager resolved server-level credentials from the first while the
//! refresh path and the code exchange read the second.
//!
//! Every assertion lives in one test: the environment is process-global, and
//! this binary runs nothing else that reads it.

use std::env;

use pierre_auth::config::oauth::get_oauth_config;
use pierre_auth::config::OAuthConfig;

#[test]
fn server_config_and_runtime_lookups_read_provider_settings_alike() {
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
    ] {
        env::remove_var(key);
    }
    // A deployment that names the Strava app by its PIERRE_ aliases only.
    env::set_var("PIERRE_STRAVA_CLIENT_ID", "alias-client");
    env::set_var("PIERRE_STRAVA_CLIENT_SECRET", "alias-secret");
    env::set_var("BASE_URL", "https://app.example.test");

    let server = OAuthConfig::from_env();
    let runtime = get_oauth_config("strava");

    assert_eq!(server.strava.client_id.as_deref(), Some("alias-client"));
    assert_eq!(server.strava.client_secret.as_deref(), Some("alias-secret"));
    assert!(server.strava.enabled, "both credentials are set");
    assert_eq!(server.strava.client_id, runtime.client_id);
    assert_eq!(server.strava.client_secret, runtime.client_secret);

    // No redirect is configured: neither reader invents one, and the caller
    // builds the server's own callback URL.
    assert_eq!(server.strava.redirect_uri, None);
    assert_eq!(runtime.redirect_uri, None);

    // Default scopes are the provider's, whichever way they are read.
    assert_eq!(server.strava.scopes, vec!["activity:read_all"]);
    assert_eq!(server.strava.scopes, runtime.scopes);
    assert_eq!(server.garmin.scopes, vec!["wellness:read", "activities:read"]);
    assert_eq!(server.garmin.scopes, get_oauth_config("garmin").scopes);
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
    assert_eq!(server.whoop.scopes, get_oauth_config("whoop").scopes);

    // A provider with no credentials is disabled both ways.
    assert!(!server.garmin.enabled);
    assert!(!get_oauth_config("garmin").enabled);

    env::remove_var("PIERRE_STRAVA_CLIENT_ID");
    env::remove_var("PIERRE_STRAVA_CLIENT_SECRET");
    env::remove_var("BASE_URL");
}
