// ABOUTME: Pins pierre-providers' hand-written Debug impls that hide signing secrets and sessions
// ABOUTME: A future derive(Debug) on the Terra validator or the sciotte provider fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Each test builds a value with a distinctive sentinel in every hidden field,
//! formats it with `{:?}` and `{:#?}`, and asserts no sentinel comes out.

#[cfg(any(
    feature = "provider-terra",
    feature = "provider-sciotte",
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
use std::fmt::Debug;

#[cfg(any(
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
use pierre_providers::core::{CredentialKind, OAuth2Credentials, ProviderConfig};
#[cfg(any(
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
use secrecy::SecretString;

/// Formats `value` both ways, asserts none of `secrets` appears in either,
/// and returns the compact form for further checks.
#[cfg(any(
    feature = "provider-terra",
    feature = "provider-sciotte",
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
fn printed_without(value: &impl Debug, secrets: &[&str]) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    for secret in secrets {
        assert!(!compact.contains(secret), "{secret} leaked: {compact}");
        assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    }
    compact
}

#[cfg(feature = "provider-terra")]
#[test]
fn a_terra_signature_validator_never_prints_its_signing_secret() {
    use pierre_providers::terra::WebhookSignatureValidator;

    let validator = WebhookSignatureValidator::new("sentinel-terra-secret-7f3a".to_owned());
    let printed = printed_without(&validator, &["sentinel-terra-secret-7f3a"]);
    assert!(
        printed.contains("signing_secret: \"[REDACTED]\""),
        "{printed}"
    );
}

#[cfg(feature = "provider-sciotte")]
#[tokio::test]
async fn a_sciotte_provider_never_prints_the_session_it_holds() {
    use std::str::FromStr;

    use chrono::Utc;
    use dravr_sciotte::models::{AthleteId, AuthSession};
    use pierre_providers::core::{
        CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig,
    };
    use pierre_providers::sciotte_provider::SciotteProvider;
    use secrecy::SecretString;

    let config = ProviderConfig {
        name: "trainingpeaks".to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    };
    let provider = SciotteProvider::delegated(config, AthleteId::from_str("12345").unwrap(), None);
    let session = AuthSession {
        session_id: "sentinel-session-7f3a".to_owned(),
        cookies: vec![],
        created_at: Utc::now(),
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: SecretString::default(),
            access_token: Some(SecretString::from(serde_json::to_string(&session).unwrap())),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .unwrap();
    assert!(provider.is_authenticated().await, "the session is held");

    let printed = printed_without(&provider, &["sentinel-session-7f3a"]);
    assert!(printed.contains("trainingpeaks"), "{printed}");
}

/// The credentials an OAuth provider holds once connected, a sentinel in
/// every secret.
#[cfg(any(
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
fn sentinel_oauth_credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("sentinel-client-secret-7f3a"),
        access_token: Some(SecretString::from("sentinel-access-7f3a")),
        refresh_token: Some(SecretString::from("sentinel-refresh-7f3a")),
        expires_at: None,
        scopes: vec!["read".to_owned()],
        kind: CredentialKind::OAuthBearer,
        request_budget: None,
    }
}

/// Every secret [`sentinel_oauth_credentials`] holds.
#[cfg(any(
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
const OAUTH_SECRETS: [&str; 3] = [
    "sentinel-client-secret-7f3a",
    "sentinel-access-7f3a",
    "sentinel-refresh-7f3a",
];

/// A provider config with no endpoints, for a provider that never calls out.
#[cfg(any(
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
fn bare_config(name: &str) -> ProviderConfig {
    ProviderConfig {
        name: name.to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    }
}

#[cfg(feature = "provider-strava")]
#[tokio::test]
async fn a_connected_strava_provider_never_prints_its_tokens() {
    use pierre_providers::core::FitnessProvider;
    use pierre_providers::strava_provider::StravaProvider;

    let provider = StravaProvider::with_config(bare_config("strava"));
    provider
        .set_credentials(sentinel_oauth_credentials())
        .await
        .unwrap();
    let printed = printed_without(&provider, &OAUTH_SECRETS);
    assert!(printed.contains("client-1"), "{printed}");
}

#[cfg(feature = "provider-whoop")]
#[tokio::test]
async fn a_connected_whoop_provider_never_prints_its_tokens() {
    use pierre_providers::core::FitnessProvider;
    use pierre_providers::whoop_provider::WhoopProvider;

    let provider = WhoopProvider::with_config(bare_config("whoop"));
    provider
        .set_credentials(sentinel_oauth_credentials())
        .await
        .unwrap();
    let printed = printed_without(&provider, &OAUTH_SECRETS);
    assert!(printed.contains("client-1"), "{printed}");
}

#[cfg(feature = "provider-garmin")]
#[tokio::test]
async fn a_connected_garmin_provider_never_prints_its_tokens() {
    use pierre_providers::core::FitnessProvider;
    use pierre_providers::garmin_provider::GarminProvider;

    let provider = GarminProvider::with_config(bare_config("garmin"));
    provider
        .set_credentials(sentinel_oauth_credentials())
        .await
        .unwrap();
    let printed = printed_without(&provider, &OAUTH_SECRETS);
    assert!(printed.contains("client-1"), "{printed}");
}

#[cfg(feature = "provider-coros")]
#[tokio::test]
async fn a_connected_coros_provider_never_prints_its_tokens() {
    use pierre_providers::core::FitnessProvider;
    use pierre_providers::coros_provider::CorosProvider;

    let provider = CorosProvider::with_config(bare_config("coros"));
    provider
        .set_credentials(sentinel_oauth_credentials())
        .await
        .unwrap();
    let printed = printed_without(&provider, &OAUTH_SECRETS);
    assert!(printed.contains("client-1"), "{printed}");
}

#[cfg(feature = "provider-wahoo")]
#[tokio::test]
async fn a_connected_wahoo_provider_never_prints_its_tokens() {
    use pierre_providers::core::FitnessProvider;
    use pierre_providers::wahoo_provider::WahooProvider;

    let provider = WahooProvider::with_config(bare_config("wahoo"));
    provider
        .set_credentials(sentinel_oauth_credentials())
        .await
        .unwrap();
    let printed = printed_without(&provider, &OAUTH_SECRETS);
    assert!(printed.contains("client-1"), "{printed}");
}

#[cfg(any(
    feature = "provider-strava",
    feature = "provider-whoop",
    feature = "provider-garmin",
    feature = "provider-coros",
    feature = "provider-wahoo"
))]
#[test]
fn a_refresh_request_never_prints_its_client_secret_or_refresh_token() {
    use pierre_providers::utils::RefreshRequest;

    let request = RefreshRequest::form_fields(
        "strava",
        "https://www.strava.com/oauth/token",
        "client-1",
        "sentinel-client-secret-7f3a",
        "sentinel-refresh-token-7f3a",
    );
    let printed = printed_without(
        &request,
        &["sentinel-client-secret-7f3a", "sentinel-refresh-token-7f3a"],
    );
    assert!(printed.contains("RefreshRequest"), "{printed}");
    assert!(printed.contains("client-1"), "{printed}");
}
