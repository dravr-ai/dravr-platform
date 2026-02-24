// ABOUTME: HTTP client utilities for OAuth flows
// ABOUTME: Provides configured reqwest clients with appropriate timeouts for OAuth operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::timeouts;
use pierre_core::errors::{AppError, AppResult};
use reqwest::Client;
use std::sync::OnceLock;
use std::time::Duration;

/// OAuth HTTP client timeout configuration
#[derive(Debug, Clone)]
pub struct OAuthHttpConfig {
    /// Total request timeout in seconds
    pub timeout_secs: u64,
    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,
}

impl Default for OAuthHttpConfig {
    fn default() -> Self {
        Self {
            timeout_secs: timeouts::OAUTH_CLIENT_TIMEOUT_SECS,
            connect_timeout_secs: timeouts::OAUTH_CLIENT_CONNECT_TIMEOUT_SECS,
        }
    }
}

/// Global OAuth HTTP client configuration
static OAUTH_HTTP_CONFIG: OnceLock<OAuthHttpConfig> = OnceLock::new();

/// Initialize OAuth HTTP client configuration
///
/// Must be called once at server startup. Uses defaults if not called.
pub fn initialize_oauth_http(config: OAuthHttpConfig) {
    let _ = OAUTH_HTTP_CONFIG.set(config);
}

/// Create a new HTTP client optimized for OAuth flows
///
/// Uses configured timeouts from `initialize_oauth_http()`, or defaults
/// to the constants defined in pierre-core.
///
/// # Errors
///
/// Returns an error if the HTTP client cannot be built (e.g. TLS backend unavailable).
pub fn oauth_client() -> AppResult<Client> {
    let config = OAUTH_HTTP_CONFIG.get().cloned().unwrap_or_default();

    Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .build()
        .map_err(|e| AppError::internal(format!("Failed to build OAuth HTTP client: {e}")))
}
