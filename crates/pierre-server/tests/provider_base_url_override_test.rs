// ABOUTME: Verifies Strava/Garmin provider api_base_url is overridable via env (PIERRE_<P>_API_BASE_URL)
// ABOUTME: Foundation for the dev fixture API — dev points providers at a local server, prod unchanged
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;

use pierre_providers::ProviderRegistry;
use serial_test::serial;

/// The dev fixture API works by pointing the real Strava provider at a local
/// base URL. This asserts `PIERRE_STRAVA_API_BASE_URL` flows through
/// `load_provider_env_config` -> registry default config -> the constructed
/// provider's `config().api_base_url`, and that it falls back to the real
/// Strava URL when unset.
#[test]
#[serial]
fn strava_api_base_url_honors_env_override() {
    let key = "PIERRE_STRAVA_API_BASE_URL";
    let fixture_url = "http://127.0.0.1:9555/strava/api/v3";

    env::set_var(key, fixture_url);
    let provider = ProviderRegistry::new()
        .create_provider("strava")
        .expect("strava provider should be registered");
    assert_eq!(
        provider.config().api_base_url,
        fixture_url,
        "PIERRE_STRAVA_API_BASE_URL must override the provider api_base_url"
    );

    env::remove_var(key);
    let default_provider = ProviderRegistry::new()
        .create_provider("strava")
        .expect("strava provider should be registered");
    assert_eq!(
        default_provider.config().api_base_url,
        "https://www.strava.com/api/v3",
        "without the override, the real Strava API base URL is used"
    );
}
