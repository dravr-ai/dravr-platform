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
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// Called once at server startup by
/// `pierre_config::utils::http_client::initialize_http_clients` with the
/// configured `OAUTH_CLIENT_TIMEOUT_SECS` / `OAUTH_CLIENT_CONNECT_TIMEOUT_SECS`.
/// The first call wins; without one the compiled defaults apply.
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
    let config = effective_config();

    Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
        .build()
        .map_err(|e| AppError::internal(format!("Failed to build OAuth HTTP client: {e}")))
}

/// The timeouts an OAuth client is built with: the configured ones, or the
/// compiled defaults while [`initialize_oauth_http`] has not been called.
fn effective_config() -> OAuthHttpConfig {
    OAUTH_HTTP_CONFIG.get().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{effective_config, initialize_oauth_http, oauth_client, OAuthHttpConfig};

    /// The only test in this crate that initialises the process-wide
    /// configuration, so it reads the uninitialised state before it does.
    #[test]
    fn a_client_takes_the_first_configured_timeouts_and_the_defaults_before_any() {
        assert_eq!(
            effective_config(),
            OAuthHttpConfig::default(),
            "bug: a process nobody configured builds its client from the compiled defaults"
        );

        let configured = OAuthHttpConfig {
            timeout_secs: 45,
            connect_timeout_secs: 20,
        };
        initialize_oauth_http(configured.clone());
        assert_eq!(
            effective_config(),
            configured,
            "bug: the configured timeouts did not reach the OAuth client"
        );

        initialize_oauth_http(OAuthHttpConfig {
            timeout_secs: 1,
            connect_timeout_secs: 1,
        });
        assert_eq!(
            effective_config(),
            configured,
            "bug: a second initialisation replaced the first"
        );
        assert!(
            oauth_client().is_ok(),
            "bug: no client could be built from the configured timeouts"
        );
    }
}
