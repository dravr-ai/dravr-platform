// ABOUTME: Server-level HTTP client utilities extending pierre-core's shared clients
// ABOUTME: Adds OAuth and config-driven client initialization on top of core singletons
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::environment::HttpClientConfig;
use pierre_auth::http::{initialize_oauth_http, OAuthHttpConfig};
use pierre_core::http_client::{
    api_client as core_api_client, initialize_api_client, SharedHttpClient,
};

/// Hand the configured HTTP client timeouts to the process-wide clients
///
/// Call once at server startup, before any HTTP client is created: the shared
/// API client in pierre-core, used by all provider crates, takes the API
/// timeouts, and the OAuth client in pierre-auth takes the
/// `OAUTH_CLIENT_TIMEOUT_SECS` / `OAUTH_CLIENT_CONNECT_TIMEOUT_SECS` ones.
/// The first call wins; a client asked for before any call is built with the
/// compiled defaults.
pub fn initialize_http_clients(config: &HttpClientConfig) {
    // Propagate API client timeouts to the shared singleton in pierre-core,
    // so pierre-providers and pierre-llm use the server's configured values.
    initialize_api_client(
        config.api_client_timeout_secs,
        config.api_client_connect_timeout_secs,
    );

    // OAuth token exchanges build their client in pierre-auth, which cannot
    // read this crate's config: hand it the configured timeouts.
    initialize_oauth_http(OAuthHttpConfig {
        timeout_secs: config.oauth_client_timeout_secs,
        connect_timeout_secs: config.oauth_client_connect_timeout_secs,
    });
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
