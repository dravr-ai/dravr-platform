// ABOUTME: Network configuration types for HTTP clients, the session cookie, CORS, and TLS
// ABOUTME: Handles timeouts, connection settings, and security transport options
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::timeouts;
use pierre_core::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

/// HTTP client timeout configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpClientConfig {
    /// OAuth client request timeout in seconds
    pub oauth_client_timeout_secs: u64,
    /// OAuth client connect timeout in seconds
    pub oauth_client_connect_timeout_secs: u64,
    /// API client request timeout in seconds
    pub api_client_timeout_secs: u64,
    /// API client connect timeout in seconds
    pub api_client_connect_timeout_secs: u64,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            oauth_client_timeout_secs: timeouts::OAUTH_CLIENT_TIMEOUT_SECS,
            oauth_client_connect_timeout_secs: timeouts::OAUTH_CLIENT_CONNECT_TIMEOUT_SECS,
            api_client_timeout_secs: timeouts::API_CLIENT_TIMEOUT_SECS,
            api_client_connect_timeout_secs: timeouts::API_CLIENT_CONNECT_TIMEOUT_SECS,
        }
    }
}

impl HttpClientConfig {
    /// Load HTTP client configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            oauth_client_timeout_secs: env::var("OAUTH_CLIENT_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(15),
            oauth_client_connect_timeout_secs: env::var("OAUTH_CLIENT_CONNECT_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5),
            api_client_timeout_secs: env::var("API_CLIENT_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60),
            api_client_connect_timeout_secs: env::var("API_CLIENT_CONNECT_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
        }
    }
}

/// The OAuth session cookie's settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCookieConfig {
    /// OAuth session cookie Max-Age in seconds
    pub max_age_secs: u64,
}

impl Default for SessionCookieConfig {
    fn default() -> Self {
        Self {
            max_age_secs: timeouts::SESSION_COOKIE_MAX_AGE_SECS,
        }
    }
}

impl SessionCookieConfig {
    /// Load the session cookie lifetime from the environment
    ///
    /// # Errors
    ///
    /// Returns an error if `SESSION_COOKIE_MAX_AGE_SECS` is not a number
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            max_age_secs: env_var_or(
                "SESSION_COOKIE_MAX_AGE_SECS",
                &timeouts::SESSION_COOKIE_MAX_AGE_SECS.to_string(),
            )
            .parse()
            .map_err(|e| {
                AppError::invalid_input(format!("Invalid SESSION_COOKIE_MAX_AGE_SECS value: {e}"))
            })?,
        })
    }
}

/// CORS (Cross-Origin Resource Sharing) configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CorsConfig {
    /// Comma-separated list of allowed origins
    pub allowed_origins: String,
    /// Allow localhost in development mode
    pub allow_localhost_dev: bool,
}

impl CorsConfig {
    /// Load CORS configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            allowed_origins: env::var("CORS_ALLOWED_ORIGINS").unwrap_or_default(),
            allow_localhost_dev: env::var("CORS_ALLOW_LOCALHOST_DEV")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(true),
        }
    }
}

/// TLS configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    /// Enable TLS
    pub enabled: bool,
    /// Path to TLS certificate
    pub cert_path: Option<PathBuf>,
    /// Path to TLS private key
    pub key_path: Option<PathBuf>,
}

impl TlsConfig {
    /// Load TLS configuration from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            enabled: env_var_or("TLS_ENABLED", "false").parse().unwrap_or(false),
            cert_path: env::var("TLS_CERT_PATH").ok().map(PathBuf::from),
            key_path: env::var("TLS_KEY_PATH").ok().map(PathBuf::from),
        }
    }
}

/// Parse comma-separated CORS origins
#[must_use]
pub fn parse_origins(origins_str: &str) -> Vec<String> {
    if origins_str == "*" {
        vec!["*".into()]
    } else {
        origins_str
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Get environment variable or default value
fn env_var_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}
