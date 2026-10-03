// ABOUTME: Comprehensive test suite for Garmin Connect provider implementation
// ABOUTME: Tests provider creation, configuration, OAuth flow, and data conversion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin Connect provider test suite.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-garmin")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use pierre_config::environment::HttpClientConfig;
use pierre_mcp_server::constants::{init_server_config, oauth_providers};
use pierre_mcp_server::utils::http_client::initialize_http_clients;
use pierre_providers::core::{CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::garmin_provider::GarminProvider;
use pierre_providers::registry::{get_supported_providers, global_registry};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once};
use tokio::net::TcpListener;
use url::form_urlencoded;

/// Ensure HTTP clients and server config are initialized only once across all tests
static INIT_HTTP_CLIENTS: Once = Once::new();
static INIT_SERVER_CONFIG: Once = Once::new();

fn ensure_http_clients_initialized() {
    // Initialize server config first (required for provider defaults)
    INIT_SERVER_CONFIG.call_once(|| {
        let _ = init_server_config();
    });

    INIT_HTTP_CLIENTS.call_once(|| {
        initialize_http_clients(HttpClientConfig::default());
    });
}

#[test]
fn test_garmin_provider_creation() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    assert_eq!(provider.name(), oauth_providers::GARMIN);
    assert_eq!(provider.config().name, oauth_providers::GARMIN);
    // Garmin's OAuth2 PKCE endpoints (Garmin Connect Developer Program
    // "OAuth2.0 PKCE Specification"), not the OAuth 1.0a ones.
    assert_eq!(
        provider.config().auth_url,
        "https://connect.garmin.com/oauth2Confirm"
    );
    assert_eq!(
        provider.config().token_url,
        "https://diauth.garmin.com/di-oauth2-service/oauth/token"
    );
    assert_eq!(
        provider.config().api_base_url,
        "https://apis.garmin.com/wellness-api/rest"
    );
}

#[test]
fn test_garmin_provider_with_custom_config() {
    ensure_http_clients_initialized();
    let custom_config = ProviderConfig {
        name: oauth_providers::GARMIN.to_owned(),
        auth_url: "https://custom.garmin.com/auth".to_owned(),
        token_url: "https://custom.garmin.com/token".to_owned(),
        api_base_url: "https://custom.garmin.com/api".to_owned(),
        revoke_url: Some("https://custom.garmin.com/revoke".to_owned()),
        default_scopes: vec!["custom:scope".to_owned()],
    };

    let provider = GarminProvider::with_config(custom_config.clone());

    assert_eq!(provider.config().name, custom_config.name);
    assert_eq!(provider.config().auth_url, custom_config.auth_url);
    assert_eq!(provider.config().token_url, custom_config.token_url);
    assert_eq!(provider.config().api_base_url, custom_config.api_base_url);
}

#[tokio::test]
async fn test_garmin_provider_authentication_lifecycle() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Initially not authenticated
    assert!(!provider.is_authenticated().await);

    // Set valid credentials
    let credentials = OAuth2Credentials {
        client_id: "test_client_id".to_owned(),
        client_secret: "test_client_secret".to_owned(),
        access_token: Some("test_access_token".to_owned()),
        refresh_token: Some("test_refresh_token".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        scopes: vec!["wellness:read".to_owned(), "activities:read".to_owned()],
        kind: CredentialKind::OAuthBearer,
    };

    provider
        .set_credentials(credentials)
        .await
        .expect("Failed to set credentials");

    // Now authenticated
    assert!(provider.is_authenticated().await);
}

#[tokio::test]
async fn test_garmin_provider_expired_token() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Set expired credentials
    let credentials = OAuth2Credentials {
        client_id: "test_client_id".to_owned(),
        client_secret: "test_client_secret".to_owned(),
        access_token: Some("expired_token".to_owned()),
        refresh_token: Some("test_refresh_token".to_owned()),
        expires_at: Some(Utc::now() - chrono::Duration::hours(1)), // Already expired
        scopes: vec!["wellness:read".to_owned()],
        kind: CredentialKind::OAuthBearer,
    };

    provider
        .set_credentials(credentials)
        .await
        .expect("Failed to set credentials");

    // Not authenticated due to expired token
    assert!(!provider.is_authenticated().await);
}

#[tokio::test]
async fn test_garmin_provider_no_expiry() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Credentials with no expiry time
    let credentials = OAuth2Credentials {
        client_id: "test_client_id".to_owned(),
        client_secret: "test_client_secret".to_owned(),
        access_token: Some("test_access_token".to_owned()),
        refresh_token: Some("test_refresh_token".to_owned()),
        expires_at: None, // No expiry
        scopes: vec!["wellness:read".to_owned()],
        kind: CredentialKind::OAuthBearer,
    };

    provider
        .set_credentials(credentials)
        .await
        .expect("Failed to set credentials");

    // Authenticated (no expiry means valid indefinitely)
    assert!(provider.is_authenticated().await);
}

// Note: sport type mapping is tested indirectly through activity conversion
// The parse_sport_type method is private and tested via integration tests

#[test]
fn test_garmin_provider_default() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::default();
    assert_eq!(provider.name(), oauth_providers::GARMIN);
}

#[test]
fn test_garmin_provider_scopes() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();
    // Garmin's scope is fixed server-side: the provider requests none.
    assert!(provider.config().default_scopes.is_empty());
}

#[test]
fn test_garmin_provider_endpoints() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();
    let config = provider.config();

    // Verify all required endpoints are configured
    assert!(config.auth_url.starts_with("https://"));
    assert!(config.token_url.starts_with("https://"));
    assert!(config.api_base_url.starts_with("https://"));
    assert!(config.revoke_url.is_some());
    assert!(config.revoke_url.as_ref().unwrap().starts_with("https://"));
}

#[tokio::test]
async fn test_garmin_provider_get_athlete_requires_auth() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Attempt to get athlete without authentication
    let result = provider.get_athlete().await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No credentials available"));
}

#[tokio::test]
async fn test_garmin_provider_get_activities_requires_auth() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Attempt to get activities without authentication
    let result = provider.get_activities(Some(10), None).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No credentials available"));
}

#[tokio::test]
async fn test_garmin_provider_get_activity_requires_auth() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Attempt to get specific activity without authentication
    let result = provider.get_activity("12345").await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No credentials available"));
}

#[tokio::test]
async fn test_garmin_provider_get_stats_requires_auth() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Attempt to get stats without authentication
    let result = provider.get_stats().await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No credentials available"));
}
#[tokio::test]
async fn test_garmin_provider_refresh_token_no_credentials() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Attempt to refresh without credentials
    let result = provider.refresh_token_if_needed().await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No credentials available"));
}

#[tokio::test]
async fn test_garmin_provider_refresh_token_not_needed() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Set credentials that don't need refresh (expires in 2 hours)
    let credentials = OAuth2Credentials {
        client_id: "test_client_id".to_owned(),
        client_secret: "test_client_secret".to_owned(),
        access_token: Some("test_access_token".to_owned()),
        refresh_token: Some("test_refresh_token".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::hours(2)),
        scopes: vec!["wellness:read".to_owned()],
        kind: CredentialKind::OAuthBearer,
    };

    provider
        .set_credentials(credentials)
        .await
        .expect("Failed to set credentials");

    // Refresh should succeed without actually refreshing
    let result = provider.refresh_token_if_needed().await;
    assert!(result.is_ok());
}

#[test]
fn test_garmin_in_provider_registry() {
    ensure_http_clients_initialized();
    let registry = global_registry();
    // Verify Garmin is in the list of all providers using the registry
    let all_providers = registry.supported_providers();
    assert!(all_providers.contains(&oauth_providers::GARMIN));
    assert!(registry.is_supported(oauth_providers::GARMIN));
}

#[test]
fn test_garmin_provider_factory() {
    ensure_http_clients_initialized();
    let registry = global_registry();

    // Verify Garmin is supported
    assert!(registry.is_supported(oauth_providers::GARMIN));

    // Verify we can create a Garmin provider
    let provider = registry
        .create_provider(oauth_providers::GARMIN)
        .expect("Failed to create Garmin provider");

    assert_eq!(provider.name(), oauth_providers::GARMIN);
}

#[test]
fn test_garmin_in_supported_providers_list() {
    let supported = get_supported_providers();
    assert!(supported.contains(&oauth_providers::GARMIN));
}

// Activity type conversion is tested through integration tests with real API responses

#[tokio::test]
async fn test_garmin_credentials_without_access_token() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();

    // Credentials without access token
    let credentials = OAuth2Credentials {
        client_id: "test_client_id".to_owned(),
        client_secret: "test_client_secret".to_owned(),
        access_token: None, // No access token
        refresh_token: Some("test_refresh_token".to_owned()),
        expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        scopes: vec!["wellness:read".to_owned()],
        kind: CredentialKind::OAuthBearer,
    };

    provider
        .set_credentials(credentials)
        .await
        .expect("Failed to set credentials");

    // Not authenticated without access token
    assert!(!provider.is_authenticated().await);
}

#[test]
fn test_garmin_provider_config_urls() {
    ensure_http_clients_initialized();
    let provider = GarminProvider::new();
    let config = provider.config();

    // Verify URLs don't have trailing slashes
    assert!(!config.api_base_url.ends_with('/'));
    assert!(!config.auth_url.ends_with('/'));
    assert!(!config.token_url.ends_with('/'));
}

/// A Garmin client whose stored access token is inside the refresh window
/// refreshes it before its next API call, the way every OAuth provider client
/// does: Garmin's refresh form (client in the body), the rotated pair handed
/// to the write-back callback, and the call made with the new access token.
/// The token and API endpoints are a local mock reached through the
/// provider's configuration.
#[tokio::test]
async fn test_garmin_api_call_refreshes_a_token_inside_the_window() {
    ensure_http_clients_initialized();
    let token_forms: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::default();
    let bearers: Arc<Mutex<Vec<String>>> = Arc::default();
    let forms = Arc::clone(&token_forms);
    let seen = Arc::clone(&bearers);
    let app = Router::new()
        .route(
            "/token",
            post(move |body: String| {
                let forms = Arc::clone(&forms);
                async move {
                    forms.lock().unwrap().push(
                        form_urlencoded::parse(body.as_bytes())
                            .into_owned()
                            .collect(),
                    );
                    Json(json!({
                        "access_token": "garmin_rotated_access",
                        "token_type": "bearer",
                        "refresh_token": "garmin_rotated_refresh",
                        "expires_in": 86_400
                    }))
                }
            }),
        )
        .route(
            "/user/id",
            get(move |headers: HeaderMap| {
                let seen = Arc::clone(&seen);
                async move {
                    seen.lock().unwrap().push(
                        headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_owned(),
                    );
                    Json(json!({ "userId": "garmin-user-1" }))
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let provider = GarminProvider::with_config(ProviderConfig {
        name: oauth_providers::GARMIN.to_owned(),
        auth_url: "https://connect.garmin.com/oauth2Confirm".to_owned(),
        token_url: format!("{base}/token"),
        api_base_url: base,
        revoke_url: None,
        default_scopes: Vec::new(),
    });
    let written_back: Arc<Mutex<Vec<OAuth2Credentials>>> = Arc::default();
    let sink = Arc::clone(&written_back);
    provider.set_token_refresh_callback(Arc::new(move |credentials| {
        let sink = Arc::clone(&sink);
        Box::pin(async move {
            sink.lock().unwrap().push(credentials);
        })
    }));
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "garmin_client".to_owned(),
            client_secret: "garmin_secret".to_owned(),
            access_token: Some("garmin_old_access".to_owned()),
            refresh_token: Some("garmin_old_refresh".to_owned()),
            expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
            scopes: Vec::new(),
            kind: CredentialKind::OAuthBearer,
        })
        .await
        .unwrap();

    let athlete = provider.get_athlete().await.unwrap();

    assert_eq!(athlete.id, "garmin-user-1");
    let forms = token_forms.lock().unwrap().clone();
    assert_eq!(forms.len(), 1, "one refresh before the call");
    let expected: HashMap<String, String> = [
        ("client_id", "garmin_client"),
        ("client_secret", "garmin_secret"),
        ("grant_type", "refresh_token"),
        ("refresh_token", "garmin_old_refresh"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(forms[0], expected);
    assert_eq!(
        *bearers.lock().unwrap(),
        vec!["Bearer garmin_rotated_access".to_owned()]
    );
    let written_back = written_back.lock().unwrap();
    assert_eq!(written_back.len(), 1, "the rotated pair is written back");
    assert_eq!(
        written_back[0].refresh_token.as_deref(),
        Some("garmin_rotated_refresh")
    );
}
