// ABOUTME: Verifies provider endpoints are overridable through the server's config (PIERRE_<P>_API_BASE_URL, _SCOPES)
// ABOUTME: Foundation for the dev fixture API — dev points providers at a local server, prod unchanged
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_auth::config::oauth::parse_scopes;
use pierre_config::environment::{ProviderEndpointOverride, ProviderEndpointOverrides};
use pierre_providers::ProviderRegistry;

/// Overrides that move only Strava's endpoints.
fn strava_overrides(endpoints: ProviderEndpointOverride) -> ProviderEndpointOverrides {
    ProviderEndpointOverrides::from([("strava".to_owned(), endpoints)])
}

/// The dev fixture API works by pointing the real Strava provider at a local
/// base URL. This asserts an API base override (`PIERRE_STRAVA_API_BASE_URL`,
/// read into `ServerConfig.oauth.endpoints` at startup) flows through the
/// registry's default config to the constructed provider's
/// `config().api_base_url`, and that the real Strava URL stands without one.
#[test]
fn strava_api_base_url_honors_the_override() {
    let fixture_url = "http://127.0.0.1:9555/strava/api/v3";

    let provider = ProviderRegistry::new()
        .with_endpoint_overrides(&strava_overrides(ProviderEndpointOverride {
            api_base_url: Some(fixture_url.to_owned()),
            ..ProviderEndpointOverride::default()
        }))
        .create_provider("strava")
        .expect("strava provider should be registered");
    assert_eq!(
        provider.config().api_base_url,
        fixture_url,
        "the override must replace the provider api_base_url"
    );

    let default_provider = ProviderRegistry::new()
        .with_endpoint_overrides(&ProviderEndpointOverrides::new())
        .create_provider("strava")
        .expect("strava provider should be registered");
    assert_eq!(
        default_provider.config().api_base_url,
        "https://www.strava.com/api/v3",
        "without the override, the real Strava API base URL is used"
    );
}

/// An override moves only the fields it sets: the token URL keeps Strava's
/// own when only the API base moves, and another provider is left alone.
#[test]
fn an_override_replaces_only_the_fields_it_sets() {
    let registry = ProviderRegistry::new().with_endpoint_overrides(&strava_overrides(
        ProviderEndpointOverride {
            token_url: Some("http://127.0.0.1:9555/oauth/token".to_owned()),
            ..ProviderEndpointOverride::default()
        },
    ));

    let strava = registry
        .default_config("strava")
        .expect("strava is registered");
    assert_eq!(strava.token_url, "http://127.0.0.1:9555/oauth/token");
    assert_eq!(strava.api_base_url, "https://www.strava.com/api/v3");
    assert_eq!(
        strava.revoke_url.as_deref(),
        Some("https://www.strava.com/oauth/revoke")
    );
    let whoop = registry
        .default_config("whoop")
        .expect("whoop is registered");
    assert_eq!(
        whoop.token_url, "https://api.prod.whoop.com/oauth/oauth2/token",
        "an override names its provider"
    );
}

/// `PIERRE_<PROVIDER>_SCOPES` reads commas or whitespace, so an override in a
/// provider's own space-separated form (WHOOP's, Wahoo's) stays one scope per
/// entry rather than collapsing into a single scope named by the whole string;
/// the registry then asks for exactly those scopes.
#[test]
fn provider_scopes_override_splits_on_commas_or_spaces() {
    for value in [
        "read activity:read_all",
        "read,activity:read_all",
        " read ,  activity:read_all ",
    ] {
        let registry = ProviderRegistry::new().with_endpoint_overrides(&strava_overrides(
            ProviderEndpointOverride {
                scopes: Some(parse_scopes(value)),
                ..ProviderEndpointOverride::default()
            },
        ));
        let scopes = &registry
            .default_config("strava")
            .expect("strava provider should be registered")
            .default_scopes;
        assert_eq!(scopes, &["read", "activity:read_all"], "from {value:?}");
    }
}
