// ABOUTME: Server-level HTTP client utilities extending pierre-core's shared clients
// ABOUTME: Adds OAuth and config-driven client initialization on top of core singletons
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::environment::HttpClientConfig;
use pierre_core::http_client::{
    api_client as core_api_client, initialize_api_client, SharedHttpClient,
};
use reqwest::{Client, ClientBuilder};
use std::sync::OnceLock;
use std::time::Duration;

/// Global HTTP client configuration
static CLIENT_CONFIG: OnceLock<HttpClientConfig> = OnceLock::new();

/// Get client configuration with fallback to defaults
///
/// Returns defaults if HTTP client configuration was not initialized at server startup
fn get_config() -> &'static HttpClientConfig {
    static DEFAULT_CONFIG: OnceLock<HttpClientConfig> = OnceLock::new();
    CLIENT_CONFIG
        .get()
        .unwrap_or_else(|| DEFAULT_CONFIG.get_or_init(HttpClientConfig::default))
}

/// Initialize HTTP client configuration
///
/// Must be called once at server startup before any HTTP clients are created.
/// This also initializes the shared API client in pierre-core used by all provider crates.
///
/// # Panics
/// Panics if called more than once (configuration cannot be changed after initialization)
pub fn initialize_http_clients(config: HttpClientConfig) {
    // Propagate API client timeouts to the shared singleton in pierre-core,
    // so pierre-providers and pierre-llm use the server's configured values.
    initialize_api_client(
        config.api_client_timeout_secs,
        config.api_client_connect_timeout_secs,
    );

    assert!(
        CLIENT_CONFIG.set(config).is_ok(),
        "HTTP client configuration already initialized"
    );
}

/// Get or create the shared HTTP client with configured timeout settings
///
/// Delegates to `pierre_core::http_client::api_client()` for the base singleton.
/// Prefer this over creating new clients for better performance.
///
/// # Returns
/// A reference to the shared `SharedHttpClient`
#[must_use]
pub fn shared_client() -> &'static SharedHttpClient {
    core_api_client()
}

/// Create a new HTTP client with custom timeout settings
///
/// Use this when you need specific timeout configurations
/// that differ from the shared client defaults.
///
/// # Arguments
/// * `timeout_secs` - Request timeout in seconds
/// * `connect_timeout_secs` - Connection timeout in seconds
///
/// # Returns
/// A new `reqwest::Client` with custom timeouts
///
/// # Errors
/// Returns a default client if custom client creation fails
#[must_use]
pub fn create_client_with_timeout(timeout_secs: u64, connect_timeout_secs: u64) -> Client {
    ClientBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(connect_timeout_secs))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Create a new HTTP client optimized for OAuth flows
///
/// This client has configured timeouts optimized for OAuth token exchanges.
/// Configuration must be initialized via `initialize_http_clients()` at server startup.
///
/// # Returns
/// A new `reqwest::Client` optimized for OAuth operations
///
/// # Panics
/// Panics if HTTP client configuration was not initialized at server startup
#[must_use]
pub fn oauth_client() -> Client {
    let config = get_config();

    create_client_with_timeout(
        config.oauth_client_timeout_secs,
        config.oauth_client_connect_timeout_secs,
    )
}

/// Create a new HTTP client optimized for API calls
///
/// This client has configured timeouts suitable for external API calls.
/// Configuration must be initialized via `initialize_http_clients()` at server startup.
///
/// # Returns
/// A new `reqwest::Client` optimized for API operations
///
/// # Panics
/// Panics if HTTP client configuration was not initialized at server startup
#[must_use]
pub fn api_client() -> Client {
    let config = get_config();

    create_client_with_timeout(
        config.api_client_timeout_secs,
        config.api_client_connect_timeout_secs,
    )
}
